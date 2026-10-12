use super::*;
use pretty_assertions::assert_eq;
use std::collections::HashSet;

#[tokio::test]
async fn overview_retains_channels_only_for_still_pending_inbound_requests() {
    let request_thread = ThreadId::new();
    for request in [
        exec_approval_request(
            request_thread,
            "turn-1",
            "approval-1",
            /*approval_id*/ None,
        ),
        request_user_input_request(request_thread, "turn-1", "input-1"),
    ] {
        let mut app = make_test_app().await;
        let running_thread = ThreadId::new();
        for thread_id in [request_thread, running_thread] {
            app.thread_event_channels.insert(
                thread_id,
                ThreadEventChannel::new_with_session(
                    THREAD_EVENT_CHANNEL_CAPACITY,
                    test_thread_session(thread_id, app.config.cwd.to_path_buf()),
                    Vec::new(),
                ),
            );
            app.agents_overview
                .dispatched_requests
                .insert(thread_id, Vec::new());
        }
        app.thread_event_channels[&running_thread]
            .store
            .lock()
            .await
            .set_active_turn_id("outbound-turn".to_string());
        app.thread_event_channels[&request_thread]
            .store
            .lock()
            .await
            .push_request(request.clone());
        assert_eq!(
            app.pending_app_server_requests
                .note_server_request(&request),
            None
        );

        app.reset_thread_event_state().await;

        assert_eq!(
            app.thread_event_channels
                .keys()
                .copied()
                .collect::<HashSet<_>>(),
            HashSet::from([request_thread]),
        );
        assert!(
            app.pending_app_server_requests
                .contains_server_request(&request)
        );
        assert_eq!(
            app.thread_event_channels[&request_thread]
                .store
                .lock()
                .await
                .pending_replay_requests(),
            vec![request.clone()],
        );
        assert!(
            app.agents_overview
                .dispatched_requests
                .contains_key(&running_thread)
        );

        // Even a nonempty retained list must not keep the channel once the server resolves the
        // approval/input. Channel retention depends on actionable state, not overview membership.
        let request_id = request.id();
        assert!(
            app.pending_app_server_requests
                .resolve_notification(&request_thread.to_string(), request_id)
                .is_some()
        );
        app.thread_event_channels[&request_thread]
            .store
            .lock()
            .await
            .push_notification(ServerNotification::ServerRequestResolved(
                antex_app_server_protocol::ServerRequestResolvedNotification {
                    thread_id: request_thread.to_string(),
                    request_id: request_id.clone(),
                },
            ));
        assert_eq!(
            app.agents_overview.dispatched_requests[&request_thread],
            vec![request.clone()]
        );

        app.reset_thread_event_state().await;

        assert!(app.thread_event_channels.is_empty());
        assert!(
            !app.pending_app_server_requests
                .contains_server_request(&request)
        );
    }
}
