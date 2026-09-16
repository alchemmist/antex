use ratatui::style::Stylize;
use ratatui::text::Line;
use serde_json::Value;

use super::ChatWidget;
use crate::app_event::AppEvent;
use crate::bottom_pane::SelectionItem;
use crate::bottom_pane::SelectionViewParams;
use crate::bottom_pane::custom_prompt_view::CustomPromptView;
use crate::bottom_pane::popup_consts::standard_popup_hint_line;
use crate::status_indicator_widget::StatusDetailsCapitalization;
use crate::workflow::StoredWorkflowRun;
use crate::workflow::WorkflowDefinition;
use crate::workflow::WorkflowField;
use crate::workflow::WorkflowFieldKind;
use crate::workflow::WorkflowUpdate;

impl ChatWidget {
    pub(crate) fn show_workflow_picker(&mut self, definitions: Vec<WorkflowDefinition>) {
        let items = definitions
            .into_iter()
            .map(|definition| {
                let description = if definition.manifest.description.is_empty() {
                    format!("{} workflow", definition.source)
                } else {
                    format!(
                        "{} · {}",
                        definition.source, definition.manifest.description
                    )
                };
                SelectionItem {
                    name: definition.manifest.title.clone(),
                    description: Some(description),
                    actions: vec![Box::new(move |tx| {
                        tx.send(AppEvent::ConfigureWorkflow(Box::new(definition.clone())));
                    })],
                    dismiss_on_select: true,
                    ..Default::default()
                }
            })
            .collect();
        self.bottom_pane.show_selection_view(SelectionViewParams {
            title: Some("Python workflows".to_string()),
            subtitle: Some(
                "Project .codex/workflows overrides ~/.antex/workflows and built-ins".to_string(),
            ),
            footer_hint: Some(standard_popup_hint_line()),
            items,
            is_searchable: true,
            search_placeholder: Some("Search workflows".to_string()),
            ..Default::default()
        });
        self.request_redraw();
    }

    pub(crate) fn show_workflow_resume_picker(&mut self, runs: Vec<StoredWorkflowRun>) {
        let items = runs
            .into_iter()
            .map(|run| SelectionItem {
                name: run.manifest.title.clone(),
                description: Some(format!(
                    "{} · {:?} · {}",
                    run.run_id,
                    run.status,
                    run.updated_at.format("%Y-%m-%d %H:%M")
                )),
                actions: vec![Box::new(move |tx| {
                    tx.send(AppEvent::ResumeWorkflow(Box::new(run.clone())));
                })],
                dismiss_on_select: true,
                ..Default::default()
            })
            .collect();
        self.bottom_pane.show_selection_view(SelectionViewParams {
            title: Some("Resume workflow".to_string()),
            subtitle: Some("Resume from the latest persisted checkpoint".to_string()),
            footer_hint: Some(standard_popup_hint_line()),
            items,
            ..Default::default()
        });
        self.request_redraw();
    }

