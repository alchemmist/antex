use super::session_lifecycle::ThreadAttachPresentation;
use super::*;
use crate::app_server_session::ProviderThreadTarget;
use crate::chatwidget::ThreadInputStateRestoreMode;

impl App {
    pub(super) fn model_picker_providers(&self) -> Vec<(String, String)> {
        let effective = self.config.config_layer_stack.effective_config();
        let configured = effective
            .get("model_providers")
            .and_then(toml::Value::as_table);
        let mut providers: Vec<_> = self
            .config
            .model_providers
            .iter()
            .filter(|(id, _)| {
                id.as_str() == "openai"
                    || id.as_str() == self.config.model_provider_id
                    || configured.is_some_and(|entries| entries.contains_key(id.as_str()))
            })
            .map(|(id, provider)| {
                (
                    id.clone(),
                    if provider.name.trim().is_empty() {
                        id.clone()
                    } else {
                        provider.name.clone()
                    },
                )
            })
            .collect();
        let current = &self.chat_widget.config_ref().model_provider_id;
        providers.sort_by_key(|(id, _)| (id != current && id != "openai", id.clone()));
        providers
    }

    pub(super) async fn select_provider_model(
        &mut self,
        tui: &mut tui::Tui,
        server: &mut AppServerSession,
        provider: String,
        model: ModelPreset,
    ) -> Result<()> {
        if provider == self.chat_widget.config_ref().model_provider_id {
            self.select_session_model(
                server,
                model.model,
                (!model.supported_reasoning_efforts.is_empty())
                    .then_some(model.default_reasoning_effort),
            )
            .await;
            return Ok(());
        }
        if self.chat_widget.has_running_task() || self.chat_widget.is_external_writer_view() {
            self.chat_widget.add_info_message(
                "Finish the current turn before switching providers.".into(),
                /*hint*/ None,
            );
            return Ok(());
        }
        let Some(info) = self.config.model_providers.get(&provider).cloned() else {
            return Ok(());
        };
        let Some(thread_id) = self.chat_widget.thread_id() else {
            return Ok(());
        };
        let ready = server
            .thread_read(thread_id, /*include_turns*/ false)
            .await
            .is_ok_and(|thread| {
                !thread.ephemeral
                    && matches!(thread.status, antex_app_server_protocol::ThreadStatus::Idle)
            });
        if !ready {
            self.chat_widget.add_info_message(
                "Provider switching requires an idle, saved session.".into(),
                /*hint*/ None,
            );
            return Ok(());
        }
        let history = server
            .request_handle()
            .request_typed::<antex_app_server_protocol::ThreadTurnsListResponse>(
                antex_app_server_protocol::ClientRequest::ThreadTurnsList {
                    request_id: antex_app_server_protocol::RequestId::String(format!(
                        "provider-history-{}",
                        uuid::Uuid::new_v4()
                    )),
                    params: antex_app_server_protocol::ThreadTurnsListParams {
                        thread_id: thread_id.to_string(),
                        cursor: None,
                        limit: Some(1),
                        sort_direction: None,
                        items_view: None,
                    },
                },
            )
            .await;
        let fresh = match history {
            Ok(_) => false,
            Err(antex_app_server_client::TypedRequestError::Server { source, .. })
                if source.code == -32600
                    && source.message
                        == format!(
                            "thread {thread_id} is not materialized yet; thread/turns/list is unavailable before first user message"
                        ) =>
            {
                true
            }
            Err(error) => return Err(error.into()),
        };
        let original = self.chat_widget.config_ref().clone();
        let mut config = original.clone();
        config.model_provider_id = provider.clone();
        config.model_provider = info;
        config.model_providers = self.config.model_providers.clone();
        config.model = Some(model.model.clone());
        config.model_reasoning_effort = (!model.supported_reasoning_efforts.is_empty())
            .then_some(model.default_reasoning_effort.clone());
        config.service_tier = None;
        let input = self.chat_widget.capture_thread_input_state();
        let models = self
            .model_catalog
            .providers
            .iter()
            .find(|entry| entry.id == provider)
            .map(|entry| entry.models.clone())
            .unwrap_or_else(|| self.model_catalog.models.clone());
        if !fresh && server.thread_unsubscribe(thread_id).await.is_err() {
            self.chat_widget.add_info_message(
                "Unable to detach the current session; provider unchanged.".into(),
                /*hint*/ None,
            );
            return Ok(());
        }
        let target = if fresh {
            ProviderThreadTarget::New
        } else {
            ProviderThreadTarget::Resume(thread_id)
        };
        let result = server
            .select_provider_thread(&self.local_settings, config.clone(), models, target)
            .await;
        let started = match result {
            Ok(started) if started.session.model_provider_id == provider => started,
            result => {
                if !fresh && result.is_err() {
                    let _ = server
                        .select_provider_thread(
                            &self.local_settings,
                            original,
                            self.model_catalog.models.clone(),
                            ProviderThreadTarget::Resume(thread_id),
                        )
                        .await;
                }
                return Err(result.err().unwrap_or_else(|| {
                    color_eyre::eyre::eyre!("The session is still using its previous provider")
                }));
            }
        };
        if fresh {
            let _ = server.thread_unsubscribe(thread_id).await;
        }
        if let Some(models) = self
            .model_catalog
            .providers
            .iter()
            .find(|entry| entry.id == provider)
            .map(|entry| entry.models.clone())
        {
            Arc::make_mut(&mut self.model_catalog).models = models;
        }
        self.config = config;
        self.replace_chat_widget_with_app_server_thread(
            tui,
            started,
            ThreadAttachPresentation::SessionLineage,
            /*initial_user_message*/ None,
        )
        .await?;
        self.chat_widget.restore_thread_input_state(
            input,
            ThreadInputStateRestoreMode {
                preserve_in_flight_turn: false,
            },
        );
        self.chat_widget.requires_openai_auth = self.config.model_provider.requires_openai_auth;
        self.chat_widget.set_model(&model.model);
        self.chat_widget
            .set_reasoning_effort(self.config.model_reasoning_effort.clone());
        self.chat_widget
            .set_plan_mode_reasoning_effort(self.config.model_reasoning_effort.clone());
        Ok(())
    }
}
