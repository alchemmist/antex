use super::*;

impl ChatWidget {
    pub(super) fn set_subagent_mode(&mut self, policy: SubagentSpawnPolicy) {
        self.subagents_enabled = policy == SubagentSpawnPolicy::Allow;
        if let Some(thread_id) = self.thread_id {
            if self.subagents_enabled {
                self.subagent_mode_threads.insert(thread_id);
            } else {
                self.subagent_mode_threads.remove(&thread_id);
            }
            self.app_event_tx
                .send(AppEvent::SetSubagentMode { thread_id, policy });
        }
        self.refresh_status_surfaces();
        let message = if self.subagents_enabled {
            "Subagents enabled."
        } else {
            "Subagents disabled."
        };
        let hint = None;
        self.add_info_message(message.to_string(), hint);
    }
}
