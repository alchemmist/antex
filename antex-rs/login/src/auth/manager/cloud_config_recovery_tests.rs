use super::*;
use crate::auth::manager::publication_tests::LoadedPause;
use crate::auth::manager::test_authority::document;
use crate::auth::manager::test_authority::manager;
use base64::Engine;
use pretty_assertions::assert_eq;

fn legacy(account: Option<&str>, user: Option<&str>) -> AuthDotJson {
    let mut doc = document(AuthMode::Chatgpt, "unused");
    let claims = serde_json::json!({"https://api.openai.com/auth": {
        "chatgpt_account_id": account, "chatgpt_user_id": user
    }});
    let payload = base64::engine::general_purpose::URL_SAFE_NO_PAD
        .encode(serde_json::to_vec(&claims).unwrap());
    let tokens = doc.tokens.as_mut().unwrap();
    tokens.id_token = parse_chatgpt_jwt_claims(&format!("e30.{payload}.sig")).unwrap();
    tokens.account_id = account.map(str::to_string);
    tokens.access_token = format!("access-{}", user.unwrap_or("unknown"));
    doc
}

fn save(home: &Path, doc: &AuthDotJson) {
    save_auth(
        home,
        doc,
        AuthCredentialsStoreMode::File,
        AuthKeyringBackendKind::default(),
    )
    .unwrap();
}

#[tokio::test]
async fn metadata_rebind_invalidates_the_original_sampling_guard_and_never_resets_budget() {
    let home = tempfile::tempdir().unwrap();
    let original = legacy(Some("workspace-a"), Some("user-a"));
    let manager = manager(home.path(), &original, AuthCredentialsStoreMode::File).await;
    let mut sampling = manager.unauthorized_recovery();
    let mut metadata =
        CloudConfigRecovery::new(manager.clone(), &manager.auth_cached().unwrap()).unwrap();
    let rebound = legacy(Some("workspace-a"), Some("user-b"));
    save(home.path(), &rebound);
    assert_eq!(
        metadata
            .next()
            .await
            .unwrap()
            .unwrap()
            .get_current_auth_json(),
        Some(rebound.clone())
    );
    assert_eq!(metadata.recovery.step_name(), "refresh_token");
    metadata.ensure_same_owner().unwrap();
    assert!(sampling.ensure_same_owner().is_err());
    assert!(!sampling.has_next());
    assert!(sampling.next().await.is_err());

    // A second user change cannot buy another reload/rebind. The remaining
    // authority step rejects the changed stored grant before any exchange.
    save(home.path(), &legacy(Some("workspace-a"), Some("user-c")));
    assert!(metadata.next().await.is_err());
    assert_eq!(metadata.recovery.step_name(), "refresh_token");
    assert_eq!(
        manager.auth_cached().unwrap().get_current_auth_json(),
        Some(rebound)
    );
}

#[tokio::test]
async fn generic_recovery_rejects_stored_foreign_account_and_user_before_publication() {
    for replacement in [
        legacy(Some("workspace-b"), Some("user-a")),
        legacy(Some("workspace-a"), Some("user-b")),
    ] {
        let home = tempfile::tempdir().unwrap();
        let original = legacy(Some("workspace-a"), Some("user-a"));
        let manager = manager(home.path(), &original, AuthCredentialsStoreMode::File).await;
        let mut sampling = manager.unauthorized_recovery();
        let revision = *manager.auth_change_state_receiver().borrow();
        save(home.path(), &replacement);
        assert!(sampling.next().await.is_err());
        assert_eq!(
            manager.auth_cached().unwrap().get_current_auth_json(),
            Some(original)
        );
        assert_eq!(*manager.auth_change_state_receiver().borrow(), revision);
        // Explicit adoption is permitted, but cannot rebind the old operation.
        manager.reload().await;
        assert!(sampling.ensure_same_owner().is_err());
        assert!(sampling.next().await.is_err());
        assert_eq!(
            manager.auth_cached().unwrap().get_current_auth_json(),
            Some(replacement)
        );
    }
}

