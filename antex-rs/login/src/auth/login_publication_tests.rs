use super::*;
use crate::AuthCredentialsStoreMode;
use crate::AuthKeyringBackendKind;
use pretty_assertions::assert_eq;
use std::time::Duration;

fn options(home: &std::path::Path) -> ServerOptions {
    let mut opts = ServerOptions::new(
        home.into(),
        "unused".into(),
        /*forced_chatgpt_workspace_id*/ None,
        AuthCredentialsStoreMode::File,
        AuthKeyringBackendKind::default(),
        crate::test_support::transport_default_auth_route_config(),
    );
    opts.open_browser = false;
    opts
}

fn grant() -> AuthDotJson {
    serde_json::from_value(serde_json::json!({
        "auth_mode":"chatgptDirect", "last_refresh":chrono::Utc::now(),
        "tokens":{"id_token":"e30.e30.sig", "access_token":"access", "refresh_token":"refresh",
            "direct":{"client_id":"issued", "expires_at":chrono::Utc::now() + chrono::Duration::hours(1),
                "scopes":["chatgpt.tokens.use.direct"]}}
    })).unwrap()
}

#[tokio::test]
async fn deadline_cancels_publication_waiter_without_delayed_write() {
    let home = tempfile::tempdir().unwrap();
    let pending = Publication::capture(&options(home.path())).unwrap();
    let storage = pending.storage.clone();
    let held = super::super::storage_transaction::begin(storage.clone())
        .await
        .unwrap();
    let attempt = Arc::new(LoginAttempt::default());
    let shutdown = tokio::sync::Notify::new();
    let deadline = Instant::now() + Duration::from_millis(30);
    assert!(
        tokio::time::timeout(
            Duration::from_secs(2),
            pending.publish(grant(), attempt.clone(), &shutdown, deadline)
        )
        .await
        .unwrap()
        .is_err()
    );
    assert!(attempt.cancel());
    drop(held);
    let after = super::super::storage_transaction::begin(storage)
        .await
        .unwrap();
    assert_eq!(after.load().unwrap(), None);
}

#[tokio::test]
async fn dropping_a_publication_waiter_invalidates_its_attempt() {
    let home = tempfile::tempdir().unwrap();
    let pending = Publication::capture(&options(home.path())).unwrap();
    let storage = pending.storage.clone();
    let held = super::super::storage_transaction::begin(storage.clone())
        .await
        .unwrap();
    let attempt = Arc::new(LoginAttempt::default());
    let notify = tokio::sync::Notify::new();
    assert!(
        tokio::time::timeout(
            Duration::from_millis(30),
            pending.publish(
                grant(),
                attempt.clone(),
                &notify,
                Instant::now() + Duration::from_secs(2)
            )
        )
        .await
        .is_err()
    );
    assert_eq!(attempt.0.load(Ordering::Acquire), CANCELED);
    drop(held);
    let after = super::super::storage_transaction::begin(storage)
        .await
        .unwrap();
    assert_eq!(after.load().unwrap(), None);
}

#[tokio::test]
async fn logout_supersedes_pending_login_even_when_store_was_already_empty() {
    for had_credentials in [false, true] {
        let home = tempfile::tempdir().unwrap();
        if had_credentials {
            crate::login_with_api_key(
                home.path(),
                "original",
                AuthCredentialsStoreMode::File,
                AuthKeyringBackendKind::default(),
            )
            .unwrap();
        }
        let pending = Publication::capture(&options(home.path())).unwrap();
        let storage = pending.storage.clone();
        let held = super::super::storage_transaction::begin(storage.clone())
            .await
            .unwrap();
        let publishing = tokio::spawn(async move {
            pending
                .publish(
                    grant(),
                    Arc::new(LoginAttempt::default()),
                    &tokio::sync::Notify::new(),
                    Instant::now() + Duration::from_secs(2),
                )
                .await
        });
        // This is the same transactional delete used by logout. It wins while
        // publication is waiting, including the None -> None case.
        held.delete().unwrap();
        drop(held);
        assert!(publishing.await.unwrap().is_err());
        assert_eq!(storage.load().unwrap(), None);
    }
}

#[tokio::test]
async fn canceled_attempt_cannot_commit_even_if_its_notification_was_consumed() {
    let home = tempfile::tempdir().unwrap();
    let pending = Publication::capture(&options(home.path())).unwrap();
    let storage = pending.storage.clone();
    let attempt = Arc::new(LoginAttempt::default());
    assert!(attempt.cancel());
    assert!(
        pending
            .publish(
                grant(),
                attempt,
                &tokio::sync::Notify::new(),
                Instant::now() + Duration::from_secs(2)
            )
            .await
            .is_err()
    );
    assert_eq!(storage.load().unwrap(), None);
}

#[test]
fn cancellation_and_commit_have_one_linearization_point() {
    let attempt = Arc::new(LoginAttempt::default());
    let committing = attempt.clone();
    let (started_tx, started_rx) = std::sync::mpsc::channel();
    let (release_tx, release_rx) = std::sync::mpsc::channel();
    let writer = std::thread::spawn(move || {
        committing.commit(Instant::now() + Duration::from_secs(2), || {
            started_tx.send(()).unwrap();
            release_rx.recv().unwrap();
            Ok(())
        })
    });
    started_rx.recv().unwrap();
    assert!(
        !attempt.cancel(),
        "publication already owns the commit; cancellation must not claim to win"
    );
    release_tx.send(()).unwrap();
    writer.join().unwrap().unwrap();
}
