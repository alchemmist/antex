//! Standalone sampling ownership across dispatch failure, backoff and WS fallback.
//! Production nests these scopes under run_turn; whole-turn regressions live below.
#[path = "turn_loop_owner_tests.rs"]
mod turn_loop;
use super::*;
use antex_http_client::Request;
use antex_http_client::RetryAfter;
use antex_http_client::StreamResponse;
use antex_http_client::TransportError;
use antex_model_provider_info::ModelProviderInfo;
use antex_protocol::auth::AuthMode;
use core_test_support::responses::*;
use http::HeaderMap;
use http::StatusCode;
use pretty_assertions::assert_eq;
use serde_json::json;
use std::sync::Mutex as StdMutex;
use std::time::Duration;
use tokio::io::AsyncReadExt;
use tokio::io::AsyncWriteExt;
use tokio::sync::Notify;

#[derive(Clone, Copy, Debug)]
enum Failure {
    Network,
    Server,
}
#[derive(Clone, Copy, Debug)]
enum SwitchAt {
    Dispatch,
    Backoff,
}

struct Script {
    failure: Option<Failure>,
    started: Notify,
    release: Notify,
    requests: StdMutex<Vec<Request>>,
}

impl Script {
    async fn send(&self, request: Request) -> Result<StreamResponse, TransportError> {
        let request_count = {
            let mut requests = self.requests.lock().unwrap();
            requests.push(request);
            requests.len()
        };
        if request_count == 1
            && let Some(failure) = self.failure
        {
            self.started.notify_one();
            self.release.notified().await;
            return Err(match failure {
                Failure::Network => TransportError::Network("scripted disconnect".to_string()),
                Failure::Server => TransportError::Http {
                    status: StatusCode::INTERNAL_SERVER_ERROR,
                    retry_after: RetryAfter::from_delay(Duration::from_secs(1)),
                    url: None,
                    headers: None,
                    body: None,
                },
            });
        }
        let response_id = format!("resp_accepted_{request_count}");
        let message_id = format!("msg_accepted_{request_count}");
        let bytes = sse(vec![
            ev_response_created(&response_id),
            ev_message_item_added(&message_id, ""),
            ev_output_text_delta("accepted"),
            ev_assistant_message(&message_id, "accepted"),
            ev_completed(&response_id),
        ])
        .into_bytes();
        Ok(StreamResponse {
            status: StatusCode::OK,
            headers: HeaderMap::from_iter([(
                http::header::CONTENT_TYPE,
                http::HeaderValue::from_static("text/event-stream"),
            )]),
            bytes: futures::stream::once(async move { Ok(bytes.into()) }).boxed(),
        })
    }
}

struct Rig {
    home: tempfile::TempDir,
    session: Arc<Session>,
    step: Arc<StepContext>,
    events: async_channel::Receiver<antex_protocol::protocol::Event>,
    script: Arc<Script>,
}

fn save_grant(home: &std::path::Path, registration: &str, token: &str) -> anyhow::Result<()> {
    std::fs::write(
        home.join("auth.json"),
        serde_json::to_vec(&json!({
            "auth_mode": "chatgptDirect",
            "tokens": {
                "id_token": "header.e30.signature", "access_token": token,
                "refresh_token": "fake-refresh",
                "direct": {"client_id": registration, "expires_at": "2099-01-01T00:00:00Z", "scopes": ["chatgpt.tokens.use.direct"]}
            },
            "last_refresh": "2099-01-01T00:00:00Z"
        }))?,
    )?;
    Ok(())
}