    pub(crate) fn show_workflow_field(
        &mut self,
        workflow_title: &str,
        field: &WorkflowField,
        index: usize,
        total: usize,
    ) {
        let title = format!(
            "{workflow_title} · {}/{} · {}",
            index + 1,
            total,
            field.label
        );
        if field.uses_model_picker() {
            self.show_workflow_model_picker(title, field);
            return;
        }
        match &field.kind {
            WorkflowFieldKind::Model => unreachable!("model fields use the model picker"),
            WorkflowFieldKind::Text { placeholder } => {
                self.show_workflow_text_field(
                    title,
                    if placeholder.is_empty() {
                        "Type a value and press Enter".to_string()
                    } else {
                        placeholder.clone()
                    },
                    field
                        .default
                        .as_ref()
                        .and_then(Value::as_str)
                        .unwrap_or_default()
                        .to_string(),
                    field.description.clone(),
                );
            }
            WorkflowFieldKind::Integer { .. } => {
                self.show_workflow_text_field(
                    title,
                    "Type an integer and press Enter".to_string(),
                    field
                        .default
                        .as_ref()
                        .and_then(Value::as_i64)
                        .map(|value| value.to_string())
                        .unwrap_or_default(),
                    field.description.clone(),
                );
            }
            WorkflowFieldKind::Boolean => {
                let default = field
                    .default
                    .as_ref()
                    .and_then(Value::as_bool)
                    .unwrap_or(false);
                let options = [(true, "Yes"), (false, "No")];
                let items = options
                    .into_iter()
                    .map(|(value, label)| SelectionItem {
                        name: label.to_string(),
                        actions: vec![Box::new(move |tx| {
                            tx.send(AppEvent::WorkflowFieldAnswered(value.to_string()));
                        })],
                        dismiss_on_select: true,
                        ..Default::default()
                    })
                    .collect();
                self.bottom_pane.show_selection_view(SelectionViewParams {
                    title: Some(title),
                    subtitle: nonempty(field.description.clone()),
                    footer_hint: Some(standard_popup_hint_line()),
                    items,
                    initial_selected_idx: Some(usize::from(!default)),
                    ..Default::default()
                });
            }
            WorkflowFieldKind::Select { options } => {
                let default = field.default.as_ref().and_then(Value::as_str);
                let initial_selected_idx = default
                    .and_then(|default| options.iter().position(|option| option.value == default));
                let items = options
                    .iter()
                    .cloned()
                    .map(|option| SelectionItem {
                        name: option.label,
                        description: nonempty(option.description),
                        actions: vec![Box::new(move |tx| {
                            tx.send(AppEvent::WorkflowFieldAnswered(option.value.clone()));
                        })],
                        dismiss_on_select: true,
                        ..Default::default()
                    })
                    .collect();
                self.bottom_pane.show_selection_view(SelectionViewParams {
                    title: Some(title),
                    subtitle: nonempty(field.description.clone()),
                    footer_hint: Some(standard_popup_hint_line()),
                    items,
                    initial_selected_idx,
                    ..Default::default()
                });
            }
        }
        self.request_redraw();
    }

    fn show_workflow_text_field(
        &mut self,
        title: String,
        placeholder: String,
        initial_text: String,
        description: String,
    ) {
        let tx = self.app_event_tx.clone();
        let view = CustomPromptView::new_allow_empty(
            title,
            placeholder,
            initial_text,
            nonempty(description),
            Box::new(move |answer| {
                tx.send(AppEvent::WorkflowFieldAnswered(answer));
            }),
        );
        self.bottom_pane.show_view(Box::new(view));
    }

