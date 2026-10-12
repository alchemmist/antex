//! Regression paths for external ChatGPT tokens with an optional, absent user
//! claim, and explicit same-Arc source lifecycle changes. No identity is invented.
use super::*;
use pretty_assertions::assert_eq;

fn no_user(token: &str) -> AntexAuth {
    AntexAuth::from_external_chatgpt_tokens(token, "workspace-one", Some("enterprise")).unwrap()
}

#[tokio::test]
async fn same_source_chatgpt_without_user_claim_recovers_and_resolves_rotations() {
    for operation in [Operation::Resolve, Operation::Refresh] {
        let home = tempfile::tempdir().unwrap();
        let initial = no_user("header.e30.initial");
        let rotated = no_user("header.e30.refreshed");
        assert_eq!(initial.get_chatgpt_user_id(), None);
        assert_eq!(rotated.get_chatgpt_user_id(), None);
        let manager =
            AuthManager::from_auth_for_testing_with_home(initial.clone(), home.path().into());
        let source = Source::new(initial.clone());
        manager.set_external_auth(source.clone()).await.unwrap();
        let mut recovery = manager.unauthorized_recovery();
        let original_generation = recovery.owner_generation();
        let epoch = manager.external_source_epoch.load(Ordering::Relaxed);
        *source.auth.lock().unwrap() = rotated.clone();
        let pause = operation.pause(&source);
        let pending = {
            let manager = manager.clone();
            tokio::spawn(async move {
                match operation {
                    Operation::Resolve => {
                        manager.auth().await.unwrap();
                    }
                    Operation::Refresh => {
                        recovery.next().await.unwrap();
                    }
                }
                recovery.ensure_same_owner().unwrap();
                recovery
            })
        };
        tokio::time::timeout(Duration::from_secs(2), pause.loaded.notified())
            .await
            .unwrap();
        assert_current(&manager, &initial);
        pause.release.notify_one();
        let recovery = tokio::time::timeout(Duration::from_secs(2), pending)
            .await
            .unwrap()
            .unwrap();
        assert_current(&manager, &rotated);
        // Keep existing credential-generation semantics; only the bounded
        // recovery exception permits rotation within this source installation.
        assert!(
            manager
                .auth_change_state_receiver()
                .borrow()
                .owner_generation
                > original_generation
        );
        assert_eq!(recovery.owner_generation(), original_generation);
        assert_eq!(manager.external_source_epoch.load(Ordering::Relaxed), epoch);
        assert_eq!(
            manager.auth().await.unwrap().get_token().unwrap(),
            "header.e30.refreshed"
        );
        recovery.ensure_same_owner().unwrap();
    }
}

#[tokio::test]
async fn explicit_same_arc_reinstall_invalidates_recovery_without_changing_equal_token_revisions() {
    for original in [
        AntexAuth::from_api_key("opaque-bearer"),
        no_user("header.e30.initial"),
    ] {
        for clear in [false, true] {
            let home = tempfile::tempdir().unwrap();
            let manager =
                AuthManager::from_auth_for_testing_with_home(original.clone(), home.path().into());
            let source: Arc<dyn ExternalAuth> = Source::new(original.clone());
            manager.set_external_auth(source.clone()).await.unwrap();
            let recovery = manager.unauthorized_recovery();
            let revisions = *manager.auth_change_state_receiver().borrow();
            if clear {
                manager.clear_external_auth();
            }
            manager.set_external_auth(source.clone()).await.unwrap();
            assert!(Arc::ptr_eq(
                &source,
                &manager.external_auth_provider().unwrap()
            ));
            assert_current(&manager, &original);
            if !clear {
                assert_eq!(*manager.auth_change_state_receiver().borrow(), revisions);
            }
            assert!(recovery.ensure_same_owner().is_err());
            manager.unauthorized_recovery().ensure_same_owner().unwrap();
        }
    }
}

#[tokio::test]
async fn suspended_results_cannot_publish_after_same_arc_clear_and_reinstall() {
    for chatgpt in [false, true] {
        for operation in [Operation::Resolve, Operation::Refresh] {
            let home = tempfile::tempdir().unwrap();
            let auth = |token| {
                if chatgpt {
                    no_user(token)
                } else {
                    AntexAuth::from_api_key(token)
                }
            };
            let initial = auth("header.e30.initial");
            let manager =
                AuthManager::from_auth_for_testing_with_home(initial.clone(), home.path().into());
            let source = Source::new(initial);
            let installed: Arc<dyn ExternalAuth> = source.clone();
            manager.set_external_auth(installed.clone()).await.unwrap();
            let recovery = manager.unauthorized_recovery();
            *source.auth.lock().unwrap() = auth("header.e30.late");
            let pause = operation.pause(&source);
            let pending = {
                let manager = manager.clone();
                tokio::spawn(async move { operation.run(&manager).await })
            };
            tokio::time::timeout(Duration::from_secs(2), pause.loaded.notified())
                .await
                .unwrap();
            manager.clear_external_auth();
            let winner = auth("header.e30.winner");
            *source.auth.lock().unwrap() = winner.clone();
            manager.set_external_auth(installed.clone()).await.unwrap();
            assert_current(&manager, &winner);
            let winning = snapshot(&manager);
            let winning_revisions = *manager.auth_change_state_receiver().borrow();
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
            assert_eq!(
                *manager.auth_change_state_receiver().borrow(),
                winning_revisions
            );
            assert!(Arc::ptr_eq(
                &installed,
                &manager.external_auth_provider().unwrap()
            ));
            assert!(recovery.ensure_same_owner().is_err());
            assert!(
                manager
                    .refresh_failure_for_auth(&manager.auth_cached().unwrap())
                    .is_none()
            );
            manager.unauthorized_recovery().ensure_same_owner().unwrap();
        }
    }
}

