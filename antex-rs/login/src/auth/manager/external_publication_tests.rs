//! Paused host providers exercise real resolve/refresh/install entry points,
//! including the process-shared ephemeral store. Credentials are synthetic.
use super::super::publication_tests::LoadedPause;
use super::*;
use base64::Engine;
use pretty_assertions::assert_eq;
use std::sync::atomic::AtomicBool;
use std::sync::atomic::Ordering;

#[path = "external_epoch_tests.rs"]
mod epoch_tests;
#[path = "request_auth_tests.rs"]
mod request_auth_tests;

struct Source {
    auth: Mutex<AntexAuth>,
    resolve_pause: Mutex<Option<Arc<LoadedPause>>>,
    refresh_pause: Mutex<Option<Arc<LoadedPause>>>,
    fail: AtomicBool,
}

impl Source {
    fn new(auth: AntexAuth) -> Arc<Self> {
        Arc::new(Self {
            auth: Mutex::new(auth),
            resolve_pause: Mutex::default(),
            refresh_pause: Mutex::default(),
            fail: AtomicBool::new(false),
        })
    }

    fn result(&self, pause: &Mutex<Option<Arc<LoadedPause>>>) -> ExternalAuthFuture<'_, AntexAuth> {
        let auth = self.auth.lock().unwrap().clone();
        let pause = pause.lock().unwrap().take();
        let fail = self.fail.load(Ordering::SeqCst);
        Box::pin(async move {
            if let Some(pause) = pause {
                pause.wait().await;
            }
            if fail {
                Err(std::io::Error::other("synthetic host failure"))
            } else {
                Ok(auth)
            }
        })
    }
}

impl ExternalAuth for Source {
    fn resolve(&self) -> ExternalAuthFuture<'_, AntexAuth> {
        self.result(&self.resolve_pause)
    }
    fn refresh(&self, _: ExternalAuthRefreshContext) -> ExternalAuthFuture<'_, AntexAuth> {
        self.result(&self.refresh_pause)
    }
    fn classify_error(&self, _: std::io::Error) -> RefreshTokenError {
        RefreshTokenError::Permanent(RefreshTokenFailedError::new(
            RefreshTokenFailedReason::Other,
            "Synthetic external failure",
        ))
    }
}

#[derive(Clone, Copy)]
enum Operation {
    Resolve,
    Refresh,
}
impl Operation {
    fn pause(self, source: &Source) -> Arc<LoadedPause> {
        let pause = Arc::new(LoadedPause::default());
        let slot = match self {
            Self::Resolve => &source.resolve_pause,
            Self::Refresh => &source.refresh_pause,
        };
        *slot.lock().unwrap() = Some(pause.clone());
        pause
    }
    async fn run(self, manager: &AuthManager) -> Result<bool, RefreshTokenError> {
        match self {
            Self::Resolve => Ok(manager.reload().await),
            Self::Refresh => manager.refresh_token_from_authority().await.map(|()| true),
        }
    }
}

#[derive(Clone, Copy)]
enum Kind {
    Api,
    Chatgpt,
}
fn credential(kind: Kind, account: &str, token: &str) -> AntexAuth {
    match kind {
        Kind::Api => AntexAuth::from_api_key(token),
        Kind::Chatgpt => {
            let claims = serde_json::json!({"nonce":token, "https://api.openai.com/auth":{
                "chatgpt_account_id":account, "chatgpt_user_id":format!("user-{account}")}});
            let payload = base64::engine::general_purpose::URL_SAFE_NO_PAD
                .encode(serde_json::to_vec(&claims).unwrap());
            AntexAuth::from_external_chatgpt_tokens(
                &format!("e30.{payload}.sig"),
                account,
                Some("enterprise"),
            )
            .unwrap()
        }
    }
}

type Snapshot = (AuthMode, String, Option<AuthDotJson>, Option<AuthDotJson>);
fn snapshot(manager: &AuthManager) -> Snapshot {
    let cached = manager.auth_cached().unwrap();
    let storage = create_auth_storage(
        manager.codex_home.clone(),
        AuthCredentialsStoreMode::Ephemeral,
        AuthKeyringBackendKind::default(),
    );
    (
        cached.api_auth_mode(),
        cached.get_token().unwrap(),
        cached.get_current_auth_json(),
        storage.load().unwrap(),
    )
}
fn assert_current(manager: &AuthManager, expected: &AntexAuth) {
    let expected_doc = expected.get_current_auth_json();
    assert_eq!(
        snapshot(manager),
        (
            expected.api_auth_mode(),
            expected.get_token().unwrap(),
            expected_doc.clone(),
            expected_doc
        )
    );
}

