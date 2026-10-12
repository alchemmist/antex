use super::*;
use pretty_assertions::assert_eq;

#[tokio::test]
async fn failed_active_auth_resolution_cannot_skip_required_policy_or_clear_cached_policy() {
    let home = tempdir().unwrap();
    let manager = Arc::new(
        AuthManager::new(
            home.path().to_path_buf(),
            /*enable_antex_api_key_env*/ false,
            AuthCredentialsStoreMode::Ephemeral,
            /*forced_chatgpt_workspace_id*/ None,
            /*chatgpt_base_url*/ None,
            AuthKeyringBackendKind::default(),
            antex_login::test_support::transport_default_auth_route_config(),
        )
        .await,
    );
    let initial = AntexAuth::from_external_chatgpt_tokens(
        &fake_chatgpt_jwt("business", Some("user-12345"), b"initial"),
        "account-12345",
        Some("business"),
    )
    .unwrap();
    let foreign = AntexAuth::from_external_chatgpt_tokens(
        &fake_chatgpt_jwt("business", Some("user-99999"), b"foreign"),
        "account-99999",
        Some("business"),
    )
    .unwrap();
    let source = Arc::new(TestExternalChatgptAuth {
        current: RwLock::new(initial.clone()),
        refreshed: initial.clone(),
        refresh_count: AtomicUsize::new(0),
    });
    manager.set_external_auth(source.clone()).await.unwrap();
    let revision = *manager.auth_change_state_receiver().borrow();
    let fetcher = Arc::new(TokenBundleClient {
        expected_token: initial.get_token().unwrap(),
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
    assert_eq!(service.get_latest().await, Ok(Some(test_bundle())));
    assert_eq!(fetcher.request_count.load(Ordering::SeqCst), 1);

    *source.current.write().unwrap() = foreign;
    // A failed authoritative lookup is not an anonymous/unenrolled account,
    // even when an identity-matched bundle is already available on disk.
    for _ in 0..2 {
        assert_eq!(
            service.load_startup_bundle().await.unwrap_err().code(),
            CloudConfigBundleLoadErrorCode::Auth
        );
        assert!(service.refresh_cache_once().await);
    }
    assert_eq!(fetcher.request_count.load(Ordering::SeqCst), 1);
    assert_eq!(source.refresh_count.load(Ordering::SeqCst), 0);
    assert_eq!(
        manager.auth_cached().unwrap().get_token_data().unwrap(),
        initial.get_token_data().unwrap()
    );
    assert_eq!(*manager.auth_change_state_receiver().borrow(), revision);
    assert_eq!(service.get_latest().await, Ok(Some(test_bundle())));
    assert_eq!(
        service
            .cache
            .load(Some("user-12345"), Some("account-12345"))
            .await
            .unwrap()
            .bundle,
        test_bundle()
    );
}
