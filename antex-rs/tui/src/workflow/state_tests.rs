use pretty_assertions::assert_eq;
use serde_json::json;

use super::*;

#[tokio::test]
async fn creates_checkpoints_and_lists_resumable_runs() {
    let temp = tempfile::tempdir().expect("tempdir");
    let script = temp.path().join("workflow.py");
    tokio::fs::write(&script, "WORKFLOW = {}\n")
        .await
        .expect("write script");
    let definition = WorkflowDefinition {
        manifest: serde_json::from_value(json!({"id":"demo","title":"Demo"})).expect("manifest"),
        script_path: script,
        source: "test".to_string(),
    };
    let mut params = Map::new();
    params.insert("scope".to_string(), json!("src"));

    let mut run = create_run(temp.path(), definition, params.clone())
        .await
        .expect("create run");
    run.checkpoint(json!({"completed": 7}))
        .await
        .expect("checkpoint");
    run.set_status(WorkflowRunStatus::Paused, None)
        .await
        .expect("pause");

    let runs = list_resumable_runs(temp.path()).await.expect("list runs");
    assert_eq!(runs.len(), 1);
    assert_eq!(runs[0].params, params);
    assert_eq!(runs[0].state, json!({"completed": 7}));
    assert_eq!(runs[0].status, WorkflowRunStatus::Paused);
}