#[tokio::test]
async fn late_external_resolve_and_refresh_cannot_overwrite_replacement_cache_or_ephemeral_document()
 {
    for kind in [Kind::Api, Kind::Chatgpt] {
        for operation in [Operation::Resolve, Operation::Refresh] {
            let home = tempfile::tempdir().unwrap();
            let initial = credential(kind, "account-a", "a-original");
            let manager =
                AuthManager::from_auth_for_testing_with_home(initial.clone(), home.path().into());
            let a = Source::new(initial);
            manager.set_external_auth(a.clone()).await.unwrap();
            *a.auth.lock().unwrap() = credential(kind, "account-a", "a-late");
            let pause = operation.pause(&a);
            let pending = {
                let manager = manager.clone();
                tokio::spawn(async move { operation.run(&manager).await })
            };
            tokio::time::timeout(Duration::from_secs(2), pause.loaded.notified())
                .await
                .unwrap();
            let winning_auth = credential(kind, "account-b", "b-winning");
            let b: Arc<dyn ExternalAuth> = Source::new(winning_auth.clone());
            manager.set_external_auth(b.clone()).await.unwrap();
            assert_current(&manager, &winning_auth);
            let winning = snapshot(&manager);
            let revisions = *manager.auth_change_state_receiver().borrow();
            let b_recovery = manager.unauthorized_recovery();
            pause.release.notify_one();
            let result = tokio::time::timeout(Duration::from_secs(2), pending)
                .await
                .unwrap()
                .unwrap();
            match operation {
                Operation::Resolve => assert!(!result.unwrap()),
                Operation::Refresh => assert!(result.is_err()),
            }
            assert_eq!(snapshot(&manager), winning);
            assert_eq!(*manager.auth_change_state_receiver().borrow(), revisions);
            assert!(Arc::ptr_eq(&b, &manager.external_auth_provider().unwrap()));
            b_recovery.ensure_same_owner().unwrap();
            assert!(
                manager
                    .refresh_failure_for_auth(&manager.auth_cached().unwrap())
                    .is_none()
            );
        }
    }
}

#[tokio::test]
async fn same_external_source_can_publish_legitimate_resolve_and_refresh_rotations() {
    for kind in [Kind::Api, Kind::Chatgpt] {
        for operation in [Operation::Resolve, Operation::Refresh] {
            let home = tempfile::tempdir().unwrap();
            let initial = credential(kind, "account-a", "original");
            let manager =
                AuthManager::from_auth_for_testing_with_home(initial.clone(), home.path().into());
            let source = Source::new(initial);
            manager.set_external_auth(source.clone()).await.unwrap();
            let recovery = manager.unauthorized_recovery();
            let rotated = credential(kind, "account-a", "rotated");
            *source.auth.lock().unwrap() = rotated.clone();
            let pause = operation.pause(&source);
            let pending = {
                let manager = manager.clone();
                tokio::spawn(async move { operation.run(&manager).await })
            };
            tokio::time::timeout(Duration::from_secs(2), pause.loaded.notified())
                .await
                .unwrap();
            pause.release.notify_one();
            tokio::time::timeout(Duration::from_secs(2), pending)
                .await
                .unwrap()
                .unwrap()
                .unwrap();
            assert_current(&manager, &rotated);
            recovery.ensure_same_owner().unwrap();
        }
    }
}

#[tokio::test]
async fn automatic_same_source_results_cannot_change_account_before_publication() {
    for operation in [Operation::Resolve, Operation::Refresh] {
        let home = tempfile::tempdir().unwrap();
        let original = credential(Kind::Chatgpt, "account-a", "a");
        let manager =
            AuthManager::from_auth_for_testing_with_home(original.clone(), home.path().into());
        let source = Source::new(original.clone());
        manager.set_external_auth(source.clone()).await.unwrap();
        *source.auth.lock().unwrap() = credential(Kind::Chatgpt, "account-b", "b");
        let result = operation.run(&manager).await;
        match operation {
            Operation::Resolve => assert!(!result.unwrap()),
            Operation::Refresh => assert!(result.is_err()),
        }
        assert_current(&manager, &original);
    }
}

