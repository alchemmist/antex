use super::app_server::configured_thread_session;
use super::*;
use pretty_assertions::assert_eq;

#[tokio::test]
async fn subagent_mode_survives_resume_and_persists_off() {
    let (mut original, _rx, _ops) = make_chatwidget_manual(None).await;
    let session = configured_thread_session(ThreadId::new());
    original.handle_thread_session(session.clone());
    original.dispatch_command(SlashCommand::MultiAgents);
    let settings = original.local_settings.clone();
    drop(original);

    let (mut resumed, _rx, mut ops) = make_chatwidget_manual(None).await;
    resumed.local_settings = settings.clone();
    resumed.handle_thread_session(session.clone());
    assert_eq!(
        resumed.selected_subagent_spawn_policy(),
        SubagentSpawnPolicy::Allow
    );
    assert_chatwidget_snapshot!(
        "subagent_mode_after_resume",
        render_bottom_popup(&resumed, 80)
    );
    resumed
        .bottom_pane
        .set_composer_text("delegate this".to_string(), Vec::new(), Vec::new());
    resumed.handle_key_event(KeyEvent::new(KeyCode::Enter, KeyModifiers::NONE));
    assert_matches!(
        next_submit_op(&mut ops),
        Op::UserTurn {
            subagent_spawn_policy: SubagentSpawnPolicy::Allow,
            ..
        }
    );
    resumed.dispatch_command(SlashCommand::MultiAgents);
    drop(resumed);

    let (mut disabled, _rx, _ops) = make_chatwidget_manual(None).await;
    disabled.local_settings = settings;
    disabled.handle_thread_session(session);
    assert_eq!(
        disabled.selected_subagent_spawn_policy(),
        SubagentSpawnPolicy::Disallow
    );
}

#[tokio::test]
async fn subagent_mode_enabled_before_thread_start_is_saved() {
    let (mut original, _rx, _ops) = make_chatwidget_manual(None).await;
    original.dispatch_command(SlashCommand::MultiAgents);
    let session = configured_thread_session(ThreadId::new());
    original.handle_thread_session(session.clone());
    let settings = original.local_settings.clone();
    drop(original);

    let (mut resumed, _rx, _ops) = make_chatwidget_manual(None).await;
    resumed.local_settings = settings;
    resumed.handle_thread_session(session);
    assert_eq!(
        resumed.selected_subagent_spawn_policy(),
        SubagentSpawnPolicy::Allow
    );
    resumed.handle_thread_session_quiet(configured_thread_session(ThreadId::new()));
    assert_eq!(
        resumed.selected_subagent_spawn_policy(),
        SubagentSpawnPolicy::Disallow
    );
}

#[tokio::test]
async fn subagent_mode_reload_observes_disable_in_another_client() {
    let (mut original, _rx, _ops) = make_chatwidget_manual(None).await;
    let session = configured_thread_session(ThreadId::new());
    original.handle_thread_session(session.clone());
    original.dispatch_command(SlashCommand::MultiAgents);
    let (mut other, _rx, _ops) = make_chatwidget_manual(None).await;
    other.local_settings = original.local_settings.clone();
    other.handle_thread_session(session.clone());
    other.dispatch_command(SlashCommand::MultiAgents);

    original.handle_thread_session_quiet(session);
    assert_eq!(
        original.selected_subagent_spawn_policy(),
        SubagentSpawnPolicy::Disallow
    );
}
