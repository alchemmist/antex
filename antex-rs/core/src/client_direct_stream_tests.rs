//! Positive public Responses exchange through the real ModelClientSession::stream path.
//! Only network I/O is scripted; route setup, auth, request construction and decoding are real.
use super::*;
use antex_http_client::HttpTransport;
use antex_http_client::Request;
use antex_http_client::Response;
use antex_http_client::StreamResponse;
use http::HeaderMap;
use http::HeaderValue;
use http::Method;
use http::StatusCode;
use pretty_assertions::assert_eq;
use serde_json::Value;

#[derive(Default)]
struct ScriptState {
    replies: VecDeque<String>,
    requests: Vec<Request>,
}

#[derive(Clone)]
pub(super) struct ScriptedSse(Arc<Mutex<ScriptState>>);

impl ScriptedSse {
    pub(super) fn new(replies: impl IntoIterator<Item = Vec<Value>>) -> Self {
        Self(Arc::new(Mutex::new(ScriptState {
            replies: replies
                .into_iter()
                .map(|events| {
                    events
                        .into_iter()
                        .map(|event| format!("data: {event}\n\n"))
                        .collect()
                })
                .collect(),
            requests: Vec::new(),
        })))
    }
}

impl HttpTransport for ScriptedSse {
    async fn execute(&self, _request: Request) -> Result<Response, TransportError> {
        panic!("public Responses must use streaming HTTP");
    }

    async fn stream(&self, request: Request) -> Result<StreamResponse, TransportError> {
        let payload = {
            let mut state = self.0.lock().unwrap();
            state.requests.push(request);
            state
                .replies
                .pop_front()
                .expect("unexpected extra inference request")
        };
        // Single-byte chunks split JSON, SSE delimiters and the UTF-8 checkmark deliberately.
        let bytes = payload
            .into_bytes()
            .into_iter()
            .map(|byte| Ok(vec![byte].into()));
        Ok(StreamResponse {
            status: StatusCode::OK,
            headers: HeaderMap::from_iter([
                (
                    http::header::CONTENT_TYPE,
                    HeaderValue::from_static("text/event-stream"),
                ),
                (
                    http::HeaderName::from_static("openai-model"),
                    HeaderValue::from_static("gpt-6-astra"),
                ),
            ]),
            bytes: futures::stream::iter(bytes).boxed(),
        })
    }
}

#[derive(Debug, Default, PartialEq)]
struct DecodedExchange {
    created: Vec<Option<String>>,
    models: Vec<String>,
    rate_limits: Vec<antex_protocol::protocol::RateLimitSnapshot>,
    deltas: Vec<String>,
    added: Vec<ResponseItem>,
    done: Vec<ResponseItem>,
    completed: Vec<(String, Option<bool>)>,
}

