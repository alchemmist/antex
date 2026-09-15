//! Live shell-command mirroring into a dedicated tmux window.

use antex_app_server_protocol::CommandExecutionSource;
use antex_app_server_protocol::CommandExecutionStatus;
use antex_app_server_protocol::ThreadItem;
use antex_protocol::ThreadId;
use std::collections::HashMap;
use std::io::Write;
use std::sync::Arc;
use std::sync::atomic::AtomicBool;
use std::sync::atomic::Ordering;
use std::sync::mpsc::Receiver;
use std::sync::mpsc::SyncSender;
use std::sync::mpsc::TrySendError;
use std::sync::mpsc::sync_channel;
use std::thread::JoinHandle;
use std::time::Duration;

#[cfg(unix)]
use uuid::Uuid;

#[cfg(unix)]
mod tmux;
#[cfg(unix)]
use tmux::create_tmux_viewer;
#[cfg(unix)]
use tmux::signal_viewer_done;
#[cfg(unix)]
use tmux::tmux_pane_exists;
#[cfg(unix)]
use tmux::tmux_pane_width;
#[cfg(unix)]
use tmux::tmux_status;

const LOG_CHANNEL_CAPACITY: usize = 512;
const LOG_DROPPED_MARKER: &str = "\n[tmux command log dropped output while its writer was busy]\n";
const DEFAULT_SEPARATOR_WIDTH: usize = 80;
const GREEN_COMMAND_PROMPT: &str = "\x1b[32m$\x1b[39m";
const VIEWER_RESPONSE_TIMEOUT: Duration = Duration::from_secs(3);

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub(crate) enum TmuxViewerState {
    AlreadyOpen,
    Reopened,
}

pub(crate) struct TmuxCommandLog {
    thread_id: ThreadId,
    tx: Option<SyncSender<LogMessage>>,
    output_dropped: Arc<AtomicBool>,
    worker: Option<JoinHandle<()>>,
}

impl TmuxCommandLog {
    pub(crate) fn start(thread_id: ThreadId, cwd: &str) -> Option<Self> {
        let tmux_pane = std::env::var("TMUX_PANE").ok()?;
        std::env::var_os("TMUX")?;

        let (tx, rx) = std::sync::mpsc::sync_channel(LOG_CHANNEL_CAPACITY);
        let output_dropped = Arc::new(AtomicBool::new(false));
        let worker_output_dropped = Arc::clone(&output_dropped);
        let thread_id_text = thread_id.to_string();
        let cwd = cwd.to_string();
        let worker = std::thread::Builder::new()
            .name("codex-tmux-command-log".to_string())
            .spawn(move || {
                if let Err(err) = run_worker(
                    &thread_id_text,
                    &cwd,
                    &tmux_pane,
                    rx,
                    &worker_output_dropped,
                ) {
                    tracing::warn!(%err, "tmux command log stopped");
                }
            })
            .ok()?;

        Some(Self {
            thread_id,
            tx: Some(tx),
            output_dropped,
            worker: Some(worker),
        })
    }

    pub(crate) fn is_for(&self, thread_id: ThreadId) -> bool {
        self.thread_id == thread_id
    }

    pub(crate) fn ensure_viewer(&self) -> Result<TmuxViewerState, String> {
        let Some(tx) = self.tx.as_ref() else {
            return Err("tmux command log has stopped".to_string());
        };
        let (reply_tx, reply_rx) = sync_channel(1);
        tx.try_send(LogMessage::EnsureViewer { reply_tx })
            .map_err(|err| match err {
                TrySendError::Full(_) => "tmux command log is busy; try again".to_string(),
                TrySendError::Disconnected(_) => "tmux command log has stopped".to_string(),
            })?;
        reply_rx
            .recv_timeout(VIEWER_RESPONSE_TIMEOUT)
            .map_err(|_| "timed out while checking the tmux command log window".to_string())?
    }

    pub(crate) fn record_started(&self, item: &ThreadItem) {
        let ThreadItem::CommandExecution {
            id,
            command,
            cwd,
            source,
            ..
        } = item
        else {
            return;
        };
        if *source == CommandExecutionSource::UnifiedExecInteraction {
            return;
        }
        self.send(LogMessage::Started {
            call_id: id.clone(),
            command: command.clone(),
            cwd: cwd.to_string(),
        });
    }

    pub(crate) fn record_output(&self, call_id: &str, output: &str) {
        if output.is_empty() {
            return;
        }
        self.send(LogMessage::Output {
            call_id: call_id.to_string(),
            output: output.to_string(),
        });
    }

    pub(crate) fn record_stdin(&self, process_id: &str, stdin: &str) {
        if stdin.is_empty() {
            return;
        }
        self.send(LogMessage::Stdin {
            process_id: process_id.to_string(),
            stdin: stdin.to_string(),
        });
    }

