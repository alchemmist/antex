use super::session::Session;
use super::session::SessionSettingsUpdate;
use super::step_context::StepContext;
use super::thread_settings;
use super::turn_context::TurnContext;
use crate::environment_selection::TurnEnvironmentSnapshot;
use crate::function_tool::FunctionCallError;
use antex_protocol::protocol::TurnEnvironmentSelections;
use antex_utils_absolute_path::AbsolutePathBuf;
use arc_swap::ArcSwap;
use std::sync::Arc;
use std::sync::atomic::AtomicBool;
use std::sync::atomic::Ordering;

struct WorkingDirectoryUpdate {
    cwd: AbsolutePathBuf,
    environments: TurnEnvironmentSnapshot,
}

impl Session {
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
        let current = self.state.lock().await.session_configuration.clone();
        let mut selections = self.services.turn_environments.selections();
        if selections.len() != 1
            || selections[0].cwd != *environment.cwd()
            || selections[0].environment_id != environment.selection.environment_id
        {
            return Err(fail(
                "the session directory changed; retry from the current session context".into(),
            ));
        }
        selections[0].cwd = cwd.clone().into();
        let commit = self
            .update_settings_if(
                SessionSettingsUpdate {
                    environments: Some(TurnEnvironmentSelections::new(cwd.clone(), selections)),
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
        let environments = self.services.turn_environments.snapshot().await;
        step.turn.extension_data.insert(WorkingDirectoryUpdate {
            cwd: cwd.clone(),
            environments,
        });
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

impl TurnContext {
    pub(super) fn with_working_directory_update(self: Arc<Self>) -> Arc<Self> {
        let Some(update) = self.extension_data.get::<WorkingDirectoryUpdate>() else {
            return self;
        };
        let mut config = self.config.as_ref().clone();
        config.cwd = update.cwd.clone();
        Arc::new(Self {
            sub_id: self.sub_id.clone(),
            trace_id: self.trace_id.clone(),
            realtime_active: self.realtime_active,
            code_mode_available: self.code_mode_available,
            config: Arc::new(config),
            configured_token_budget: self.configured_token_budget.clone(),
            use_model_token_budget_defaults: self.use_model_token_budget_defaults,
            auth_manager: self.auth_manager.clone(),
            initial_settings: Arc::clone(&self.initial_settings),
            disabled_plugin_ids: self.disabled_plugin_ids.clone(),
            current_settings: ArcSwap::from(self.current_settings.load_full()),
            session_telemetry: self.session_telemetry.clone(),
            provider: self.provider.clone(),
            session_source: self.session_source.clone(),
            history_mode: self.history_mode,
            parent_thread_id: self.parent_thread_id,
            originator: self.originator.clone(),
            environments: update.environments.clone(),
            #[allow(deprecated)]
            cwd: update.cwd.clone(),
            current_date: self.current_date.clone(),
            timezone: self.timezone.clone(),
            app_server_client_name: self.app_server_client_name.clone(),
            developer_instructions: self.developer_instructions.clone(),
            multi_agent_version: self.multi_agent_version,
            subagent_spawn_policy: self.subagent_spawn_policy,
            network: self.network.clone(),
            windows_sandbox_level: self.windows_sandbox_level,
            available_models: self.available_models.clone(),
            unified_exec_shell_mode: self.unified_exec_shell_mode.clone(),
            final_output_json_schema: self.final_output_json_schema.clone(),
            dynamic_tools: self.dynamic_tools.clone(),
            turn_metadata_state: Arc::clone(&self.turn_metadata_state),
            extension_data: Arc::clone(&self.extension_data),
            turn_timing_state: Arc::clone(&self.turn_timing_state),
            terminal_error: Arc::clone(&self.terminal_error),
            server_model_warning_emitted: AtomicBool::new(
                self.server_model_warning_emitted.load(Ordering::Relaxed),
            ),
            model_verification_emitted: AtomicBool::new(
                self.model_verification_emitted.load(Ordering::Relaxed),
            ),
            cyber_access_program: self.cyber_access_program,
        })
    }
}
