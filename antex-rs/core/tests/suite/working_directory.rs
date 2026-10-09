use antex_core::TurnInputRequest;
use antex_history::RolloutItem;
use antex_protocol::models::PermissionProfile;
use antex_protocol::permissions::FileSystemAccessMode;
use antex_protocol::permissions::FileSystemSandboxEntry;
use antex_protocol::permissions::FileSystemSandboxPolicy;
use antex_protocol::permissions::NetworkSandboxPolicy;
use antex_protocol::protocol::AskForApproval;
use antex_protocol::protocol::EventMsg;
use antex_protocol::protocol::SandboxPolicy;
use antex_protocol::protocol::ThreadSettingsOverrides;
use antex_protocol::user_input::UserInput;
use anyhow::Result;
use core_test_support::responses;
use core_test_support::skip_if_no_network;
use core_test_support::test_antex::test_antex;
use core_test_support::wait_for_event;
use pretty_assertions::assert_eq;
use serde_json::json;

#[tokio::test]
async fn directory_change_updates_following_tool_and_persisted_settings() -> Result<()> {
    skip_if_no_network!(Ok(()));
    let server = responses::start_mock_server().await;
    let mut builder = test_antex();
    let test = builder.build(&server).await?;
    core_test_support::submit_thread_settings(
        &test.antex,
        ThreadSettingsOverrides {
            approval_policy: Some(AskForApproval::Never),
            sandbox_policy: Some(SandboxPolicy::DangerFullAccess),
            ..Default::default()
        },
    )
    .await?;
    let mut expected_settings = test.antex.thread_settings_snapshot().await;
    let destination = test.config.cwd.join("task-worktree");
    expected_settings.cwd = destination.clone();
    std::fs::create_dir(destination.as_path())?;
    std::fs::write(
        destination.join("AGENTS.md"),
        "Use the task worktree verification marker.",
    )?;
    let response = responses::mount_sse_sequence(
        &server,
        vec![
            responses::sse(vec![
                responses::ev_response_created("change-directory"),
                responses::ev_function_call("change", "set_working_directory", &json!({"path": destination}).to_string()),
                responses::ev_completed("change-directory"),
            ]),
            responses::sse(vec![
                responses::ev_response_created("write-file"),
                responses::ev_apply_patch_custom_tool_call("write", "*** Begin Patch\n*** Add File: task-result.txt\n+created in the worktree\n*** End Patch"),
                responses::ev_completed("write-file"),
            ]),
            responses::sse(vec![responses::ev_response_created("done"), responses::ev_completed("done")]),
            responses::sse(vec![
                responses::ev_response_created("next-turn"),
                responses::ev_apply_patch_custom_tool_call("followup", "*** Begin Patch\n*** Add File: followup-result.txt\n+still in the worktree\n*** End Patch"),
                responses::ev_completed("next-turn"),
            ]),
            responses::sse(vec![responses::ev_response_created("followup-done"), responses::ev_completed("followup-done")]),
        ],
    ).await;
    test.antex
        .start_or_steer_turn(TurnInputRequest::user_input(vec![UserInput::Text {
            text: "Move this session to the task worktree and create the result there.".into(),
            text_elements: Vec::new(),
        }]))
        .await?;
    let applied = wait_for_event(&test.antex, |event| matches!(event, EventMsg::ThreadSettingsApplied(applied) if applied.thread_settings.cwd == destination)).await;
    wait_for_event(&test.antex, |event| {
        matches!(event, EventMsg::TurnComplete(_))
    })
    .await;
    assert_eq!(
        std::fs::read_to_string(destination.join("task-result.txt"))?,
        "created in the worktree\n"
    );
    assert!(!test.config.cwd.join("task-result.txt").exists());
    let requests = response.requests();
    assert_eq!(requests.len(), 3);
    assert!(
        serde_json::to_string(&requests[1].input())?.contains("task worktree verification marker")
    );
    assert_eq!(
        test.antex.thread_settings_snapshot().await,
        expected_settings
    );
    test.submit_text_turn("Create another result in our task directory.")
        .await?;
    assert_eq!(
        std::fs::read_to_string(destination.join("followup-result.txt"))?,
        "still in the worktree\n"
    );
    assert!(!test.config.cwd.join("followup-result.txt").exists());
    assert_eq!(response.requests().len(), 5);
    assert_eq!(
        test.antex.thread_settings_snapshot().await,
        expected_settings
    );
    test.antex.flush_rollout().await?;
    let rollout = std::fs::read_to_string(test.antex.rollout_path().expect("rollout path"))?;
    let EventMsg::ThreadSettingsApplied(applied) = applied else {
        unreachable!()
    };
    let persisted = rollout
        .lines()
        .map(antex_rollout::parse_rollout_line)
        .collect::<std::result::Result<Vec<_>, _>>()?
        .into_iter()
        .filter_map(|line| match line.item {
            RolloutItem::EventMsg(EventMsg::ThreadSettingsApplied(event))
                if event.thread_settings.cwd == destination =>
            {
                Some(event.thread_settings)
            }
            _ => None,
        })
        .next_back()
        .expect("directory checkpoint persisted");
    assert_eq!(persisted, applied.thread_settings);
    Ok(())
}

