use super::*;
#[cfg(test)]
use crate::status::format_tokens_compact;

const IMPLEMENTATION_REQUESTS: &[&str] = &[
    "implement",
    "implement it",
    "implement the plan",
    "implement this plan",
    "start implementation",
    "start implementing",
    "begin implementation",
    "begin implementing",
    "реализуй",
    "реализуй план",
    "реализуй этот план",
    "начинай реализацию",
    "начни реализацию",
    "приступай к реализации",
];

impl ChatWidget {
    pub(super) fn maybe_begin_explicit_plan_implementation(&mut self, message: &UserMessage) {
        if self.turn_lifecycle.agent_turn_running
            || !self.collaboration_modes_enabled()
            || self.active_mode_kind() != ModeKind::Plan
            || self
                .transcript
                .latest_proposed_plan_markdown
                .as_deref()
                .is_none_or(str::is_empty)
            || !is_explicit_implementation_request(&message.text)
        {
            return;
        }

        let Some(default_mask) =
            collaboration_modes::default_mode_mask(self.model_catalog.as_ref())
        else {
            return;
        };
        self.begin_plan_implementation();
        self.set_collaboration_mask_from_user_action(default_mask);
    }

    pub(crate) fn begin_plan_implementation(&mut self) {
        self.bottom_pane.begin_task_plan();
    }

    // Keep the upstream explicit picker available without automatically interrupting
    // the fork's compact plan flow when a proposed plan finishes streaming.
    #[cfg(test)]
    pub(crate) fn open_plan_implementation_prompt(&mut self) {
        let default_mask = collaboration_modes::default_mode_mask(self.model_catalog.as_ref());
        let context_usage_label = self.plan_implementation_context_usage_label();
        self.bottom_pane
            .show_selection_view(plan_implementation::selection_view_params(
                default_mask,
                self.transcript.latest_proposed_plan_markdown.as_deref(),
                context_usage_label.as_deref(),
            ));
        self.notify(Notification::PlanModePrompt {
            title: plan_implementation::PLAN_IMPLEMENTATION_TITLE.to_string(),
        });
    }

    #[cfg(test)]
    pub(super) fn plan_implementation_context_usage_label(&self) -> Option<String> {
        let info = self.token_info.as_ref()?;
        let percent = self.context_remaining_percent(info);
        let used_tokens = self.context_used_tokens(info, percent.is_some());
        if let Some(percent) = percent {
            let used_percent = 100 - percent.clamp(0, 100);
            return (used_percent > 0).then(|| format!("{used_percent}% used"));
        }
        used_tokens
            .filter(|tokens| *tokens > 0)
            .map(|tokens| format!("{} used", format_tokens_compact(tokens)))
    }
}

fn is_explicit_implementation_request(text: &str) -> bool {
    let normalized = text
        .trim()
        .trim_end_matches(|character: char| character.is_ascii_punctuation() || character == '…')
        .trim()
        .to_lowercase();
    IMPLEMENTATION_REQUESTS.contains(&normalized.as_str())
}

#[cfg(test)]
#[path = "plan_follow_up_tests.rs"]
mod tests;
