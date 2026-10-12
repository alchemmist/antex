use super::super::AuthConfig;
use super::super::AuthCredentialsStoreMode;
use super::super::AuthDotJson;
use super::super::AuthKeyringBackendKind;
use super::super::AuthMode;
use super::super::ForcedLoginMethod;
use super::super::ManagedAuthPolicy;
use super::super::ReloadOutcome;
use super::super::create_auth_storage;
use super::super::recovery::RecoveryOwner;
use super::*;
use pretty_assertions::assert_eq;

fn commit_refresh(
    auth: &ChatgptAuth,
    original: AuthDotJson,
    refreshed: crate::TokenData,
) -> std::io::Result<()> {
    let updated = refreshed_document(original.clone(), refreshed)?;
    auth.storage().replace_if_matches(&original, &updated)
}

fn document() -> AuthDotJson {
    serde_json::from_value(serde_json::json!({
        "auth_mode": "chatgptDirect",
        "tokens": {
            "id_token": "e30.e30.sig",
            "access_token": "direct-access-secret",
            "refresh_token": "direct-refresh-secret",
            "account_id": null,
            "direct": {
                "client_id": "issued-registration",
                "expires_at": Utc::now() + chrono::Duration::hours(1),
                "scopes": ["chatgpt.tokens.use.direct"]
            }
        },
        "last_refresh": Utc::now()
    }))
    .unwrap()
}

async fn load(home: &std::path::Path, doc: &AuthDotJson) -> AntexAuth {
    crate::save_auth(
        home,
        doc,
        AuthCredentialsStoreMode::File,
        AuthKeyringBackendKind::default(),
    )
    .unwrap();
    AntexAuth::from_auth_storage(
        home,
        AuthCredentialsStoreMode::File,
        /*chatgpt_base_url*/ None,
        AuthKeyringBackendKind::default(),
        &crate::test_support::transport_default_auth_route_config(),
    )
    .await
    .unwrap()
    .unwrap()
}

#[tokio::test]
async fn direct_storage_reload_getters_rotation_and_logout_work_without_codex_account_claims() {
    let home = tempfile::tempdir().unwrap();
    let original = document();
    let auth = load(home.path(), &original).await;
    assert_eq!(auth.api_auth_mode(), AuthMode::ChatgptDirect);
    assert!(auth.is_chatgpt_auth());
    assert!(!auth.uses_antex_backend());
    assert_eq!(auth.get_token().unwrap(), "direct-access-secret");
    assert_eq!(auth.get_account_id(), None);
    assert_eq!(
        auth.get_token_data().unwrap(),
        original.tokens.clone().unwrap()
    );
    let debug = format!("{auth:?} {original:?}");
    assert!(!debug.contains("direct-access-secret"));
    assert!(!debug.contains("direct-refresh-secret"));
    assert!(!debug.contains("e30.e30.sig"));
    let manager = AuthManager::from_auth_for_testing_with_home(auth.clone(), home.path().into());
    let owner = RecoveryOwner::capture(&manager);
    assert!(matches!(
        manager
            .reload_matching_owner(&owner, Some(&auth))
            .await
            .unwrap(),
        ReloadOutcome::ReloadedNoChange
    ));
    let mut rotated = original.tokens.clone().unwrap();
    rotated.access_token = "rotated-access".into();
    rotated.refresh_token = "rotated-refresh".into();
    let AntexAuth::ChatgptDirect(managed) = &auth else {
        panic!("wrong auth mode")
    };
    commit_refresh(managed, original.clone(), rotated.clone()).unwrap();
    assert!(matches!(
        manager
            .reload_matching_owner(&owner, Some(&auth))
            .await
            .unwrap(),
        ReloadOutcome::ReloadedChanged
    ));
    assert_eq!(
        manager.auth_cached().unwrap().get_token_data().unwrap(),
        rotated
    );
    assert_eq!(
        manager
            .auth_change_state_receiver()
            .borrow()
            .owner_generation,
        0
    );
    let mut expired = original;
    expired
        .tokens
        .as_mut()
        .unwrap()
        .direct
        .as_mut()
        .unwrap()
        .expires_at = Utc::now() - chrono::Duration::seconds(1);
    let expired = load(home.path(), &expired).await;
    assert!(expired.get_token().is_err());
    assert!(AuthManager::should_refresh_proactively(&expired));
    assert!(manager.logout().await.unwrap());
    assert_eq!(manager.auth_cached(), None);
    assert!(!home.path().join("auth.json").exists());
}

#[tokio::test]
async fn incomplete_or_mislabeled_direct_credentials_are_not_loaded_as_legacy() {
    let home = tempfile::tempdir().unwrap();
    let route = crate::test_support::transport_default_auth_route_config();
    let mut missing_metadata = document();
    missing_metadata.tokens.as_mut().unwrap().direct = None;
    let mut mislabeled = document();
    mislabeled.auth_mode = Some(AuthMode::Chatgpt);
    let mut missing_scope = document();
    missing_scope
        .tokens
        .as_mut()
        .unwrap()
        .direct
        .as_mut()
        .unwrap()
        .scopes
        .clear();
    for doc in [missing_metadata, mislabeled, missing_scope] {
        assert!(
            AntexAuth::from_auth_dot_json(
                home.path(),
                doc,
                AuthCredentialsStoreMode::File,
                /*chatgpt_base_url*/ None,
                AuthKeyringBackendKind::default(),
                /*agent_identity_authapi_base_url*/ None,
                &route
            )
            .await
            .is_err()
        );
    }
}