#[tokio::test]
async fn late_external_failure_does_not_poison_an_equal_bearer_from_a_new_source() {
    for operation in [Operation::Resolve, Operation::Refresh] {
        let home = tempfile::tempdir().unwrap();
        let auth = AntexAuth::from_api_key("identical-opaque-bearer");
        let manager =
            AuthManager::from_auth_for_testing_with_home(auth.clone(), home.path().into());
        let a = Source::new(auth.clone());
        manager.set_external_auth(a.clone()).await.unwrap();
        a.fail.store(true, Ordering::SeqCst);
        let pause = operation.pause(&a);
        let pending = {
            let manager = manager.clone();
            tokio::spawn(async move { operation.run(&manager).await })
        };
        tokio::time::timeout(Duration::from_secs(2), pause.loaded.notified())
            .await
            .unwrap();
        manager
            .set_external_auth(Source::new(auth.clone()))
            .await
            .unwrap();
        pause.release.notify_one();
        let _ = tokio::time::timeout(Duration::from_secs(2), pending)
            .await
            .unwrap()
            .unwrap();
        assert_current(&manager, &auth);
        assert!(manager.refresh_failure_for_auth(&auth).is_none());
    }
}

#[tokio::test(flavor = "multi_thread", worker_threads = 2)]
async fn pending_replacement_keeps_the_old_source_and_cache_coherent_until_ephemeral_commit() {
    let home = tempfile::tempdir().unwrap();
    let a_auth = credential(Kind::Chatgpt, "account-a", "a");
    let manager = AuthManager::from_auth_for_testing_with_home(a_auth.clone(), home.path().into());
    let a: Arc<dyn ExternalAuth> = Source::new(a_auth.clone());
    manager.set_external_auth(a.clone()).await.unwrap();
    let storage = create_auth_storage(
        home.path().into(),
        AuthCredentialsStoreMode::Ephemeral,
        AuthKeyringBackendKind::default(),
    );
    let held = storage.try_transaction().unwrap().unwrap();
    let b_auth = credential(Kind::Chatgpt, "account-b", "b");
    let b: Arc<dyn ExternalAuth> = Source::new(b_auth.clone());
    let (polled, waiting) = tokio::sync::oneshot::channel();
    let installing = {
        let manager = manager.clone();
        let b = b.clone();
        tokio::spawn(async move {
            let mut install = Box::pin(manager.set_external_auth(b));
            std::future::poll_fn(|cx| {
                assert!(install.as_mut().poll(cx).is_pending());
                std::task::Poll::Ready(())
            })
            .await;
            polled.send(()).unwrap();
            install.await
        })
    };
    tokio::time::timeout(Duration::from_secs(2), waiting)
        .await
        .unwrap()
        .unwrap();
    assert!(Arc::ptr_eq(&a, &manager.external_auth_provider().unwrap()));
    assert_current(&manager, &a_auth);
    manager.unauthorized_recovery().ensure_same_owner().unwrap();
    drop(held);
    tokio::time::timeout(Duration::from_secs(2), installing)
        .await
        .unwrap()
        .unwrap()
        .unwrap();
    assert!(Arc::ptr_eq(&b, &manager.external_auth_provider().unwrap()));
    assert_current(&manager, &b_auth);
}

#[tokio::test]
async fn failed_replacement_validation_preserves_working_source_cache_and_shared_document() {
    let home = tempfile::tempdir().unwrap();
    let auth = credential(Kind::Chatgpt, "account-a", "a");
    let manager = AuthManager::from_auth_for_testing_with_home(auth.clone(), home.path().into());
    let a: Arc<dyn ExternalAuth> = Source::new(auth.clone());
    manager.set_external_auth(a.clone()).await.unwrap();
    manager.set_forced_chatgpt_workspace_id(Some(vec!["account-a".into()]));
    let recovery = manager.unauthorized_recovery();
    let b = Source::new(credential(Kind::Chatgpt, "account-b", "b"));
    let pause = Operation::Resolve.pause(&b);
    let pending = {
        let manager = manager.clone();
        tokio::spawn(async move { manager.set_external_auth(b).await })
    };
    tokio::time::timeout(Duration::from_secs(2), pause.loaded.notified())
        .await
        .unwrap();
    assert!(Arc::ptr_eq(&a, &manager.external_auth_provider().unwrap()));
    assert_current(&manager, &auth);
    pause.release.notify_one();
    assert!(
        tokio::time::timeout(Duration::from_secs(2), pending)
            .await
            .unwrap()
            .unwrap()
            .is_err()
    );
    assert!(Arc::ptr_eq(&a, &manager.external_auth_provider().unwrap()));
    assert_current(&manager, &auth);
    recovery.ensure_same_owner().unwrap();
}
