//! Whole-turn regressions: real dispatch, held dynamic tool, and automatic continuation.
use super::*;
use antex_protocol::dynamic_tools::DynamicToolCallOutputContentItem;
use antex_protocol::dynamic_tools::DynamicToolFunctionSpec;
use antex_protocol::dynamic_tools::DynamicToolResponse;
use antex_protocol::dynamic_tools::DynamicToolSpec;
use base64::Engine;
use pretty_assertions::assert_eq;

const PRIVATE_INPUT: &str = "private input belonging to the original automatic turn";
const PRIVATE_OUTPUT: &str = "private held-tool output belonging to A";
const CALL_ID: &str = "owner-held-tool";

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
enum Replacement {
    Registration,
    SameWorkspaceDifferentUser,
    SameOwnerRotation,
}

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
enum Continuation {
    Sampling,
    LocalCompaction,
}

#[derive(Clone, Copy, Debug)]
enum Start {
    Cold,
    Prewarmed,
}

struct ToolScript {
    requests: StdMutex<Vec<Request>>,
}

impl ToolScript {
    async fn send(&self, request: Request) -> Result<StreamResponse, TransportError> {
        let count = {
            let mut requests = self.requests.lock().unwrap();
            requests.push(request);
            requests.len()
        };
        let events = if count == 1 {
            vec![
                ev_response_created("resp_tool"),
                ev_function_call(CALL_ID, "held_tool", "{}"),
                ev_completed("resp_tool"),
            ]
        } else {
            let id = format!("resp_{count}");
            vec![
                ev_response_created(&id),
                ev_assistant_message(&format!("msg_{count}"), "accepted"),
                ev_completed(&id),
            ]
        };
        let bytes = sse(events).into_bytes();
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

struct ToolRig {
    home: tempfile::TempDir,
    session: Arc<Session>,
    turn: Arc<TurnContext>,
    events: async_channel::Receiver<antex_protocol::protocol::Event>,
    script: Arc<ToolScript>,
}

fn save_tool_grant(
    home: &std::path::Path,
    registration: &str,
    token: &str,
    user: Option<&str>,
) -> anyhow::Result<()> {
    save_grant(home, registration, token)?;
    if let Some(user) = user {
        // Only the fixture's explicit temporary home is read/written. Same
        // workspace and registration deliberately do not conceal a changed user.
        let path = home.join("auth.json");
        let mut grant: serde_json::Value = serde_json::from_slice(&std::fs::read(&path)?)?;
        let claims = json!({"https://api.openai.com/auth": {
            "chatgpt_account_id": "shared-workspace", "chatgpt_user_id": user
        }});
        let claims =
            base64::engine::general_purpose::URL_SAFE_NO_PAD.encode(serde_json::to_vec(&claims)?);
        grant["tokens"]["id_token"] = json!(format!("header.{claims}.signature"));
        grant["tokens"]["account_id"] = json!("shared-workspace");
        std::fs::write(path, serde_json::to_vec(&grant)?)?;
    }
    Ok(())
}

async fn tool_rig(replacement: Replacement) -> anyhow::Result<ToolRig> {
    let home = tempfile::tempdir()?;
    let user = (replacement == Replacement::SameWorkspaceDifferentUser).then_some("user-A");
    save_tool_grant(home.path(), "registration-A", "token-A", user)?;
    let auth = AntexAuth::from_auth_storage(
        home.path(),
        antex_login::AuthCredentialsStoreMode::File,
        /*chatgpt_base_url*/ None,
        antex_login::AuthKeyringBackendKind::default(),
        &antex_login::test_support::transport_default_auth_route_config(),
    )
    .await?
    .unwrap();
    if user.is_some() {
        assert_eq!(
            (
                auth.get_account_id().as_deref(),
                auth.get_chatgpt_user_id().as_deref()
            ),
            (Some("shared-workspace"), Some("user-A"))
        );
    }
    let tools = vec![DynamicToolSpec::Function(DynamicToolFunctionSpec {
        name: "held_tool".to_string(),
        description: "Held until the test explicitly replaces authentication".to_string(),
        input_schema: json!({"type": "object", "properties": {}}),
        defer_loading: false,
    })];
    let (mut session, turn, events) =
        crate::session::tests::make_session_and_context_with_auth_config_home_and_rx(
            auth,
            tools,
            home.path(),
            |config| {
                config.model_provider =
                    ModelProviderInfo::create_openai_provider(/*base_url*/ None);
                config.model_provider.request_max_retries = Some(0);
                config.model_provider.stream_max_retries = Some(1);
                config.model = Some("gpt-6-astra".to_string());
                config.base_instructions = Some("Test instructions".to_string());
                config.model_auto_compact_token_limit = Some(i64::MAX);
                config.model_post_turn_compact_threshold_percent = 0;
                for feature in [
                    Feature::TokenBudget,
                    Feature::CodeMode,
                    Feature::CodeModeOnly,
                ] {
                    let _ = config.features.disable(feature);
                }
            },
        )
        .await;
    let script = Arc::new(ToolScript {
        requests: StdMutex::new(Vec::new()),
    });
    let injected = Arc::clone(&script);
    Arc::get_mut(&mut session)
        .unwrap()
        .services
        .model_client
        .set_test_stream_transport(move |request| {
            let injected = Arc::clone(&injected);
            async move { injected.send(request).await }
        });
    // The fixture does not spawn a RegularTask. Install its normal pending-tool
    // storage, but call the actual run_turn rather than driving samplers by hand.
    *session.active_turn.lock().await = Some(crate::state::ActiveTurn::default());
    assert_eq!(
        turn.provider.capabilities().remote_compaction,
        RemoteCompactionSupport::Unsupported
    );
    Ok(ToolRig {
        home,
        session,
        turn,
        events,
        script,
    })
}

fn user_input(text: &str) -> Vec<TurnInput> {
    vec![TurnInput::UserInput {
        metadata: Default::default(),
        content: vec![UserInput::Text {
            text: text.to_string(),
            text_elements: Vec::new(),
        }],
        client_id: None,
    }]
}

fn request_body(request: &Request) -> anyhow::Result<serde_json::Value> {
    let bytes = request
        .prepare_body_for_send()
        .map_err(anyhow::Error::msg)?
        .body
        .unwrap();
    Ok(serde_json::from_slice(&bytes)?)
}

#[tokio::test]
async fn automatic_turn_keeps_owner_through_tool_loop_and_local_compaction() -> anyhow::Result<()> {
    for replacement in [
        Replacement::Registration,
        Replacement::SameWorkspaceDifferentUser,
        Replacement::SameOwnerRotation,
    ] {
        for continuation in [Continuation::Sampling, Continuation::LocalCompaction] {
            for start in [Start::Cold, Start::Prewarmed] {
                let rig = tool_rig(replacement).await?;
                let manager = &rig.session.services.auth_manager;
                let (prewarmed, next_prewarmed) = match start {
                    Start::Cold => (None, None),
                    Start::Prewarmed => {
                        // Direct mode has no websocket warmup. Exercise auth prewarm
                        // and supply sessions created under A, even to the NEW B turn.
                        rig.session.services.model_client.prewarm_auth().await?;
                        (
                            Some(rig.session.services.model_client.new_session()),
                            Some(rig.session.services.model_client.new_session()),
                        )
                    }
                };
                let mut requirements = McpStartupRequirements::default();
                let turn = run_turn(
                    Arc::clone(&rig.session),
                    Arc::clone(&rig.turn),
                    user_input(PRIVATE_INPUT),
                    &mut requirements,
                    prewarmed,
                    CancellationToken::new(),
                );
                tokio::pin!(turn);
                let mut observed = Vec::new();
                tokio::time::timeout(Duration::from_secs(5), async {
                    loop {
                        tokio::select! {
                            biased;
                            event = rig.events.recv() => {
                                let event = event?.msg;
                                let held = matches!(&event, EventMsg::DynamicToolCallRequest(request)
                                    if request.call_id == CALL_ID);
                                observed.push(event);
                                if held { break; }
                            }
                            result = &mut turn => panic!("turn ended before held tool: {result:?}"),
                        }
                    }
                    Ok::<_, anyhow::Error>(())
                }).await??;
                assert_eq!(rig.script.requests.lock().unwrap().len(), 1);
                let registration = if replacement == Replacement::Registration {
                    "registration-B"
                } else {
                    "registration-A"
                };
                let user =
                    (replacement == Replacement::SameWorkspaceDifferentUser).then_some("user-B");
                save_tool_grant(rig.home.path(), registration, "installed-token", user)?;
                manager.reload().await;
                // This must be an accepted EXPLICIT install, not an automatic
                // producer rejection being mistaken for consumer owner enforcement.
                assert_eq!(
                    manager.auth_cached().unwrap().get_token()?,
                    "installed-token"
                );
                if user.is_some() {
                    let installed = manager.auth_cached().unwrap();
                    assert_eq!(
                        (
                            installed.get_account_id().as_deref(),
                            installed.get_chatgpt_user_id().as_deref()
                        ),
                        (Some("shared-workspace"), user)
                    );
                }
                if continuation == Continuation::LocalCompaction {
                    // Real run_turn rollover branch, with no steer/second user input.
                    rig.session.request_new_context_window().await;
                }
                rig.session
                    .notify_dynamic_tool_response(
                        CALL_ID,
                        DynamicToolResponse {
                            content_items: vec![DynamicToolCallOutputContentItem::InputText {
                                text: PRIVATE_OUTPUT.to_string(),
                            }],
                            success: true,
                        },
                    )
                    .await;
                let result = tokio::time::timeout(Duration::from_secs(5), &mut turn).await??;
                while let Ok(event) = rig.events.try_recv() {
                    observed.push(event.msg);
                }
                let errors = observed
                    .iter()
                    .filter_map(|event| match event {
                        EventMsg::Error(error) => Some(error.clone()),
                        _ => None,
                    })
                    .collect::<Vec<_>>();
                assert!(
                    !observed
                        .iter()
                        .any(|event| matches!(event, EventMsg::StreamError(_))),
                    "owner change is terminal, not a retry: {replacement:?} {continuation:?} {start:?}"
                );
                let compact_started = observed.iter().any(|event| matches!(event,
                    EventMsg::ItemStarted(event) if matches!(&event.item, TurnItem::ContextCompaction(_))));
                assert_eq!(
                    compact_started,
                    continuation == Continuation::LocalCompaction
                );
                let rotated = replacement == Replacement::SameOwnerRotation;
                if rotated {
                    assert_eq!((result, errors), (Some("accepted".to_string()), Vec::new()));
                    assert_eq!(*rig.turn.terminal_error.lock().await, None);
                } else {
                    let error = CodexErr::RefreshTokenFailed(
                        antex_protocol::auth::RefreshTokenFailedError::new(
                            antex_protocol::auth::RefreshTokenFailedReason::Other,
                            "Authentication owner changed (account changed or authentication source replaced); restart the request after signing in",
                        ),
                    );
                    let expected = error.to_error_event(/*message_prefix*/ None);
                    assert_eq!((result, errors), (None, vec![expected.clone()]));
                    assert_eq!(*rig.turn.terminal_error.lock().await, Some(expected));
                    let history = rig.session.clone_history().await;
                    assert!(
                        serde_json::to_string(&history.raw_items().collect::<Vec<_>>())?
                            .contains(PRIVATE_OUTPUT)
                    );
                }
                let old_count = {
                    let requests = rig.script.requests.lock().unwrap();
                    let expected_count = if rotated {
                        if continuation == Continuation::LocalCompaction {
                            3
                        } else {
                            2
                        }
                    } else {
                        1
                    };
                    assert_eq!(
                        requests.len(),
                        expected_count,
                        "{replacement:?} {continuation:?} {start:?}: no B dispatch for the old turn"
                    );
                    assert!(
                        requests
                            .iter()
                            .all(|request| request.url == "https://api.openai.com/v1/responses")
                    );
                    assert_eq!(
                        requests[0].headers[http::header::AUTHORIZATION],
                        "Bearer token-A"
                    );
                    assert!(
                        request_body(&requests[0])?["input"]
                            .to_string()
                            .contains(PRIVATE_INPUT)
                    );
                    if rotated {
                        let followup = request_body(&requests[1])?;
                        assert!(followup["input"].to_string().contains(PRIVATE_INPUT));
                        assert!(followup["input"].to_string().contains(PRIVATE_OUTPUT));
                        assert!(
                            requests[1..]
                                .iter()
                                .all(|request| request.headers[http::header::AUTHORIZATION]
                                    == "Bearer installed-token")
                        );
                        if continuation == Continuation::LocalCompaction {
                            assert!(followup["input"].as_array().unwrap().contains(
                                &serde_json::to_value(user_message_item(
                                    crate::compact::SUMMARIZATION_PROMPT
                                ))?
                            ));
                        }
                    }
                    requests.len()
                };
                // A genuinely new run_turn may bind B. No history rewriting is
                // required, and a session prepared under A must not pin it forever.
                save_tool_grant(rig.home.path(), "registration-B", "new-turn-B", user)?;
                manager.reload().await;
                assert_eq!(manager.auth_cached().unwrap().get_token()?, "new-turn-B");
                let next_context = rig.session.new_default_turn().await;
                let mut next_requirements = McpStartupRequirements::default();
                let next = tokio::time::timeout(
                    Duration::from_secs(5),
                    run_turn(
                        Arc::clone(&rig.session),
                        next_context,
                        user_input("new user turn for B"),
                        &mut next_requirements,
                        next_prewarmed,
                        CancellationToken::new(),
                    ),
                )
                .await??;
                assert_eq!(next, Some("accepted".to_string()));
                while let Ok(event) = rig.events.try_recv() {
                    assert!(!matches!(
                        event.msg,
                        EventMsg::Error(_) | EventMsg::StreamError(_)
                    ));
                }
                let requests = rig.script.requests.lock().unwrap();
                assert_eq!(requests.len(), old_count + 1);
                let last = requests.last().unwrap();
                assert_eq!(
                    (
                        last.url.as_str(),
                        last.headers[http::header::AUTHORIZATION].to_str()?
                    ),
                    ("https://api.openai.com/v1/responses", "Bearer new-turn-B")
                );
                assert!(
                    request_body(last)?["input"]
                        .to_string()
                        .contains("new user turn for B")
                );
            }
        }
    }
    Ok(())
}
