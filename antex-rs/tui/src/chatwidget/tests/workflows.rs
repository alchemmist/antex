use super::*;
use crate::workflow::WorkflowDefinition;
use crate::workflow::WorkflowUpdate;

#[tokio::test]
async fn workflow_picker_snapshot() {
    let (mut chat, _rx, _op_rx) = make_chatwidget_manual(/*model_override*/ None).await;
    let definitions = [
        (
            "github-bot-pr-maintenance",
            "GitHub bot PR maintenance",
            "Review or safely merge bot pull requests across owned GitHub repositories.",
            "/tmp/github.py",
        ),
        (
            "ruff-cleanup",
            "Ruff cleanup",
            "Fix a large Ruff backlog in small agent batches.",
            "/tmp/ruff.py",
        ),
    ]
    .into_iter()
    .map(|(id, title, description, script_path)| WorkflowDefinition {
        manifest: serde_json::from_value(json!({
            "id": id,
            "title": title,
            "description": description,
        }))
        .expect("manifest"),
        script_path: PathBuf::from(script_path),
        source: "built-in".to_string(),
    })
    .collect();
    chat.show_workflow_picker(definitions);

    assert_chatwidget_snapshot!(
        "workflow_picker",
        normalize_snapshot_paths(render_bottom_popup(&chat, /*width*/ 80))
    );
}

#[tokio::test]
async fn workflow_optional_text_field_can_submit_empty_value() {
    let (mut chat, mut rx, _op_rx) = make_chatwidget_manual(/*model_override*/ None).await;
    let field = serde_json::from_value(json!({
        "id": "model",
        "label": "Model",
        "type": "text",
        "default": ""
    }))
    .expect("field");
    chat.show_workflow_field("Demo", &field, 0, 1);

    chat.handle_key_event(KeyEvent::new(KeyCode::Enter, KeyModifiers::NONE));

    assert_matches!(
        rx.try_recv(),
        Ok(AppEvent::WorkflowFieldAnswered(answer)) if answer.is_empty()
    );
}

#[tokio::test]
async fn workflow_lifecycle_history_snapshot() {
    let (mut chat, mut rx, _op_rx) = make_chatwidget_manual(/*model_override*/ None).await;
    chat.handle_workflow_update(&WorkflowUpdate::Started {
        run_id: "20260805-120000-12345678".to_string(),
        title: "Ruff cleanup".to_string(),
    });
    chat.handle_workflow_update(&WorkflowUpdate::Completed {
        run_id: "20260805-120000-12345678".to_string(),
        title: "Ruff cleanup".to_string(),
        result: json!({"passes": 12, "agent_calls": 48}),
        agent_calls: 48,
        shell_calls: 13,
    });

    let rendered = drain_insert_history(&mut rx)
        .into_iter()
        .flatten()
        .map(|line| line.to_string())
        .collect::<Vec<_>>()
        .join("\n");
    assert_chatwidget_snapshot!("workflow_lifecycle_history", rendered);
}

#[tokio::test]
async fn workflow_agent_status_retains_current_phase_snapshot() {
    let (mut chat, _rx, _op_rx) = make_chatwidget_manual(/*model_override*/ None).await;
    let run_id = "20260825-161943-f2b3fe76".to_string();
    chat.handle_workflow_update(&WorkflowUpdate::Started {
        run_id: run_id.clone(),
        title: "GitHub bot PR maintenance".to_string(),
    });
    chat.handle_workflow_update(&WorkflowUpdate::Progress {
        run_id: run_id.clone(),
        message: "Repositories 6-10: alpha, beta, gamma, delta, epsilon".to_string(),
        current: Some(5),
        total: Some(42),
    });
    chat.handle_workflow_update(&WorkflowUpdate::AgentFinished {
        run_id,
        completed: 3,
        total: 5,
        success: true,
        phase: Some("Repositories 6-10: alpha, beta, gamma, delta, epsilon".to_string()),
        phase_current: Some(5),
        phase_total: Some(42),
    });

    assert_chatwidget_snapshot!(
        "workflow_agent_status_with_phase",
        render_bottom_popup(&chat, /*width*/ 96)
    );
}

#[tokio::test]
async fn workflow_agent_activity_status_snapshot() {
    let (mut chat, _rx, _op_rx) = make_chatwidget_manual(/*model_override*/ None).await;
    chat.handle_workflow_update(&WorkflowUpdate::Started {
        run_id: "20260828-144940-1cbae1fc".to_string(),
        title: "PR babysitter".to_string(),
    });
    chat.handle_workflow_update(&WorkflowUpdate::AgentActivity {
        run_id: "20260828-144940-1cbae1fc".to_string(),
        agent: 1,
        total: 3,
        message: "calling MCP: github.get_pull_request".to_string(),
        idle_seconds: 17,
        phase: Some("Repairing 4 review items and 3 failed checks".to_string()),
        phase_current: Some(2),
        phase_total: Some(7),
    });

    assert_chatwidget_snapshot!(
        "workflow_agent_activity_status",
        render_bottom_popup(&chat, /*width*/ 96)
    );
}

#[tokio::test]
async fn workflow_model_picker_uses_catalog_for_legacy_and_model_fields() {
    for (id, kind) in [
        ("model", "text"),
        ("worker_model", "text"),
        ("reviewer", "model"),
    ] {
        let (mut chat, mut rx, _op_rx) = make_chatwidget_manual(Some("gpt-5.4")).await;
        set_fast_mode_test_catalog(&mut chat);
        let field = serde_json::from_value(json!({
            "id": id, "label": "Model", "type": kind, "default": ""
        }))
        .expect("field");
        chat.show_workflow_field("Demo", &field, 0, 1);
        if id == "model" {
            assert_chatwidget_snapshot!("workflow_model_picker", render_bottom_popup(&chat, 90));
        }
        chat.handle_key_event(KeyCode::Down.into());
        chat.handle_key_event(KeyCode::Enter.into());
        let answer = match rx.try_recv().expect("model selection") {
            AppEvent::WorkflowFieldAnswered(answer) => answer,
            _ => panic!("expected workflow answer"),
        };
        assert!(
            chat.model_catalog
                .try_list_models()
                .expect("catalog")
                .iter()
                .any(|model| model.show_in_picker && model.model == answer)
        );
        assert_eq!(chat.current_model(), "gpt-5.4");
    }
}

#[tokio::test]
async fn workflow_model_picker_preserves_custom_default_and_optional_inheritance() {
    for (required, inherit) in [(true, false), (false, false), (false, true)] {
        let (mut chat, mut rx, _op_rx) = make_chatwidget_manual(Some("gpt-5.4")).await;
        let field = serde_json::from_value(json!({
            "id": "worker_model", "label": "Worker", "type": "text",
            "default": "custom-workflow-model", "required": required
        }))
        .expect("field");
        chat.show_workflow_field("Demo", &field, 0, 1);
        if inherit {
            for ch in "Use current Antex model".chars() {
                chat.handle_key_event(KeyCode::Char(ch).into());
            }
        }
        chat.handle_key_event(KeyCode::Enter.into());
        assert_matches!(rx.try_recv(), Ok(AppEvent::WorkflowFieldAnswered(answer))
            if answer == if inherit { "" } else { "custom-workflow-model" });
    }
}
