use super::*;
use pretty_assertions::assert_eq;

#[tokio::test]
async fn tmux_binding_tracks_primary_activation_not_viewed_subagents() -> Result<()> {
    let mut app = make_test_app().await;
    let root = ThreadId::new();
    app.enqueue_primary_thread_session(
        test_thread_session(root, test_path_buf("/tmp/project")),
        Vec::new(),
    )
    .await?;
    assert_eq!(
        crate::tmux_session::tests::PUBLISHED_THREAD.get(),
        Some(root)
    );
    app.active_thread_id = Some(ThreadId::new());
    app.sync_agent_status_ui();
    assert_eq!(
        crate::tmux_session::tests::PUBLISHED_THREAD.get(),
        Some(root)
    );
    let next = ThreadId::new();
    app.active_thread_id = None;
    app.enqueue_primary_thread_session(
        test_thread_session(next, test_path_buf("/tmp/project")),
        Vec::new(),
    )
    .await?;
    assert_eq!(
        crate::tmux_session::tests::PUBLISHED_THREAD.get(),
        Some(next)
    );
    Ok(())
}

#[tokio::test]
async fn primary_directory_follows_session_and_survives_viewing_subagents() -> Result<()> {
    let mut app = make_test_app().await;
    let primary = ThreadId::new();
    let worktree = test_absolute_path("/tmp/task worktree");
    app.enqueue_primary_thread_session(
        test_thread_session(primary, worktree.to_path_buf()),
        Vec::new(),
    )
    .await?;
    assert_eq!(app.config.cwd, worktree);
    assert_eq!(
        crate::tmux_session::tests::SYNCHRONIZED_CWD.with_borrow(Clone::clone),
        Some(worktree.to_path_buf())
    );
    let child = ThreadId::new();
    app.active_thread_id = Some(child);
    app.synchronize_primary_directory(child, &test_absolute_path("/tmp/child"));
    assert_eq!(app.config.cwd, worktree);
    let next = test_absolute_path("/tmp/another task");
    app.synchronize_primary_directory(primary, &next);
    assert_eq!(
        (
            app.config.cwd.clone(),
            app.runtime_working_directory_override.clone(),
            crate::tmux_session::tests::SYNCHRONIZED_CWD.with_borrow(Clone::clone),
        ),
        (next.clone(), None, Some(next.to_path_buf()))
    );
    Ok(())
}

#[tokio::test]
async fn remote_primary_directory_does_not_change_local_terminal_directory() -> Result<()> {
    let mut app = make_test_app().await;
    let primary = ThreadId::new();
    app.primary_thread_id = Some(primary);
    app.app_server_target = crate::AppServerTarget::Remote {
        endpoint: crate::resolve_remote_addr("ws://127.0.0.1:8765")?,
    };
    let previous = app.config.cwd.clone();
    crate::tmux_session::tests::SYNCHRONIZED_CWD.replace(/*t*/ None);
    app.synchronize_primary_directory(primary, &test_absolute_path("/tmp/remote-worktree"));
    assert_eq!(app.config.cwd, previous);
    assert_eq!(
        crate::tmux_session::tests::SYNCHRONIZED_CWD.with_borrow(Clone::clone),
        None
    );
    Ok(())
}

#[tokio::test]
async fn only_local_primary_settings_changes_update_adopted_directory() -> Result<()> {
    let mut app = make_test_app().await;
    let primary = ThreadId::new();
    let original = app.config.cwd.clone();
    app.enqueue_primary_thread_session(
        test_thread_session(primary, original.to_path_buf()),
        Vec::new(),
    )
    .await?;
    assert_eq!(app.adopted_working_directory, None);
    let worktree = test_absolute_path("/tmp/adopted-worktree");
    let mut settings = antex_app_server_protocol::ThreadSettings {
        disabled_plugin_ids: Vec::new(),
        cwd: worktree.clone(),
        runtime_workspace_roots: None,
        approval_policy: AskForApproval::OnRequest,
        approvals_reviewer: antex_app_server_protocol::ApprovalsReviewer::AutoReview,
        sandbox_policy: antex_app_server_protocol::SandboxPolicy::ReadOnly {
            network_access: false,
        },
        active_permission_profile: None,
        model: "gpt-test".to_string(),
        model_provider: "openai".to_string(),
        service_tier: None,
        effort: None,
        summary: None,
        collaboration_mode: app.chat_widget.effective_collaboration_mode(),
        multi_agent_mode: Default::default(),
        personality: None,
    };
    app.apply_thread_settings_to_cached_session(ThreadId::new(), &settings)
        .await;
    assert_eq!(app.adopted_working_directory, None);
    app.apply_thread_settings_to_cached_session(primary, &settings)
        .await;
    assert_eq!(app.adopted_working_directory, Some(worktree.to_path_buf()));
    app.app_server_target = crate::AppServerTarget::Remote {
        endpoint: crate::resolve_remote_addr("ws://127.0.0.1:8765")?,
    };
    settings.cwd = test_absolute_path("/tmp/remote-adopted-worktree");
    app.apply_thread_settings_to_cached_session(primary, &settings)
        .await;
    assert_eq!(app.adopted_working_directory, Some(worktree.to_path_buf()));
    Ok(())
}
