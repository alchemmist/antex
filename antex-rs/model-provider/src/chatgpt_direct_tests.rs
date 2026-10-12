use super::*;
use crate::WorkspaceRoutingContext;
use crate::create_model_provider;
use crate::models_identity;
use antex_api::ResponsesApiRequest;
use antex_api::ResponsesClient;
use antex_api::ResponsesOptions;
use antex_http_client::HttpClientFactory;
use antex_http_client::HttpTransport;
use antex_http_client::OutboundProxyPolicy;
use antex_http_client::Response;
use antex_http_client::StreamResponse;
use antex_http_client::TransportError;
use antex_login::AuthCredentialsStoreMode;
use antex_login::AuthKeyringBackendKind;
use antex_login::default_client::ClientRedirectPolicy;
use antex_model_provider_info::ModelProviderInfo;
use antex_models_manager::manager::RefreshStrategy;
use pretty_assertions::assert_eq;
use serde_json::json;
use std::path::Path;
use std::sync::Mutex;

fn save_grant(home: &Path, client_id: &str, access_token: &str) {
    std::fs::write(
        home.join("auth.json"),
        serde_json::to_vec(&json!({
            "auth_mode": "chatgptDirect",
            "tokens": {
                "id_token": "header.e30.signature",
                "access_token": access_token,
                "refresh_token": "test-refresh",
                "direct": {
                    "client_id": client_id,
                    "expires_at": "2099-01-01T00:00:00Z",
                    "scopes": ["chatgpt.tokens.use.direct"]
                }
            },
            "last_refresh": "2099-01-01T00:00:00Z"
        }))
        .unwrap(),
    )
    .unwrap();
}

async fn manager(home: &Path) -> Arc<AuthManager> {
    Arc::new(
        AuthManager::new(
            home.to_path_buf(),
            /*enable_antex_api_key_env*/ false,
            AuthCredentialsStoreMode::File,
            /*forced_chatgpt_workspace_id*/ None,
            /*chatgpt_base_url*/ None,
            AuthKeyringBackendKind::default(),
            antex_login::test_support::transport_default_auth_route_config(),
        )
        .await,
    )
}

#[tokio::test]
async fn direct_credentials_are_bound_to_responses_and_follow_managed_rotation() {
    let home = tempfile::tempdir().unwrap();
    save_grant(home.path(), "issued-client", "initial-access");
    let manager = manager(home.path()).await;
    let info = ModelProviderInfo::create_openai_provider(/*base_url*/ None);
    let provider = create_model_provider(info.clone(), Some(manager.clone()));
    let initial = manager.auth_cached().unwrap();
    let identity = models_identity::identity(&info, Some(&initial)).unwrap();
    let auth = provider.api_auth().await.unwrap();
    assert!(auth.to_auth_headers().is_empty());
    assert!(auth.resolve_auth_headers().await.is_err());

    for url in [
        "https://chatgpt.com/backend-api/codex/responses",
        "https://api.openai.com.evil.test/v1/responses",
        "http://api.openai.com/v1/responses",
        "https://api.openai.com:444/v1/responses",
        "https://user@api.openai.com/v1/responses",
        "https://api.openai.com/v1/responses?forward=elsewhere",
        "https://api.openai.com/v1/responses/compact",
        "https://api.openai.com/v1/models",
    ] {
        assert!(
            auth.apply_auth(Request::new(Method::POST, url.to_string()))
                .await
                .is_err()
        );
    }
    let request = || {
        Request::new(
            Method::POST,
            "https://api.openai.com/v1/responses".to_string(),
        )
    };
    let mut first = request();
    first.headers.insert(
        "chatgpt-account-id",
        HeaderValue::from_static("do-not-send"),
    );
    first
        .headers
        .insert("x-oai-attestation", HeaderValue::from_static("do-not-send"));
    let first = auth.apply_auth(first).await.unwrap();
    assert_eq!(
        first.headers,
        HeaderMap::from_iter([(
            http::header::AUTHORIZATION,
            HeaderValue::from_static("Bearer initial-access")
        )])
    );

    save_grant(home.path(), "issued-client", "rotated-access");
    manager.reload().await;
    let rotated = auth.apply_auth(request()).await.unwrap();
    assert_eq!(
        rotated.headers[http::header::AUTHORIZATION],
        "Bearer rotated-access"
    );
    assert_eq!(
        models_identity::identity(&info, manager.auth_cached().as_ref()).unwrap(),
        identity
    );

    save_grant(home.path(), "other-issued-client", "other-account-access");
    manager.reload().await;
    assert!(auth.apply_auth(request()).await.is_err());
    assert_ne!(
        models_identity::identity(&info, manager.auth_cached().as_ref()).unwrap(),
        identity
    );
}

#[derive(Clone)]
struct CaptureSseRequest(Arc<Mutex<Vec<Request>>>);

