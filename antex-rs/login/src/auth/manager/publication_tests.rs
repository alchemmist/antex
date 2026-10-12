//! Production proactive entry points, paused after a real storage load.
use super::test_authority::document;
use super::test_authority::manager;
use super::*;
use pretty_assertions::assert_eq;
use tokio::sync::Notify;

#[derive(Default)]
pub(super) struct LoadedPause {
    pub(super) loaded: Notify,
    pub(super) release: Notify,
}

impl LoadedPause {
    pub(super) async fn wait(&self) {
        self.loaded.notify_one();
        self.release.notified().await;
    }
}

fn stale(mode: AuthMode) -> AuthDotJson {
    let mut auth = document(mode, "registration-a");
    auth.last_refresh = Some(Utc::now() - chrono::Duration::days(9));
    if let Some(direct) = auth.tokens.as_mut().unwrap().direct.as_mut() {
        direct.expires_at = Utc::now() - chrono::Duration::seconds(1);
    }
    auth
}

fn save(home: &std::path::Path, doc: &AuthDotJson) {
    save_auth(
        home,
        doc,
        AuthCredentialsStoreMode::File,
        AuthKeyringBackendKind::default(),
    )
    .unwrap();
}

#[tokio::test]
async fn proactive_loaded_a_cannot_publish_over_b_or_return_a_credential_for_dispatch() {
    for mode in [AuthMode::Chatgpt, AuthMode::ChatgptDirect] {
        let home = tempfile::tempdir().unwrap();
        let original = stale(mode);
        let manager = manager(home.path(), &original, AuthCredentialsStoreMode::File).await;
        let mut rotated = document(mode, "registration-a");
        rotated.tokens.as_mut().unwrap().access_token = "fresh-a-rotation".into();
        save(home.path(), &rotated);
        let pause = Arc::new(LoadedPause::default());
        *manager.proactive_load_pause.lock().unwrap() = Some(pause.clone());
        let pending = {
            let manager = manager.clone();
            tokio::spawn(async move { manager.auth_for_request().await })
        };
        tokio::time::timeout(Duration::from_secs(2), pause.loaded.notified())
            .await
            .unwrap();
        // Request acquisition -> proactive refresh -> actual load of rotated A is now paused.
        let winner = document(AuthMode::ChatgptDirect, "registration-b");
        save(home.path(), &winner);
        manager.reload().await;
        let winning_revision = *manager.auth_change_state_receiver().borrow();
        pause.release.notify_one();
        assert!(matches!(
            tokio::time::timeout(Duration::from_secs(2), pending)
                .await
                .unwrap()
                .unwrap(),
            Err(RefreshTokenError::Permanent(_))
        ));
        assert_eq!(
            manager.auth_cached().unwrap().get_current_auth_json(),
            Some(winner.clone())
        );
        assert_eq!(
            *manager.auth_change_state_receiver().borrow(),
            winning_revision
        );
        let storage = create_auth_storage(
            home.path().into(),
            AuthCredentialsStoreMode::File,
            AuthKeyringBackendKind::default(),
        );
        assert_eq!(storage.load().unwrap(), Some(winner));
        // No credential was returned for a request or authority retry. The
        // fixture additionally denies non-injected production network routes.
    }
}

#[tokio::test]
async fn proactive_owner_is_captured_before_waiting_for_the_refresh_semaphore() {
    for mode in [AuthMode::Chatgpt, AuthMode::ChatgptDirect] {
        let home = tempfile::tempdir().unwrap();
        let manager = manager(home.path(), &stale(mode), AuthCredentialsStoreMode::File).await;
        let held = manager.refresh_lock.acquire().await.unwrap();
        let mut pending = Box::pin(manager.auth_for_request());
        std::future::poll_fn(|cx| {
            assert!(pending.as_mut().poll(cx).is_pending());
            std::task::Poll::Ready(())
        })
        .await;
        let winner = document(AuthMode::ChatgptDirect, "registration-b");
        save(home.path(), &winner);
        manager.reload().await;
        drop(held);
        assert!(matches!(
            tokio::time::timeout(Duration::from_secs(2), pending)
                .await
                .unwrap(),
            Err(RefreshTokenError::Permanent(_))
        ));
        assert_eq!(
            manager.auth_cached().unwrap().get_current_auth_json(),
            Some(winner)
        );
    }
}

#[tokio::test]
async fn proactive_same_owner_rotation_and_explicit_file_reload_policy_are_preserved() {
    for mode in [AuthMode::Chatgpt, AuthMode::ChatgptDirect] {
        let home = tempfile::tempdir().unwrap();
        let manager = manager(home.path(), &stale(mode), AuthCredentialsStoreMode::File).await;
        let mut rotated = document(mode, "registration-a");
        rotated.tokens.as_mut().unwrap().access_token = "fresh-a-rotation".into();
        save(home.path(), &rotated);
        assert_eq!(
            manager.auth().await.unwrap().get_current_auth_json(),
            Some(rotated.clone())
        );
        let different = document(AuthMode::ChatgptDirect, "registration-b");
        save(home.path(), &different);
        assert_eq!(
            manager.auth().await.unwrap().get_current_auth_json(),
            Some(rotated)
        );
        let stale_again = stale(mode);
        save(home.path(), &stale_again);
        manager.reload().await;
        save(home.path(), &different);
        assert_eq!(
            manager.auth().await.unwrap().get_current_auth_json(),
            Some(stale_again)
        );
        manager.reload().await;
        assert_eq!(
            manager.auth_cached().unwrap().get_current_auth_json(),
            Some(different)
        );
    }
}