async fn rig(
    mut provider: ModelProviderInfo,
    mode: AuthMode,
    failure: Option<Failure>,
) -> anyhow::Result<Rig> {
    let home = tempfile::tempdir()?;
    save_grant(home.path(), "registration-A", "token-A")?;
    let auth = if mode == AuthMode::ChatgptDirect {
        AntexAuth::from_auth_storage(
            home.path(),
            antex_login::AuthCredentialsStoreMode::File,
            /*chatgpt_base_url*/ None,
            antex_login::AuthKeyringBackendKind::default(),
            &antex_login::test_support::transport_default_auth_route_config(),
        )
        .await?
        .unwrap()
    } else {
        AntexAuth::create_dummy_chatgpt_auth_for_testing()
    };
    // Isolate the SAMPLER retry budget, not the HTTP library's inner retries.
    provider.request_max_retries = Some(0);
    provider.stream_max_retries = Some(1);
    let (mut session, turn, events) =
        crate::session::tests::make_session_and_context_with_auth_config_home_and_rx(
            auth,
            Vec::new(),
            home.path(),
            move |config| {
                config.model_provider = provider;
                config.model = Some("gpt-6-astra".to_string());
                config.base_instructions = Some("Test instructions".to_string());
            },
        )
        .await;
    let script = Arc::new(Script {
        failure,
        started: Notify::new(),
        release: Notify::new(),
        requests: StdMutex::new(Vec::new()),
    });
    let injected = script.clone();
    Arc::get_mut(&mut session)
        .unwrap()
        .services
        .model_client
        .set_test_stream_transport(move |request| {
            let injected = injected.clone();
            async move { injected.send(request).await }
        });
    let step = StepContext::for_test(turn);
    // Real sampler retries rebuild input from history, not from a copied test request.
    session
        .record_conversation_items(
            &step.turn,
            &step.settings.model_info,
            &[user_message_item("private original input")],
        )
        .await;
    Ok(Rig {
        home,
        session,
        step,
        events,
        script,
    })
}

async fn sample(
    rig: &Rig,
    client: &mut ModelClientSession,
    text: &str,
) -> AntexResult<(SamplingRequestResult, Vec<ResponseItem>)> {
    run_sampling_request(
        rig.session.clone(),
        rig.step.clone(),
        Arc::new(antex_extension_api::ExtensionData::new("owner-test")),
        Arc::new(tokio::sync::Mutex::new(
            crate::turn_diff_tracker::TurnDiffTracker::new(),
        )),
        client,
        vec![user_message_item(text)],
        CancellationToken::new(),
    )
    .await
}

fn assert_owner_changed(error: &CodexErr) {
    match error.details() {
        CodexErrorDetails::RefreshTokenFailed(failure) => assert_eq!(
            failure,
            &antex_protocol::auth::RefreshTokenFailedError::new(
                antex_protocol::auth::RefreshTokenFailedReason::Other,
                "Authentication owner changed (account changed or authentication source replaced); restart the request after signing in",
            )
        ),
        other => panic!("expected terminal owner change, got {other:?}"),
    }
    assert!(error.retry_delay(/*retry_count*/ 1).is_none());
}