impl HttpTransport for CaptureSseRequest {
    async fn execute(&self, _request: Request) -> Result<Response, TransportError> {
        panic!("Responses must use the streaming transport");
    }

    async fn stream(&self, request: Request) -> Result<StreamResponse, TransportError> {
        self.0.lock().unwrap().push(request);
        Err(TransportError::Network(
            "injected transport stop".to_string(),
        ))
    }
}

#[tokio::test]
async fn direct_responses_use_public_sse_route_without_backend_headers() {
    let home = tempfile::tempdir().unwrap();
    save_grant(home.path(), "issued-client", "direct-access");
    let manager = manager(home.path()).await;
    let provider = create_model_provider(
        ModelProviderInfo::create_openai_provider(/*base_url*/ None),
        Some(manager),
    );
    assert!(!provider.supports_attestation());
    assert_eq!(
        provider.capabilities().remote_compaction,
        crate::RemoteCompactionSupport::Unsupported
    );
    let route = provider
        .responses_api_provider(&WorkspaceRoutingContext::new(
            "https://chatgpt.com/backend-api".to_string(),
        ))
        .await
        .unwrap();
    assert!(matches!(
        route.redirect_policy,
        ClientRedirectPolicy::Reject
    ));
    assert!(!provider.include_internal_metadata(&route.provider));
    let requests = Arc::new(Mutex::new(Vec::new()));
    let mut api_provider = route.provider;
    api_provider.retry.max_attempts = 0;
    let client = ResponsesClient::new(
        CaptureSseRequest(requests.clone()),
        api_provider,
        provider.api_auth().await.unwrap(),
    );
    let result = client
        .stream_request(
            ResponsesApiRequest {
                model: "gpt-6-astra".to_string(),
                input: Vec::new(),
                tools: None,
                tool_choice: "auto".to_string(),
                parallel_tool_calls: true,
                reasoning: None,
                store: false,
                stream: true,
                stream_options: None,
                include: Vec::new(),
                service_tier: None,
                prompt_cache_key: None,
                text: None,
                client_metadata: None,
                access_programs: None,
            },
            ResponsesOptions::default(),
        )
        .await;
    assert!(result.is_err());
    let requests = requests.lock().unwrap();
    assert_eq!(requests.len(), 1);
    assert_eq!(requests[0].url, "https://api.openai.com/v1/responses");
    assert_eq!(
        requests[0].headers[http::header::AUTHORIZATION],
        "Bearer direct-access"
    );
    assert!(!requests[0].headers.contains_key("chatgpt-account-id"));
    assert!(!requests[0].headers.contains_key("x-oai-attestation"));
}

#[tokio::test]
async fn direct_picker_ignores_legacy_cache_and_keeps_bundled_api_models() {
    let home = tempfile::tempdir().unwrap();
    save_grant(home.path(), "issued-client", "direct-access");
    let mut legacy_model = antex_models_manager::bundled_models_response()
        .unwrap()
        .models
        .remove(0);
    legacy_model.slug = "legacy-only-model".to_string();
    let legacy_auth = AntexAuth::create_dummy_chatgpt_auth_for_testing();
    let legacy_identity = models_identity::identity(
        &ModelProviderInfo::create_openai_provider(/*base_url*/ None),
        Some(&legacy_auth),
    )
    .unwrap();
    let legacy_cache = serde_json::to_vec(&json!({
        "fetched_at": "2099-01-01T00:00:00Z",
        "client_version": antex_models_manager::client_version_to_whole(),
        "identity": legacy_identity,
        "models": [legacy_model]
    }))
    .unwrap();
    std::fs::write(home.path().join("models_cache.json"), &legacy_cache).unwrap();
    let provider = create_model_provider(
        ModelProviderInfo::create_openai_provider(/*base_url*/ None),
        Some(manager(home.path()).await),
    );
    let models = provider.models_manager(
        home.path().to_path_buf(),
        /*config_model_catalog*/ None,
    );
    let presets = models
        .list_models(
            RefreshStrategy::Online,
            HttpClientFactory::new(OutboundProxyPolicy::ReqwestDefault),
        )
        .await;
    assert!(presets.iter().any(|model| model.model == "gpt-6-astra"));
    assert!(
        presets
            .iter()
            .all(|model| model.supported_in_api && model.model != "legacy-only-model")
    );
    assert_eq!(
        std::fs::read(home.path().join("models_cache.json")).unwrap(),
        legacy_cache
    );
    let initial = models.try_get_remote_models().unwrap();
    assert!(
        initial
            .iter()
            .all(|model| !model.use_responses_lite && !model.supports_reasoning_effort_updates)
    );
    models
        .refresh_after_auth_change(HttpClientFactory::new(OutboundProxyPolicy::ReqwestDefault))
        .await;
    assert_eq!(models.get_remote_models().await, initial);
}
