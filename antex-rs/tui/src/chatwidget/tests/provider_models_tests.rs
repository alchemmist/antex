use super::*;
use crate::model_catalog::ProviderCatalog;
use pretty_assertions::assert_eq;

#[tokio::test]
async fn provider_picker_keeps_duplicate_model_names_distinct() {
    let (mut chat, mut events, _) = make_chatwidget_manual(Some("gpt-5.5")).await;
    chat.thread_id = Some(ThreadId::new());
    chat.open_model_popup();
    let request_id = chat.model_popup_request_id.unwrap();
    let mut model = chat.model_catalog.models[0].clone();
    model.model = "shared-model".into();
    model.display_name = "Shared Model".into();
    model.description = "Same model identifier, different credentials".into();
    let catalogs = vec![
        ProviderCatalog {
            id: "openai".into(),
            name: "OpenAI subscription".into(),
            models: vec![model.clone()],
            unavailable: false,
        },
        ProviderCatalog {
            id: "litellm".into(),
            name: "LiteLLM".into(),
            models: vec![model],
            unavailable: false,
        },
        ProviderCatalog {
            id: "offline".into(),
            name: "Offline provider".into(),
            models: Vec::new(),
            unavailable: true,
        },
    ];
    assert!(chat.on_provider_models_loaded(request_id, catalogs, /*complete*/ true));
    assert_chatwidget_snapshot!(
        "provider_models_shared_picker",
        render_bottom_popup(&chat, /*width*/ 100)
    );
    while events.try_recv().is_ok() {}
    chat.handle_key_event(KeyCode::Down.into());
    chat.handle_key_event(KeyCode::Enter.into());
    let selected = std::iter::from_fn(|| events.try_recv().ok()).find_map(|event| match event {
        AppEvent::OpenProviderReasoning {
            provider, model, ..
        } => Some((provider, model.model)),
        _ => None,
    });
    assert_eq!(selected, Some(("litellm".into(), "shared-model".into())));
}

#[tokio::test]
async fn provider_picker_rejects_stale_catalog_and_stays_closed_after_dismissal() {
    let (mut chat, _, _) = make_chatwidget_manual(Some("gpt-5.5")).await;
    chat.thread_id = Some(ThreadId::new());
    chat.open_model_popup();
    let request_id = chat.model_popup_request_id.unwrap();
    assert!(!chat.on_provider_models_loaded(
        uuid::Uuid::new_v4(),
        Vec::new(),
        /*complete*/ true
    ));
    chat.handle_key_event(KeyCode::Esc.into());
    chat.on_provider_models_loaded(
        request_id,
        vec![ProviderCatalog {
            id: "litellm".into(),
            name: "LiteLLM".into(),
            models: Vec::new(),
            unavailable: true,
        }],
        /*complete*/ true,
    );
    assert!(chat.no_modal_or_popup_active());
}

#[tokio::test]
async fn provider_picker_refreshes_an_already_open_all_models_menu() {
    let (mut chat, _, _) = make_chatwidget_manual(Some("gpt-5.5")).await;
    chat.thread_id = Some(ThreadId::new());
    chat.open_model_popup();
    let request_id = chat.model_popup_request_id.expect("catalog request");
    chat.open_all_models_popup();
    let model = chat.model_catalog.models[0].clone();
    assert!(chat.on_provider_models_loaded(
        request_id,
        vec![ProviderCatalog {
            id: "litellm".into(),
            name: "LiteLLM".into(),
            models: vec![model],
            unavailable: false
        }],
        /*complete*/ true,
    ));
    assert!(render_bottom_popup(&chat, /*width*/ 100).contains("LiteLLM"));
    assert!(
        chat.bottom_pane
            .selected_index_for_present_view(
                super::super::model_popups::ALL_MODELS_SELECTION_VIEW_ID
            )
            .is_some()
    );
}

#[tokio::test]
async fn provider_picker_keeps_api_models_available_during_subscription_reserve() {
    let (mut chat, _, _) =
        make_chatwidget_manual(Some(crate::model_catalog::LUNA_RESERVE_MODEL)).await;
    chat.thread_id = Some(ThreadId::new());
    chat.config.model_provider.requires_openai_auth = true;
    let mut luna = chat.model_catalog.models[0].clone();
    luna.model = crate::model_catalog::LUNA_MODEL.into();
    luna.description.clear();
    std::sync::Arc::make_mut(&mut chat.model_catalog).models = vec![luna.clone()];
    chat.open_model_popup();
    let request_id = chat.model_popup_request_id.expect("catalog request");
    let mut ordinary = luna.clone();
    ordinary.model = "ordinary-model".into();
    ordinary.display_name = "Ordinary model".into();
    let mut api = luna.clone();
    api.model = "qwen-local".into();
    api.display_name = "Qwen Local".into();
    assert!(chat.on_provider_models_loaded(
        request_id,
        vec![
            ProviderCatalog {
                id: "openai".into(),
                name: "OpenAI".into(),
                models: vec![luna, ordinary],
                unavailable: false
            },
            ProviderCatalog {
                id: "litellm".into(),
                name: "LiteLLM".into(),
                models: vec![api],
                unavailable: false
            },
        ],
        /*complete*/ true,
    ));
    let rendered = render_bottom_popup(&chat, /*width*/ 100);
    assert!(rendered.contains("Qwen Local"));
    assert!(rendered.contains("Luna Reserve"));
    assert!(!rendered.contains("Ordinary model"));
    assert_chatwidget_snapshot!("provider_models_reserve_and_api", rendered);
}