    pub(crate) fn handle_workflow_update(&mut self, update: &WorkflowUpdate) {
        match update {
            WorkflowUpdate::Started { run_id, title } => {
                self.add_plain_history_lines(vec![Line::from(vec![
                    "● ".green(),
                    format!("Workflow started: {title}").bold(),
                    format!("  {run_id}").dim(),
                ])]);
                self.bottom_pane.set_task_running(/*running*/ true);
                self.set_status_header(format!("Workflow {title}: starting"));
            }
            WorkflowUpdate::Progress {
                message,
                current,
                total,
                ..
            } => {
                self.set_status(
                    "Workflow".to_string(),
                    Some(format_workflow_phase(Some(message), *current, *total)),
                    StatusDetailsCapitalization::Preserve,
                    /*details_max_lines*/ 2,
                );
            }
            WorkflowUpdate::AgentBatchStarted {
                count,
                parallelism,
                phase,
                phase_current,
                phase_total,
                ..
            } => {
                self.set_status(
                    format!("Workflow: starting {count} agents ({parallelism} parallel)"),
                    Some(format_workflow_phase(
                        phase.as_deref(),
                        *phase_current,
                        *phase_total,
                    )),
                    StatusDetailsCapitalization::Preserve,
                    /*details_max_lines*/ 2,
                );
            }
            WorkflowUpdate::AgentFinished {
                completed,
                total,
                success,
                phase,
                phase_current,
                phase_total,
                ..
            } => {
                let outcome = if *success { "done" } else { "failed" };
                self.set_status(
                    format!("Workflow: agents {completed}/{total} · {outcome}"),
                    Some(format_workflow_phase(
                        phase.as_deref(),
                        *phase_current,
                        *phase_total,
                    )),
                    StatusDetailsCapitalization::Preserve,
                    /*details_max_lines*/ 2,
                );
            }
            WorkflowUpdate::AgentActivity {
                agent,
                total,
                message,
                idle_seconds,
                phase,
                phase_current,
                phase_total,
                ..
            } => {
                let activity = if *idle_seconds == 0 {
                    message.clone()
                } else {
                    format!("{message} · {idle_seconds}s since new activity")
                };
                let phase = format_workflow_phase(phase.as_deref(), *phase_current, *phase_total);
                self.set_status(
                    format!("Workflow: agent {agent}/{total}"),
                    Some(format!("{activity}\n{phase}")),
                    StatusDetailsCapitalization::Preserve,
                    /*details_max_lines*/ 3,
                );
            }
            WorkflowUpdate::Checkpointed { .. } => {}
            WorkflowUpdate::Completed {
                title,
                result,
                agent_calls,
                shell_calls,
                ..
            } => {
                self.finish_workflow_status();
                self.add_plain_history_lines(vec![Line::from(vec![
                    "✓ ".green(),
                    format!("Workflow completed: {title}").bold(),
                    format!("  {agent_calls} agents · {shell_calls} shell calls").dim(),
                ])]);
                if !result.is_null() {
                    self.add_plain_history_lines(vec![format_workflow_result(result).dim().into()]);
                }
            }
            WorkflowUpdate::Paused { title, run_id } => {
                self.finish_workflow_status();
                self.add_plain_history_lines(vec![Line::from(vec![
                    "Ⅱ ".dim(),
                    format!("Workflow paused: {title}").bold(),
                    format!("  resume with /workflow resume · {run_id}").dim(),
                ])]);
            }
            WorkflowUpdate::Cancelled { title, run_id } => {
                self.finish_workflow_status();
                self.add_plain_history_lines(vec![Line::from(vec![
                    "■ ".dim(),
                    format!("Workflow cancelled: {title}").bold(),
                    format!("  checkpoint retained · {run_id}").dim(),
                ])]);
            }
            WorkflowUpdate::Failed {
                title,
                run_id,
                error,
            } => {
                self.finish_workflow_status();
                self.add_error_message(format!(
                    "Workflow failed: {title} ({run_id})\n{error}\nResume with /workflow resume."
                ));
            }
        }
        self.request_redraw();
    }

    fn finish_workflow_status(&mut self) {
        self.bottom_pane.set_task_running(/*running*/ false);
        self.maybe_send_next_queued_input();
    }
}

fn nonempty(value: String) -> Option<String> {
    (!value.is_empty()).then_some(value)
}

fn format_workflow_phase(
    message: Option<&str>,
    current: Option<u64>,
    total: Option<u64>,
) -> String {
    match (message, current, total) {
        (Some(message), Some(current), Some(total)) => format!("{message} [{current}/{total}]"),
        (Some(message), _, _) => message.to_string(),
        (None, _, _) => "working".to_string(),
    }
}

fn format_workflow_result(result: &Value) -> String {
    let result =
        serde_json::to_string(result).unwrap_or_else(|_| "<unprintable result>".to_string());
    let mut end = result.len().min(2_000);
    while !result.is_char_boundary(end) {
        end = end.saturating_sub(1);
    }
    if end < result.len() {
        format!("  Result: {}…", &result[..end])
    } else {
        format!("  Result: {result}")
    }
}
