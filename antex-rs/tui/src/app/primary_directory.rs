use super::App;
use antex_protocol::ThreadId;
use antex_utils_absolute_path::AbsolutePathBuf;

impl App {
    pub(super) fn synchronize_primary_directory(
        &mut self,
        thread_id: ThreadId,
        cwd: &AbsolutePathBuf,
    ) {
        if self.primary_thread_id != Some(thread_id)
            || crate::uses_remote_workspace_or_environment(
                &self.app_server_target,
                self.environment_manager.as_ref(),
            )
        {
            return;
        }
        if self.config.cwd != *cwd {
            self.config.cwd = cwd.clone();
            self.file_search.update_search_dir(cwd.to_path_buf());
        }
        if let Err(error) = crate::tmux_session::synchronize_directory(
            thread_id,
            self.config.codex_home.as_path(),
            cwd.as_path(),
        ) {
            tracing::warn!(%error, "failed to synchronize primary session directory");
        }
    }
}
