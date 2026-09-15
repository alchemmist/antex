use antex_app_server_protocol::ThreadContextReadParams;
use antex_app_server_protocol::ThreadContextReadResponse;
use antex_app_server_protocol::ThreadInjectItemsParams;
use antex_app_server_protocol::ThreadInjectItemsResponse;
use antex_app_server_protocol::ThreadStartParams;
use antex_app_server_protocol::ThreadStartResponse;
use antex_protocol::models::ContentItem;
use antex_protocol::models::ResponseItem;
use anyhow::Result;
use app_test_support::TestAppServer;
use pretty_assertions::assert_eq;
use tempfile::TempDir;

#[tokio::test]
async fn reads_exact_system_prompt_and_model_visible_history() -> Result<()> {
    let codex_home = TempDir::new()?;
    let mut app = TestAppServer::builder()
        .with_antex_home(codex_home.path())
        .build_initialized()
        .await?;
    let system_prompt = "Exact test system prompt";
    let start_id = app
        .send_thread_start_request_with_auto_env(ThreadStartParams {
            base_instructions: Some(system_prompt.to_string()),
            ..Default::default()
        })
        .await?;
    let ThreadStartResponse { thread, .. } = app.read_response(start_id).await?;
    let context_item = ResponseItem::Message {
        id: None,
        role: "developer".to_string(),
        content: vec![ContentItem::InputText {
            text: "Visible developer context".to_string(),
        }],
        phase: None,
        internal_chat_message_metadata_passthrough: None,
    };
    let inject_id = app
        .send_thread_inject_items_request(ThreadInjectItemsParams {
            thread_id: thread.id.clone(),
            items: vec![serde_json::to_value(&context_item)?],
        })
        .await?;
    let _: ThreadInjectItemsResponse = app.read_response(inject_id).await?;

    let inspect_id = app
        .send_thread_context_read_request(ThreadContextReadParams {
            thread_id: thread.id,
        })
        .await?;
    let response: ThreadContextReadResponse = app.read_response(inspect_id).await?;

    assert_eq!(response.base_instructions, system_prompt);
    assert!(response.items.iter().any(|item| {
        matches!(
            item,
            ResponseItem::Message { role, content, .. }
                if role == "developer"
                    && content == &vec![ContentItem::InputText {
                        text: "Visible developer context".to_string(),
                    }]
        )
    }));
    assert_eq!(response.used_tokens, None);
    assert_eq!(response.context_window, None);
    assert_eq!(response.latest_model_request, None);
    Ok(())
}