#[tokio::test]
async fn metadata_rejects_foreign_account_mode_and_unknown_scope_without_publishing() {
    let original = legacy(Some("workspace-a"), Some("user-a"));
    let mut external_mode = original.clone();
    external_mode.auth_mode = Some(AuthMode::ChatgptAuthTokens);
    let mut api_mode = original.clone();
    api_mode.auth_mode = Some(AuthMode::ApiKey);
    api_mode.openai_api_key = Some("synthetic-key".into());
    for replacement in [
        legacy(Some("workspace-b"), Some("user-b")),
        legacy(/*account*/ None, Some("user-b")),
        legacy(Some("workspace-a"), /*user*/ None),
        legacy(Some("workspace-a"), Some(" ")),
        document(AuthMode::ChatgptDirect, "foreign-registration"),
        external_mode,
        api_mode,
    ] {
        let home = tempfile::tempdir().unwrap();
        let manager = manager(home.path(), &original, AuthCredentialsStoreMode::File).await;
        let mut metadata =
            CloudConfigRecovery::new(manager.clone(), &manager.auth_cached().unwrap()).unwrap();
        let revision = *manager.auth_change_state_receiver().borrow();
        save(home.path(), &replacement);
        assert_eq!(
            metadata.next().await.unwrap_err().to_string(),
            REFRESH_TOKEN_ACCOUNT_MISMATCH_MESSAGE
        );
        assert_eq!(
            manager.auth_cached().unwrap().get_current_auth_json(),
            Some(original.clone())
        );
        assert_eq!(*manager.auth_change_state_receiver().borrow(), revision);
        assert_eq!(
            load_auth_dot_json(
                home.path(),
                AuthCredentialsStoreMode::File,
                AuthKeyringBackendKind::default()
            )
            .unwrap(),
            Some(replacement)
        );
    }
    for original in [
        legacy(/*account*/ None, Some("user-a")),
        legacy(Some("workspace-a"), /*user*/ None),
    ] {
        let home = tempfile::tempdir().unwrap();
        let manager = manager(home.path(), &original, AuthCredentialsStoreMode::File).await;
        let mut metadata =
            CloudConfigRecovery::new(manager.clone(), &manager.auth_cached().unwrap()).unwrap();
        save(home.path(), &legacy(Some("workspace-a"), Some("user-b")));
        assert!(metadata.next().await.is_err());
        assert_eq!(
            manager.auth_cached().unwrap().get_current_auth_json(),
            Some(original)
        );
    }
}

struct FixedSource(AntexAuth);

impl ExternalAuth for FixedSource {
    fn resolve(&self) -> ExternalAuthFuture<'_, AntexAuth> {
        Box::pin(async { Ok(self.0.clone()) })
    }

    fn refresh(&self, _: ExternalAuthRefreshContext) -> ExternalAuthFuture<'_, AntexAuth> {
        self.resolve()
    }
}

#[tokio::test]
async fn metadata_loaded_before_wait_cannot_overwrite_new_owner_source_or_source_epoch() {
    for transition in ["user", "account", "mode", "source", "source-aba"] {
        let home = tempfile::tempdir().unwrap();
        let original = legacy(Some("workspace-a"), Some("user-a"));
        let manager = manager(home.path(), &original, AuthCredentialsStoreMode::File).await;
        let mut metadata =
            CloudConfigRecovery::new(manager.clone(), &manager.auth_cached().unwrap()).unwrap();
        save(home.path(), &legacy(Some("workspace-a"), Some("user-b")));
        let pause = Arc::new(LoadedPause::default());
        *manager.proactive_load_pause.lock().unwrap() = Some(pause.clone());
        let pending = tokio::spawn(async move { metadata.next().await });
        tokio::time::timeout(Duration::from_secs(2), pause.loaded.notified())
            .await
            .unwrap();
        match transition {
            "user" | "account" | "mode" => {
                let winner = match transition {
                    "user" => legacy(Some("workspace-a"), Some("user-c")),
                    "account" => legacy(Some("workspace-c"), Some("user-a")),
                    "mode" => document(AuthMode::ChatgptDirect, "registration-c"),
                    _ => unreachable!(),
                };
                save(home.path(), &winner);
                manager.reload().await;
            }
            "source" => {
                let external = AntexAuth::from_external_chatgpt_tokens(
                    "e30.e30.sig",
                    "workspace-c",
                    /*chatgpt_plan_type*/ None,
                )
                .unwrap();
                manager
                    .set_external_auth(Arc::new(FixedSource(external)))
                    .await
                    .unwrap();
            }
            "source-aba" => {
                // None -> clear -> None still advances the source lifetime.
                manager.clear_external_auth();
            }
            _ => unreachable!(),
        }
        let winner = manager.auth_cached().unwrap().get_current_auth_json();
        let revision = *manager.auth_change_state_receiver().borrow();
        let ephemeral = load_auth_dot_json(
            home.path(),
            AuthCredentialsStoreMode::Ephemeral,
            AuthKeyringBackendKind::default(),
        )
        .unwrap();
        pause.release.notify_one();
        assert!(
            tokio::time::timeout(Duration::from_secs(2), pending)
                .await
                .unwrap()
                .unwrap()
                .is_err()
        );
        assert_eq!(
            manager.auth_cached().unwrap().get_current_auth_json(),
            winner
        );
        assert_eq!(*manager.auth_change_state_receiver().borrow(), revision);
        assert_eq!(
            load_auth_dot_json(
                home.path(),
                AuthCredentialsStoreMode::Ephemeral,
                AuthKeyringBackendKind::default()
            )
            .unwrap(),
            ephemeral
        );
    }
}
