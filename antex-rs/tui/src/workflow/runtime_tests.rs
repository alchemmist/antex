use pretty_assertions::assert_eq;
use serde_json::Map;
use serde_json::json;
use tokio::sync::watch;

use super::*;
use crate::workflow::WorkflowDefinition;
use crate::workflow::create_run;
use crate::workflow::list_resumable_runs;

#[cfg(unix)]
#[tokio::test]
async fn python_workflow_runs_shell_agent_checkpoint_and_completion_end_to_end() {
    use std::os::unix::fs::PermissionsExt;

    if Command::new(python_program())
        .arg("--version")
        .output()
        .await
        .is_err()
    {
        return;
    }
    let temp = tempfile::tempdir().expect("tempdir");
    let script = temp.path().join("demo.py");
    tokio::fs::write(
        &script,
        r#"
WORKFLOW = {"id": "demo", "title": "Demo", "guardrails": {"max_agent_calls": 3, "max_shell_calls": 3, "max_parallel_agents": 2, "timeout_seconds": 30}}
def run(ctx):
    ctx.progress("starting", current=0, total=1)
    import sys
    shell = ctx.shell([sys.executable, "-c", "print('shell-ok')"])
    agent = ctx.agent("fix one issue")
    ctx.checkpoint({"done": True})
    return {"shell": shell["stdout"].strip(), "agent": agent["message"]}
"#,
    )
    .await
    .expect("write workflow");
    let fake_antex = temp.path().join("codex");
    tokio::fs::write(
        &fake_antex,
        r#"#!/bin/sh
cat >/dev/null
printf '%s\n' '{"type":"item.completed","item":{"id":"msg","type":"agent_message","text":"agent-ok"}}'
sleep 6
printf '%s\n' '{"type":"turn.completed","usage":{"input_tokens":1,"cached_input_tokens":0,"cache_write_input_tokens":0,"output_tokens":1,"reasoning_output_tokens":0}}'
"#,
    )
    .await
    .expect("write fake codex");
    let mut permissions = std::fs::metadata(&fake_antex)
        .expect("metadata")
        .permissions();
    permissions.set_mode(0o755);
    std::fs::set_permissions(&fake_antex, permissions).expect("chmod fake codex");
    let manifest = serde_json::from_value(json!({
        "id": "demo",
        "title": "Demo",
        "guardrails": {"max_agent_calls": 3, "max_shell_calls": 3, "max_parallel_agents": 2, "timeout_seconds": 30}
    }))
    .expect("manifest");
    let run = create_run(
        temp.path(),
        WorkflowDefinition {
            manifest,
            script_path: script,
            source: "test".to_string(),
        },
        Map::new(),
    )
    .await
    .expect("create run");
    let (_control_tx, control_rx) = watch::channel(WorkflowControl::Run);
    let (update_tx, mut update_rx) = mpsc::unbounded_channel();

    run_workflow(
        WorkflowRunRequest {
            run,
            workspace: temp.path().to_path_buf(),
            antex_exe: fake_antex,
        },
        control_rx,
        update_tx,
    )
    .await;

    let mut terminal = None;
    let mut agent_activity = Vec::new();
    let mut agent_idle_seconds = Vec::new();
    while let Ok(update) = update_rx.try_recv() {
        if let WorkflowUpdate::AgentActivity {
            message,
            idle_seconds,
            ..
        } = &update
        {
            agent_activity.push(message.clone());
            agent_idle_seconds.push(*idle_seconds);
        }
        if update.is_terminal() {
            terminal = Some(update);
        }
    }
    let Some(WorkflowUpdate::Completed {
        title,
        result,
        agent_calls,
        shell_calls,
        ..
    }) = terminal
    else {
        panic!("expected completed workflow update");
    };
    assert_eq!(title, "Demo");
    assert_eq!(result, json!({"shell": "shell-ok", "agent": "agent-ok"}));
    assert_eq!(agent_calls, 1);
    assert_eq!(shell_calls, 1);
    assert_eq!(
        agent_activity,
        vec!["preparing workflow result", "preparing workflow result"]
    );
    assert_eq!(agent_idle_seconds.len(), 2);
    assert_eq!(agent_idle_seconds[0], 0);
    assert!(agent_idle_seconds[1] >= 4);
    assert_eq!(
        list_resumable_runs(temp.path()).await.expect("list runs"),
        Vec::new()
    );
}

