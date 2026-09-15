use serde_json::Map;
use serde_json::Value;
use tokio::sync::mpsc;
use tokio::sync::watch;

use super::App;
use crate::app_event::AppEvent;
use crate::workflow::StoredWorkflowRun;
use crate::workflow::WorkflowControl;
use crate::workflow::WorkflowDefinition;
use crate::workflow::WorkflowRunRequest;
use crate::workflow::WorkflowUpdate;
use crate::workflow::create_run;
use crate::workflow::discover_workflows;
use crate::workflow::list_resumable_runs;
use crate::workflow::run_workflow;

#[derive(Default)]
pub(super) struct WorkflowAppState {
    configuration: Option<WorkflowConfiguration>,
    active: Option<ActiveWorkflow>,
}

struct WorkflowConfiguration {
    definition: WorkflowDefinition,
    field_index: usize,
    params: Map<String, Value>,
}

struct ActiveWorkflow {
    run_id: String,
    control: watch::Sender<WorkflowControl>,
}

impl App {
    pub(super) async fn open_workflow_picker(&mut self, workflow_id: Option<String>) {
        if self.workflow_state.active.is_some() {
            self.chat_widget.add_info_message(
                "A workflow is already running.".to_string(),
                Some("Use /workflow pause or /workflow stop.".to_string()),
            );
            return;
        }
        if self.chat_widget.has_running_task() {
            self.chat_widget.add_info_message(
                "Wait for the active Antex turn before starting a workflow.".to_string(),
                /*hint*/ None,
            );
            return;
        }
        let (definitions, diagnostics) =
            discover_workflows(self.config.codex_home.as_path(), self.config.cwd.as_path()).await;
        for diagnostic in diagnostics.into_iter().take(5) {
            self.chat_widget
                .add_error_message(format!("Workflow discovery: {diagnostic}"));
        }
        if let Some(workflow_id) = workflow_id {
            if let Some(definition) = definitions
                .into_iter()
                .find(|definition| definition.manifest.id == workflow_id)
            {
                self.configure_workflow(definition).await;
            } else {
                self.chat_widget
                    .add_error_message(format!("No Python workflow named `{workflow_id}`."));
            }
            return;
        }
        if definitions.is_empty() {
            self.chat_widget.add_info_message(
                "No Python workflows found.".to_string(),
                Some("Add a .py workflow to .codex/workflows or ~/.antex/workflows.".to_string()),
            );
            return;
        }
        self.chat_widget.show_workflow_picker(definitions);
    }

    pub(super) async fn configure_workflow(&mut self, definition: WorkflowDefinition) {
        if self.workflow_state.active.is_some() {
            self.chat_widget
                .add_error_message("A workflow is already running.".to_string());
            return;
        }
        if self.chat_widget.has_running_task() {
            self.chat_widget.add_info_message(
                "Wait for the active Antex turn before starting a workflow.".to_string(),
                /*hint*/ None,
            );
            return;
        }
        self.workflow_state.configuration = Some(WorkflowConfiguration {
            definition,
            field_index: 0,
            params: Map::new(),
        });
        self.show_or_start_configured_workflow().await;
    }

    pub(super) async fn answer_workflow_field(&mut self, answer: String) {
        let Some(configuration) = self.workflow_state.configuration.as_mut() else {
            self.chat_widget
                .add_error_message("No workflow is being configured.".to_string());
            return;
        };
        let Some(field) = configuration
            .definition
            .manifest
            .fields
            .get(configuration.field_index)
            .cloned()
        else {
            self.show_or_start_configured_workflow().await;
            return;
        };
        match field.parse_answer(&answer) {
            Ok(value) => {
                configuration.params.insert(field.id, value);
                configuration.field_index = configuration.field_index.saturating_add(1);
                self.show_or_start_configured_workflow().await;
            }
            Err(err) => {
                self.chat_widget.add_error_message(err);
                self.show_current_workflow_field();
            }
        }
    }

