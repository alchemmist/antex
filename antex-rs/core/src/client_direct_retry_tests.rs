//! Exercise the real ModelClientSession HTTP retry loop with a paused network response.
use super::stream_tests::ScriptedSse;
use super::*;
use crate::client_common::ResponseStream;
use antex_http_client::HttpTransport;
use antex_http_client::Request;
use antex_http_client::StreamResponse;
use antex_login::ExternalAuth;
use antex_login::ExternalAuthFuture;
use antex_login::ExternalAuthRefreshContext;
use antex_protocol::config_types::ModelProviderAuthInfo;
use http::StatusCode;
use pretty_assertions::assert_eq;
use serde_json::Value;

struct Paused401 {
    started: Notify,
    release: Notify,
    requests: Mutex<Vec<Request>>,
    success: ScriptedSse,
}

impl Paused401 {
    fn install(client: &mut ModelClient) -> Arc<Self> {
        let fixture = Arc::new(Self {
            started: Notify::new(),
            release: Notify::new(),
            requests: Mutex::new(Vec::new()),
            success: success_reply(),
        });
        let injected = fixture.clone();
        Arc::get_mut(&mut client.state).unwrap().test_transport = Some(
            crate::client::test_transport::StreamHook::new(move |request| {
                let injected = injected.clone();
                async move { injected.send(request).await }
            }),
        );
        fixture
    }

    async fn send(&self, request: Request) -> Result<StreamResponse, TransportError> {
        let first = {
            let mut requests = self.requests.lock().unwrap();
            requests.push(request.clone());
            requests.len() == 1
        };
        if first {
            self.started.notify_one();
            self.release.notified().await;
            Err(TransportError::Http {
                status: StatusCode::UNAUTHORIZED,
                retry_after: None,
                url: None,
                headers: None,
                body: None,
            })
        } else {
            self.success.stream(request).await
        }
    }
}

fn success_reply() -> ScriptedSse {
    ScriptedSse::new([vec![
        json!({"type": "response.created", "response": {"id": "resp_recovered"}}),
        json!({"type": "response.output_text.delta", "delta": "Recovered"}),
        json!({"type": "response.output_item.done", "item": {
            "type": "message", "id": "msg_recovered", "role": "assistant",
            "content": [{"type": "output_text", "text": "Recovered"}]
        }}),
        json!({"type": "response.completed", "response": {"id": "resp_recovered", "end_turn": true}}),
    ]])
}

async fn start_request(client: &ModelClient) -> antex_protocol::error::Result<ResponseStream> {
    let model = antex_models_manager::bundled_models_response()
        .unwrap()
        .models
        .into_iter()
        .find(|model| model.slug == "gpt-6-astra")
        .unwrap();
    let prompt =
        Prompt {
            input: vec![serde_json::from_value(json!({
            "type": "message", "role": "user",
            "content": [{"type": "input_text", "text": "Private prompt for the original owner"}]
        })).unwrap()],
            base_instructions: BaseInstructions {
                text: String::new(),
                ..Default::default()
            },
            ..Default::default()
        };
    let metadata = test_responses_metadata_for_client(
        client,
        Some("turn_retry"),
        format!("{}:0", client.state.thread_id),
        /*parent_thread_id*/ None,
        TestAntexResponsesRequestKind::Turn,
    );
    client
        .new_session()
        .stream(
            &prompt,
            &model,
            &test_session_telemetry(),
            /*effort*/ None,
            antex_protocol::config_types::ReasoningSummary::None,
            /*service_tier*/ None,
            &metadata,
            &InferenceTraceContext::disabled(),
        )
        .await
}

async fn assert_success(mut stream: ResponseStream) -> anyhow::Result<()> {
    let mut transcript = Vec::new();
    tokio::time::timeout(Duration::from_secs(5), async {
        while let Some(event) = stream.next().await {
            transcript.push(match event? {
                ResponseEvent::Created { response_id } => json!({"created": response_id}),
                ResponseEvent::ServerModel(model) => json!({"model": model}),
                ResponseEvent::RateLimits(limits) => json!({"rate_limits": limits}),
                ResponseEvent::OutputTextDelta(delta) => json!({"delta": delta}),
                ResponseEvent::OutputItemDone(item) => json!({"item": item}),
                ResponseEvent::Completed {
                    response_id,
                    end_turn,
                    token_usage,
                    usage_metadata,
                } => {
                    assert!(token_usage.is_none() && usage_metadata.is_none());
                    json!({"completed": response_id, "end_turn": end_turn})
                }
                other => panic!("unexpected event: {other:?}"),
            });
        }
        Ok::<_, anyhow::Error>(())
    })
    .await??;
    assert_eq!(
        transcript,
        vec![
            json!({"model": "gpt-6-astra"}),
            json!({"rate_limits": empty_response_rate_limits()}),
            json!({"created": "resp_recovered"}),
            json!({"delta": "Recovered"}),
            json!({"item": {
                "type": "message", "id": "msg_recovered", "role": "assistant",
                "content": [{"type": "output_text", "text": "Recovered"}]
            }}),
            json!({"completed": "resp_recovered", "end_turn": true}),
        ]
    );
    Ok(())
}