#[tokio::test]
async fn same_arc_reinstall_cannot_inherit_a_late_failure_for_an_identical_bearer() {
    for original in [
        AntexAuth::from_api_key("same-bearer"),
        no_user("header.e30.initial"),
    ] {
        let home = tempfile::tempdir().unwrap();
        let manager =
            AuthManager::from_auth_for_testing_with_home(original.clone(), home.path().into());
        let source = Source::new(original.clone());
        manager.set_external_auth(source.clone()).await.unwrap();
        source.fail.store(true, Ordering::SeqCst);
        let pause = Operation::Refresh.pause(&source);
        let pending = {
            let manager = manager.clone();
            tokio::spawn(async move { manager.refresh_token_from_authority().await })
        };
        tokio::time::timeout(Duration::from_secs(2), pause.loaded.notified())
            .await
            .unwrap();
        source.fail.store(false, Ordering::SeqCst);
        let revisions = *manager.auth_change_state_receiver().borrow();
        manager.set_external_auth(source).await.unwrap();
        assert_eq!(*manager.auth_change_state_receiver().borrow(), revisions);
        pause.release.notify_one();
        assert!(
            tokio::time::timeout(Duration::from_secs(2), pending)
                .await
                .unwrap()
                .unwrap()
                .is_err()
        );
        assert_current(&manager, &original);
        assert!(manager.refresh_failure_for_auth(&original).is_none());
    }
}

#[tokio::test]
async fn external_chatgpt_optional_user_and_mode_changes_still_fail_before_publication() {
    let mut headers = http::HeaderMap::new();
    headers.insert(
        http::header::AUTHORIZATION,
        http::HeaderValue::from_static("Bearer header-token"),
    );
    headers.insert(
        "chatgpt-account-id",
        http::HeaderValue::from_static("workspace-one"),
    );
    let absent = no_user("header.e30.initial");
    let present = credential(Kind::Chatgpt, "workspace-one", "known-user");
    for (initial, rejected) in [
        (absent.clone(), present.clone()),
        (present, absent.clone()),
        (
            absent.clone(),
            AntexAuth::Headers(AuthHeaders::new(headers)),
        ),
        (
            absent,
            credential(Kind::Chatgpt, "other-workspace", "other-account"),
        ),
    ] {
        for operation in [Operation::Resolve, Operation::Refresh] {
            let home = tempfile::tempdir().unwrap();
            let manager =
                AuthManager::from_auth_for_testing_with_home(initial.clone(), home.path().into());
            let source = Source::new(initial.clone());
            manager.set_external_auth(source.clone()).await.unwrap();
            *source.auth.lock().unwrap() = rejected.clone();
            let before = snapshot(&manager);
            let result = operation.run(&manager).await;
            match operation {
                Operation::Resolve => assert!(!result.unwrap()),
                Operation::Refresh => assert!(result.is_err()),
            }
            assert_eq!(snapshot(&manager), before);
        }
    }
}

#[tokio::test]
async fn clearing_an_empty_source_slot_invalidates_a_pending_first_installation() {
    let home = tempfile::tempdir().unwrap();
    let seed = AntexAuth::from_api_key("working-seed");
    let manager = AuthManager::from_auth_for_testing_with_home(seed.clone(), home.path().into());
    let source = Source::new(no_user("header.e30.initial"));
    let pause = Operation::Resolve.pause(&source);
    let pending = {
        let manager = manager.clone();
        tokio::spawn(async move { manager.set_external_auth(source).await })
    };
    tokio::time::timeout(Duration::from_secs(2), pause.loaded.notified())
        .await
        .unwrap();
    manager.clear_external_auth();
    pause.release.notify_one();
    assert!(
        tokio::time::timeout(Duration::from_secs(2), pending)
            .await
            .unwrap()
            .unwrap()
            .is_err()
    );
    assert!(manager.external_auth_provider().is_none());
    assert_current(&manager, &seed);
}