#[tokio::test]
async fn provider_picker_accepts_ready_models_before_discovery_finishes() {
    let (mut chat, _, _) = make_chatwidget_manual(Some("gpt-5.5")).await;
    chat.thread_id = Some(ThreadId::new());
    chat.open_model_popup();
    let request_id = chat.model_popup_request_id.unwrap();
    let mut model = chat.model_catalog.models[0].clone();
    Arc::make_mut(&mut chat.model_catalog).models = vec![model.clone()];
    model.model = "qwen-local".into();
    model.display_name = "Qwen Local".into();
    let ready = ProviderCatalog {
        id: "litellm".into(),
        name: "LiteLLM".into(),
        models: vec![model],
        unavailable: false,
    };
    assert!(chat.on_provider_models_loaded(
        request_id,
        vec![ready.clone()],
        /*complete*/ false
    ));
    assert_eq!(chat.model_popup_request_id, Some(request_id));
    let rendered = render_bottom_popup(&chat, /*width*/ 100);
    assert!(rendered.contains("Qwen Local"));
    assert_chatwidget_snapshot!("provider_models_partial_picker", rendered);
    assert!(
        chat.model_catalog
            .providers
            .iter()
            .any(|catalog| catalog.id == "openai")
    );
    chat.handle_key_event(KeyCode::Esc.into());
    assert!(chat.on_provider_models_loaded(request_id, vec![ready], /*complete*/ true));
    assert_eq!(chat.model_popup_request_id, None);
    assert!(chat.no_modal_or_popup_active());
    assert!(!chat.on_provider_models_loaded(request_id, Vec::new(), /*complete*/ true));
    chat.open_model_popup();
    let next_request_id = chat.model_popup_request_id.unwrap();
    assert!(!chat.on_provider_models_loaded(request_id, Vec::new(), /*complete*/ false));
    assert_eq!(chat.model_popup_request_id, Some(next_request_id));
}

#[tokio::test]
async fn provider_discovery_publishes_litellm_while_another_provider_is_pending()
-> color_eyre::Result<()> {
    let endpoint = wiremock::MockServer::start().await;
    wiremock::Mock::given(wiremock::matchers::path("/fast/models"))
        .respond_with(
            wiremock::ResponseTemplate::new(200).set_body_json(serde_json::json!({
                "data": [{"id": "qwen-local"}]
            })),
        )
        .mount(&endpoint)
        .await;
    wiremock::Mock::given(wiremock::matchers::path("/slow/models"))
        .respond_with(
            wiremock::ResponseTemplate::new(200)
                .set_body_json(serde_json::json!({"data": [{"id": "slow-model"}]}))
                .set_delay(std::time::Duration::from_secs(/*secs*/ 30)),
        )
        .mount(&endpoint)
        .await;
    let home = tempfile::tempdir()?;
    let url = endpoint.uri();
    std::fs::write(
        home.path().join("config.toml"),
        format!(
            r#"
model_provider = "litellm"
model = "qwen-local"
[model_providers.litellm]
name = "LiteLLM"
base_url = "{url}/fast"
wire_api = "responses"
[model_providers.slow]
name = "Slow"
base_url = "{url}/slow"
wire_api = "responses"
"#
        ),
    )?;
    let config = ConfigBuilder::default()
        .codex_home(home.path().to_path_buf())
        .build()
        .await?;
    let server = crate::start_embedded_app_server_for_picker(&config).await?;
    let (tx, mut rx) = tokio::sync::mpsc::unbounded_channel();
    let request_id = uuid::Uuid::new_v4();
    server.fetch_provider_models(
        request_id,
        vec![
            ("slow".into(), "Slow".into()),
            ("litellm".into(), "LiteLLM".into()),
        ],
        crate::provider_model_cache::ProviderModelCache::new(&config),
        AppEventSender::new(tx),
    );
    let event = tokio::time::timeout(std::time::Duration::from_secs(/*secs*/ 3), rx.recv())
        .await?
        .expect("provider event");
    let AppEvent::ProviderModelsLoaded {
        request_id: actual_id,
        catalogs,
        complete,
    } = event
    else {
        panic!("expected provider models");
    };
    assert_eq!(
        (actual_id, complete, catalogs.len()),
        (request_id, false, 1)
    );
    assert_eq!(
        (
            catalogs[0].id.as_str(),
            catalogs[0].models[0].model.as_str()
        ),
        ("litellm", "qwen-local")
    );
    Ok(())
}

