use antex_core::TurnInputRequest;
use antex_features::Feature;
use antex_protocol::config_types::ApprovalsReviewer;
use antex_protocol::models::PermissionProfile;
use antex_protocol::protocol::AskForApproval;
use antex_protocol::protocol::EventMsg;
use antex_protocol::protocol::Op;
use antex_protocol::protocol::ReviewDecision;
use antex_protocol::user_input::UserInput;
use anyhow::Result;
use core_test_support::responses;
use core_test_support::skip_if_no_network;
use core_test_support::test_antex::test_antex;
use core_test_support::wait_for_event;
use pretty_assertions::assert_eq;
use serde_json::json;

#[tokio::test]
async fn force_push_requires_fresh_user_confirmation_even_with_approvals_disabled() -> Result<()> {
    skip_if_no_network!(Ok(()));
    let server = responses::start_mock_server().await;
    // Never modify a remote even if a regression allows this command to execute.
    let args = json!({"cmd": "git push --force --dry-run", "yield_time_ms": 1000}).to_string();
    let requests = responses::mount_sse_sequence(
        &server,
        vec![
            responses::sse(vec![
                responses::ev_response_created("first"),
                responses::ev_function_call("push-1", "exec_command", &args),
                responses::ev_completed("first"),
            ]),
            responses::sse(vec![
                responses::ev_response_created("second"),
                responses::ev_function_call("push-2", "exec_command", &args),
                responses::ev_completed("second"),
            ]),
            responses::sse(vec![
                responses::ev_response_created("done"),
                responses::ev_completed("done"),
            ]),
        ],
    )
    .await;
    let test = test_antex()
        .with_config(|config| {
            config
                .features
                .enable(Feature::ShellTool)
                .expect("shell tool");
            config
                .features
                .enable(Feature::UnifiedExec)
                .expect("unified exec");
            config
                .permissions
                .approval_policy
                .set(AskForApproval::Never)
                .expect("approval policy");
            config
                .permissions
                .set_permission_profile(PermissionProfile::Disabled)
                .expect("permissions");
            config.approvals_reviewer = ApprovalsReviewer::User;
        })
        .build_with_auto_env(&server)
        .await?;
    test.antex
        .start_or_steer_turn(TurnInputRequest::user_input(vec![UserInput::Text {
            text: "Check the force push twice without modifying any remote.".into(),
            text_elements: Vec::new(),
        }]))
        .await?;
    for (call_id, decision) in [
        ("push-1", ReviewDecision::Approved),
        (
            "push-2",
            ReviewDecision::denied("force push declined by user"),
        ),
    ] {
        let event = wait_for_event(&test.antex, |event| {
            matches!(
                event,
                EventMsg::ExecApprovalRequest(_) | EventMsg::TurnComplete(_)
            )
        })
        .await;
        let EventMsg::ExecApprovalRequest(approval) = event else {
            panic!("expected explicit force-push confirmation before completion");
        };
        assert_eq!(approval.call_id, call_id);
        assert_eq!(approval.proposed_execpolicy_amendment, None);
        assert_eq!(
            approval.available_decisions,
            Some(vec![
                ReviewDecision::Approved,
                ReviewDecision::denied("force push declined by user"),
            ])
        );
        test.antex
            .submit(Op::ExecApproval {
                id: approval.approval_id.unwrap_or(approval.call_id),
                turn_id: Some(approval.turn_id),
                decision,
            })
            .await?;
    }
    wait_for_event(&test.antex, |event| {
        matches!(event, EventMsg::TurnComplete(_))
    })
    .await;
    assert!(
        requests.requests()[2]
            .function_call_output("push-2")
            .to_string()
            .contains("force push declined by user")
    );
    Ok(())
}
