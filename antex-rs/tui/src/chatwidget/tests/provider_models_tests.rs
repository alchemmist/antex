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
    assert!(chat.on_provider_models_loaded(request_id, catalogs));
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
    assert!(!chat.on_provider_models_loaded(uuid::Uuid::new_v4(), Vec::new()));
    chat.handle_key_event(KeyCode::Esc.into());
    chat.on_provider_models_loaded(
        request_id,
        vec![ProviderCatalog {
            id: "litellm".into(),
            name: "LiteLLM".into(),
            models: Vec::new(),
            unavailable: true,
        }],
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
        }]
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
        ]
    ));
    let rendered = render_bottom_popup(&chat, /*width*/ 100);
    assert!(rendered.contains("Qwen Local"));
    assert!(rendered.contains("Luna Reserve"));
    assert!(!rendered.contains("Ordinary model"));
    assert_chatwidget_snapshot!("provider_models_reserve_and_api", rendered);
}