    async fn show_or_start_configured_workflow(&mut self) {
        let should_start =
            self.workflow_state
                .configuration
                .as_ref()
                .is_some_and(|configuration| {
                    configuration.field_index >= configuration.definition.manifest.fields.len()
                });
        if !should_start {
            self.show_current_workflow_field();
            return;
        }
        let Some(configuration) = self.workflow_state.configuration.take() else {
            return;
        };
        match create_run(
            self.config.codex_home.as_path(),
            configuration.definition,
            configuration.params,
        )
        .await
        {
            Ok(run) => self.launch_workflow(run),
            Err(err) => self
                .chat_widget
                .add_error_message(format!("Failed to create workflow run: {err}")),
        }
    }

    fn show_current_workflow_field(&mut self) {
        let Some(configuration) = self.workflow_state.configuration.as_ref() else {
            return;
        };
        let Some(field) = configuration
            .definition
            .manifest
            .fields
            .get(configuration.field_index)
        else {
            return;
        };
        self.chat_widget.show_workflow_field(
            &configuration.definition.manifest.title,
            field,
            configuration.field_index,
            configuration.definition.manifest.fields.len(),
        );
    }

    pub(super) async fn open_workflow_resume_picker(&mut self) {
        if self.workflow_state.active.is_some() {
            self.chat_widget
                .add_error_message("Pause or stop the active workflow first.".to_string());
            return;
        }
        if self.chat_widget.has_running_task() {
            self.chat_widget.add_info_message(
                "Wait for the active Antex turn before resuming a workflow.".to_string(),
                /*hint*/ None,
            );
            return;
        }
        match list_resumable_runs(self.config.codex_home.as_path()).await {
            Ok(runs) if runs.is_empty() => self.chat_widget.add_info_message(
                "No resumable workflow runs.".to_string(),
                /*hint*/ None,
            ),
            Ok(runs) => self.chat_widget.show_workflow_resume_picker(runs),
            Err(err) => self
                .chat_widget
                .add_error_message(format!("Failed to list workflow runs: {err}")),
        }
    }

    pub(super) fn resume_workflow(&mut self, run: StoredWorkflowRun) {
        if self.workflow_state.active.is_some() {
            self.chat_widget
                .add_error_message("A workflow is already running.".to_string());
            return;
        }
        self.launch_workflow(run);
    }

    fn launch_workflow(&mut self, run: StoredWorkflowRun) {
        let antex_exe = match std::env::current_exe() {
            Ok(path) => path,
            Err(err) => {
                self.chat_widget.add_error_message(format!(
                    "Cannot locate the Antex executable for workflow agents: {err}"
                ));
                return;
            }
        };
        let run_id = run.run_id.clone();
        let (control_tx, control_rx) = watch::channel(WorkflowControl::Run);
        let (update_tx, mut update_rx) = mpsc::unbounded_channel::<WorkflowUpdate>();
        let app_event_tx = self.app_event_tx.clone();
        tokio::spawn(async move {
            while let Some(update) = update_rx.recv().await {
                app_event_tx.send(AppEvent::WorkflowUpdate(update));
            }
        });
        let request = WorkflowRunRequest {
            run,
            workspace: self.config.cwd.to_path_buf(),
            antex_exe,
        };
        tokio::spawn(run_workflow(request, control_rx, update_tx));
        self.workflow_state.active = Some(ActiveWorkflow {
            run_id,
            control: control_tx,
        });
    }

    pub(super) fn control_workflow(&mut self, control: WorkflowControl) {
        let Some(active) = self.workflow_state.active.as_ref() else {
            self.chat_widget
                .add_info_message("No workflow is running.".to_string(), /*hint*/ None);
            return;
        };
        if active.control.send(control).is_err() {
            self.chat_widget
                .add_error_message("The workflow runtime has already stopped.".to_string());
        }
    }

    pub(super) fn apply_workflow_update(&mut self, update: WorkflowUpdate) {
        let Some(active) = self.workflow_state.active.as_ref() else {
            return;
        };
        if update.run_id() != active.run_id {
            return;
        }
        let terminal = update.is_terminal();
        self.chat_widget.handle_workflow_update(&update);
        if terminal {
            self.workflow_state.active = None;
        }
    }
}
