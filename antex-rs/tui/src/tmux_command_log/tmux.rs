use std::fs::OpenOptions;
use std::os::unix::fs::OpenOptionsExt;
use std::path::Path;
use std::process::Command;

pub(super) struct TmuxViewer {
    pub(super) window_id: String,
    pub(super) pane_id: String,
}

pub(super) fn create_tmux_viewer(
    current_pane: &str,
    thread_id: &str,
    log_path: &Path,
    done_path: &Path,
) -> std::io::Result<TmuxViewer> {
    let session_id = tmux_stdout(&["display-message", "-p", "-t", current_pane, "#{session_id}"])?;
    let session_id = session_id.trim();
    let window_name = format!("cl-{}", thread_id.chars().take(8).collect::<String>());
    let viewer_command = viewer_shell_command(log_path, done_path, std::process::id());

    let mut last_error = None;
    for _ in 0..3 {
        let indexes = tmux_stdout(&["list-windows", "-t", session_id, "-F", "#{window_index}"])?;
        let target = format!("{session_id}:{}", next_window_index(&indexes));
        let output = Command::new("tmux")
            .args([
                "new-window",
                "-d",
                "-P",
                "-F",
                "#{window_id}\t#{pane_id}",
                "-t",
                &target,
                "-n",
                &window_name,
                &viewer_command,
            ])
            .output()?;
        if output.status.success() {
            let created = String::from_utf8_lossy(&output.stdout);
            let Some((window_id, pane_id)) = created.trim().split_once('\t') else {
                return Err(std::io::Error::other(
                    "tmux returned an invalid window identifier",
                ));
            };
            configure_tmux_viewer(window_id, pane_id, thread_id, &window_name)?;
            return Ok(TmuxViewer {
                window_id: window_id.to_string(),
                pane_id: pane_id.to_string(),
            });
        }
        last_error = Some(String::from_utf8_lossy(&output.stderr).trim().to_string());
    }

    Err(std::io::Error::other(last_error.unwrap_or_else(|| {
        "failed to create tmux window".to_string()
    })))
}

fn configure_tmux_viewer(
    window_id: &str,
    pane_id: &str,
    thread_id: &str,
    window_name: &str,
) -> std::io::Result<()> {
    tmux_status(&[
        "set-option",
        "-w",
        "-t",
        window_id,
        "automatic-rename",
        "off",
    ])?;
    tmux_status(&[
        "set-option",
        "-w",
        "-t",
        window_id,
        "@codex_log_session_id",
        thread_id,
    ])?;
    tmux_status(&["set-option", "-p", "-t", pane_id, "remain-on-exit", "on"])?;
    tmux_status(&["rename-window", "-t", window_id, window_name])
}

fn tmux_stdout(args: &[&str]) -> std::io::Result<String> {
    let output = Command::new("tmux").args(args).output()?;
    if output.status.success() {
        Ok(String::from_utf8_lossy(&output.stdout).to_string())
    } else {
        Err(std::io::Error::other(
            String::from_utf8_lossy(&output.stderr).trim().to_string(),
        ))
    }
}

pub(super) fn tmux_status(args: &[&str]) -> std::io::Result<()> {
    let output = Command::new("tmux").args(args).output()?;
    if output.status.success() {
        Ok(())
    } else {
        Err(std::io::Error::other(
            String::from_utf8_lossy(&output.stderr).trim().to_string(),
        ))
    }
}

pub(super) fn tmux_pane_exists(pane_id: &str) -> bool {
    tmux_stdout(&["display-message", "-p", "-t", pane_id, "#{pane_id}"])
        .is_ok_and(|found| found.trim() == pane_id)
}

pub(super) fn tmux_pane_width(pane_id: &str) -> Option<usize> {
    tmux_stdout(&["display-message", "-p", "-t", pane_id, "#{pane_width}"])
        .ok()?
        .trim()
        .parse::<usize>()
        .ok()
        .filter(|width| (1..=4_096).contains(width))
}

fn next_window_index(indexes: &str) -> u32 {
    indexes
        .lines()
        .filter_map(|index| index.parse::<u32>().ok())
        .max()
        .unwrap_or_default()
        .saturating_add(1)
}

fn viewer_shell_command(log_path: &Path, done_path: &Path, parent_pid: u32) -> String {
    let log_path = shell_quote(&log_path.to_string_lossy());
    let done_path = shell_quote(&done_path.to_string_lossy());
    format!(
        "log_path={log_path}; done_path={done_path}; parent_pid={parent_pid}; \
         tail -n +1 -f \"$log_path\" & tail_pid=$!; \
         while kill -0 \"$parent_pid\" 2>/dev/null && [ ! -e \"$done_path\" ]; do sleep 0.2; done; \
         sleep 0.2; kill \"$tail_pid\" 2>/dev/null; wait \"$tail_pid\" 2>/dev/null; \
         rm -f \"$log_path\" \"$done_path\""
    )
}

fn shell_quote(value: &str) -> String {
    format!("'{}'", value.replace('\'', "'\\''"))
}

pub(super) fn signal_viewer_done(done_path: &Path) -> std::io::Result<()> {
    OpenOptions::new()
        .create_new(true)
        .write(true)
        .mode(0o600)
        .open(done_path)
        .map(drop)
}
