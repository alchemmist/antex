//! Exercise the real temp-script command provider, including recovery captured
//! before its first resolve. No production command, credential or host is used.
use super::super::test_authority;
use super::*;
use pretty_assertions::assert_eq;

#[tokio::test]
async fn original_lazy_source_can_materialize_and_rotate_opaque_bearers() {
    for capture_before_setup in [true, false] {
        let script =
            ProviderAuthScript::new(&["first-bearer", "second-bearer", "third-bearer"]).unwrap();
        let mut config = script.auth_config();
        config.refresh_interval_ms = 0;
        let manager = AuthManager::external_bearer_only(config);
        assert_eq!(manager.auth_cached(), None);
        if !capture_before_setup {
            manager.auth().await.unwrap();
        }
        let mut recovery = manager.unauthorized_recovery();
        let original_generation = recovery.owner_generation();
        let original_source = manager.external_auth_provider().unwrap();
        recovery.ensure_same_owner().unwrap();
        assert_eq!(
            manager.auth().await.unwrap().get_token().unwrap(),
            "first-bearer"
        );
        // This is the guard core invokes AFTER lazy auth/client setup.
        recovery.ensure_same_owner().unwrap();
        assert!(recovery.has_next());
        recovery.next().await.unwrap();
        assert_eq!(
            manager.auth_cached().unwrap().get_token().unwrap(),
            "second-bearer"
        );
        recovery.ensure_same_owner().unwrap();
        assert!(!recovery.has_next());
        manager.refresh_token_from_authority().await.unwrap();
        assert_eq!(
            manager.auth_cached().unwrap().get_token().unwrap(),
            "third-bearer"
        );
        recovery.ensure_same_owner().unwrap();
        assert!(Arc::ptr_eq(
            &original_source,
            &manager.external_auth_provider().unwrap()
        ));
        assert_eq!(recovery.owner_generation(), original_generation);
        assert!(manager.auth_change_state_tx.borrow().owner_generation > original_generation);
    }
}

#[tokio::test]
async fn replacement_external_arc_is_rejected_even_when_token_and_generation_are_unchanged() {
    for capture_before_setup in [true, false] {
        let script = ProviderAuthScript::new(&["same-bearer"]).unwrap();
        let manager = AuthManager::external_bearer_only(script.auth_config());
        if !capture_before_setup {
            manager.auth().await.unwrap();
        }
        let recovery = manager.unauthorized_recovery();
        manager.auth().await.unwrap();
        recovery.ensure_same_owner().unwrap();
        let before = *manager.auth_change_state_tx.borrow();
        manager
            .set_external_auth(Arc::new(StaticExternalAuth(AntexAuth::from_api_key(
                "same-bearer",
            ))))
            .await
            .unwrap();
        assert_eq!(*manager.auth_change_state_tx.borrow(), before);
        assert!(recovery.ensure_same_owner().is_err());
        assert_eq!(recovery.unavailable_reason(), "auth_owner_changed");
        assert!(!recovery.has_next());
    }
}

#[tokio::test]
async fn same_external_source_does_not_authorize_other_modes_accounts_or_direct_registrations() {
    let home = tempfile::tempdir().unwrap();
    let direct_manager = test_authority::manager(
        home.path(),
        &test_authority::document(AuthMode::ChatgptDirect, "unrelated-direct-registration"),
        AuthCredentialsStoreMode::File,
    )
    .await;
    for different in [
        external_header_auth(/*account_id*/ None),
        external_header_auth(Some("different-account")),
        direct_manager.auth_cached().unwrap(),
    ] {
        let script = ProviderAuthScript::new(&["opaque-bearer"]).unwrap();
        let manager = AuthManager::external_bearer_only(script.auth_config());
        let recovery = manager.unauthorized_recovery();
        let source = manager.external_auth_provider().unwrap();
        manager.auth().await.unwrap();
        recovery.ensure_same_owner().unwrap();
        manager.set_cached_auth(Some(different));
        assert!(Arc::ptr_eq(
            &source,
            &manager.external_auth_provider().unwrap()
        ));
        assert!(recovery.ensure_same_owner().is_err());
    }
}

#[tokio::test]
async fn removing_the_original_source_invalidates_even_an_unmaterialized_owner() {
    for materialize in [false, true] {
        let script = ProviderAuthScript::new(&["opaque-bearer"]).unwrap();
        let manager = AuthManager::external_bearer_only(script.auth_config());
        let recovery = manager.unauthorized_recovery();
        if materialize {
            manager.auth().await.unwrap();
        }
        recovery.ensure_same_owner().unwrap();
        manager.clear_external_auth();
        assert_eq!(manager.auth_cached(), None);
        assert!(manager.external_auth_provider().is_none());
        assert!(recovery.ensure_same_owner().is_err());
    }
}

#[test]
fn no_external_source_does_not_gain_a_lazy_materialization_exception() {
    let manager = AuthManager::from_optional_auth_for_testing(/*auth*/ None);
    let recovery = manager.unauthorized_recovery();
    recovery.ensure_same_owner().unwrap();
    manager.set_cached_auth(Some(AntexAuth::from_api_key("unrelated-bearer")));
    assert!(recovery.ensure_same_owner().is_err());
}
