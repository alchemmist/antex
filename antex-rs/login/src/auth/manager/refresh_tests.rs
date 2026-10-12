use super::super::test_authority::Authority;
use super::super::test_authority::document;
use super::super::test_authority::manager;
use super::super::test_authority::refresh;
use super::*;
use pretty_assertions::assert_eq;
use serde_json::json;

#[tokio::test]
async fn paused_legacy_refresh_cannot_merge_into_direct_login_or_resurrect_logout() {
    for replace_with_direct in [true, false] {
        let home = tempfile::tempdir().unwrap();
        let original = document(AuthMode::Chatgpt, "unused");
        let manager = manager(home.path(), &original, AuthCredentialsStoreMode::File).await;
        let mut authority = Authority::start(|_, request| {
            assert_eq!(request["refresh_token"], "refresh-original");
            (
                200,
                json!({"access_token":"legacy-rotated", "refresh_token":"legacy-next"}),
            )
        })
        .await;
        let running = {
            let manager = manager.clone();
            let url = authority.url.clone();
            tokio::spawn(async move { refresh(&manager, &url).await })
        };
        authority.requests.recv().await.unwrap();
        let expected = if replace_with_direct {
            let direct = document(AuthMode::ChatgptDirect, "new-direct-registration");
            save_auth(
                home.path(),
                &direct,
                AuthCredentialsStoreMode::File,
                AuthKeyringBackendKind::default(),
            )
            .unwrap();
            Some(direct)
        } else {
            manager.logout().await.unwrap();
            None
        };
        authority.release();
        assert!(running.await.unwrap().is_err());
        let stored = create_auth_storage(
            home.path().into(),
            AuthCredentialsStoreMode::File,
            AuthKeyringBackendKind::default(),
        );
        assert_eq!(stored.load().unwrap(), expected);
        assert_eq!(authority.calls(), 1);
    }
}

#[tokio::test]
async fn two_independent_managers_consume_a_single_use_refresh_token_once() {
    for mode in [
        AuthCredentialsStoreMode::File,
        AuthCredentialsStoreMode::Ephemeral,
    ] {
        let home = tempfile::tempdir().unwrap();
        let original = document(AuthMode::ChatgptDirect, "same-registration");
        let first = manager(home.path(), &original, mode).await;
        let second = manager(home.path(), &original, mode).await;
        let mut authority = Authority::start(|index, request| {
            assert_eq!(request["client_id"], "same-registration");
            assert_eq!(request["resource"], crate::direct_oauth::RESOURCE);
            if index != 0 || request["refresh_token"] != "refresh-original" {
                return (
                    400,
                    json!({"error":"invalid_grant", "error_description":"single-use token reused"}),
                );
            }
            (
                200,
                json!({"access_token":"rotated-access", "refresh_token":"single-use-next",
                "scope":"chatgpt.tokens.use.direct", "expires_in":3600}),
            )
        })
        .await;
        let one = {
            let manager = first.clone();
            let url = authority.url.clone();
            tokio::spawn(async move { refresh(&manager, &url).await })
        };
        authority.requests.recv().await.unwrap();
        let two = {
            let manager = second.clone();
            let url = authority.url.clone();
            tokio::spawn(async move { refresh(&manager, &url).await })
        };
        // Both were constructed from the old grant. The second must wait/re-read
        // even if it is first polled after the response is released.
        authority.release();
        one.await.unwrap().unwrap();
        two.await.unwrap().unwrap();
        assert_eq!(authority.calls(), 1);
        assert_eq!(
            first.auth_cached().unwrap().get_token_data().unwrap(),
            second.auth_cached().unwrap().get_token_data().unwrap()
        );
        assert_eq!(
            second
                .auth_cached()
                .unwrap()
                .get_token_data()
                .unwrap()
                .refresh_token,
            "single-use-next"
        );
        if mode == AuthCredentialsStoreMode::Ephemeral {
            assert_eq!(std::fs::read_dir(home.path()).unwrap().count(), 0);
        }
    }
}

#[tokio::test]
async fn canceled_refresh_waiter_never_consumes_a_token_later() {
    let home = tempfile::tempdir().unwrap();
    let original = document(AuthMode::ChatgptDirect, "registration");
    let manager = manager(home.path(), &original, AuthCredentialsStoreMode::File).await;
    let storage = create_auth_storage(
        home.path().into(),
        AuthCredentialsStoreMode::File,
        AuthKeyringBackendKind::default(),
    );
    let held = storage.try_refresh_lease().unwrap().unwrap();
    let mut authority = Authority::start(|_, _| {
        (
            200,
            json!({"access_token":"next", "refresh_token":"next-refresh",
        "scope":"chatgpt.tokens.use.direct", "expires_in":3600}),
        )
    })
    .await;
    assert!(
        tokio::time::timeout(Duration::from_millis(30), refresh(&manager, &authority.url))
            .await
            .is_err()
    );
    assert_eq!(authority.calls(), 0);
    drop(held);
    let running = {
        let manager = manager.clone();
        let url = authority.url.clone();
        tokio::spawn(async move { refresh(&manager, &url).await })
    };
    authority.requests.recv().await.unwrap();
    authority.release();
    running.await.unwrap().unwrap();
    assert_eq!(authority.calls(), 1);
}