#[tokio::test]
async fn sampling_owner_survives_network_and_5xx_failures_and_backoff_but_not_new_operations()
-> anyhow::Result<()> {
    for failure in [Failure::Network, Failure::Server] {
        for when in [SwitchAt::Dispatch, SwitchAt::Backoff] {
            for registration in ["registration-B", "registration-A"] {
                let rig = rig(
                    ModelProviderInfo::create_openai_provider(/*base_url*/ None),
                    AuthMode::ChatgptDirect,
                    Some(failure),
                )
                .await?;
                let mut client = rig.session.services.model_client.new_session();
                let manager = &rig.session.services.auth_manager;
                {
                    let request = sample(&rig, &mut client, "private original input");
                    tokio::pin!(request);
                    tokio::time::timeout(Duration::from_secs(5), async {
                        tokio::select! {
                            _ = rig.script.started.notified() => {}
                            result = &mut request => panic!("sampling ended before dispatch: {result:?}"),
                        }
                    }).await?;
                    // Keep the real sampler future parked, not spawned in a separate retry task.
                    if matches!(when, SwitchAt::Backoff) {
                        tokio::time::pause();
                    }
                    if matches!(when, SwitchAt::Dispatch) {
                        save_grant(rig.home.path(), registration, "rotated-token")?;
                        manager.reload().await;
                    }
                    rig.script.release.notify_one();
                    if matches!(when, SwitchAt::Backoff) {
                        // Drive the ACTUAL sampler until its retry notification, then poll it
                        // into its sleep. The owner was still A when dispatch returned its error.
                        tokio::time::timeout(Duration::from_secs(5), async {
                            loop {
                                tokio::select! {
                                    biased;
                                    event = rig.events.recv() => {
                                        if matches!(event?.msg, EventMsg::StreamError(_)) { break; }
                                    }
                                    result = &mut request => panic!("expected sampler backoff: {result:?}"),
                                }
                            }
                            Ok::<_, anyhow::Error>(())
                        }).await??;
                        assert!(futures::poll!(&mut request).is_pending());
                        assert_eq!(rig.script.requests.lock().unwrap().len(), 1);
                        let parked_at = tokio::time::Instant::now();
                        save_grant(rig.home.path(), registration, "rotated-token")?;
                        manager.reload().await;
                        assert_eq!(
                            tokio::time::Instant::now(),
                            parked_at,
                            "switch occurred before retry deadline"
                        );
                        tokio::time::advance(Duration::from_secs(1)).await;
                        tokio::time::resume();
                    }
                    let result = tokio::time::timeout(Duration::from_secs(5), &mut request).await?;
                    if registration == "registration-B" {
                        assert_owner_changed(&result.unwrap_err());
                        assert_eq!(
                            rig.script.requests.lock().unwrap().len(),
                            1,
                            "{failure:?} {when:?}: no B dispatch"
                        );
                    } else {
                        let (output, input) = result?;
                        assert_eq!(
                            (output.needs_follow_up, output.last_agent_message, input),
                            (
                                false,
                                Some("accepted".to_string()),
                                vec![user_message_item("private original input")]
                            )
                        );
                        let requests = rig.script.requests.lock().unwrap();
                        assert_eq!(
                            requests.len(),
                            2,
                            "{failure:?} {when:?}: actual sampler must retry"
                        );
                        assert_eq!(
                            requests[1].headers[http::header::AUTHORIZATION],
                            "Bearer rotated-token"
                        );
                    }
                }
                assert_eq!(manager.auth_cached().unwrap().get_token()?, "rotated-token");
                {
                    let requests = rig.script.requests.lock().unwrap();
                    assert_eq!(
                        requests[0].headers[http::header::AUTHORIZATION],
                        "Bearer token-A"
                    );
                    assert!(
                        requests
                            .iter()
                            .all(|r| r.url == "https://api.openai.com/v1/responses")
                    );
                    let body = requests[0]
                        .prepare_body_for_send()
                        .map_err(anyhow::Error::msg)?
                        .body
                        .unwrap();
                    let body: serde_json::Value = serde_json::from_slice(&body)?;
                    assert!(
                        body["input"]
                            .as_array()
                            .unwrap()
                            .contains(&serde_json::to_value(user_message_item(
                                "private original input"
                            ))?)
                    );
                }
                // Both success and terminal failure must release the scope. Reuse the SAME
                // ModelClientSession for a genuinely new operation explicitly issued under B.
                let previous_count = rig.script.requests.lock().unwrap().len();
                save_grant(rig.home.path(), "registration-B", "new-operation-B")?;
                manager.reload().await;
                let (output, input) = tokio::time::timeout(
                    Duration::from_secs(5),
                    sample(&rig, &mut client, "new operation for B"),
                )
                .await??;
                assert_eq!(
                    (output.needs_follow_up, output.last_agent_message, input),
                    (
                        false,
                        Some("accepted".to_string()),
                        vec![user_message_item("new operation for B")]
                    )
                );
                let requests = rig.script.requests.lock().unwrap();
                assert_eq!(requests.len(), previous_count + 1);
                let new_request = requests.last().unwrap();
                assert_eq!(
                    new_request.headers[http::header::AUTHORIZATION],
                    "Bearer new-operation-B"
                );
                let body: serde_json::Value = serde_json::from_slice(
                    &new_request
                        .prepare_body_for_send()
                        .map_err(anyhow::Error::msg)?
                        .body
                        .unwrap(),
                )?;
                let original: serde_json::Value = serde_json::from_slice(
                    &requests[0]
                        .prepare_body_for_send()
                        .map_err(anyhow::Error::msg)?
                        .body
                        .unwrap(),
                )?;
                let mut expected_input = original["input"].as_array().unwrap().clone();
                assert_eq!(
                    expected_input.pop(),
                    Some(json!(user_message_item("private original input")))
                );
                expected_input.push(json!(user_message_item("new operation for B")));
                assert_eq!(body["input"], json!(expected_input));
            }
        }
    }
    Ok(())
}

