use super::session::Session;
use super::session::SessionSettingsUpdate;
use super::step_context::StepContext;
use super::thread_settings;
use crate::function_tool::FunctionCallError;
use antex_protocol::protocol::TurnEnvironmentSelections;
use antex_utils_absolute_path::AbsolutePathBuf;

impl Session {
    #[expect(
        clippy::await_holding_invalid_type,
        reason = "persistent cwd and active environments must publish atomically with step capture"
    )]
    pub(crate) async fn set_working_directory(
        &self,
        step: &StepContext,
        path: &str,
    ) -> Result<String, FunctionCallError> {
        let fail = |message: String| FunctionCallError::RespondToModel(message);
        let cwd = AbsolutePathBuf::from_absolute_path_checked(path)
            .map_err(|error| fail(format!("expected an absolute working directory: {error}")))?;
        let environment = step
            .environments
            .single_local_environment()
            .ok_or_else(|| {
                fail("changing the working directory requires exactly one local environment".into())
            })?;
        let old_cwd = environment
            .cwd()
            .to_abs_path()
            .map_err(|error| fail(error.to_string()))?;
        let policy = environment
            .permission_profile_with_workspace_roots()
            .file_system_sandbox_policy();
        if !policy.can_read_local_path_with_cwd(cwd.as_path(), old_cwd.as_path()) {
            return Err(fail(
                "the destination is outside the current filesystem permissions".into(),
            ));
        }
        if policy.get_readable_roots_with_cwd(old_cwd.as_path())
            != policy.get_readable_roots_with_cwd(cwd.as_path())
            || policy.get_writable_roots_with_cwd(old_cwd.as_path())
                != policy.get_writable_roots_with_cwd(cwd.as_path())
            || policy.get_unreadable_roots_with_cwd(old_cwd.as_path())
                != policy.get_unreadable_roots_with_cwd(cwd.as_path())
            || policy.get_unreadable_globs_with_cwd(old_cwd.as_path())
                != policy.get_unreadable_globs_with_cwd(cwd.as_path())
        {
            return Err(fail("changing this directory would reinterpret filesystem permissions; configure absolute permission roots first".into()));
        }
        if !tokio::fs::metadata(cwd.as_path())
            .await
            .map_err(|error| fail(format!("cannot open working directory: {error}")))?
            .is_dir()
        {
            return Err(fail(
                "the working directory must be an existing directory".into(),
            ));
        }
        let guard = thread_settings::acquire_persistence_lock(self).await;
        // Serialize publication with step capture and live environment updates. Already captured
        // steps keep their immutable environment, without cloning or replacing the TurnContext.
        let active = self.active_turn.lock().await;
        if !active
            .as_ref()
            .and_then(|active| active.task.as_ref())
            .is_some_and(|task| {
                std::sync::Arc::ptr_eq(&task.turn_context, &step.turn)
                    && !task.cancellation_token.is_cancelled()
            })
        {
            return Err(fail("the originating turn is no longer active".into()));
        }
        let current = self.state.lock().await.session_configuration.clone();
        let mut selections = self.services.turn_environments.selections();
        if selections.len() != 1 || selections[0] != environment.selection() {
            return Err(fail(
                "the session directory changed; retry from the current session context".into(),
            ));
        }
        selections[0].cwd = cwd.clone().into();
        let commit = self
            .update_settings_if(
                SessionSettingsUpdate {
                    environments: Some(TurnEnvironmentSelections::new(
                        cwd.clone(),
                        selections.clone(),
                    )),
                    runtime_workspace_roots: Some(current.runtime_workspace_roots.clone()),
                    permission_profile: Some(current.permission_profile()),
                    active_permission_profile: current.active_permission_profile(),
                    profile_workspace_roots: Some(
                        current
                            .permission_profile_state
                            .profile_workspace_roots()
                            .to_vec(),
                    ),
                    ..Default::default()
                },
                |live, _| {
                    live.cwd() == current.cwd()
                        && live.permission_profile() == current.permission_profile()
                },
            )
            .await
            .map_err(|error| fail(error.to_string()))?
            .ok_or_else(|| {
                fail("session settings changed; retry from the current session context".into())
            })?;
        self.services
            .turn_environments
            .update_selections(&selections);
        drop(active);
        thread_settings::emit_applied(self, step.turn.sub_id.clone(), commit.snapshot).await;
        drop(guard);
        self.checkpoint_thread_settings().await.map_err(|error| {
            fail(format!(
                "working directory changed but persistence failed: {error}"
            ))
        })?;
        Ok(format!(
            "Session working directory changed to {}. End this tool batch or functions.exec cell now. Commands issued in the next model step use this directory; calls already issued in this step and existing processes retain their working directories.",
            cwd.display()
        ))
    }
}