#[tokio::test]
async fn provider_discovery_reads_every_catalog_page() -> color_eyre::Result<()> {
    let endpoint = wiremock::MockServer::start().await;
    let data: Vec<_> = (0..265)
        .map(|index| serde_json::json!({"id": format!("model-{index:03}")}))
        .collect();
    wiremock::Mock::given(wiremock::matchers::path("/v1/models"))
        .respond_with(
            wiremock::ResponseTemplate::new(200).set_body_json(serde_json::json!({ "data": data })),
        )
        .mount(&endpoint)
        .await;
    let home = tempfile::tempdir()?;
    std::fs::write(
        home.path().join("config.toml"),
        format!(
            r#"
[model_providers.cursor]
name = "Cursor subscription"
base_url = "{}/v1"
wire_api = "responses"
"#,
            endpoint.uri()
        ),
    )?;
    let config = ConfigBuilder::default()
        .codex_home(home.path().to_path_buf())
        .build()
        .await?;
    let server = crate::start_embedded_app_server_for_picker(&config).await?;
    let (tx, mut rx) = tokio::sync::mpsc::unbounded_channel();
    let request_id = uuid::Uuid::new_v4();
    server.fetch_provider_models(
        request_id,
        vec![("cursor".into(), "Cursor subscription".into())],
        crate::provider_model_cache::ProviderModelCache::new(&config),
        AppEventSender::new(tx),
    );
    let mut catalogs = Vec::new();
    let mut complete = false;
    while !complete {
        let event = tokio::time::timeout(std::time::Duration::from_secs(/*secs*/ 8), rx.recv())
            .await?
            .expect("provider event");
        let AppEvent::ProviderModelsLoaded {
            catalogs: loaded,
            complete: finished,
            ..
        } = event
        else {
            panic!("expected provider models");
        };
        catalogs = loaded;
        complete = finished;
    }
    assert_eq!(
        catalogs
            .iter()
            .find(|catalog| catalog.id == "cursor")
            .map(|catalog| catalog.models.len()),
        Some(265)
    );
    Ok(())
}

#[tokio::test]
async fn provider_picker_displays_disk_cached_models_before_fetch_completes() -> anyhow::Result<()>
{
    let home = tempfile::tempdir()?;
    std::fs::write(
        home.path().join("config.toml"),
        r#"
[model_providers.litellm]
name = "LiteLLM"
base_url = "http://127.0.0.1:9/v1"
"#,
    )?;
    let config = ConfigBuilder::default()
        .codex_home(home.path().to_path_buf())
        .build()
        .await?;
    let (mut chat, mut events, _) = make_chatwidget_manual(Some("gpt-5.5")).await;
    chat.thread_id = Some(ThreadId::new());
    let mut model = chat.model_catalog.models[0].clone();
    model.model = "qwen-local".into();
    model.display_name = "Qwen Local".into();
    let catalog = ProviderCatalog {
        id: "litellm".into(),
        name: "LiteLLM".into(),
        models: vec![model],
        unavailable: false,
    };
    crate::provider_model_cache::ProviderModelCache::new(&config).store(&[catalog])?;
    Arc::make_mut(&mut chat.model_catalog).providers =
        crate::provider_model_cache::ProviderModelCache::new(&config).load(&config);
    while events.try_recv().is_ok() {}
    chat.open_model_popup();
    assert!(render_bottom_popup(&chat, /*width*/ 100).contains("Qwen Local"));
    assert!(
        std::iter::from_fn(|| events.try_recv().ok())
            .any(|event| matches!(event, AppEvent::FetchModels { .. }))
    );
    Ok(())
}

#[tokio::test]
async fn eliza_picker_preserves_other_providers() {
    let (mut chat, _, _) = make_chatwidget_manual(Some("gpt-5.5")).await;
    chat.thread_id = Some(ThreadId::new());
    chat.open_model_popup();
    let request_id = chat.model_popup_request_id.expect("catalog request");
    let base = chat.model_catalog.models[0].clone();
    let catalogs = [
        ("openai", "OpenAI", "gpt-5.5"),
        ("cursor", "Cursor subscription", "composer"),
        ("litellm", "LiteLLM", "proxy-model"),
        ("eliza", "Yandex Internal", "internal-model"),
    ]
    .into_iter()
    .map(|(id, name, slug)| {
        let mut model = base.clone();
        model.model = slug.into();
        model.display_name = slug.into();
        model.description.clear();
        ProviderCatalog {
            id: id.into(),
            name: name.into(),
            models: vec![model],
            unavailable: false,
        }
    })
    .collect();
    assert!(chat.on_provider_models_loaded(request_id, catalogs, /*complete*/ true));
    assert_chatwidget_snapshot!(
        "eliza_picker_preserves_other_providers",
        render_bottom_popup(&chat, /*width*/ 100)
    );
}
