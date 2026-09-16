use super::ChatWidget;
use crate::app_event::AppEvent;
use crate::bottom_pane::SelectionItem;
use crate::bottom_pane::SelectionViewParams;
use crate::bottom_pane::popup_consts::standard_popup_hint_line;
use crate::workflow::WorkflowField;
use serde_json::Value;

impl ChatWidget {
    pub(super) fn show_workflow_model_picker(&mut self, title: String, field: &WorkflowField) {
        let default = field
            .default
            .as_ref()
            .and_then(Value::as_str)
            .unwrap_or_default();
        let mut choices = Vec::new();
        if !field.required {
            choices.push((
                String::new(),
                "Use current Antex model".to_string(),
                self.current_model().to_string(),
            ));
        }
        let presets = self.model_catalog.try_list_models().unwrap_or_default();
        for preset in presets.into_iter().filter(|preset| preset.show_in_picker) {
            if choices.iter().any(|(value, _, _)| value == &preset.model) {
                continue;
            }
            choices.push((preset.model.clone(), preset.model, preset.description));
        }
        if !default.is_empty() && !choices.iter().any(|(value, _, _)| value == default) {
            choices.push((
                default.to_string(),
                default.to_string(),
                "Workflow default (not in current catalog)".to_string(),
            ));
        }
        if choices.is_empty() {
            let model = self.current_model().to_string();
            choices.push((model.clone(), model, "Current Antex model".to_string()));
        }
        let initial_selected_idx = choices.iter().position(|(value, _, _)| value == default);
        let items = choices
            .into_iter()
            .map(|(value, name, description)| SelectionItem {
                search_value: Some(format!("{name} {description}")),
                name,
                description: Some(description),
                actions: vec![Box::new(move |tx| {
                    tx.send(AppEvent::WorkflowFieldAnswered(value.clone()));
                })],
                dismiss_on_select: true,
                ..Default::default()
            })
            .collect();
        self.bottom_pane.show_selection_view(SelectionViewParams {
            title: Some(title),
            subtitle: Some(field.description.clone()),
            footer_hint: Some(standard_popup_hint_line()),
            items,
            initial_selected_idx,
            is_searchable: true,
            search_placeholder: Some("Search models".to_string()),
            ..Default::default()
        });
        self.request_redraw();
    }
}
