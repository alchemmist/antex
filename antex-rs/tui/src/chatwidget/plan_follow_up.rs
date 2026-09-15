use super::*;

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
