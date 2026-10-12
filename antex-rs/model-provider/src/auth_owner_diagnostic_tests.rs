//! Retained header lifetime is narrower than a generic operation's lifetime.
use super::*;
use antex_login::AuthCredentialsStoreMode;
use antex_login::AuthKeyringBackendKind;
use antex_login::ExternalAuth;
use antex_login::ExternalAuthFuture;
use antex_login::ExternalAuthRefreshContext;
use antex_login::auth::login_with_chatgpt_auth_tokens;
use base64::Engine;
use http::header::AUTHORIZATION;
use pretty_assertions::assert_eq;
use std::path::Path;

fn token(user: Option<&str>, signature: &str) -> String {
    let claims = serde_json::json!({"https://api.openai.com/auth": {"chatgpt_user_id": user}});
    let payload = base64::engine::general_purpose::URL_SAFE_NO_PAD
        .encode(serde_json::to_vec(&claims).unwrap());
    format!("header.{payload}.{signature}")
}

fn save(home: &Path, token: &str, account: &str) {
    login_with_chatgpt_auth_tokens(home, token, account, /*chatgpt_plan_type*/ None).unwrap();
}

async fn manager(home: &Path) -> Arc<AuthManager> {
    Arc::new(
        AuthManager::new(
            home.into(),
            /*enable_antex_api_key_env*/ false,
            AuthCredentialsStoreMode::Ephemeral,
            /*forced_chatgpt_workspace_id*/ None,
            /*chatgpt_base_url*/ None,
            AuthKeyringBackendKind::default(),
            antex_login::test_support::transport_default_auth_route_config(),
        )
        .await,
    )
}

async fn assert_headers(provider: &SharedAuthProvider, bearer: &str) {
    let mut expected = HeaderMap::new();
    expected.insert(
        AUTHORIZATION,
        HeaderValue::from_str(&format!("Bearer {bearer}")).unwrap(),
    );
    expected.insert("ChatGPT-Account-ID", HeaderValue::from_static("account-a"));
    assert_eq!(provider.resolve_auth_headers().await.unwrap(), expected);
    assert_eq!(provider.to_auth_headers(), expected);
}

#[tokio::test]
async fn retained_host_headers_follow_sparse_rotations_without_relaxing_generic_operations() {
    let home = tempfile::tempdir().unwrap();
    save(home.path(), "header.e30.initial", "account-a");
    let manager = manager(home.path()).await;
    assert!(!manager.has_external_auth());
    let original = manager.auth_cached().unwrap();
    assert_eq!(original.get_chatgpt_user_id(), None);
    let operation = manager.unauthorized_recovery();
    let provider = auth_provider_from_auth_manager(manager.clone(), &original);
    for bearer in ["header.e30.rotated", "header.e30.rotated-again"] {
        save(home.path(), bearer, "account-a");
        manager.reload().await;
        assert_headers(&provider, bearer).await;
        assert!(operation.ensure_same_owner().is_err());
    }
}

#[derive(Clone, Copy)]
enum Boundary {
    Account,
    User,
    Logout,
    SourceEpoch,
}

#[tokio::test]
async fn retained_host_headers_cannot_revive_after_account_user_logout_or_source_aba() {
    for boundary in [
        Boundary::Account,
        Boundary::User,
        Boundary::Logout,
        Boundary::SourceEpoch,
    ] {
        let home = tempfile::tempdir().unwrap();
        let original = token(Some("user-a"), "initial");
        save(home.path(), &original, "account-a");
        let manager = manager(home.path()).await;
        let provider =
            auth_provider_from_auth_manager(manager.clone(), &manager.auth_cached().unwrap());
        match boundary {
            Boundary::Account => {
                save(home.path(), &original, "account-b");
                manager.reload().await;
            }
            Boundary::User => {
                save(
                    home.path(),
                    &token(Some("user-b"), "other-user"),
                    "account-a",
                );
                manager.reload().await;
            }
            Boundary::Logout => {
                manager.logout().await.unwrap();
            }
            Boundary::SourceEpoch => manager.clear_external_auth(),
        }
        assert!(provider.resolve_auth_headers().await.is_err());
        save(home.path(), &original, "account-a");
        manager.reload().await;
        assert!(provider.to_auth_headers().is_empty());
        assert!(provider.resolve_auth_headers().await.is_err());
        let fresh =
            auth_provider_from_auth_manager(manager.clone(), &manager.auth_cached().unwrap());
        assert_headers(&fresh, &original).await;
    }
}

#[tokio::test]
async fn absent_to_known_user_aba_does_not_inherit_sparse_host_header_lifetime() {
    let home = tempfile::tempdir().unwrap();
    save(home.path(), "header.e30.initial", "account-a");
    let manager = manager(home.path()).await;
    let provider =
        auth_provider_from_auth_manager(manager.clone(), &manager.auth_cached().unwrap());
    save(home.path(), &token(Some("user-a"), "known"), "account-a");
    manager.reload().await;
    save(home.path(), "header.e30.initial", "account-a");
    manager.reload().await;
    assert!(provider.to_auth_headers().is_empty());
    assert!(provider.resolve_auth_headers().await.is_err());
}

struct Source {
    auth: AntexAuth,
    fail: AtomicBool,
}

impl ExternalAuth for Source {
    fn resolve(&self) -> ExternalAuthFuture<'_, AntexAuth> {
        Box::pin(async {
            if self.fail.load(Ordering::SeqCst) {
                Err(std::io::Error::other("active source unavailable"))
            } else {
                Ok(self.auth.clone())
            }
        })
    }

    fn refresh(&self, _context: ExternalAuthRefreshContext) -> ExternalAuthFuture<'_, AntexAuth> {
        self.resolve()
    }
}

#[tokio::test]
async fn host_header_exception_never_accepts_equal_token_source_installation_or_failed_resolution()
{
    let home = tempfile::tempdir().unwrap();
    save(home.path(), "header.e30.initial", "account-a");
    let manager = manager(home.path()).await;
    let original = manager.auth_cached().unwrap();
    let provider = auth_provider_from_auth_manager(manager.clone(), &original);
    let source = Arc::new(Source {
        auth: original.clone(),
        fail: AtomicBool::new(false),
    });
    let revisions = *manager.auth_change_state_receiver().borrow();
    manager.set_external_auth(source.clone()).await.unwrap();
    assert_eq!(*manager.auth_change_state_receiver().borrow(), revisions);
    assert!(provider.resolve_auth_headers().await.is_err());
    let external_provider = auth_provider_from_auth_manager(manager.clone(), &original);
    assert_headers(&external_provider, "header.e30.initial").await;
    source.fail.store(true, Ordering::SeqCst);
    for _ in 0..2 {
        assert!(external_provider.resolve_auth_headers().await.is_err());
        assert_eq!(manager.auth_cached(), Some(original.clone()));
    }
    source.fail.store(false, Ordering::SeqCst);
    manager.set_external_auth(source).await.unwrap();
    assert_eq!(*manager.auth_change_state_receiver().borrow(), revisions);
    assert!(external_provider.to_auth_headers().is_empty());
    assert!(external_provider.resolve_auth_headers().await.is_err());
    let fresh = auth_provider_from_auth_manager(manager, &original);
    assert_headers(&fresh, "header.e30.initial").await;
}
