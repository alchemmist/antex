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
