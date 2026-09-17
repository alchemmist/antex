use super::*;
use antex_app_server_protocol::JSONRPCMessage;
use futures::SinkExt;
use futures::StreamExt;
use pretty_assertions::assert_eq;
use serde_json::json;
use tokio::net::TcpListener;
use tokio_tungstenite::tungstenite::Message;

#[tokio::test]
async fn fast_selection_updates_the_captured_live_turn_and_future_settings() -> Result<()> {
    for outcome in ["applied", "targetUnavailable"] {
        let mut app = make_test_app().await;
        let thread_id = ThreadId::new();
        let channel = ThreadEventChannel::new(/*capacity*/ 4);
        channel
            .store
            .lock()
            .await
            .set_active_turn_id("turn-1".to_string());
        app.thread_event_channels.insert(thread_id, channel);
        let listener = TcpListener::bind("127.0.0.1:0").await?;
        let endpoint = crate::resolve_remote_addr(&format!("ws://{}", listener.local_addr()?))?;
        let server = tokio::spawn(async move {
            let (stream, _) = listener.accept().await?;
            let mut socket = tokio_tungstenite::accept_async(stream).await?;
            let mut captured = Vec::new();
            while let Some(Ok(Message::Text(text))) = socket.next().await {
                let JSONRPCMessage::Request(request) = serde_json::from_str(&text)? else {
                    continue;
                };
                let result = match request.method.as_str() {
                    "initialize" => json!({"userAgent": "live-tier-test/1.0.0"}),
                    "thread/settings/update" => {
                        captured.push((request.method, request.params));
                        json!({})
                    }
                    "turn/settings/update" => {
                        captured.push((request.method, request.params));
                        json!({"status": outcome})
                    }
                    method => panic!("unexpected request: {method}"),
                };
                socket
                    .send(Message::Text(
                        json!({"id": request.id, "result": result})
                            .to_string()
                            .into(),
                    ))
                    .await?;
            }
            Ok::<_, color_eyre::Report>(captured)
        });
        let mut session = AppServerSession::new(
            crate::connect_remote_app_server(endpoint).await?,
            crate::app_server_session::ThreadParamsMode::Remote,
        );
        for tier in ["priority", "default"] {
            let op = AppCommand::OverrideTurnContext {
                cwd: None,
                approval_policy: None,
                approvals_reviewer: None,
                permission_profile: None,
                active_permission_profile: None,
                model: None,
                effort: None,
                summary: None,
                service_tier: Some(Some(tier.to_string())),
                collaboration_mode: None,
                personality: None,
            };
            app.sync_override_turn_context_settings(&mut session, thread_id, &op)
                .await;
        }
        session.shutdown().await?;
        let captured = server.await??;
        assert_eq!(
            captured
                .iter()
                .map(|(method, params)| (
                    method.as_str(),
                    params
                        .as_ref()
                        .and_then(|params| params.get("threadId"))
                        .cloned(),
                    params
                        .as_ref()
                        .and_then(|params| params.get("turnId"))
                        .cloned(),
                    params
                        .as_ref()
                        .and_then(|params| params.get("serviceTier"))
                        .cloned(),
                ))
                .collect::<Vec<_>>(),
            vec![
                (
                    "thread/settings/update",
                    Some(json!(thread_id)),
                    None,
                    Some(json!("priority"))
                ),
                (
                    "turn/settings/update",
                    Some(json!(thread_id)),
                    Some(json!("turn-1")),
                    Some(json!("priority"))
                ),
                (
                    "thread/settings/update",
                    Some(json!(thread_id)),
                    None,
                    Some(json!("default"))
                ),
                (
                    "turn/settings/update",
                    Some(json!(thread_id)),
                    Some(json!("turn-1")),
                    Some(json!("default"))
                ),
            ]
        );
    }
    Ok(())
}
