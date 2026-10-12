//! Diagnostic6: local legacy reload is advisory, active resolution is not.
use super::super::publication_tests::LoadedPause;
use super::super::test_authority::document;
use super::super::test_authority::manager;
use super::*;
use pretty_assertions::assert_eq;

fn stale(mode: AuthMode) -> AuthDotJson {
    let mut auth = document(mode, "registration-a");
    auth.last_refresh = Some(Utc::now() - chrono::Duration::days(9));
    if let Some(direct) = auth.tokens.as_mut().unwrap().direct.as_mut() {
        direct.expires_at = Utc::now() - chrono::Duration::seconds(1);
    }
    auth
}

fn save(home: &std::path::Path, auth: &AuthDotJson) {
    save_auth(
        home,
        auth,
        AuthCredentialsStoreMode::File,
        AuthKeyringBackendKind::default(),
    )
    .unwrap();
}

#[tokio::test]
async fn skipped_legacy_reload_keeps_cached_request_auth_but_explicit_refresh_stays_strict() {
    for missing_home in [false, true] {
        let home = tempfile::tempdir().unwrap();
        let original = stale(AuthMode::Chatgpt);
        let manager = manager(home.path(), &original, AuthCredentialsStoreMode::File).await;
        let revisions = *manager.auth_change_state_receiver().borrow();
        let owner = manager.unauthorized_recovery();
        if missing_home {
            // Reproduce the OLD CloudConfig helper's actual lifetime, not a
            // substitute loader or a fixture change in that suite.
            drop(home);
        } else {
            let mut other = original.clone();
            other.tokens.as_mut().unwrap().account_id = Some("workspace-b".into());
            save(home.path(), &other);
        }
        for _ in 0..2 {
            let auth = manager.auth_for_request().await.unwrap().unwrap();
            let (paired, _factory) = manager
                .auth_for_request_with_http_client_factory()
                .await
                .unwrap()
                .unwrap();
            assert_eq!(
                (
                    auth.get_current_auth_json(),
                    paired.get_current_auth_json(),
                    *manager.auth_change_state_receiver().borrow()
                ),
                (Some(original.clone()), Some(original.clone()), revisions)
            );
            owner.ensure_same_owner().unwrap();
        }
        assert_eq!(
            manager.refresh_token().await.unwrap_err().to_string(),
            REFRESH_TOKEN_ACCOUNT_MISMATCH_MESSAGE
        );
    }
}

#[derive(Clone, Copy)]
enum Replacement {
    Account,
    Logout,
    AccountAba,
    SourceEpoch,
}

#[tokio::test]
async fn skipped_reload_cannot_return_a_pair_after_cached_owner_or_epoch_replacement() {
    for replacement in [
        Replacement::Account,
        Replacement::Logout,
        Replacement::AccountAba,
        Replacement::SourceEpoch,
    ] {
        let home = tempfile::tempdir().unwrap();
        let original = stale(AuthMode::Chatgpt);
        let manager = manager(home.path(), &original, AuthCredentialsStoreMode::File).await;
        let mut other = original.clone();
        other.tokens.as_mut().unwrap().account_id = Some("workspace-b".into());
        save(home.path(), &other);
        let pause = Arc::new(LoadedPause::default());
        *manager.proactive_load_pause.lock().unwrap() = Some(pause.clone());
        let pending = {
            let manager = manager.clone();
            tokio::spawn(async move { manager.auth_for_request_with_http_client_factory().await })
        };
        tokio::time::timeout(Duration::from_secs(/*secs*/ 2), pause.loaded.notified())
            .await
            .unwrap();
        match replacement {
            Replacement::Account => {
                manager.reload().await;
            }
            Replacement::Logout => {
                manager.logout().await.unwrap();
            }
            Replacement::AccountAba => {
                manager.reload().await;
                save(home.path(), &original);
                manager.reload().await;
            }
            Replacement::SourceEpoch => manager.clear_external_auth(),
        }
        let winner = manager
            .auth_cached()
            .and_then(|auth| auth.get_current_auth_json());
        let revisions = *manager.auth_change_state_receiver().borrow();
        pause.release.notify_one();
        assert!(
            tokio::time::timeout(Duration::from_secs(/*secs*/ 2), pending)
                .await
                .unwrap()
                .unwrap()
                .is_err()
        );
        assert_eq!(
            (
                manager
                    .auth_cached()
                    .and_then(|auth| auth.get_current_auth_json()),
                *manager.auth_change_state_receiver().borrow()
            ),
            (winner, revisions)
        );
    }
}

