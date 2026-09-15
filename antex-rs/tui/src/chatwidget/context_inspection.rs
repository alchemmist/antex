use super::*;
use crate::app_event::ContextInspectionPurpose;
use antex_protocol::models::ContentItem;
use antex_protocol::models::ResponseItem;
use antex_protocol::protocol::ContextInspection;
use ratatui::style::Stylize;
use std::fs::OpenOptions;
use std::io::Write;
use std::path::Path;
use std::process::Command;

#[derive(Default)]
struct ContextCounts {
    user_messages: usize,
    assistant_messages: usize,
    developer_messages: usize,
    tool_calls: usize,
    tool_outputs: usize,
    reasoning_items: usize,
    compaction_items: usize,
    developer_labels: Vec<String>,
}

impl ChatWidget {
    pub(super) fn request_context_inspection(&mut self, purpose: ContextInspectionPurpose) {
        let Some(thread_id) = self.thread_id else {
            self.add_error_message("Context is unavailable before the session starts.".into());
            return;
        };
        self.app_event_tx
            .send(AppEvent::RequestContextInspection { thread_id, purpose });
    }

    pub(crate) fn handle_context_inspection(
        &mut self,
        purpose: ContextInspectionPurpose,
        result: Result<ContextInspection, String>,
    ) {
        let inspection = match result {
            Ok(inspection) => inspection,
            Err(error) => {
                self.add_error_message(format!("Failed to inspect context: {error}"));
                return;
            }
        };
        match purpose {
            ContextInspectionPurpose::Summary => self.add_context_summary(inspection),
            ContextInspectionPurpose::SystemPrompt => {
                self.open_system_prompt(inspection.latest_model_request)
            }
        }
    }

    fn add_context_summary(&mut self, inspection: ContextInspection) {
        self.add_to_history(history_cell::PlainHistoryCell::new(context_summary_lines(
            &inspection,
        )));
    }

    fn open_system_prompt(&mut self, request: Option<String>) {
        let Some(thread_id) = self.thread_id else {
            self.add_error_message(
                "System prompt is unavailable before the session starts.".into(),
            );
            return;
        };
        let Some(request) = request else {
            self.add_error_message(
                "No model request has been sent yet. Submit a prompt first.".into(),
            );
            return;
        };
        let document = match model_request_document(&request) {
            Ok(document) => document,
            Err(error) => {
                self.add_error_message(format!("Failed to format model request: {error}"));
                return;
            }
        };
        let short_id = thread_id.to_string().chars().take(8).collect::<String>();
        let path = self
            .config
            .codex_home
            .join("system-prompts")
            .join(format!("system-prompt-{short_id}.md"));
        match write_system_prompt(&path, &document)
            .and_then(|()| open_in_tmux_nvim(&path, &format!("sp-{short_id}")))
        {
            Ok(()) => self.add_info_message(
                format!("Opened latest model request in tmux: {}", path.display()),
                /*hint*/ None,
            ),
            Err(error) => self.add_error_message(format!("Failed to open system prompt: {error}")),
        }
    }
}

fn model_request_document(request: &str) -> Result<String, serde_json::Error> {
    let request: serde_json::Value = serde_json::from_str(request)?;
    let request = serde_json::to_string_pretty(&request)?;
    Ok(format!(
        "# Latest model request\n\nThis is the complete logical Responses API request from the latest model call. WebSocket transport may send it incrementally using `previous_response_id`.\n\n```json\n{request}\n```\n"
    ))
}

fn context_summary_lines(inspection: &ContextInspection) -> Vec<Line<'static>> {
    let counts = summarize_items(&inspection.items);
    let mut lines = vec![vec!["Context".bold()].into()];
    if let Some(token_info) = &inspection.token_info {
        let used = token_info.total_token_usage.total_tokens;
        let usage = token_info.model_context_window.map_or_else(
            || format!("{used} tokens"),
            |window| {
                let percent = if window > 0 {
                    used.saturating_mul(100) / window
                } else {
                    0
                };
                format!("{used} / {window} tokens ({percent}%)")
            },
        );
        lines.push(vec!["  Tokens: ".dim(), usage.into()].into());
    }
    lines.push(
        vec![
            "  System prompt: ".dim(),
            format!(
                "{} characters",
                inspection.base_instructions.text.chars().count()
            )
            .into(),
        ]
        .into(),
    );
    lines.push(
        vec![
            "  Conversation: ".dim(),
            format!(
                "{} user · {} assistant",
                counts.user_messages, counts.assistant_messages
            )
            .into(),
        ]
        .into(),
    );
    lines.push(
        vec![
            "  Developer context: ".dim(),
            counts.developer_messages.to_string().into(),
        ]
        .into(),
    );
    for label in &counts.developer_labels {
        lines.push(vec!["    - ".dim(), label.clone().into()].into());
    }
    lines.push(
        vec![
            "  Tools: ".dim(),
            format!(
                "{} calls · {} outputs",
                counts.tool_calls, counts.tool_outputs
            )
            .into(),
        ]
        .into(),
    );
    if counts.reasoning_items > 0 || counts.compaction_items > 0 {
        lines.push(
            vec![
                "  Internal: ".dim(),
                format!(
                    "{} reasoning · {} compaction",
                    counts.reasoning_items, counts.compaction_items
                )
                .into(),
            ]
            .into(),
        );
    }
    lines
}

