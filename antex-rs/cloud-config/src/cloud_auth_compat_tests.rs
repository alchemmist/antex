use super::*;
use pretty_assertions::assert_eq;

async fn fresh_manager(home: &Path) -> Arc<AuthManager> {
    write_auth_json(
        home,
        fresh_document(Some("user-12345"), Some("account-12345")),
    )
    .unwrap();
    Arc::new(
        AuthManager::new(
            home.to_path_buf(),
            /*enable_antex_api_key_env*/ false,
            AuthCredentialsStoreMode::File,
            /*forced_chatgpt_workspace_id*/ None,
            /*chatgpt_base_url*/ None,
            AuthKeyringBackendKind::default(),
            antex_login::test_support::transport_default_auth_route_config(),
        )
        .await,
    )
}

fn fresh_document(user: Option<&str>, account: Option<&str>) -> serde_json::Value {
    chatgpt_auth_json_with_last_refresh(
        "business",
        user,
        account,
        "stale-access-token",
        "test-refresh-token",
        "3025-01-01T00:00:00Z",
    )
}

#[tokio::test]
async fn metadata_get_rebind_does_not_rebind_an_existing_sampling_guard() {
    let home = tempdir().unwrap();
    let manager = fresh_manager(home.path()).await;
    let mut sampling = manager.unauthorized_recovery();
    let mut replacement = fresh_document(Some("user-99999"), Some("account-12345"));
    replacement["tokens"]["access_token"] = json!("fresh-access-token");
    write_auth_json(home.path(), replacement).unwrap();
    let fetcher = Arc::new(TokenBundleClient {
        expected_token: "fresh-access-token".into(),
        bundle: test_bundle(),
        request_count: AtomicUsize::new(0),
    });
    let cache_home = tempdir().unwrap();
    let service = CloudConfigBundleService::new(
        manager.clone(),
        fetcher.clone(),
        cache_home.path().into(),
        CLOUD_CONFIG_BUNDLE_TIMEOUT,
    );
    assert_eq!(service.load_startup_bundle().await, Ok(Some(test_bundle())));
    assert_eq!(fetcher.request_count.load(Ordering::SeqCst), 2);
    assert!(sampling.ensure_same_owner().is_err());
    assert!(!sampling.has_next());
    assert!(sampling.next().await.is_err());
    assert_eq!(
        manager.auth_cached().unwrap().get_chatgpt_user_id(),
        Some("user-99999".into())
    );
    assert!(
        service
            .cache
            .load(Some("user-12345"), Some("account-12345"))
            .await
            .is_err()
    );
    assert_eq!(
        service
            .cache
            .load(Some("user-99999"), Some("account-12345"))
            .await
            .unwrap()
            .bundle,
        test_bundle()
    );
}

#[tokio::test]
async fn metadata_get_never_rebinds_external_users_or_accounts() {
    for (user, account) in [
        ("user-99999", "account-12345"),
        ("user-12345", "account-99999"),
    ] {
        let home = tempdir().unwrap();
        let manager = fresh_manager(home.path()).await;
        let initial = AntexAuth::from_external_chatgpt_tokens(
            &fake_chatgpt_jwt("business", Some("user-12345"), b"initial"),
            "account-12345",
            Some("business"),
        )
        .unwrap();
        let refreshed = AntexAuth::from_external_chatgpt_tokens(
            &fake_chatgpt_jwt("business", Some(user), b"foreign"),
            account,
            Some("business"),
        )
        .unwrap();
        let source = Arc::new(TestExternalChatgptAuth {
            current: RwLock::new(initial.clone()),
            refreshed,
            refresh_count: AtomicUsize::new(0),
        });
        manager.set_external_auth(source.clone()).await.unwrap();
        let sampling = manager.unauthorized_recovery();
        let revision = *manager.auth_change_state_receiver().borrow();
        let stored = antex_login::auth::load_auth_dot_json(
            home.path(),
            AuthCredentialsStoreMode::Ephemeral,
            AuthKeyringBackendKind::default(),
        )
        .unwrap();
        let fetcher = Arc::new(UnauthorizedBundleClient {
            message: "GET /config/bundle failed: 401".into(),
            request_count: AtomicUsize::new(0),
        });
        let cache_home = tempdir().unwrap();
        let service = CloudConfigBundleService::new(
            manager.clone(),
            fetcher.clone(),
            cache_home.path().into(),
            CLOUD_CONFIG_BUNDLE_TIMEOUT,
        );
        assert_eq!(
            service.load_startup_bundle().await.unwrap_err().code(),
            CloudConfigBundleLoadErrorCode::Auth
        );
        assert_eq!(fetcher.request_count.load(Ordering::SeqCst), 1);
        assert_eq!(source.refresh_count.load(Ordering::SeqCst), 1);
        assert_eq!(
            manager.auth_cached().unwrap().get_token_data().unwrap(),
            initial.get_token_data().unwrap()
        );
        assert_eq!(*manager.auth_change_state_receiver().borrow(), revision);
        assert_eq!(
            antex_login::auth::load_auth_dot_json(
                home.path(),
                AuthCredentialsStoreMode::Ephemeral,
                AuthKeyringBackendKind::default()
            )
            .unwrap(),
            stored
        );
        sampling.ensure_same_owner().unwrap();
        assert!(!service.cache.path().exists());
    }
}

#[tokio::test]
async fn metadata_get_rejects_foreign_scope_and_mode_without_cache_publication_or_retry() {
    let mut external_mode = fresh_document(Some("user-99999"), Some("account-12345"));
    external_mode["auth_mode"] = json!("chatgptAuthTokens");
    let api_mode = json!({"auth_mode":"apikey", "OPENAI_API_KEY":"synthetic-key"});
    for replacement in [
        fresh_document(Some("user-99999"), Some("account-99999")),
        fresh_document(Some("user-99999"), /*account*/ None),
        fresh_document(/*user*/ None, Some("account-12345")),
        external_mode,
        api_mode,
    ] {
        let home = tempdir().unwrap();
        let manager = fresh_manager(home.path()).await;
        let original = manager.auth_cached().unwrap().get_token_data().unwrap();
        let revision = *manager.auth_change_state_receiver().borrow();
        write_auth_json(home.path(), replacement).unwrap();
        let fetcher = Arc::new(UnauthorizedBundleClient {
            message: "GET /config/bundle failed: 401".into(),
            request_count: AtomicUsize::new(0),
        });
        let cache_home = tempdir().unwrap();
        let service = CloudConfigBundleService::new(
            manager.clone(),
            fetcher.clone(),
            cache_home.path().into(),
            CLOUD_CONFIG_BUNDLE_TIMEOUT,
        );
        assert_eq!(
            service.load_startup_bundle().await.unwrap_err(),
            CloudConfigBundleLoadError::new(
                CloudConfigBundleLoadErrorCode::Auth,
                Some(401),
                "Your access token could not be refreshed because you have since logged out or signed in to another account. Please sign in again.",
            )
        );
        assert_eq!(fetcher.request_count.load(Ordering::SeqCst), 1);
        assert_eq!(
            manager.auth_cached().unwrap().get_token_data().unwrap(),
            original
        );
        assert_eq!(*manager.auth_change_state_receiver().borrow(), revision);
        assert!(!service.cache.path().exists());
    }
}
