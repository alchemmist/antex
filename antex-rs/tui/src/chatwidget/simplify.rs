use super::user_messages::UserMessageHistoryOverride;
use super::*;

const SIMPLIFY_PROMPT: &str = include_str!("../../assets/prompt_for_simplify_command.md");

impl ChatWidget {
    pub(super) fn submit_simplify(&mut self, focus: &str) {
        if !self.is_session_configured()
            || self.input_queue.rate_limit_recovery_pending
            || self.pending_image_submission.is_some()
        {
            self.add_error_message(
                "Wait for the session to be ready, then run /simplify again.".to_string(),
            );
            return;
        }
        let focus = focus.trim();
        let command = if focus.is_empty() {
            "/simplify".to_string()
        } else {
            format!("/simplify {focus}")
        };
        let prompt = if focus.is_empty() {
            SIMPLIFY_PROMPT.to_string()
        } else {
            format!("{SIMPLIFY_PROMPT}\nUser-provided scope or focus:\n{focus}")
        };
        self.submit_user_message_with_history_and_shell_escape_policy(
            prompt.into(),
            UserMessageHistoryRecord::Override(UserMessageHistoryOverride {
                text: command,
                text_elements: Vec::new(),
            }),
            ShellEscapePolicy::Disallow,
            SubagentSpawnPolicy::Allow,
            UserMessageSource::Prompt,
        );
    }
}