#[tokio::test]
async fn invalid_grant_is_cached_only_for_unchanged_credentials_and_new_login_clears_it() {
    let home = tempfile::tempdir().unwrap();
    let mut original = document(AuthMode::ChatgptDirect, "registration-a");
    original
        .tokens
        .as_mut()
        .unwrap()
        .direct
        .as_mut()
        .unwrap()
        .expires_at = Utc::now() - chrono::Duration::seconds(1);
    let manager = manager(home.path(), &original, AuthCredentialsStoreMode::File).await;
    let mut authority = Authority::start(|_, _| {
        (
            400,
            json!({"error":"invalid_grant",
        "error_description":"issuer-secret access-original refresh-original"}),
        )
    })
    .await;
    let running = {
        let manager = manager.clone();
        let url = authority.url.clone();
        tokio::spawn(async move { refresh(&manager, &url).await })
    };
    authority.requests.recv().await.unwrap();
    authority.release();
    let error = running.await.unwrap().unwrap_err();
    assert!(matches!(error, RefreshTokenError::Permanent(_)));
    assert!(!format!("{error:?}").contains("issuer-secret"));
    assert!(!format!("{error:?}").contains("refresh-original"));
    assert!(
        manager
            .refresh_failure_for_auth(&manager.auth_cached().unwrap())
            .is_some()
    );
    // This is the same cache gate called by proactive auth()/401 recovery.
    assert!(refresh(&manager, &authority.url).await.is_err());
    assert_eq!(authority.calls(), 1);
    for _ in 0..2 {
        let cached = manager.auth().await.unwrap();
        assert!(cached.get_token().is_err());
        assert!(manager.refresh_failure_for_auth(&cached).is_some());
    }
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
            .refresh_failure_for_auth(&manager.auth_cached().unwrap())
            .is_none()
    );
    assert!(refresh(&manager, &authority.url).await.is_err());
    assert_eq!(authority.calls(), 2);
}

#[tokio::test]
async fn server_failures_remain_retryable_and_policy_denial_keeps_its_type() {
    let home = tempfile::tempdir().unwrap();
    let original = document(AuthMode::ChatgptDirect, "registration");
    let manager = manager(home.path(), &original, AuthCredentialsStoreMode::File).await;
    let mut authority = Authority::start(|_, _| {
        (
            503,
            json!({"error":"invalid_grant", "error_description":"private-body"}),
        )
    })
    .await;
    let running = {
        let manager = manager.clone();
        let url = authority.url.clone();
        tokio::spawn(async move { refresh(&manager, &url).await })
    };
    authority.requests.recv().await.unwrap();
    authority.release();
    assert!(matches!(
        running.await.unwrap(),
        Err(RefreshTokenError::Transient(_))
    ));
    assert!(matches!(
        refresh(&manager, &authority.url).await,
        Err(RefreshTokenError::Transient(_))
    ));
    assert_eq!(authority.calls(), 2);
    assert!(
        manager
            .refresh_failure_for_auth(&manager.auth_cached().unwrap())
            .is_none()
    );
    let policy = antex_http_client::NetworkPolicyController::default();
    let route = AuthRouteConfig::from_http_client_factory(
        HttpClientFactory::new(OutboundProxyPolicy::ReqwestDefault)
            .with_network_policy(policy.policy()),
    );
    let client = crate::direct_oauth::client(&route).unwrap();
    let result = crate::direct_oauth::refresh(&client, original.tokens.as_ref().unwrap()).await;
    assert!(matches!(result, Err(RefreshTokenError::Policy(_))));
}

#[tokio::test]
async fn late_failure_for_a_does_not_poison_b() {
    let home = tempfile::tempdir().unwrap();
    let manager = manager(
        home.path(),
        &document(AuthMode::ChatgptDirect, "registration-a"),
        AuthCredentialsStoreMode::File,
    )
    .await;
    let mut authority =
        Authority::start(|_, _| (400, json!({"error":"refresh_token_revoked"}))).await;
    let running = {
        let manager = manager.clone();
        let url = authority.url.clone();
        tokio::spawn(async move { refresh(&manager, &url).await })
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
    authority.release();
    assert!(running.await.unwrap().is_err());
    let current = manager.auth_cached().unwrap();
    assert_eq!(
        current.get_token_data().unwrap(),
        replacement.tokens.unwrap()
    );
    assert!(manager.refresh_failure_for_auth(&current).is_none());
}
