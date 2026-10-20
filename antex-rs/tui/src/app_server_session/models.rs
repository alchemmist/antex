//! Fetch picker models without blocking the event loop and keep new-thread defaults in sync.

use super::AppServerSession;
use super::model_preset_from_api_model;
use crate::app_event::AppEvent;
use crate::app_event_sender::AppEventSender;
use antex_app_server_protocol::ClientRequest;
use antex_app_server_protocol::ModelListParams;
use antex_app_server_protocol::ModelListResponse;
use antex_app_server_protocol::RequestId;
use antex_protocol::openai_models::ModelPreset;
use uuid::Uuid;

impl AppServerSession {
    pub(crate) fn set_available_models(&mut self, models: Vec<ModelPreset>) {
        if let Some(default) = models
            .iter()
            .find(|model| model.is_default)
            .or(models.first())
        {
            self.default_model = Some(default.model.clone());
        }
        self.available_models = models;
    }

    pub(crate) fn fetch_models(&self, request_id: Uuid, app_event_tx: AppEventSender) {
        let request_handle = self.request_handle();
        tokio::spawn(async move {
            let result = request_handle
                .request_typed::<ModelListResponse>(ClientRequest::ModelList {
                    request_id: RequestId::String(format!("model-list-{request_id}")),
                    params: ModelListParams {
                        model_provider: None,
                        cursor: None,
                        limit: None,
                        include_hidden: Some(true),
                    },
                })
                .await
                .map(|response| {
                    response
                        .data
                        .into_iter()
                        .map(model_preset_from_api_model)
                        .collect()
                })
                .map_err(|err| err.to_string());
            app_event_tx.send(AppEvent::ModelsLoaded { request_id, result });
        });
    }
}

impl AppServerSession {
    pub(crate) fn fetch_provider_models(
        &self,
        request_id: Uuid,
        providers: Vec<(String, String)>,
        cache: crate::provider_model_cache::ProviderModelCache,
        tx: AppEventSender,
    ) {
        let handle = self.request_handle();
        tokio::spawn(async move {
            use futures::StreamExt;
            let mut pending =
                futures::stream::iter(providers.into_iter().take(/*n*/ 16).map(|(id, name)| {
                    let handle = handle.clone();
                    async move {
                        let result = tokio::time::timeout(
                            std::time::Duration::from_secs(/*secs*/ 8),
                            async {
                                let mut models = Vec::new();
                                let mut cursor = None;
                                loop {
                                    if models.len() >= 512 {
                                        break;
                                    }
                                    let response = handle
                                        .request_typed::<ModelListResponse>(
                                            ClientRequest::ModelList {
                                                request_id: RequestId::String(format!(
                                                    "provider-models-{request_id}-{id}-{}",
                                                    cursor.as_deref().unwrap_or("0")
                                                )),
                                                params: ModelListParams {
                                                    model_provider: Some(id.clone()),
                                                    cursor: cursor.clone(),
                                                    limit: Some(256),
                                                    include_hidden: Some(true),
                                                },
                                            },
                                        )
                                        .await
                                        .map_err(|_| ())?;
                                    models.extend(
                                        response.data.into_iter().map(model_preset_from_api_model),
                                    );
                                    let Some(next) = response.next_cursor else {
                                        break;
                                    };
                                    if cursor.as_ref() == Some(&next) {
                                        break;
                                    }
                                    cursor = Some(next);
                                }
                                models.truncate(/*len*/ 512);
                                Ok::<_, ()>(models)
                            },
                        )
                        .await;
                        let models = match result {
                            Ok(Ok(models)) => Some(models),
                            _ => None,
                        };
                        crate::model_catalog::ProviderCatalog {
                            id,
                            name,
                            unavailable: models.is_none(),
                            models: models.unwrap_or_default(),
                        }
                    }
                }))
                .buffer_unordered(/*n*/ 4);
            let mut catalogs = Vec::new();
            while let Some(catalog) = pending.next().await {
                catalogs.push(catalog);
                catalogs.sort_by(|a, b| a.id.cmp(&b.id));
                if let Err(error) = cache.store(&catalogs) {
                    tracing::debug!(%error, "Unable to cache provider models");
                }
                tx.send(AppEvent::ProviderModelsLoaded {
                    request_id,
                    catalogs: catalogs.clone(),
                    complete: false,
                });
            }
            tx.send(AppEvent::ProviderModelsLoaded {
                request_id,
                catalogs,
                complete: true,
            });
        });
    }
}

pub(crate) enum ProviderThreadTarget {
    New,
    Resume(antex_protocol::ThreadId),
}

impl AppServerSession {
    pub(crate) async fn select_provider_thread(
        &mut self,
        local_settings: &crate::local_settings::LocalSettings,
        config: crate::legacy_core::config::Config,
        models: Vec<ModelPreset>,
        target: ProviderThreadTarget,
    ) -> color_eyre::eyre::Result<super::AppServerStartedThread> {
        let original_provider = self
            .model_provider_override
            .replace(config.model_provider_id.clone());
        let original_models = std::mem::replace(&mut self.available_models, models);
        let original_default = self.default_model.clone();
        let result = match target {
            ProviderThreadTarget::New => {
                self.start_thread_with_session_start_source(
                    local_settings,
                    &config,
                    /*session_start_source*/ None,
                    /*remote_cwd_override*/ None,
                    /*selected_profile*/ None,
                )
                .await
            }
            ProviderThreadTarget::Resume(id) => {
                self.resume_thread(
                    local_settings,
                    config,
                    id,
                    super::ResumeModelSettings::OverrideFromCurrentConfig,
                )
                .await
            }
        };
        self.model_provider_override = original_provider;
        self.available_models = original_models;
        self.default_model = original_default;
        result
    }
}