#[tokio::test]
async fn direct_public_sse_tool_round_trip_preserves_reasoning_and_managed_identity()
-> anyhow::Result<()> {
    let (home, mut client, manager) = direct_client().await?;
    // Even when Codex extensions are enabled, a direct request must stay in the public dialect.
    let state = Arc::get_mut(&mut client.state).unwrap();
    state.output_token_replay_enabled = true;
    state.concurrent_reasoning_summaries_enabled = true;
    state.reasoning_effort_override_enabled = true;
    let provider = client.state.provider.clone();
    let model = antex_models_manager::bundled_models_response()?
        .models
        .into_iter()
        .find(|model| model.slug == "gpt-6-astra")
        .expect("bundled Astra metadata");
    let identity = || {
        antex_model_provider::test_support::models_cache_entry(
            provider.info(),
            manager.auth_cached().as_ref(),
            Vec::new(),
        )
        .identity
    };
    let initial_identity = identity().expect("direct grant cache identity");
    let initial_owner = client.auth_owner_generation();
    let original_prompt = serde_json::from_value(json!({
        "type": "message", "role": "user",
        "content": [{"type": "input_text", "text": "Look up the answer."}]
    }))?;
    let mut prompt = Prompt {
        input: vec![original_prompt],
        tools: vec![ToolSpec::Function(ResponsesApiTool {
            name: "lookup".to_string(),
            description: "Look up the answer".to_string(),
            strict: false,
            defer_loading: None,
            parameters: JsonSchema::default(),
            output_schema: None,
        })]
        .into(),
        parallel_tool_calls: true,
        base_instructions: BaseInstructions {
            text: String::new(),
            ..Default::default()
        },
        ..Default::default()
    };
    let reasoning_json = json!({
        "type": "reasoning", "id": "rs_public_1", "summary": [],
        "encrypted_content": "opaque-public-reasoning"
    });
    let call_json = json!({
        "type": "function_call", "id": "fc_public_1", "call_id": "call_lookup_1",
        "name": "lookup", "arguments": "{}"
    });
    let message_added_json = json!({
        "type": "message", "id": "msg_public_2", "role": "assistant", "content": []
    });
    let message_done_json = json!({
        "type": "message", "id": "msg_public_2", "role": "assistant",
        "content": [{"type": "output_text", "text": "Found ✓"}]
    });
    let expected_first_items: Vec<ResponseItem> = vec![
        serde_json::from_value(reasoning_json.clone())?,
        serde_json::from_value(call_json.clone())?,
    ];
    let expected_final_item: ResponseItem = serde_json::from_value(message_done_json.clone())?;
    let transport = ScriptedSse::new([
        vec![
            json!({"type": "response.created", "response": {"id": "resp_tools"}}),
            json!({"type": "response.output_item.done", "item": reasoning_json}),
            json!({"type": "response.output_item.done", "item": call_json}),
            json!({"type": "response.completed", "response": {"id": "resp_tools", "end_turn": false}}),
        ],
        vec![
            json!({"type": "response.created", "response": {"id": "resp_answer"}}),
            json!({"type": "response.output_item.added", "item": message_added_json.clone()}),
            json!({"type": "response.output_text.delta", "delta": "Found "}),
            json!({"type": "response.output_text.delta", "delta": "✓"}),
            json!({"type": "response.output_item.done", "item": message_done_json}),
            json!({"type": "response.completed", "response": {"id": "resp_answer", "end_turn": true}}),
        ],
    ]);
    let injected = transport.clone();
    Arc::get_mut(&mut client.state).unwrap().test_transport = Some(
        crate::client::test_transport::StreamHook::new(move |request| {
            let injected = injected.clone();
            async move { injected.stream(request).await }
        }),
    );
    let mut session = client.new_session();
    let metadata = test_responses_metadata_for_client(
        &client,
        Some("turn_direct"),
        format!("{}:0", client.state.thread_id),
        /*parent_thread_id*/ None,
        TestAntexResponsesRequestKind::Turn,
    );
    let mut expected_inputs = Vec::new();
    for expected in [
        DecodedExchange {
            created: vec![Some("resp_tools".to_string())],
            models: vec!["gpt-6-astra".to_string()],
            rate_limits: vec![empty_response_rate_limits()],
            done: expected_first_items.clone(),
            completed: vec![("resp_tools".to_string(), Some(false))],
            ..Default::default()
        },
        DecodedExchange {
            created: vec![Some("resp_answer".to_string())],
            models: vec!["gpt-6-astra".to_string()],
            rate_limits: vec![empty_response_rate_limits()],
            deltas: vec!["Found ".to_string(), "✓".to_string()],
            added: vec![serde_json::from_value(message_added_json.clone())?],
            done: vec![expected_final_item],
            completed: vec![("resp_answer".to_string(), Some(true))],
        },
    ] {
        let setup = client
            .current_client_setup(crate::client::ClientRouting::Workspace)
            .await?;
        assert!(matches!(
            setup.redirect_policy,
            antex_login::default_client::ClientRedirectPolicy::Reject
        ));
        assert!(!client.responses_websocket_enabled());
        let include_internal = provider.include_internal_metadata(&setup.api_provider);
        assert!(!include_internal);
        expected_inputs.push(serde_json::to_value(&prompt.input)?);
        assert!(
            setup
                .api_auth
                .apply_auth(Request::new(
                    Method::POST,
                    "https://api.openai.com/v1/responses/compact".to_string(),
                ))
                .await
                .is_err()
        );
        let mut stream = session
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
            .await?;
        let decoded = tokio::time::timeout(Duration::from_secs(5), async {
            let mut decoded = DecodedExchange::default();
            while let Some(event) = stream.next().await {
                match event? {
                    ResponseEvent::Created { response_id } => decoded.created.push(response_id),
                    ResponseEvent::ServerModel(model) => decoded.models.push(model),
                    ResponseEvent::RateLimits(limits) => decoded.rate_limits.push(limits),
                    ResponseEvent::OutputTextDelta(delta) => decoded.deltas.push(delta),
                    ResponseEvent::OutputItemAdded(item) => decoded.added.push(item),
                    ResponseEvent::OutputItemDone(item) => decoded.done.push(item),
                    ResponseEvent::Completed {
                        response_id,
                        end_turn,
                        token_usage,
                        usage_metadata,
                    } => {
                        assert!(token_usage.is_none() && usage_metadata.is_none());
                        decoded.completed.push((response_id, end_turn));
                    }
                    other => panic!("unexpected event in public SSE fixture: {other:?}"),
                }
            }
            Ok::<_, anyhow::Error>(decoded)
        })
        .await??;
        assert_eq!(decoded, expected);
        if decoded.completed[0].1 == Some(false) {
            // Feed the actually decoded call and reasoning back, rather than fixture copies.
            let ResponseItem::FunctionCall {
                name,
                arguments,
                call_id,
                ..
            } = &decoded.done[1]
            else {
                panic!("expected the decoded lookup function call");
            };
            assert_eq!(
                (name.as_str(), serde_json::from_str::<Value>(arguments)?),
                ("lookup", json!({}))
            );
            let output = serde_json::from_value(json!({
                "type": "function_call_output", "call_id": call_id, "output": "42"
            }))?;
            prompt.input.extend(decoded.done);
            prompt.input.push(output);
            save_direct_grant(
                home.path(),
                "test-issued-client",
                "test-direct-access-rotated",
            )?;
            manager.reload().await;
            assert_eq!(
                (identity(), client.auth_owner_generation()),
                (Some(initial_identity.clone()), initial_owner)
            );
        }
    }
    let state = transport.0.lock().unwrap();
    assert!(state.replies.is_empty());
    assert_eq!(state.requests.len(), 2);
    let mut bodies = Vec::new();
    for (request, token) in state
        .requests
        .iter()
        .zip(["test-direct-access", "test-direct-access-rotated"])
    {
        assert_eq!(
            (&request.method, request.url.as_str()),
            (&Method::POST, "https://api.openai.com/v1/responses")
        );
        assert_eq!(
            request.headers[http::header::AUTHORIZATION],
            format!("Bearer {token}")
        );
        assert!(request.headers[http::header::AUTHORIZATION].is_sensitive());
        assert_eq!(request.headers[http::header::ACCEPT], "text/event-stream");
        let backend_headers = request
            .headers
            .keys()
            .map(http::HeaderName::as_str)
            .filter(|name| {
                name.starts_with("x-codex-")
                    || name.starts_with("x-openai-internal-")
                    || [
                        "chatgpt-account-id",
                        "x-openai-fedramp",
                        "x-oai-attestation",
                        "openai-beta",
                        "session_id",
                        "conversation_id",
                    ]
                    .contains(name)
            })
            .collect::<Vec<_>>();
        assert_eq!(backend_headers, Vec::<&str>::new());
        assert_eq!(
            request.compression,
            antex_http_client::RequestCompression::None
        );
        let prepared = request
            .prepare_body_for_send()
            .map_err(anyhow::Error::msg)?;
        bodies.push(serde_json::from_slice::<Value>(
            &prepared.body.expect("request body"),
        )?);
    }
    assert_eq!(bodies[0]["model"], "gpt-6-astra");
    assert_eq!(bodies[0]["include"], json!(["reasoning.encrypted_content"]));
    assert_eq!(
        (bodies[0]["stream"].clone(), bodies[0]["store"].clone()),
        (json!(true), json!(false))
    );
    assert_eq!(
        bodies[0]["tools"],
        json!([{
            "type": "function", "name": "lookup", "description": "Look up the answer",
            "strict": false, "parameters": serde_json::to_value(JsonSchema::default())?
        }])
    );
    for (body, input) in bodies.iter().zip(expected_inputs) {
        assert_eq!(body["input"], input);
        for field in [
            "client_metadata",
            "access_programs",
            "stream_options",
            "previous_response_id",
        ] {
            assert!(
                body.get(field).is_none(),
                "unexpected Codex framing field {field}"
            );
        }
        assert!(body["reasoning"].get("context").is_none());
    }
    let mut expected_second_body = bodies[0].clone();
    expected_second_body["input"] = serde_json::to_value(&prompt.input)?;
    assert_eq!(bodies[1], expected_second_body);
    assert_eq!(
        bodies[1]["input"][1]["encrypted_content"],
        "opaque-public-reasoning"
    );
    assert_eq!(
        bodies[1]["input"][3],
        json!({"type": "function_call_output", "call_id": "call_lookup_1", "output": "42"})
    );
    Ok(())
}
