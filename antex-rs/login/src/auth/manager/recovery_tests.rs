use super::super::test_authority::Authority;
use super::super::test_authority::document;
use super::super::test_authority::manager;
use super::*;
use pretty_assertions::assert_eq;
use serde_json::json;

#[tokio::test]
async fn in_flight_401_cannot_retry_a_prompt_under_a_new_registration_or_mode() {
    for original_mode in [AuthMode::ChatgptDirect, AuthMode::Chatgpt] {
        let home = tempfile::tempdir().unwrap();
        let original = document(original_mode, "registration-a");
        let manager = manager(home.path(), &original, AuthCredentialsStoreMode::File).await;
        let mut authority = Authority::start(|_, _| (401, json!({"error":"unauthorized"}))).await;
        let mut recovery = manager.unauthorized_recovery();
        let generation = recovery.owner_generation();
        let client = antex_http_client::HttpClientBuilder::new()
            .build_direct()
            .unwrap();
        let token = manager.auth_cached().unwrap().get_token().unwrap();
        let running = {
            let manager = manager.clone();
            let url = authority.url.clone();
            tokio::spawn(async move {
                let response = client
                    .post(&url)
                    .header("Authorization", format!("Bearer {token}"))
                    .json(&json!({"input":"original private prompt"}))
                    .send()
                    .await
                    .unwrap();
                assert_eq!(response.status().as_u16(), 401);
                recovery.next().await?;
                // The outer retry loop must retain/check this same operation binding.
                recovery.ensure_same_owner()?;
                let auth = manager.auth_cached().unwrap();
                let _ = client
                    .post(url)
                    .header(
                        "Authorization",
                        format!("Bearer {}", auth.get_token().unwrap()),
                    )
                    .json(&json!({"input":"original private prompt"}))
                    .send()
                    .await
                    .unwrap();
                Ok::<_, RefreshTokenError>(())
            })
        };
        authority.requests.recv().await.unwrap();
        let replacement = document(AuthMode::ChatgptDirect, "registration-b");
        save_auth(
            home.path(),
            &replacement,
            AuthCredentialsStoreMode::File,
            AuthKeyringBackendKind::default(),
        )
        .unwrap();
        manager.reload().await;
        assert!(manager.auth_change_state_tx.borrow().owner_generation > generation);
        authority.release();
        assert!(running.await.unwrap().is_err());
        assert_eq!(authority.calls(), 1);
        let b = manager.auth_cached().unwrap();
        assert_eq!(b.get_token_data().unwrap(), replacement.tokens.unwrap());
        assert!(manager.refresh_failure_for_auth(&b).is_none());
    }
}

#[tokio::test]
async fn owner_is_checked_between_reload_refresh_and_outer_retry_dispatch() {
    let home = tempfile::tempdir().unwrap();
    let original = document(AuthMode::ChatgptDirect, "registration-a");
    let manager = manager(home.path(), &original, AuthCredentialsStoreMode::File).await;
    let mut recovery = manager.unauthorized_recovery();
    recovery.next().await.unwrap();
    assert_eq!(recovery.step_name(), "refresh_token");
    let b = document(AuthMode::ChatgptDirect, "registration-b");
    save_auth(
        home.path(),
        &b,
        AuthCredentialsStoreMode::File,
        AuthKeyringBackendKind::default(),
    )
    .unwrap();
    manager.reload().await;
    assert!(recovery.ensure_same_owner().is_err());
    assert!(!recovery.has_next());
    assert_eq!(recovery.unavailable_reason(), "auth_owner_changed");
    assert!(recovery.next().await.is_err());
    assert!(
        manager
            .refresh_failure_for_auth(&manager.auth_cached().unwrap())
            .is_none()
    );
    // A -> B -> A must not conceal the intervening owner transition.
    save_auth(
        home.path(),
        &original,
        AuthCredentialsStoreMode::File,
        AuthKeyringBackendKind::default(),
    )
    .unwrap();
    manager.reload().await;
    assert!(recovery.ensure_same_owner().is_err());
}

#[tokio::test]
async fn rotation_within_the_original_direct_registration_remains_recoverable() {
    let home = tempfile::tempdir().unwrap();
    let original = document(AuthMode::ChatgptDirect, "registration-a");
    let manager = manager(home.path(), &original, AuthCredentialsStoreMode::File).await;
    let mut recovery = manager.unauthorized_recovery();
    let generation = recovery.owner_generation();
    let mut rotated = original;
    rotated.tokens.as_mut().unwrap().access_token = "new-access".into();
    rotated.tokens.as_mut().unwrap().refresh_token = "new-refresh".into();
    save_auth(
        home.path(),
        &rotated,
        AuthCredentialsStoreMode::File,
        AuthKeyringBackendKind::default(),
    )
    .unwrap();
    // Rotation may arrive via shared-manager reload while the request is in flight.
    manager.reload().await;
    assert_eq!(
        manager.auth_change_state_tx.borrow().owner_generation,
        generation
    );
    recovery.next().await.unwrap();
    recovery.ensure_same_owner().unwrap();
    assert!(recovery.has_next());
    assert_eq!(
        manager.auth_cached().unwrap().get_token_data().unwrap(),
        rotated.tokens.unwrap()
    );
}

#[tokio::test]
async fn guarded_reload_publication_cannot_overwrite_a_concurrently_installed_owner() {
    let home = tempfile::tempdir().unwrap();
    let original = document(AuthMode::ChatgptDirect, "registration-a");
    let manager = manager(home.path(), &original, AuthCredentialsStoreMode::File).await;
    let owner = RecoveryOwner::capture(&manager);
    let stale_loaded = manager.auth_cached();
    let replacement = document(AuthMode::ChatgptDirect, "registration-b");
    save_auth(
        home.path(),
        &replacement,
        AuthCredentialsStoreMode::File,
        AuthKeyringBackendKind::default(),
    )
    .unwrap();
    manager.reload().await;
    assert!(
        manager
            .set_cached_auth_with_owner(stale_loaded, Some(&owner))
            .is_err()
    );
    assert_eq!(
        manager.auth_cached().unwrap().get_token_data().unwrap(),
        replacement.tokens.unwrap()
    );
}

#[tokio::test]
async fn owner_is_rechecked_after_waiting_for_the_per_manager_refresh_semaphore() {
    let home = tempfile::tempdir().unwrap();
    let original = document(AuthMode::ChatgptDirect, "registration-a");
    let manager = manager(home.path(), &original, AuthCredentialsStoreMode::File).await;
    let mut recovery = manager.unauthorized_recovery();
    recovery.next().await.unwrap();
    let permit = manager.refresh_lock.acquire().await.unwrap();
    let waiting = tokio::spawn(async move { recovery.next().await });
    let b = document(AuthMode::ChatgptDirect, "registration-b");
    save_auth(
        home.path(),
        &b,
        AuthCredentialsStoreMode::File,
        AuthKeyringBackendKind::default(),
    )
    .unwrap();
    manager.reload().await;
    drop(permit);
    assert!(waiting.await.unwrap().is_err());
    assert!(
        manager
            .refresh_failure_for_auth(&manager.auth_cached().unwrap())
            .is_none()
    );
}
