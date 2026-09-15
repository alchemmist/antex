use pretty_assertions::assert_eq;
use tokio::sync::watch;

use super::*;

#[cfg(unix)]
#[tokio::test]
async fn nested_antex_jsonl_is_reduced_to_a_bounded_agent_result() {
    use std::os::unix::fs::PermissionsExt;

    let temp = tempfile::tempdir().expect("tempdir");
    let fake_antex = temp.path().join("codex");
    tokio::fs::write(
        &fake_antex,
r#"#!/bin/sh
printf '%s\n' "$@" > "$0.args"
printf '%s' "$ANTEX_WORKFLOW_FORBID_QUALITY_GRAPH_IGNORE" > "$0.guard"
cat >/dev/null
printf '%s\n' '{"type":"turn.started"}'
printf '%s\n' '{"type":"item.started","item":{"id":"cmd","type":"command_execution","command":"git status"}}'
printf '%s\n' '{"type":"item.completed","item":{"id":"mcp","type":"mcp_tool_call","server":"github","tool":"get_pull_request"}}'
printf '%s\n' '{"type":"item.completed","item":{"id":"msg","type":"agent_message","text":"fixed"}}'
printf '%s\n' '{"type":"turn.completed","usage":{"input_tokens":12,"cached_input_tokens":8,"cache_write_input_tokens":0,"output_tokens":3,"reasoning_output_tokens":0}}'
"#,
    )
    .await
    .expect("write fake codex");
    let mut permissions = std::fs::metadata(&fake_antex)
        .expect("metadata")
        .permissions();
    permissions.set_mode(0o755);
    std::fs::set_permissions(&fake_antex, permissions).expect("chmod fake codex");
    let (_control_tx, control_rx) = watch::channel(WorkflowControl::Run);
    let (activity_tx, mut activity_rx) = mpsc::unbounded_channel();

    let result = execute_agent(
        AgentRequest {
            prompt: "Fix one issue".to_string(),
            model: None,
            reasoning_effort: Some("low".to_string()),
            developer_instructions: Some("Never weaken quality gates.".to_string()),
            forbid_quality_graph_ignore: true,
            cwd: None,
            timeout_seconds: Some(10),
            sandbox: AgentSandbox::DangerFullAccess,
            approval_mode: AgentApprovalMode::AutoReview,
        },
        AgentExecutionContext {
            default_model: None,
            default_cwd: None,
            default_timeout_seconds: None,
            antex_exe: &fake_antex,
            workspace: temp.path(),
            control: control_rx,
            agent_index: 2,
            activity_tx,
        },
    )
    .await
    .expect("agent result");

    assert_eq!(
        result,
        AgentResult {
            success: true,
            exit_code: Some(0),
            message: "fixed".to_string(),
            error: String::new(),
            input_tokens: 12,
            cached_input_tokens: 8,
            output_tokens: 3,
        }
    );
    let args = tokio::fs::read_to_string(fake_antex.with_extension("args"))
        .await
        .expect("read nested codex args");
    assert!(args.contains("model_reasoning_effort=\"low\""));
    assert!(args.contains("developer_instructions=\"Never weaken quality gates.\""));
    assert!(!args.lines().any(|argument| argument == "--sandbox"));
    assert!(args.lines().any(|argument| argument == "--approve-for-me"));
    let guard = tokio::fs::read_to_string(fake_antex.with_extension("guard"))
        .await
        .expect("read nested codex guard");
    assert_eq!(guard, "1");
    assert_eq!(
        std::iter::from_fn(|| activity_rx.try_recv().ok()).collect::<Vec<_>>(),
        vec![
            AgentActivity {
                agent_index: 2,
                message: "model started working".to_string(),
            },
            AgentActivity {
                agent_index: 2,
                message: "running command: git status".to_string(),
            },
            AgentActivity {
                agent_index: 2,
                message: "finished MCP call: github.get_pull_request".to_string(),
            },
            AgentActivity {
                agent_index: 2,
                message: "preparing workflow result".to_string(),
            },
        ]
    );
}
