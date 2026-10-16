use super::*;
use crate::model_catalog::LUNA_MODEL;
use crate::model_catalog::LUNA_RESERVE_MODEL;
use crate::model_catalog::ProviderCatalog;

impl ChatWidget {
    pub(crate) fn on_provider_models_loaded(
        &mut self,
        request_id: uuid::Uuid,
        catalogs: Vec<ProviderCatalog>,
    ) -> bool {
        if !self.model_popup_request_is_current(request_id) {
            return false;
        }
        self.model_popup_request_id = None;
        if self.model_catalog.providers.is_empty() {
            let provider = &self.config.model_provider_id;
            for id in &mut self.model_popup_model_ids {
                *id = format!("model:{}:{}{}", provider.len(), provider, id);
            }
        }
        if let Some(current) = catalogs
            .iter()
            .find(|catalog| catalog.id == self.config.model_provider_id && !catalog.unavailable)
        {
            Arc::make_mut(&mut self.model_catalog).models = current.models.clone();
        }
        Arc::make_mut(&mut self.model_catalog).providers = catalogs;
        self.set_daybreak_enabled(self.daybreak_enabled);
        self.refresh_effective_service_tier();
        self.refresh_model_dependent_surfaces();
        true
    }

    pub(super) fn open_provider_models_popup(&mut self, view_id: &'static str) {
        let thread_id = self.thread_id();
        let mut items = Vec::new();
        let mut ids = Vec::new();
        for catalog in &self.model_catalog.providers {
            let restricted = self.config.model_provider.requires_openai_auth
                && self.restrict_model_picker_to_luna_reserve()
                && catalog.id == self.config.model_provider_id;
            let models: Vec<_> = if restricted {
                let normal = self
                    .rate_limit_snapshots_by_limit_id
                    .values()
                    .find(|snapshot| snapshot.limit_name == LUNA_RESERVE_MODEL)
                    .and_then(|snapshot| snapshot.normal_model_slug.as_deref());
                normal
                    .and_then(|slug| catalog.models.iter().find(|model| model.model == slug))
                    .or_else(|| {
                        catalog
                            .models
                            .iter()
                            .find(|model| model.model == LUNA_MODEL)
                    })
                    .cloned()
                    .map(|mut model| {
                        model.model = LUNA_RESERVE_MODEL.into();
                        model.display_name = "Luna Reserve".into();
                        model
                    })
                    .into_iter()
                    .collect()
            } else {
                catalog
                    .models
                    .iter()
                    .filter(|model| model.show_in_picker)
                    .cloned()
                    .collect()
            };
            if catalog.unavailable || models.is_empty() {
                ids.push(format!("unavailable:{}", catalog.id));
                items.push(SelectionItem {
                    name: format!("{} ({})", catalog.name, catalog.id),
                    description: Some(
                        if catalog.unavailable {
                            "Unable to load models; check endpoint and credentials"
                        } else {
                            "No models available"
                        }
                        .into(),
                    ),
                    dismiss_on_select: false,
                    ..Default::default()
                });
            }
            for model in &models {
                let provider = catalog.id.clone();
                let origin_provider = self.config.model_provider_id.clone();
                let model = model.clone();
                ids.push(format!(
                    "model:{}:{}{}",
                    provider.len(),
                    provider,
                    model.model
                ));
                let selected = provider == self.config.model_provider_id
                    && model.model == self.current_model();
                items.push(SelectionItem {
                    name: format!("{} · {} ({})", model.display_name, catalog.name, catalog.id),
                    description: (!model.description.is_empty()).then(|| model.description.clone()),
                    is_current: selected,
                    actions: vec![Box::new(move |tx| {
                        tx.send(AppEvent::OpenProviderReasoning {
                            thread_id,
                            origin_provider: origin_provider.clone(),
                            provider: provider.clone(),
                            model: model.clone(),
                        })
                    })],
                    dismiss_on_select: true,
                    ..Default::default()
                });
            }
        }
        self.show_model_selection_view(
            ids,
            SelectionViewParams {
                view_id: Some(view_id),
                items,
                header: self.model_menu_header(
                    "Select Model",
                    "Subscription and configured providers · current session",
                ),
                ..SelectionViewParams::picker()
            },
        );
    }
}

impl ChatWidget {
    pub(crate) fn open_provider_reasoning_popup(&mut self, provider: String, model: ModelPreset) {
        if self.config.model_provider.requires_openai_auth
            && provider == self.config.model_provider_id
            && model.model == LUNA_RESERVE_MODEL
        {
            self.open_reasoning_popup(model);
            return;
        }
        if model.supported_reasoning_efforts.len() <= 1 {
            self.provider_model_action(&provider, &model)(&self.app_event_tx);
            return;
        }
        let items = model
            .supported_reasoning_efforts
            .iter()
            .map(|option| {
                let mut selected = model.clone();
                selected.default_reasoning_effort = option.effort.clone();
                let action = self.provider_model_action(&provider, &selected);
                SelectionItem {
                    name: format!("{:?}", option.effort),
                    description: Some(option.description.clone()),
                    is_current: option.effort == model.default_reasoning_effort,
                    actions: vec![action],
                    dismiss_on_select: true,
                    ..Default::default()
                }
            })
            .collect();
        self.bottom_pane.show_selection_view(SelectionViewParams {
            title: Some(format!("Reasoning effort · {}", model.display_name)),
            items,
            ..SelectionViewParams::picker()
        });
    }
}

impl ChatWidget {
    fn provider_model_action(&self, provider: &str, model: &ModelPreset) -> SelectionAction {
        let provider = provider.to_string();
        let origin_provider = self.config.model_provider_id.clone();
        let model = model.clone();
        let thread_id = self.thread_id();
        Box::new(move |tx| {
            tx.send(AppEvent::SelectProviderModel {
                thread_id,
                origin_provider: origin_provider.clone(),
                provider: provider.clone(),
                model: model.clone(),
            })
        })
    }
}