#[tokio::test]
async fn paused_workflow_retains_checkpoint_and_resumes() {
    if Command::new(python_program())
        .arg("--version")
        .output()
        .await
        .is_err()
    {
        return;
    }
    let temp = tempfile::tempdir().expect("tempdir");
    let script = temp.path().join("pausable.py");
    tokio::fs::write(
        &script,
        r#"
WORKFLOW = {"id": "pausable", "title": "Pausable", "guardrails": {"max_agent_calls": 1, "max_shell_calls": 2, "max_parallel_agents": 1, "timeout_seconds": 30}}
def run(ctx):
    if ctx.state.get("ready"):
        return {"resumed": True}
    import sys
    ctx.checkpoint({"ready": True})
    ctx.shell([sys.executable, "-c", "import time; time.sleep(30)"])
    return {"resumed": False}
"#,
    )
    .await
    .expect("write workflow");
    let manifest = serde_json::from_value(json!({
        "id": "pausable",
        "title": "Pausable",
        "guardrails": {"max_agent_calls": 1, "max_shell_calls": 2, "max_parallel_agents": 1, "timeout_seconds": 30}
    }))
    .expect("manifest");
    let run = create_run(
        temp.path(),
        WorkflowDefinition {
            manifest,
            script_path: script,
            source: "test".to_string(),
        },
        Map::new(),
    )
    .await
    .expect("create run");
    let (control_tx, control_rx) = watch::channel(WorkflowControl::Run);
    let (update_tx, mut update_rx) = mpsc::unbounded_channel();
    let request = WorkflowRunRequest {
        run,
        workspace: temp.path().to_path_buf(),
        antex_exe: temp.path().join("unused-codex"),
    };
    let task = tokio::spawn(run_workflow(request, control_rx, update_tx));

    loop {
        let update = tokio::time::timeout(Duration::from_secs(10), update_rx.recv())
            .await
            .expect("checkpoint update timeout")
            .expect("workflow update");
        if matches!(update, WorkflowUpdate::Checkpointed { .. }) {
            control_tx
                .send(WorkflowControl::Pause)
                .expect("pause workflow");
        }
        if matches!(update, WorkflowUpdate::Paused { .. }) {
            break;
        }
    }
    task.await.expect("workflow task");
    let mut runs = list_resumable_runs(temp.path())
        .await
        .expect("list paused runs");
    assert_eq!(runs.len(), 1);
    let paused = runs.remove(0);
    assert_eq!(paused.status, WorkflowRunStatus::Paused);
    assert_eq!(paused.state, json!({"ready": true}));

    let (_control_tx, control_rx) = watch::channel(WorkflowControl::Run);
    let (update_tx, mut update_rx) = mpsc::unbounded_channel();
    run_workflow(
        WorkflowRunRequest {
            run: paused,
            workspace: temp.path().to_path_buf(),
            antex_exe: temp.path().join("unused-codex"),
        },
        control_rx,
        update_tx,
    )
    .await;

    let terminal = std::iter::from_fn(|| update_rx.try_recv().ok())
        .find(WorkflowUpdate::is_terminal)
        .expect("terminal update");
    let WorkflowUpdate::Completed { result, .. } = terminal else {
        panic!("expected completed workflow update");
    };
    assert_eq!(result, json!({"resumed": true}));
    assert_eq!(
        list_resumable_runs(temp.path())
            .await
            .expect("list completed runs"),
        Vec::new()
    );
}