    pub(crate) fn record_completed(&self, item: &ThreadItem) {
        let ThreadItem::CommandExecution {
            id,
            source,
            status,
            aggregated_output,
            exit_code,
            duration_ms,
            ..
        } = item
        else {
            return;
        };
        if *source == CommandExecutionSource::UnifiedExecInteraction {
            return;
        }
        self.send(LogMessage::Completed {
            call_id: id.clone(),
            status: status.clone(),
            aggregated_output: aggregated_output.clone(),
            exit_code: *exit_code,
            duration_ms: *duration_ms,
        });
    }

    fn send(&self, message: LogMessage) {
        let Some(tx) = self.tx.as_ref() else {
            return;
        };
        match tx.try_send(message) {
            Ok(()) | Err(TrySendError::Disconnected(_)) => {}
            Err(TrySendError::Full(_)) => {
                self.output_dropped.store(true, Ordering::Release);
            }
        }
    }
}

impl Drop for TmuxCommandLog {
    fn drop(&mut self) {
        if let Some(tx) = self.tx.take() {
            let _ = tx.send(LogMessage::Shutdown);
        }
        if let Some(worker) = self.worker.take() {
            let _ = worker.join();
        }
    }
}

#[derive(Debug)]
enum LogMessage {
    Started {
        call_id: String,
        command: String,
        cwd: String,
    },
    Output {
        call_id: String,
        output: String,
    },
    Stdin {
        process_id: String,
        stdin: String,
    },
    Completed {
        call_id: String,
        status: CommandExecutionStatus,
        aggregated_output: Option<String>,
        exit_code: Option<i32>,
        duration_ms: Option<i64>,
    },
    EnsureViewer {
        reply_tx: SyncSender<Result<TmuxViewerState, String>>,
    },
    Shutdown,
}

struct LogWriter<W> {
    writer: W,
    calls_with_output: HashMap<String, bool>,
    ends_with_newline: bool,
    separator_width: usize,
}

impl<W: Write> LogWriter<W> {
    fn new(mut writer: W, thread_id: &str, cwd: &str) -> std::io::Result<Self> {
        writeln!(writer, "Antex command log")?;
        writeln!(writer, "session: {thread_id}")?;
        writeln!(writer, "cwd: {cwd}")?;
        writer.flush()?;
        Ok(Self {
            writer,
            calls_with_output: HashMap::new(),
            ends_with_newline: true,
            separator_width: DEFAULT_SEPARATOR_WIDTH,
        })
    }

    fn set_separator_width(&mut self, separator_width: usize) {
        self.separator_width = separator_width.max(1);
    }

    fn write_dropped_marker_if_needed(
        &mut self,
        output_dropped: &AtomicBool,
    ) -> std::io::Result<()> {
        if output_dropped.swap(false, Ordering::AcqRel) {
            self.write_text(LOG_DROPPED_MARKER)?;
        }
        Ok(())
    }

    fn handle(&mut self, message: LogMessage) -> std::io::Result<bool> {
        match message {
            LogMessage::Started {
                call_id,
                command,
                cwd,
            } => {
                self.ensure_newline()?;
                write!(
                    self.writer,
                    "\n{GREEN_COMMAND_PROMPT} {command}\n  cwd: {cwd}\n\n"
                )?;
                self.ends_with_newline = true;
                self.calls_with_output.insert(call_id, false);
            }
            LogMessage::Output { call_id, output } => {
                self.write_text(&output)?;
                self.calls_with_output.insert(call_id, true);
            }
            LogMessage::Stdin { process_id, stdin } => {
                self.ensure_newline()?;
                write!(self.writer, "[stdin {process_id}] {stdin}")?;
                self.ends_with_newline = stdin.ends_with('\n');
                self.ensure_newline()?;
            }
            LogMessage::Completed {
                call_id,
                status,
                aggregated_output,
                exit_code,
                duration_ms,
            } => {
                let output_seen = self.calls_with_output.remove(&call_id).unwrap_or(false);
                if !output_seen
                    && let Some(output) = aggregated_output.as_deref()
                    && !output.is_empty()
                {
                    self.write_text(output)?;
                }
                self.ensure_newline()?;
                writeln!(
                    self.writer,
                    "{}",
                    completion_summary(status, exit_code, duration_ms)
                )?;
                writeln!(self.writer, "{}", "─".repeat(self.separator_width))?;
                self.ends_with_newline = true;
            }
            LogMessage::EnsureViewer { .. } => {}
            LogMessage::Shutdown => {
                self.ensure_newline()?;
                writeln!(self.writer, "\n[codex session ended]")?;
                self.writer.flush()?;
                return Ok(false);
            }
        }
        self.writer.flush()?;
        Ok(true)
    }

