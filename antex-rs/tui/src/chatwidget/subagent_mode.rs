use super::*;

impl ChatWidget {
    fn subagent_mode_path(&self, thread_id: ThreadId) -> std::path::PathBuf {
        self.local_settings
            .codex_home
            .join("subagent-modes")
            .join(format!("{thread_id}.enabled"))
            .into_path_buf()
    }

    pub(super) fn restore_subagent_mode(&mut self, thread_id: ThreadId) {
        if self.thread_id.is_none() && self.subagents_enabled {
            if let Err(err) = self.persist_subagent_mode(thread_id) {
                self.add_error_message(format!("Failed to save subagent mode: {err}"));
            }
        } else {
            self.subagents_enabled =
                match std::fs::read_to_string(self.subagent_mode_path(thread_id)) {
                    Ok(value) => value == "enabled",
                    Err(err) if err.kind() == std::io::ErrorKind::NotFound => false,
                    Err(err) => {
                        self.add_error_message(format!("Failed to restore subagent mode: {err}"));
                        self.subagent_mode_threads.contains(&thread_id)
                    }
                };
        }
        if self.subagents_enabled {
            self.subagent_mode_threads.insert(thread_id);
        } else {
            self.subagent_mode_threads.remove(&thread_id);
        }
    }

    fn persist_subagent_mode(&self, thread_id: ThreadId) -> std::io::Result<()> {
        let path = self.subagent_mode_path(thread_id);
        if self.subagents_enabled {
            antex_utils_path::write_atomically(&path, "enabled")
        } else {
            match std::fs::remove_file(path) {
                Ok(()) => Ok(()),
                Err(err) if err.kind() == std::io::ErrorKind::NotFound => Ok(()),
                Err(err) => Err(err),
            }
        }
    }

    pub(super) fn set_subagent_mode(&mut self, policy: SubagentSpawnPolicy) {
        self.subagents_enabled = policy == SubagentSpawnPolicy::Allow;
        if let Some(thread_id) = self.thread_id {
            if self.subagents_enabled {
                self.subagent_mode_threads.insert(thread_id);
            } else {
                self.subagent_mode_threads.remove(&thread_id);
            }
            if let Err(err) = self.persist_subagent_mode(thread_id) {
                self.add_error_message(format!("Failed to save subagent mode: {err}"));
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
