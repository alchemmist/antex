use super::*;

impl ChatWidget {
    pub(super) fn change_working_directory(&mut self, path: &str) {
        let path = path.trim();
        if path.is_empty() {
            self.add_error_message("Usage: /cd <path>".to_string());
            return;
        }
        if !self.is_session_configured() {
            self.add_error_message(
                "Cannot change the working directory before the session is ready.".to_string(),
            );
            return;
        }

        let path = strip_matching_path_quotes(path);
        if path.is_empty() {
            self.add_error_message("Working directory path cannot be empty.".to_string());
            return;
        }
        let cwd = self.config.cwd.join(path);
        self.app_event_tx.send(AppEvent::UpdateCwd(cwd.clone()));
        self.add_info_message(
            format!("Changing working directory to {}", cwd.display()),
            /*hint*/ None,
        );
    }
}

fn strip_matching_path_quotes(path: &str) -> &str {
    if path.len() < 2 {
        return path;
    }
    if let Some(path) = path
        .strip_prefix('"')
        .and_then(|path| path.strip_suffix('"'))
    {
        return path;
    }
    if let Some(path) = path
        .strip_prefix('\'')
        .and_then(|path| path.strip_suffix('\''))
    {
        return path;
    }
    path
}