    fn write_text(&mut self, text: &str) -> std::io::Result<()> {
        self.writer.write_all(text.as_bytes())?;
        self.ends_with_newline = text.ends_with('\n');
        Ok(())
    }

    fn ensure_newline(&mut self) -> std::io::Result<()> {
        if !self.ends_with_newline {
            writeln!(self.writer)?;
            self.ends_with_newline = true;
        }
        Ok(())
    }
}

fn completion_summary(
    status: CommandExecutionStatus,
    exit_code: Option<i32>,
    duration_ms: Option<i64>,
) -> String {
    let outcome = match status {
        CommandExecutionStatus::Completed => {
            format!("✓ exit {}", exit_code.unwrap_or_default())
        }
        CommandExecutionStatus::Failed => format!("✗ exit {}", exit_code.unwrap_or(-1)),
        CommandExecutionStatus::Declined => "! declined".to_string(),
        CommandExecutionStatus::InProgress => "… still running".to_string(),
    };
    match duration_ms {
        Some(duration_ms) => format!("[{outcome} · {}]", format_duration(duration_ms)),
        None => format!("[{outcome}]"),
    }
}

fn format_duration(duration_ms: i64) -> String {
    if duration_ms < 1_000 {
        format!("{duration_ms} ms")
    } else {
        format!("{:.1} s", duration_ms as f64 / 1_000.0)
    }
}

#[cfg(unix)]
fn run_worker(
    thread_id: &str,
    cwd: &str,
    tmux_pane: &str,
    rx: Receiver<LogMessage>,
    output_dropped: &AtomicBool,
) -> std::io::Result<()> {
    use std::fs::OpenOptions;
    use std::os::unix::fs::OpenOptionsExt;

    let file_stem = format!("codex-tmux-log-{}", Uuid::new_v4());
    let log_path = std::env::temp_dir().join(format!("{file_stem}.log"));
    let done_path = std::env::temp_dir().join(format!("{file_stem}.done"));
    let file = OpenOptions::new()
        .create_new(true)
        .write(true)
        .mode(0o600)
        .open(&log_path)?;
    let mut writer = LogWriter::new(std::io::BufWriter::new(file), thread_id, cwd)?;
    let mut viewer = match create_tmux_viewer(tmux_pane, thread_id, &log_path, &done_path) {
        Ok(viewer) => viewer,
        Err(err) => {
            drop(writer);
            let _ = std::fs::remove_file(&log_path);
            return Err(err);
        }
    };

    let write_result = loop {
        let message = match rx.recv() {
            Ok(message) => message,
            Err(_) => LogMessage::Shutdown,
        };
        let message = match message {
            LogMessage::EnsureViewer { reply_tx } => {
                let result = if tmux_pane_exists(&viewer.pane_id) {
                    Ok(TmuxViewerState::AlreadyOpen)
                } else {
                    create_tmux_viewer(tmux_pane, thread_id, &log_path, &done_path)
                        .map(|reopened_viewer| {
                            viewer = reopened_viewer;
                            TmuxViewerState::Reopened
                        })
                        .map_err(|err| err.to_string())
                };
                let _ = reply_tx.send(result);
                continue;
            }
            message => message,
        };
        if matches!(&message, LogMessage::Completed { .. })
            && let Some(width) = tmux_pane_width(&viewer.pane_id)
        {
            writer.set_separator_width(width);
        }
        if let Err(err) = writer.write_dropped_marker_if_needed(output_dropped) {
            break Err(err);
        }
        match writer.handle(message) {
            Ok(true) => {}
            Ok(false) => break Ok(()),
            Err(err) => break Err(err),
        }
    };
    let flush_result = writer.writer.flush();
    drop(writer);
    let signal_result = signal_viewer_done(&done_path);
    if signal_result.is_err() {
        let _ = tmux_status(&["kill-window", "-t", &viewer.window_id]);
        let _ = std::fs::remove_file(&log_path);
        let _ = std::fs::remove_file(&done_path);
    } else if !tmux_pane_exists(&viewer.pane_id) {
        let _ = std::fs::remove_file(&log_path);
        let _ = std::fs::remove_file(&done_path);
    }
    tracing::debug!(
        window_id = %viewer.window_id,
        pane_id = %viewer.pane_id,
        "tmux command log finished"
    );
    write_result.and(flush_result).and(signal_result)
}

#[cfg(not(unix))]
fn run_worker(
    _thread_id: &str,
    _cwd: &str,
    _tmux_pane: &str,
    _rx: Receiver<LogMessage>,
    _output_dropped: &AtomicBool,
) -> std::io::Result<()> {
    Ok(())
}

#[cfg(test)]
#[path = "tmux_command_log_tests.rs"]
mod tests;