fn assert_owner_changed(result: antex_protocol::error::Result<ResponseStream>) {
    let error = match result {
        Err(error) => error,
        Ok(_) => panic!("request must terminate when its original owner changes"),
    };
    match error.details() {
        CodexErrorDetails::RefreshTokenFailed(failure) => assert_eq!(
            failure,
            &antex_protocol::auth::RefreshTokenFailedError::new(
                antex_protocol::auth::RefreshTokenFailedReason::Other,
                "Authentication owner changed (account changed or authentication source replaced); restart the request after signing in",
            ),
        ),
        other => panic!("expected permanent owner-change failure, got {other:?}"),
    }
    assert!(error.retry_delay(/*retry_count*/ 1).is_none());
}

fn wire_body(request: &Request) -> Value {
    let body = request.prepare_body_for_send().unwrap().body.unwrap();
    serde_json::from_slice(&body).unwrap()
}

#[tokio::test]
async fn inflight_401_never_resubmits_direct_or_legacy_prompt_under_new_registration()
-> anyhow::Result<()> {
    for initial_mode in [AuthMode::ChatgptDirect, AuthMode::Chatgpt] {
        let (home, mut client, manager) = direct_client().await?;
        let expected = if initial_mode == AuthMode::Chatgpt {
            let path = home.path().join("auth.json");
            let mut document: Value = serde_json::from_slice(&std::fs::read(&path)?)?;
            document["auth_mode"] = json!("chatgpt");
            document["tokens"].as_object_mut().unwrap().remove("direct");
            document["tokens"]["access_token"] = json!("legacy-access-A");
            document["tokens"]["account_id"] = json!("legacy-account-A");
            std::fs::write(path, serde_json::to_vec(&document)?)?;
            manager.reload().await;
            let mut info = ModelProviderInfo::create_openai_provider(/*base_url*/ None);
            info.supports_websockets = false; // Exercise the HTTP 401 path for legacy auth too.
            Arc::get_mut(&mut client.state).unwrap().provider =
                create_model_provider(info, Some(manager.clone()));
            (
                "https://chatgpt.com/backend-api/codex/responses",
                "Bearer legacy-access-A",
            )
        } else {
            (
                "https://api.openai.com/v1/responses",
                "Bearer test-direct-access",
            )
        };
        let paused = Paused401::install(&mut client);
        let change_owner = async {
            paused.started.notified().await;
            save_direct_grant(home.path(), "registration-B", "must-not-send-B")?;
            manager.reload().await;
            paused.release.notify_one();
            Ok::<_, anyhow::Error>(())
        };
        let (result, changed) = tokio::time::timeout(Duration::from_secs(5), async {
            tokio::join!(start_request(&client), change_owner)
        })
        .await?;
        changed?;
        let requests = paused.requests.lock().unwrap();
        assert_eq!(
            requests.len(),
            1,
            "no second prompt may be transmitted under B"
        );
        assert_eq!(
            (
                requests[0].url.as_str(),
                requests[0].headers[http::header::AUTHORIZATION].to_str()?
            ),
            expected
        );
        assert_eq!(
            wire_body(&requests[0])["input"],
            json!([{
                "type": "message", "role": "user",
                "content": [{"type": "input_text", "text": "Private prompt for the original owner"}]
            }])
        );
        assert_owner_changed(result);
        assert_eq!(
            manager.auth_cached().unwrap().get_token()?,
            "must-not-send-B"
        );
    }
    Ok(())
}

#[tokio::test]
async fn inflight_401_recovers_with_same_registration_rotation() -> anyhow::Result<()> {
    let (home, mut client, manager) = direct_client().await?;
    let owner = client.auth_owner_generation();
    let paused = Paused401::install(&mut client);
    let rotate = async {
        paused.started.notified().await;
        save_direct_grant(home.path(), "test-issued-client", "same-owner-rotated")?;
        manager.reload().await;
        paused.release.notify_one();
        Ok::<_, anyhow::Error>(())
    };
    let (result, rotated) = tokio::time::timeout(Duration::from_secs(5), async {
        tokio::join!(start_request(&client), rotate)
    })
    .await?;
    rotated?;
    assert_success(result?).await?;
    assert_eq!(client.auth_owner_generation(), owner);
    let requests = paused.requests.lock().unwrap();
    assert_eq!(requests.len(), 2);
    assert_eq!(
        requests
            .iter()
            .map(|r| (
                r.url.as_str(),
                r.headers[http::header::AUTHORIZATION].to_str().unwrap()
            ))
            .collect::<Vec<_>>(),
        vec![
            (
                "https://api.openai.com/v1/responses",
                "Bearer test-direct-access"
            ),
            (
                "https://api.openai.com/v1/responses",
                "Bearer same-owner-rotated"
            ),
        ]
    );
    assert_eq!(wire_body(&requests[0]), wire_body(&requests[1]));
    assert_eq!(wire_body(&requests[1])["model"], "gpt-6-astra");
    assert!(
        requests
            .iter()
            .all(|r| !r.headers.contains_key("chatgpt-account-id"))
    );
    Ok(())
}

