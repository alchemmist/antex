//! Exit output distinguishes a disconnected client from a stopped embedded server.
//! Persisted sessions offer a direct UUID command and an optional named picker hint.

use super::*;

/// A persisted thread that can be resumed after the TUI exits.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct ResumableThread {
    pub thread_id: ThreadId,
    pub thread_name: Option<String>,
}

/// Marks a task owned by an explicitly supplied external transport.
/// The foreground CLI cannot reconnect to or stop that transport's work.
#[derive(Debug, Clone)]
pub struct DisconnectInfo;

impl App {
    pub(super) fn exit_info(&self, exit_reason: ExitReason) -> AppExitInfo {
        let thread_id = match exit_reason {
            ExitReason::Archived(_) | ExitReason::ThreadRemoved => None,
            ExitReason::UserRequested | ExitReason::TurnInterrupted | ExitReason::Fatal(_) => {
                self.chat_widget.thread_id().or(self.primary_thread_id)
            }
        };
        let disconnect_info = thread_id.and(match &self.app_server_target {
            AppServerTarget::Embedded => None,
            AppServerTarget::Remote { .. } => Some(DisconnectInfo),
        });
        AppExitInfo {
            token_usage: self.token_usage(),
            thread_id,
            resume_hint: resumable_thread(
                thread_id,
                self.chat_widget.thread_name(),
                self.chat_widget.rollout_path().as_deref(),
            ),
            disconnect_info,
            update_action: self.pending_update_action,
            exit_reason,
        }
    }
}

impl AppExitInfo {
    /// Format the terminal summary for both the CLI and standalone TUI binaries.
    pub fn format_exit_messages(self, color_enabled: bool) -> Vec<String> {
        let color_command = |command: String| {
            if color_enabled {
                format!("\u{1b}[36m{command}\u{1b}[39m")
            } else {
                command
            }
        };
        let mut lines = Vec::new();
        if self.disconnect_info.is_some()
            && let Some(thread_id) = self.thread_id
        {
            let turn_interrupted = matches!(self.exit_reason, ExitReason::TurnInterrupted);
            let message = match self.exit_reason {
                ExitReason::UserRequested | ExitReason::Archived(_) | ExitReason::ThreadRemoved => {
                    "Disconnected from this task. Any running work continues."
                }
                ExitReason::Fatal(_) => "Disconnected from this task. Work may still be running.",
                ExitReason::TurnInterrupted => {
                    "Disconnected from this task. The current turn was stopped."
                }
            };
            lines.push(message.to_string());
            lines.push(format!(
                "To reconnect, use the client that opened thread {thread_id}."
            ));
            if !turn_interrupted {
                lines.push("Stop any remaining work from that client.".to_string());
            }
            if !self.token_usage.is_zero() {
                let usage = self.token_usage.to_string();
                lines.push(if turn_interrupted {
                    usage
                } else {
                    usage.replacen("Token usage:", "Token usage so far:", /*count*/ 1)
                });
            }
            return lines;
        }

        if !self.token_usage.is_zero() {
            lines.push(self.token_usage.to_string());
        }
        if let ExitReason::Archived(thread_id) = self.exit_reason {
            lines.push(format!("Session archived: {thread_id}"));
        } else if let Some(thread) = self.resume_hint {
            lines.push("To continue this session, run:".to_string());
            lines.push(format!(
                "  {}",
                color_command(format!("antex resume {}", thread.thread_id)),
            ));
            if let Some(thread_name) = thread.thread_name.filter(|name| !name.is_empty()) {
                lines.push(format!(
                    "Or run {} and select {}.",
                    color_command("antex resume".to_string()),
                    color_command(thread_name),
                ));
            }
        } else if let Some(thread_id) = self.thread_id {
            lines.push(format!("Session ID: {thread_id}"));
        }
        lines
    }
}
