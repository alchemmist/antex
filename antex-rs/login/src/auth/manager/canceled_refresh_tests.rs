//! Cancellation before dispatch remains cancellable (refresh_tests.rs). These
//! tests instead cancel AFTER the authority has consumed a single-use token.
use super::super::test_authority::Authority;
use super::super::test_authority::document;
use super::super::test_authority::manager;
use super::super::test_authority::refresh;
use super::*;
use pretty_assertions::assert_eq;
use serde_json::json;

#[tokio::test]
async fn canceled_dispatched_refresh_finishes_rotation_before_an_independent_waiter_runs() {
    for mode in [
        AuthCredentialsStoreMode::File,
        AuthCredentialsStoreMode::Ephemeral,
    ] {
        let home = tempfile::tempdir().unwrap();
        let original = document(AuthMode::ChatgptDirect, "same-registration");
        let first = manager(home.path(), &original, mode).await;
        let second = manager(home.path(), &original, mode).await;
        let consumed = Arc::new(Mutex::new(std::collections::HashSet::new()));
        let authority_consumed = consumed.clone();
        let mut authority = Authority::start(move |_, request| {
            let token = request["refresh_token"].as_str().unwrap().to_string();
            if !authority_consumed.lock().unwrap().insert(token) {
                return (400, json!({"error":"invalid_grant", "error_description":"refresh token reuse detected"}));
            }
            assert_eq!(request["client_id"], "same-registration");
            (200, json!({"access_token":"rotated-access", "refresh_token":"single-use-next",
                "scope":"chatgpt.tokens.use.direct", "expires_in":3600}))
        }).await;
        let first_caller = {
            let manager = first.clone();
            let url = authority.url.clone();
            tokio::spawn(async move { refresh(&manager, &url).await })
        };
        authority.requests.recv().await.unwrap();
        assert_eq!(
            *consumed.lock().unwrap(),
            std::collections::HashSet::from(["refresh-original".to_string()])
        );
        first_caller.abort();
        assert!(first_caller.await.unwrap_err().is_cancelled());
        let storage =
            create_auth_storage(home.path().into(), mode, AuthKeyringBackendKind::default());
        assert!(
            storage.try_refresh_lease().unwrap().is_none(),
            "dispatched job must still own token consumption"
        );
        let mut second_caller = {
            let manager = second.clone();
            let url = authority.url.clone();
            tokio::spawn(async move { refresh(&manager, &url).await })
        };
        // The runtime remains live. The second manager cannot consume anything
        // until the first transaction has received and persisted its rotation.
        assert!(
            tokio::time::timeout(Duration::from_millis(30), &mut second_caller)
                .await
                .is_err()
        );
        assert_eq!(authority.calls(), 1);
        authority.release();
        tokio::time::timeout(Duration::from_secs(2), second_caller)
            .await
            .unwrap()
            .unwrap()
            .unwrap();
        assert_eq!(authority.calls(), 1);
        let stored = storage.load().unwrap().unwrap();
        let rotated = stored.tokens.clone().unwrap();
        assert_eq!(rotated.access_token, "rotated-access");
        assert_eq!(rotated.refresh_token, "single-use-next");
        assert_eq!(
            second.auth_cached().unwrap().get_token_data().unwrap(),
            rotated
        );
        // Detached work owns storage, NOT the abandoned caller's manager/cache.
        assert_eq!(
            first.auth_cached().unwrap().get_token_data().unwrap(),
            original.tokens.unwrap()
        );
        first.reload().await;
        assert_eq!(
            first.auth_cached().unwrap().get_token_data().unwrap(),
            rotated
        );
        if mode == AuthCredentialsStoreMode::Ephemeral {
            assert_eq!(std::fs::read_dir(home.path()).unwrap().count(), 0);
        }
    }
}

#[tokio::test]
async fn same_registration_with_a_different_account_is_not_accepted_as_rotation() {
    let home = tempfile::tempdir().unwrap();
    let original = document(AuthMode::ChatgptDirect, "same-registration");
    let manager = manager(home.path(), &original, AuthCredentialsStoreMode::File).await;
    let mut replacement = original.clone();
    replacement.tokens.as_mut().unwrap().account_id = Some("different-account".to_string());
    save_auth(
        home.path(),
        &replacement,
        AuthCredentialsStoreMode::File,
        AuthKeyringBackendKind::default(),
    )
    .unwrap();
    let authority = Authority::start(|_, _| (500, json!({"error":"unexpected exchange"}))).await;
    assert!(
        tokio::time::timeout(Duration::from_secs(2), refresh(&manager, &authority.url))
            .await
            .unwrap()
            .is_err()
    );
    assert_eq!(authority.calls(), 0);
    assert_eq!(
        manager.auth_cached().unwrap().get_token_data().unwrap(),
        original.tokens.unwrap()
    );
    let storage = create_auth_storage(
        home.path().into(),
        AuthCredentialsStoreMode::File,
        AuthKeyringBackendKind::default(),
    );
    assert_eq!(storage.load().unwrap(), Some(replacement));
}

#[tokio::test]
async fn logout_or_new_direct_login_supersedes_a_detached_exchange_without_resurrection() {
    for original_mode in [AuthMode::ChatgptDirect, AuthMode::Chatgpt] {
        for replace_with_direct in [true, false] {
            let home = tempfile::tempdir().unwrap();
            let original = document(original_mode, "registration-a");
            let manager = manager(home.path(), &original, AuthCredentialsStoreMode::File).await;
            let mut authority = Authority::start(|_, request| {
                assert_eq!(request["refresh_token"], "refresh-original");
                (
                    200,
                    json!({"access_token":"old-owner-rotated", "refresh_token":"old-owner-next",
                    "scope":"chatgpt.tokens.use.direct", "expires_in":3600}),
                )
            })
            .await;
            let caller = {
                let manager = manager.clone();
                let url = authority.url.clone();
                tokio::spawn(async move { refresh(&manager, &url).await })
            };
            authority.requests.recv().await.unwrap();
            caller.abort();
            assert!(caller.await.unwrap_err().is_cancelled());
            let storage = create_auth_storage(
                home.path().into(),
                AuthCredentialsStoreMode::File,
                AuthKeyringBackendKind::default(),
            );
            assert!(storage.try_refresh_lease().unwrap().is_none());
            let winner = if replace_with_direct {
                let replacement = document(AuthMode::ChatgptDirect, "registration-b");
                save_auth(
                    home.path(),
                    &replacement,
                    AuthCredentialsStoreMode::File,
                    AuthKeyringBackendKind::default(),
                )
                .unwrap();
                manager.reload().await;
                Some(replacement)
            } else {
                // Refresh ownership is separate: logout need not wait for HTTP.
                tokio::time::timeout(Duration::from_secs(2), manager.logout())
                    .await
                    .unwrap()
                    .unwrap();
                None
            };
            authority.release();
            // Acquisition is a completion barrier for the detached transaction,
            // including its attempted CAS. No fabricated commit result is used.
            let _finished = tokio::time::timeout(
                Duration::from_secs(2),
                storage_transaction::refresh_lease(storage.clone()),
            )
            .await
            .unwrap()
            .unwrap();
            assert_eq!(storage.load().unwrap(), winner);
            let cached = manager.auth_cached();
            assert_eq!(
                cached.as_ref().map(|auth| auth.get_token_data().unwrap()),
                winner.and_then(|doc| doc.tokens)
            );
            if let Some(cached) = cached {
                assert!(manager.refresh_failure_for_auth(&cached).is_none());
            }
            assert_eq!(authority.calls(), 1);
        }
    }
}
