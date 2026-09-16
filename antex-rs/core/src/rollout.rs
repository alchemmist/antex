use crate::config::Config;
pub use antex_rollout::ARCHIVED_SESSIONS_SUBDIR;
pub use antex_rollout::Cursor;
pub use antex_rollout::INTERACTIVE_SESSION_SOURCES;
pub use antex_rollout::RolloutRecorder;
pub use antex_rollout::RolloutRecorderParams;
pub use antex_rollout::SESSIONS_SUBDIR;
pub use antex_rollout::SessionMeta;
pub use antex_rollout::SortDirection;
pub use antex_rollout::ThreadItem;
pub use antex_rollout::ThreadSortKey;
pub use antex_rollout::ThreadsPage;
pub use antex_rollout::append_thread_name;
pub use antex_rollout::find_archived_thread_path_by_id_str;
#[deprecated(note = "use find_thread_path_by_id_str")]
pub use antex_rollout::find_conversation_path_by_id_str;
pub use antex_rollout::find_thread_meta_by_name_str;
pub use antex_rollout::find_thread_name_by_id;
pub use antex_rollout::find_thread_names_by_ids;
pub use antex_rollout::find_thread_path_by_id_str;
pub use antex_rollout::parse_cursor;
pub use antex_rollout::read_head_for_summary;
pub use antex_rollout::read_session_meta_line;
pub use antex_rollout::rollout_date_parts;

impl antex_rollout::RolloutConfigView for Config {
    fn codex_home(&self) -> &std::path::Path {
        self.codex_home.as_path()
    }

    fn sqlite_config(&self) -> &antex_state::SqliteConfig {
        self.sqlite_config()
    }

    fn cwd(&self) -> &std::path::Path {
        self.cwd.as_path()
    }

    fn model_provider_id(&self) -> &str {
        self.model_provider_id.as_str()
    }

    fn generate_memories(&self) -> bool {
        self.memories.generate_memories
    }
}

pub(crate) mod list {
    pub use antex_rollout::find_thread_path_by_id_str;
}

#[cfg(test)]
pub(crate) mod recorder {
    pub use antex_rollout::RolloutRecorder;
}

pub(crate) use crate::session_rollout_init_error::map_session_init_error;

pub(crate) mod truncation {
    pub(crate) use crate::thread_rollout_truncation::*;
}
