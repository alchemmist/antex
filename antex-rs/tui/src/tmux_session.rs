use antex_protocol::ThreadId;

#[cfg(all(unix, not(test)))]
pub(crate) fn publish_thread_id(thread_id: Option<ThreadId>) {
    let Some(thread_id) = thread_id else {
        return;
    };
    let Ok(tmux_pane) = std::env::var("TMUX_PANE") else {
        return;
    };
    if std::env::var_os("TMUX").is_none() {
        return;
    }
    let thread_id = thread_id.to_string();

    if let Err(err) = std::process::Command::new("tmux")
        .args([
            "set-option",
            "-p",
            "-t",
            &tmux_pane,
            "@antex_thread_id",
            &thread_id,
        ])
        .status()
    {
        tracing::debug!(%err, "failed to publish Antex thread id to tmux");
    }
}

#[cfg(any(not(unix), test))]
pub(crate) fn publish_thread_id(_thread_id: Option<ThreadId>) {}