#[tokio::test]
async fn legacy_fallback_does_not_hide_cached_permanent_failure_or_policy_restrictions() {
    let home = tempfile::tempdir().unwrap();
    let original = stale(AuthMode::Chatgpt);
    let manager = manager(home.path(), &original, AuthCredentialsStoreMode::File).await;
    drop(home);
    manager.set_forced_chatgpt_workspace_id(Some(vec!["workspace-b".into()]));
    assert!(matches!(manager.auth_for_request().await,
        Err(RefreshTokenError::Transient(error)) if error.kind() == std::io::ErrorKind::PermissionDenied));
    manager.set_forced_chatgpt_workspace_id(/*workspace_id*/ None);
    let failure =
        RefreshTokenFailedError::new(RefreshTokenFailedReason::Other, "authority rejected grant");
    manager
        .record_permanent_refresh_failure_if_unchanged(&manager.auth_cached().unwrap(), &failure);
    for _ in 0..2 {
        assert_eq!(
            manager.auth_for_request().await.unwrap_err().to_string(),
            failure.to_string()
        );
        assert!(
            manager
                .auth_for_request_with_http_client_factory()
                .await
                .is_err()
        );
    }
    assert_eq!(
        manager.auth_cached().unwrap().get_current_auth_json(),
        Some(original)
    );
}

#[tokio::test]
async fn retained_header_exception_does_not_relax_managed_or_direct_replacement() {
    for mode in [AuthMode::Chatgpt, AuthMode::ChatgptDirect] {
        let home = tempfile::tempdir().unwrap();
        let mut original = document(mode, "registration-a");
        original.tokens.as_mut().unwrap().id_token =
            parse_chatgpt_jwt_claims("header.e30.initial").unwrap();
        let manager = manager(home.path(), &original, AuthCredentialsStoreMode::File).await;
        let owner = manager.unauthorized_recovery();
        let mut other = original.clone();
        other.tokens.as_mut().unwrap().access_token = "replacement".into();
        if let Some(direct) = other.tokens.as_mut().unwrap().direct.as_mut() {
            direct.client_id = "registration-b".into();
        }
        save(home.path(), &other);
        manager.reload().await;
        assert!(owner.ensure_same_owner().is_err());
        assert!(owner.ensure_same_header_provider_owner().is_err());
        save(home.path(), &original);
        manager.reload().await;
        assert!(owner.ensure_same_owner().is_err());
        assert!(owner.ensure_same_header_provider_owner().is_err());
    }
}

#[tokio::test]
async fn authority_policy_failure_and_expired_direct_grants_do_not_use_legacy_fallback() {
    for mode in [AuthMode::Chatgpt, AuthMode::ChatgptDirect] {
        let home = tempfile::tempdir().unwrap();
        let original = stale(mode);
        // Existing helper denies non-injected authority destinations. Unchanged
        // storage reaches the real authority path and must surface that policy.
        let manager = manager(home.path(), &original, AuthCredentialsStoreMode::File).await;
        assert!(matches!(
            manager.auth_for_request().await,
            Err(RefreshTokenError::Policy(_))
        ));
        if mode == AuthMode::ChatgptDirect {
            drop(home);
            assert!(manager.auth_for_request().await.is_err());
            assert!(
                manager
                    .auth_for_request_with_http_client_factory()
                    .await
                    .is_err()
            );
        }
        assert_eq!(
            manager.auth_cached().unwrap().get_current_auth_json(),
            Some(original)
        );
    }
}
