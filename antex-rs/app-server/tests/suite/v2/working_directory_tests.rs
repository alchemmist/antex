use antex_app_server_protocol::ThreadResumeParams;
use antex_app_server_protocol::ThreadResumeResponse;
use antex_app_server_protocol::ThreadStartParams;
use antex_app_server_protocol::ThreadStartResponse;
use antex_app_server_protocol::TurnStartParams;
use antex_app_server_protocol::TurnStartResponse;
use antex_app_server_protocol::UserInput;
use anyhow::Result;
use app_test_support::MockResponsesConfig;
use app_test_support::TestAppServer;
use core_test_support::responses;
use core_test_support::skip_if_no_network;
use pretty_assertions::assert_eq;
use serde_json::json;
use std::time::Duration;
use tempfile::TempDir;
use tokio::time::timeout;

#[tokio::test]
async fn agent_directory_change_survives_app_server_restart_without_cwd_override() -> Result<()> {
    skip_if_no_network!(Ok(()));
    let server = responses::start_mock_server().await;
    let home = TempDir::new()?;
    let workspace = TempDir::new()?;
    let original = std::fs::canonicalize(workspace.path())?;
    let destination = original.join("task-worktree");
    std::fs::create_dir(&destination)?;
    MockResponsesConfig::new(&server.uri())
        .with_model("gpt-5.4")
        .with_sandbox_mode("danger-full-access")
        .write(home.path())?;
    let response = responses::mount_sse_sequence(&server, vec![
        responses::sse(vec![
            responses::ev_response_created("change-directory"),
            responses::ev_function_call("change", "set_working_directory", &json!({"path": destination}).to_string()),
            responses::ev_completed("change-directory"),
        ]),
        responses::sse(vec![responses::ev_response_created("changed"), responses::ev_completed("changed")]),
        responses::sse(vec![
            responses::ev_response_created("resumed-write"),
            responses::ev_apply_patch_custom_tool_call("write", "*** Begin Patch\n*** Add File: resumed-result.txt\n+restored worktree\n*** End Patch"),
            responses::ev_completed("resumed-write"),
        ]),
        responses::sse(vec![responses::ev_response_created("done"), responses::ev_completed("done")]),
    ]).await;
    let mut first = TestAppServer::builder()
        .with_antex_home(home.path())
        .without_auto_env()
        .build_initialized()
        .await?;
    let start_id = first
        .send_thread_start_request(ThreadStartParams {
            cwd: Some(original.to_string_lossy().into_owned()),
            ..Default::default()
        })
        .await?;
    let started: ThreadStartResponse = timeout(
        Duration::from_secs(/*secs*/ 60),
        first.read_response(start_id),
    )
    .await??;
    let turn_id = first
        .send_turn_start_request(TurnStartParams {
            thread_id: started.thread.id.clone(),
            input: vec![UserInput::Text {
                text: "Adopt the task worktree.".into(),
                text_elements: Vec::new(),
            }],
            ..Default::default()
        })
        .await?;
    let _: TurnStartResponse = timeout(
        Duration::from_secs(/*secs*/ 60),
        first.read_response(turn_id),
    )
    .await??;
    timeout(
        Duration::from_secs(/*secs*/ 60),
        first.read_stream_until_notification_message("turn/completed"),
    )
    .await??;
    assert!(
        timeout(
            Duration::from_secs(/*secs*/ 60),
            first.shutdown_gracefully()
        )
        .await??
        .success()
    );
    let mut resumed = TestAppServer::builder()
        .with_antex_home(home.path())
        .without_auto_env()
        .build_initialized()
        .await?;
    let resume_id = resumed
        .send_thread_resume_request(ThreadResumeParams {
            thread_id: started.thread.id.clone(),
            ..Default::default()
        })
        .await?;
    let restored: ThreadResumeResponse = timeout(
        Duration::from_secs(/*secs*/ 60),
        resumed.read_response(resume_id),
    )
    .await??;
    assert_eq!(restored.cwd.as_path(), destination.as_path());
    assert_eq!(
        restored.runtime_workspace_roots,
        started.runtime_workspace_roots
    );
    let turn_id = resumed
        .send_turn_start_request(TurnStartParams {
            thread_id: started.thread.id,
            input: vec![UserInput::Text {
                text: "Create the result in our worktree.".into(),
                text_elements: Vec::new(),
            }],
            ..Default::default()
        })
        .await?;
    let _: TurnStartResponse = timeout(
        Duration::from_secs(/*secs*/ 60),
        resumed.read_response(turn_id),
    )
    .await??;
    timeout(
        Duration::from_secs(/*secs*/ 60),
        resumed.read_stream_until_notification_message("turn/completed"),
    )
    .await??;
    assert_eq!(
        std::fs::read_to_string(destination.join("resumed-result.txt"))?,
        "restored worktree\n"
    );
    assert!(!original.join("resumed-result.txt").exists());
    assert_eq!(response.requests().len(), 4);
    Ok(())
}
