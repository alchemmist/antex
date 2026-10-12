use super::*;
use antex_http_client::HttpClientFactory;
use antex_http_client::OutboundProxyPolicy;
use antex_models_manager::manager::ModelsEndpointClient;
use pretty_assertions::assert_eq;

#[tokio::test]
async fn models_and_factory_producers_reject_repeated_failures_before_dispatch() {
    for failure in [
        Resolution::TransientFailure,
        Resolution::PermanentFailure,
        Resolution::Auth(chatgpt_auth("workspace-b", "rejected")),
    ] {
        let server = wiremock::MockServer::start().await;
        let home = tempfile::tempdir().unwrap();
        let initial = chatgpt_auth("workspace-a", "initial");
        let manager =
            AuthManager::from_auth_for_testing_with_home(initial.clone(), home.path().into());
        let source = Arc::new(Source {
            current: Mutex::new(Resolution::Auth(initial.clone())),
            after_routing: failure.clone(),
        });
        manager.set_external_auth(source.clone()).await.unwrap();
        let endpoint = crate::models_endpoint::OpenAiModelsEndpoint::new(
            ModelProviderInfo::create_openai_provider(Some(format!("{}/v1", server.uri()))),
            Some(manager.clone()),
            /*gateway_auth_manager*/ None,
        );
        *source.current.lock().unwrap() = failure;
        for _ in 0..2 {
            assert!(
                manager
                    .auth_for_request_with_http_client_factory()
                    .await
                    .is_err()
            );
            assert!(manager.auth_with_http_client_factory().await.is_none());
            assert!(
                ModelsEndpointClient::list_models(
                    &endpoint,
                    "test",
                    HttpClientFactory::new(OutboundProxyPolicy::ReqwestDefault)
                )
                .await
                .is_err()
            );
            assert!(server.received_requests().await.unwrap().is_empty());
            assert_eq!(manager.auth_cached(), Some(initial.clone()));
        }
    }
}

#[tokio::test]
async fn explicit_provider_auth_and_local_absence_ignore_failed_ambient_sources() {
    for explicit in [false, true] {
        let server = wiremock::MockServer::start().await;
        wiremock::Mock::given(wiremock::matchers::method("GET"))
            .respond_with(wiremock::ResponseTemplate::new(/*s*/ 200).set_body_json(
                antex_protocol::openai_models::ModelsResponse { models: Vec::new() },
            ))
            .mount(&server)
            .await;
        let home = tempfile::tempdir().unwrap();
        let initial = chatgpt_auth("workspace-a", "initial");
        let manager =
            AuthManager::from_auth_for_testing_with_home(initial.clone(), home.path().into());
        let source = Arc::new(Source {
            current: Mutex::new(Resolution::Auth(initial)),
            after_routing: Resolution::TransientFailure,
        });
        manager.set_external_auth(source.clone()).await.unwrap();
        *source.current.lock().unwrap() = Resolution::TransientFailure;
        let mut info =
            ModelProviderInfo::create_openai_provider(Some(format!("{}/v1", server.uri())));
        info.requires_openai_auth = explicit;
        info.experimental_bearer_token = explicit.then(|| "explicit-provider-token".into());
        let provider = create_model_provider(info.clone(), Some(manager.clone()));
        provider.api_provider().await.unwrap();
        provider
            .responses_api_provider(&WorkspaceRoutingContext::new(
                "https://chatgpt.com/backend-api".into(),
            ))
            .await
            .unwrap();
        let headers = provider
            .api_auth_for_scope(scope())
            .await
            .unwrap()
            .auth
            .resolve_auth_headers()
            .await
            .unwrap();
        assert_eq!(
            headers
                .get(http::header::AUTHORIZATION)
                .map(|value| value.to_str().unwrap()),
            explicit.then_some("Bearer explicit-provider-token")
        );
        let endpoint = crate::models_endpoint::OpenAiModelsEndpoint::new(
            info,
            Some(manager),
            /*gateway_auth_manager*/ None,
        );
        ModelsEndpointClient::list_models(
            &endpoint,
            "test",
            HttpClientFactory::new(OutboundProxyPolicy::ReqwestDefault),
        )
        .await
        .unwrap();
        let requests = server.received_requests().await.unwrap();
        assert_eq!(requests.len(), 1);
        assert_eq!(
            requests[0]
                .headers
                .get(http::header::AUTHORIZATION)
                .map(|value| value.to_str().unwrap()),
            explicit.then_some("Bearer explicit-provider-token")
        );
    }
}

#[tokio::test]
async fn command_and_bedrock_command_paths_propagate_active_source_failure() {
    for bedrock in [false, true] {
        let home = tempfile::tempdir().unwrap();
        let mut info = if bedrock {
            ModelProviderInfo::create_amazon_bedrock_provider(/*aws*/ None)
        } else {
            ModelProviderInfo::create_openai_provider(/*base_url*/ None)
        };
        info.base_url = Some("https://proxy.example.com/v1".into());
        info.auth = Some(antex_protocol::config_types::ModelProviderAuthInfo {
            command: "unused-injected-source".into(),
            args: Vec::new(),
            timeout_ms: std::num::NonZeroU64::new(/*n*/ 5_000).unwrap(),
            refresh_interval_ms: 300_000,
            cwd: home.path().to_owned().try_into().unwrap(),
        });
        let provider = create_model_provider(info, /*auth_manager*/ None);
        let manager = provider.auth_manager().unwrap();
        let initial = AntexAuth::from_api_key("working-command-token");
        let source = Arc::new(Source {
            current: Mutex::new(Resolution::Auth(initial.clone())),
            after_routing: Resolution::PermanentFailure,
        });
        // Existing host injection installs a controlled source in the real
        // provider-scoped manager; no command or authority endpoint is executed.
        manager.set_external_auth(source.clone()).await.unwrap();
        *source.current.lock().unwrap() = Resolution::PermanentFailure;
        for _ in 0..2 {
            assert!(provider.api_auth().await.is_err());
            assert!(provider.api_auth_for_scope(scope()).await.is_err());
            assert_eq!(manager.auth_cached(), Some(initial.clone()));
        }
    }
}
