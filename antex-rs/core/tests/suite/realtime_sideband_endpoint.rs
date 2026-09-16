//! Verifies that existing-call sidebands honor trusted per-call endpoint overrides,
//! fall back to the configured endpoint, and preserve runtime authentication without
//! adding bearer credentials.

use antex_login::AntexAuth;
use antex_login::AuthHeaders;
use antex_protocol::protocol::CodexResponseHandoffMode;
use antex_protocol::protocol::ConversationStartParams;
use antex_protocol::protocol::ConversationStartTransport;
use antex_protocol::protocol::EventMsg;
use antex_protocol::protocol::Op;
use antex_protocol::protocol::RealtimeConversationVersion;
use antex_protocol::protocol::RealtimeOutputModality;
use anyhow::Result;
use core_test_support::responses::start_mock_server;
use core_test_support::responses::start_websocket_server;
use core_test_support::skip_if_no_network;
use core_test_support::test_antex::test_antex;
use core_test_support::wait_for_event;
use core_test_support::wait_for_event_match;
use http::HeaderMap;
use http::HeaderValue;
use pretty_assertions::assert_eq;
use serde_json::json;
use test_case::test_case;

#[derive(Clone, Copy)]
enum EndpointSelection {
    Configured,
    PerCall,
}

#[test_case(EndpointSelection::Configured; "configured endpoint")]
#[test_case(EndpointSelection::PerCall; "per-call endpoint")]
#[tokio::test(flavor = "multi_thread", worker_threads = 2)]
async fn existing_call_uses_selected_endpoint_and_runtime_auth(
    selection: EndpointSelection,
) -> Result<()> {
    skip_if_no_network!(Ok(()));

    let server = start_mock_server().await;
    let events = vec![vec![vec![json!({
        "type": "session.started",
        "session": { "id": "rtc_existing" }
    })]]];
    let configured_endpoint = start_websocket_server(events.clone()).await;
    let call_endpoint = start_websocket_server(events).await;
    let configured_base_url = configured_endpoint.uri().to_string();
    let mut headers = HeaderMap::new();
    headers.insert(
        "x-runtime-auth",
        HeaderValue::from_static("runtime-credential"),
    );
    let mut builder = test_antex()
        .with_auth(AntexAuth::Headers(AuthHeaders::new(headers)))
        .with_config(move |config| {
            config.experimental_realtime_ws_base_url = Some(configured_base_url);
        });
    let test = builder.build_with_auto_env(&server).await?;

    test.antex
        .submit(Op::RealtimeConversationStart(ConversationStartParams {
            client_managed_handoffs: false,
            delegation_ack_filler: None,
            flush_transcript_tail_on_session_end: false,
            codex_responses_as_items: false,
            codex_response_item_prefix: None,
            codex_response_handoff_mode: CodexResponseHandoffMode::Thinking,
            codex_response_handoff_channel_prefixes: None,
            model: None,
            output_modality: RealtimeOutputModality::Audio,
            include_startup_context: false,
            initial_items: Vec::new(),
            realtime_start_instructions: None,
            realtime_end_instructions: None,
            prompt: None,
            realtime_session_id: None,
            transport: Some(ConversationStartTransport::ExistingCall {
                call_id: "rtc_existing".to_string(),
                sideband_base_url: match selection {
                    EndpointSelection::Configured => None,
                    EndpointSelection::PerCall => Some(call_endpoint.uri().to_string()),
                },
            }),
            version: Some(RealtimeConversationVersion::V3),
            voice: None,
        }))
        .await?;

    wait_for_event_match(&test.antex, |event| match event {
        EventMsg::RealtimeConversationStarted(_) => Some(Ok(())),
        EventMsg::Error(error) => Some(Err(anyhow::anyhow!("{error:?}"))),
        _ => None,
    })
    .await?;

    let (selected, unused) = match selection {
        EndpointSelection::Configured => (&configured_endpoint, &call_endpoint),
        EndpointSelection::PerCall => (&call_endpoint, &configured_endpoint),
    };
    let handshake = selected.single_handshake();
    assert_eq!(
        (
            handshake.uri(),
            handshake.header("x-runtime-auth"),
            handshake.header("authorization"),
        ),
        (
            "/v1/live/rtc_existing",
            Some("runtime-credential".to_string()),
            None,
        ),
    );
    assert!(unused.handshakes().is_empty());

    test.antex.submit(Op::RealtimeConversationClose).await?;
    wait_for_event(&test.antex, |event| {
        matches!(event, EventMsg::RealtimeConversationClosed(_))
    })
    .await;
    test.antex.shutdown_and_wait().await?;
    configured_endpoint.shutdown().await;
    call_endpoint.shutdown().await;
    Ok(())
}