#[tokio::test]
async fn invalid_directory_keeps_session_settings_unchanged() -> Result<()> {
    skip_if_no_network!(Ok(()));
    let server = responses::start_mock_server().await;
    let test = test_antex().build(&server).await?;
    let initial = test.antex.thread_settings_snapshot().await;
    let response = responses::mount_sse_sequence(
        &server,
        vec![
            responses::sse(vec![
                responses::ev_response_created("invalid-directory"),
                responses::ev_function_call(
                    "change",
                    "set_working_directory",
                    &json!({"path": "relative-worktree"}).to_string(),
                ),
                responses::ev_completed("invalid-directory"),
            ]),
            responses::sse(vec![
                responses::ev_response_created("done"),
                responses::ev_completed("done"),
            ]),
        ],
    )
    .await;
    test.submit_text_turn("Change the session directory.")
        .await?;
    assert_eq!(test.antex.thread_settings_snapshot().await, initial);
    let output = response.requests()[1]
        .function_call_output("change")
        .to_string();
    assert!(
        output.contains("absolute"),
        "unexpected tool response: {output}"
    );
    Ok(())
}

#[tokio::test]
async fn unreadable_destination_does_not_change_session_or_permissions() -> Result<()> {
    skip_if_no_network!(Ok(()));
    core_test_support::skip_if_target_windows!(
        Ok(()),
        "Windows restricted-token sandbox cannot enforce deny-read policies"
    );
    core_test_support::skip_if_sandbox!(Ok(()));
    let server = responses::start_mock_server().await;
    let test = test_antex()
        .with_config(|config| {
            let mut policy = FileSystemSandboxPolicy::read_only();
            policy.entries.push(FileSystemSandboxEntry::new(
                config.cwd.join("private-worktree").into(),
                FileSystemAccessMode::Deny,
            ));
            config
                .permissions
                .set_permission_profile(PermissionProfile::from_runtime_permissions(
                    &policy,
                    NetworkSandboxPolicy::Restricted,
                ))
                .expect("configure denied destination");
        })
        .build(&server)
        .await?;
    let initial = test.antex.thread_settings_snapshot().await;
    let destination = test.config.cwd.join("private-worktree");
    std::fs::create_dir(destination.as_path())?;
    let response = responses::mount_sse_sequence(
        &server,
        vec![
            responses::sse(vec![
                responses::ev_response_created("denied-directory"),
                responses::ev_function_call(
                    "change",
                    "set_working_directory",
                    &json!({"path": destination}).to_string(),
                ),
                responses::ev_completed("denied-directory"),
            ]),
            responses::sse(vec![
                responses::ev_response_created("done"),
                responses::ev_completed("done"),
            ]),
        ],
    )
    .await;
    test.submit_text_turn("Move the session into the private worktree.")
        .await?;
    assert_eq!(test.antex.thread_settings_snapshot().await, initial);
    assert!(
        response.requests()[1]
            .function_call_output("change")
            .to_string()
            .contains("outside the current filesystem permissions")
    );
    Ok(())
}
