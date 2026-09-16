use super::agent_status_feed::AgentStatusHistoryCell;
use super::agent_status_feed::AgentStatusThreadPreview;
use super::session_lifecycle::LoadedSubagentBackfill;
use super::*;

impl App {
    pub(super) async fn reconcile_active_agent_liveness(
        &mut self,
        app_server: &mut AppServerSession,
    ) -> LoadedSubagentBackfill {
        let backfill = if self.primary_thread_id.is_none() {
            self.backfill_loaded_subagent_threads(app_server).await
        } else {
            LoadedSubagentBackfill::default()
        };
        let thread_ids = self
            .agent_navigation
            .ordered_path_backed_subagent_threads(self.primary_thread_id)
            .into_iter()
            .map(|(thread_id, _)| thread_id)
            .collect::<Vec<_>>();
        for thread_id in thread_ids {
            if let Some(channel) = self.thread_event_channels.get(&thread_id)
                && channel.attachment() == ThreadEventAttachment::Live
            {
                let Ok(store) = channel.store.try_lock() else {
                    continue;
                };
                let has_active_turn = store.active_turn_id().is_some();
                let has_terminal_snapshot = store
                    .turns
                    .last()
                    .is_some_and(|turn| !matches!(turn.status, TurnStatus::InProgress));
                drop(store);
                if has_active_turn {
                    self.agent_navigation.mark_running(thread_id);
                } else if has_terminal_snapshot {
                    self.agent_navigation.mark_stopped(thread_id);
                }
            } else if self.primary_thread_id.is_none()
                && !backfill.refreshed_thread_ids.contains(&thread_id)
            {
                self.refresh_agent_picker_thread_liveness(app_server, thread_id)
                    .await;
            }
        }
        self.sync_agent_status_ui();
        backfill
    }

    pub(super) fn append_active_agent_status(&mut self, _command: &'static str) {
        let entries = self
            .agent_navigation
            .ordered_path_backed_subagent_threads(self.primary_thread_id)
            .into_iter()
            .filter_map(|(thread_id, entry)| {
                if !entry.is_running || entry.is_closed {
                    return None;
                }
                Some((thread_id, entry.agent_path.as_deref()?.trim().to_string()))
            })
            .map(|(thread_id, agent_path)| {
                if let Some(channel) = self.thread_event_channels.get(&thread_id) {
                    match channel.store.try_lock() {
                        Ok(store) => AgentStatusThreadPreview::from_store(agent_path, &store),
                        Err(_) => AgentStatusThreadPreview::empty(agent_path),
                    }
                } else {
                    AgentStatusThreadPreview::empty(agent_path)
                }
            })
            .collect();
        self.chat_widget
            .add_to_history(AgentStatusHistoryCell::new(entries));
    }
}