fn summarize_items(items: &[ResponseItem]) -> ContextCounts {
    let mut counts = ContextCounts::default();
    for item in items {
        match item {
            ResponseItem::Message { role, content, .. } => match role.as_str() {
                "user" => counts.user_messages += 1,
                "assistant" => counts.assistant_messages += 1,
                "developer" | "system" => {
                    counts.developer_messages += 1;
                    if counts.developer_labels.len() < 5
                        && let Some(label) = message_label(content)
                    {
                        counts.developer_labels.push(label);
                    }
                }
                _ => {}
            },
            ResponseItem::AgentMessage { .. } => counts.assistant_messages += 1,
            ResponseItem::Reasoning { .. } => counts.reasoning_items += 1,
            ResponseItem::LocalShellCall { .. }
            | ResponseItem::FunctionCall { .. }
            | ResponseItem::ToolSearchCall { .. }
            | ResponseItem::CustomToolCall { .. }
            | ResponseItem::WebSearchCall { .. }
            | ResponseItem::ImageGenerationCall { .. } => counts.tool_calls += 1,
            ResponseItem::FunctionCallOutput { .. }
            | ResponseItem::CustomToolCallOutput { .. }
            | ResponseItem::ToolSearchOutput { .. } => counts.tool_outputs += 1,
            ResponseItem::Compaction { .. }
            | ResponseItem::CompactionTrigger { .. }
            | ResponseItem::ContextCompaction { .. } => counts.compaction_items += 1,
            ResponseItem::ConfigurationUpdate { .. }
            | ResponseItem::AdditionalTools { .. }
            | ResponseItem::Other => {}
        }
    }
    counts
}

fn message_label(content: &[ContentItem]) -> Option<String> {
    let text = content.iter().find_map(|item| match item {
        ContentItem::InputText { text } | ContentItem::OutputText { text } => Some(text.as_str()),
        _ => None,
    })?;
    let line = text.lines().find(|line| !line.trim().is_empty())?.trim();
    let mut label = line.chars().take(80).collect::<String>();
    if line.chars().count() > 80 {
        label.push('…');
    }
    Some(label)
}

fn write_system_prompt(path: &Path, prompt: &str) -> std::io::Result<()> {
    if let Some(parent) = path.parent() {
        std::fs::create_dir_all(parent)?;
    }
    let mut options = OpenOptions::new();
    options.create(true).truncate(true).write(true);
    #[cfg(unix)]
    {
        use std::os::unix::fs::OpenOptionsExt;
        options.mode(0o600);
    }
    let mut file = options.open(path)?;
    file.write_all(prompt.as_bytes())
}

fn open_in_tmux_nvim(path: &Path, window_name: &str) -> std::io::Result<()> {
    let pane = std::env::var("TMUX_PANE")
        .map_err(|_| std::io::Error::other("Antex is not running inside tmux"))?;
    if std::env::var_os("TMUX").is_none() {
        return Err(std::io::Error::other("Antex is not running inside tmux"));
    }
    let window = tmux_stdout(&["display-message", "-p", "-t", &pane, "#{window_id}"])?;
    let command = shlex::try_join(["nvim", &path.to_string_lossy()])
        .map_err(|error| std::io::Error::other(error.to_string()))?;
    let output = Command::new("tmux")
        .args(new_window_args(window.trim(), window_name, &command))
        .output()?;
    if output.status.success() {
        Ok(())
    } else {
        let stderr = String::from_utf8_lossy(&output.stderr);
        let detail = stderr.trim();
        Err(std::io::Error::other(if detail.is_empty() {
            format!("tmux exited with status {}", output.status)
        } else {
            detail.to_string()
        }))
    }
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

fn new_window_args<'a>(window: &'a str, window_name: &'a str, command: &'a str) -> [&'a str; 7] {
    ["new-window", "-a", "-t", window, "-n", window_name, command]
}

#[cfg(test)]
#[path = "context_inspection_tests.rs"]
mod tests;