#[tokio::test]
async fn legacy_426_handshake_cannot_fallback_under_new_owner_but_same_owner_can()
-> anyhow::Result<()> {
    for change_owner in [true, false] {
        let listener = tokio::net::TcpListener::bind("127.0.0.1:0").await?;
        // Existing legacy custom-provider configuration, not a new endpoint override or
        // direct-origin exemption. Authentication and the WS handshake are real.
        let base_url = format!("http://{}", listener.local_addr()?);
        let provider = ModelProviderInfo::create_openai_provider(Some(base_url.clone()));
        let rig = rig(provider, AuthMode::Chatgpt, /*failure*/ None).await?;
        let mut client = rig.session.services.model_client.new_session();
        let handshake = async {
            let (mut socket, _) = listener.accept().await?;
            let mut bytes = Vec::new();
            while !bytes.ends_with(b"\r\n\r\n") {
                bytes.push(socket.read_u8().await?);
                anyhow::ensure!(bytes.len() < 16_384, "unexpected handshake size");
            }
            let raw = String::from_utf8(bytes)?;
            let mut lines = raw.lines();
            assert_eq!(lines.next(), Some("GET /responses HTTP/1.1"));
            let mut headers = HeaderMap::new();
            for line in lines.filter(|line| !line.is_empty()) {
                let (name, value) = line.split_once(':').unwrap();
                headers.insert(
                    http::HeaderName::from_bytes(name.as_bytes())?,
                    http::HeaderValue::from_str(value.trim())?,
                );
            }
            assert_eq!(headers[http::header::AUTHORIZATION], "Bearer Access Token");
            assert_eq!(headers[http::header::UPGRADE], "websocket");
            // The client is suspended in the real handshake while the owner changes.
            if change_owner {
                save_grant(rig.home.path(), "registration-B", "must-not-send-B")?;
                rig.session.services.auth_manager.reload().await;
            }
            socket.write_all(b"HTTP/1.1 426 Upgrade Required\r\nContent-Length: 0\r\nConnection: close\r\n\r\n").await?;
            socket.shutdown().await?;
            Ok::<_, anyhow::Error>(())
        };
        let (result, server) = tokio::time::timeout(Duration::from_secs(5), async {
            tokio::join!(
                sample(&rig, &mut client, "private original input"),
                handshake
            )
        })
        .await?;
        server?;
        if change_owner {
            assert_owner_changed(&result.unwrap_err());
            assert!(
                rig.script.requests.lock().unwrap().is_empty(),
                "no HTTP fallback request under B"
            );
            assert_eq!(
                rig.session
                    .services
                    .auth_manager
                    .auth_cached()
                    .unwrap()
                    .get_token()?,
                "must-not-send-B"
            );
        } else {
            let (output, _) = result?;
            assert_eq!(
                (output.needs_follow_up, output.last_agent_message),
                (false, Some("accepted".to_string()))
            );
            let requests = rig.script.requests.lock().unwrap();
            assert_eq!(requests.len(), 1);
            assert_eq!(
                (
                    requests[0].url.as_str(),
                    requests[0].headers[http::header::AUTHORIZATION].to_str()?
                ),
                (
                    format!("{base_url}/responses").as_str(),
                    "Bearer Access Token"
                )
            );
        }
    }
    Ok(())
}