#[derive(Default)]
struct RotatingExternal(AtomicUsize);

impl ExternalAuth for RotatingExternal {
    fn resolve(&self) -> ExternalAuthFuture<'_, AntexAuth> {
        Box::pin(async move {
            Ok(AntexAuth::from_api_key(
                if self.0.load(Ordering::SeqCst) == 0 {
                    "external-initial"
                } else {
                    "external-rotated"
                },
            ))
        })
    }

    fn refresh(&self, _context: ExternalAuthRefreshContext) -> ExternalAuthFuture<'_, AntexAuth> {
        self.0.fetch_add(1, Ordering::SeqCst);
        self.resolve()
    }
}

#[tokio::test]
async fn external_provider_rotation_recovers_but_equal_token_source_replacement_does_not()
-> anyhow::Result<()> {
    for replace_source in [false, true] {
        let manager = AuthManager::from_auth_for_testing(AntexAuth::from_api_key("seed"));
        let source = Arc::new(RotatingExternal::default());
        manager.set_external_auth(source.clone()).await?;
        let mut client = test_model_client(SessionSource::Cli);
        Arc::get_mut(&mut client.state).unwrap().provider = create_model_provider(
            ModelProviderInfo {
                requires_openai_auth: true,
                ..create_oss_provider_with_base_url(
                    "https://custom.example.test/v1",
                    WireApi::Responses,
                )
            },
            Some(manager.clone()),
        );
        let paused = Paused401::install(&mut client);
        let replacement = Arc::new(RotatingExternal::default());
        let change = async {
            paused.started.notified().await;
            if replace_source {
                let token = manager.auth_cached().unwrap().get_token()?;
                manager.set_external_auth(replacement.clone()).await?;
                // A different source with the same current bearer is still a different owner.
                assert_eq!(manager.auth_cached().unwrap().get_token()?, token);
            }
            paused.release.notify_one();
            Ok::<_, anyhow::Error>(())
        };
        let (result, changed) = tokio::time::timeout(Duration::from_secs(5), async {
            tokio::join!(start_request(&client), change)
        })
        .await?;
        changed?;
        if replace_source {
            assert_eq!(paused.requests.lock().unwrap().len(), 1);
            assert_owner_changed(result);
            assert_eq!(replacement.0.load(Ordering::SeqCst), 0);
        } else {
            assert_success(result?).await?;
            let requests = paused.requests.lock().unwrap();
            assert_eq!(requests.len(), 2);
            assert_eq!(
                requests
                    .iter()
                    .map(|r| r.headers[http::header::AUTHORIZATION].to_str().unwrap())
                    .collect::<Vec<_>>(),
                vec!["Bearer external-initial", "Bearer external-rotated"]
            );
            assert_eq!(wire_body(&requests[0]), wire_body(&requests[1]));
            assert_eq!(source.0.load(Ordering::SeqCst), 1);
        }
    }
    Ok(())
}

#[tokio::test]
async fn lazy_command_bearer_materializes_on_first_real_stream_request() -> anyhow::Result<()> {
    let home = tempfile::tempdir()?;
    std::fs::write(home.path().join("token.txt"), "command-test-token")?;
    let command = ModelProviderAuthInfo {
        command: if cfg!(windows) { "cmd.exe" } else { "cat" }.to_string(),
        args: if cfg!(windows) {
            vec!["/D", "/C", "type", "token.txt"]
        } else {
            vec!["token.txt"]
        }
        .into_iter()
        .map(Into::into)
        .collect(),
        timeout_ms: std::num::NonZeroU64::new(5_000).unwrap(),
        refresh_interval_ms: 60_000,
        cwd: home.path().try_into()?,
    };
    let mut client = test_model_client(SessionSource::Cli);
    let state = Arc::get_mut(&mut client.state).unwrap();
    state.provider = create_model_provider(
        ModelProviderInfo {
            auth: Some(command),
            ..create_oss_provider_with_base_url(
                "https://custom.example.test/v1",
                WireApi::Responses,
            )
        },
        /*auth_manager*/ None,
    );
    let manager = state.provider.auth_manager().unwrap();
    assert!(manager.has_external_auth());
    assert!(manager.auth_cached().is_none());
    let requests = Arc::new(Mutex::new(Vec::new()));
    let captured = requests.clone();
    let success = success_reply();
    state.test_transport = Some(crate::client::test_transport::StreamHook::new(
        move |request| {
            let captured = captured.clone();
            let success = success.clone();
            async move {
                captured.lock().unwrap().push(request.clone());
                success.stream(request).await
            }
        },
    ));
    let result = tokio::time::timeout(Duration::from_secs(5), start_request(&client)).await?;
    assert_success(result?).await?;
    let requests = requests.lock().unwrap();
    assert_eq!(requests.len(), 1);
    assert_eq!(
        requests[0].headers[http::header::AUTHORIZATION],
        "Bearer command-test-token"
    );
    assert_eq!(requests[0].url, "https://custom.example.test/v1/responses");
    assert_eq!(manager.auth_mode(), Some(AuthMode::ApiKey));
    Ok(())
}
