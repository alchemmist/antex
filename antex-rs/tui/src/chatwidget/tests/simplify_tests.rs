use super::app_server::configured_thread_session;
use super::*;
use pretty_assertions::assert_eq;

#[tokio::test]
async fn simplify_starts_a_parallel_review_without_changing_subagent_mode() {
    let (mut chat, mut rx, mut ops) = make_chatwidget_manual(None).await;
    chat.handle_thread_session(configured_thread_session(ThreadId::new()));
    drain_insert_history(&mut rx);
    chat.dispatch_command(SlashCommand::Simplify);
    let Op::UserTurn {
        items,
        subagent_spawn_policy,
        ..
    } = next_submit_op(&mut ops)
    else {
        panic!("expected simplify turn");
    };
    assert_eq!(subagent_spawn_policy, SubagentSpawnPolicy::Allow);
    assert_eq!(
        items,
        vec![UserInput::Text {
            text: include_str!("../../../assets/prompt_for_simplify_command.md").to_string(),
            text_elements: Vec::new(),
        }]
    );
    assert_eq!(
        chat.selected_subagent_spawn_policy(),
        SubagentSpawnPolicy::Disallow
    );
    let history = drain_insert_history(&mut rx)
        .into_iter()
        .flatten()
        .collect::<Vec<_>>();
    assert_chatwidget_snapshot!("simplify_command_history", lines_to_single_string(&history));
}

#[tokio::test]
async fn simplify_preserves_user_focus_as_text() {
    let (mut chat, _rx, mut ops) = make_chatwidget_manual(None).await;
    chat.handle_thread_session(configured_thread_session(ThreadId::new()));
    chat.dispatch_command_with_args(
        SlashCommand::Simplify,
        "!src/cache reuse".to_string(),
        Vec::new(),
    );
    let Op::UserTurn {
        items,
        subagent_spawn_policy,
        ..
    } = next_submit_op(&mut ops)
    else {
        panic!("expected simplify turn");
    };
    assert_eq!(subagent_spawn_policy, SubagentSpawnPolicy::Allow);
    assert_eq!(
        items,
        vec![UserInput::Text {
            text: format!(
                "{}\nUser-provided scope or focus:\n!src/cache reuse",
                include_str!("../../../assets/prompt_for_simplify_command.md")
            ),
            text_elements: Vec::new(),
        }]
    );
}

#[tokio::test]
async fn simplify_does_not_steer_an_active_task() {
    let (mut chat, mut rx, mut ops) = make_chatwidget_manual(None).await;
    chat.handle_thread_session(configured_thread_session(ThreadId::new()));
    drain_insert_history(&mut rx);
    chat.bottom_pane.set_task_running(true);
    while ops.try_recv().is_ok() {}
    chat.dispatch_command(SlashCommand::Simplify);
    assert!(ops.try_recv().is_err());
    let history = drain_insert_history(&mut rx)
        .into_iter()
        .flatten()
        .collect::<Vec<_>>();
    assert!(lines_to_single_string(&history).contains("disabled while a task is in progress"));
}

#[tokio::test]
async fn queued_simplify_keeps_access_to_subagents() {
    let (mut chat, _rx, mut ops) = make_chatwidget_manual(None).await;
    chat.handle_thread_session(configured_thread_session(ThreadId::new()));
    chat.handle_composer_input_result(
        crate::bottom_pane::InputResult::Queued {
            text: "/simplify src/cache".to_string(),
            text_elements: Vec::new(),
            action: QueuedInputAction::ParseSlash,
            pending_pastes: Vec::new(),
        },
        false,
    );
    chat.maybe_send_next_queued_input();
    assert_matches!(
        next_submit_op(&mut ops),
        Op::UserTurn {
            subagent_spawn_policy: SubagentSpawnPolicy::Allow,
            ..
        }
    );
    assert_eq!(
        chat.selected_subagent_spawn_policy(),
        SubagentSpawnPolicy::Disallow
    );
}