#[tokio::test]
async fn direct_policy_is_chatgpt_not_api_and_workspace_restrictions_fail_closed() {
    let home = tempfile::tempdir().unwrap();
    let auth = load(home.path(), &document()).await;
    let mut config = AuthConfig {
        codex_home: home.path().into(),
        auth_credentials_store_mode: AuthCredentialsStoreMode::File,
        keyring_backend_kind: AuthKeyringBackendKind::default(),
        forced_login_method: Some(ForcedLoginMethod::Chatgpt),
        chatgpt_base_url: None,
        forced_chatgpt_workspace_id: None,
        managed_auth_policy: ManagedAuthPolicy::default(),
        auth_route_config: crate::test_support::transport_default_auth_route_config(),
    };
    assert!(config.allows_auth(&auth));
    assert!(
        config
            .load_auth(/*enable_antex_api_key_env*/ false)
            .await
            .unwrap()
            .is_some()
    );
    config.forced_login_method = Some(ForcedLoginMethod::Api);
    assert!(!config.allows_auth(&auth));
    assert!(
        config
            .load_auth(/*enable_antex_api_key_env*/ false)
            .await
            .unwrap()
            .is_none()
    );
    config.forced_login_method = Some(ForcedLoginMethod::Chatgpt);
    config.forced_chatgpt_workspace_id = Some(vec!["workspace".into()]);
    assert!(!config.allows_auth(&auth));
    assert!(
        config
            .load_auth(/*enable_antex_api_key_env*/ false)
            .await
            .unwrap()
            .is_none()
    );
}

#[tokio::test]
async fn refresh_cannot_overwrite_a_new_login_or_restore_a_logged_out_registration() {
    let home = tempfile::tempdir().unwrap();
    let original = document();
    let auth = load(home.path(), &original).await;
    let AntexAuth::ChatgptDirect(managed) = &auth else {
        panic!("wrong auth mode")
    };
    let mut rotated = original.tokens.clone().unwrap();
    rotated.refresh_token = "rotated".into();
    let storage = create_auth_storage(
        home.path().into(),
        AuthCredentialsStoreMode::File,
        AuthKeyringBackendKind::default(),
    );
    let mut replacement = original.clone();
    replacement
        .tokens
        .as_mut()
        .unwrap()
        .direct
        .as_mut()
        .unwrap()
        .client_id = "different-login".into();
    storage.save(&replacement).unwrap();
    assert!(commit_refresh(managed, original.clone(), rotated.clone()).is_err());
    assert_eq!(storage.load().unwrap(), Some(replacement));
    storage.delete().unwrap();
    assert!(commit_refresh(managed, original, rotated).is_err());
    assert_eq!(storage.load().unwrap(), None);
}

#[test]
fn simultaneous_storage_refreshes_cannot_both_commit_the_same_snapshot() {
    let home = tempfile::tempdir().unwrap();
    let original = document();
    let storage = create_auth_storage(
        home.path().into(),
        AuthCredentialsStoreMode::File,
        AuthKeyringBackendKind::default(),
    );
    storage.save(&original).unwrap();
    let barrier = std::sync::Barrier::new(2);
    let results = std::thread::scope(|scope| {
        let workers: Vec<_> = (0..2)
            .map(|index| {
                let barrier = &barrier;
                let original = &original;
                let home = home.path();
                scope.spawn(move || {
                    let storage = create_auth_storage(
                        home.into(),
                        AuthCredentialsStoreMode::File,
                        AuthKeyringBackendKind::default(),
                    );
                    let mut updated = original.clone();
                    updated.tokens.as_mut().unwrap().refresh_token = format!("rotation-{index}");
                    barrier.wait();
                    storage
                        .replace_if_matches(original, &updated)
                        .map(|()| updated)
                })
            })
            .collect();
        workers
            .into_iter()
            .map(|worker| worker.join().unwrap())
            .collect::<Vec<_>>()
    });
    let winners: Vec<_> = results.into_iter().filter_map(Result::ok).collect();
    assert_eq!(winners.len(), 1);
    assert_eq!(storage.load().unwrap(), Some(winners[0].clone()));
}

#[tokio::test]
async fn invalid_rotated_grant_is_failure_atomic() {
    let home = tempfile::tempdir().unwrap();
    let original = document();
    let auth = load(home.path(), &original).await;
    let AntexAuth::ChatgptDirect(managed) = &auth else {
        panic!("wrong auth mode")
    };
    let mut rotated = original.tokens.clone().unwrap();
    rotated.direct.as_mut().unwrap().scopes.clear();
    assert!(commit_refresh(managed, original.clone(), rotated).is_err());
    assert_eq!(managed.storage().load().unwrap(), Some(original));
}
