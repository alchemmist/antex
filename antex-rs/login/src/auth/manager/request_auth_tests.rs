//! Real request producers, including the final credential/factory snapshot.
use super::*;
use pretty_assertions::assert_eq;

#[tokio::test]
async fn request_and_factory_fail_repeatedly_without_discarding_working_credentials() {
    for incompatible in [false, true] {
        let home = tempfile::tempdir().unwrap();
        let initial = credential(Kind::Chatgpt, "account-a", "initial");
        let manager =
            AuthManager::from_auth_for_testing_with_home(initial.clone(), home.path().into());
        let source = Source::new(initial.clone());
        manager.set_external_auth(source.clone()).await.unwrap();
        let owner = manager.unauthorized_recovery();
        let before = snapshot(&manager);
        let revisions = *manager.auth_change_state_receiver().borrow();
        if incompatible {
            *source.auth.lock().unwrap() = credential(Kind::Chatgpt, "account-b", "rejected");
        } else {
            source.fail.store(true, Ordering::SeqCst);
        }
        for _ in 0..2 {
            assert!(manager.auth_for_request().await.is_err());
            assert!(
                manager
                    .auth_for_request_with_http_client_factory()
                    .await
                    .is_err()
            );
            // Compatibility metadata retains A; request credentials above remain rejected.
            assert_eq!(manager.auth().await, Some(initial.clone()));
            assert!(manager.auth_with_http_client_factory().await.is_none());
            assert_eq!(
                (
                    snapshot(&manager),
                    *manager.auth_change_state_receiver().borrow()
                ),
                (before.clone(), revisions)
            );
            owner.ensure_same_owner().unwrap();
        }
        // A failed explicit candidate must not poison the installed source.
        let candidate = Source::new(initial.clone());
        candidate.fail.store(true, Ordering::SeqCst);
        assert!(manager.set_external_auth(candidate).await.is_err());
        owner.ensure_same_owner().unwrap();
        source.fail.store(false, Ordering::SeqCst);
        *source.auth.lock().unwrap() = initial.clone();
        if !incompatible {
            // Recorded permanent failures remain terminal for THIS installation.
            assert!(manager.auth_for_request().await.is_err());
            manager.set_external_auth(source).await.unwrap();
        }
        let (auth, _factory) = manager
            .auth_for_request_with_http_client_factory()
            .await
            .unwrap()
            .unwrap();
        assert_eq!(auth, initial);
    }
}

#[tokio::test]
async fn request_snapshot_cannot_adopt_a_simultaneously_installed_source() {
    for factory in [false, true] {
        for same_arc in [false, true] {
            let home = tempfile::tempdir().unwrap();
            let initial = credential(Kind::Chatgpt, "account-a", "initial");
            let manager =
                AuthManager::from_auth_for_testing_with_home(initial.clone(), home.path().into());
            let source = Source::new(initial.clone());
            manager.set_external_auth(source.clone()).await.unwrap();
            let pause = Operation::Resolve.pause(&source);
            let pending = {
                let manager = manager.clone();
                tokio::spawn(async move {
                    if factory {
                        manager
                            .auth_for_request_with_http_client_factory()
                            .await
                            .map(|value| value.map(|(auth, _)| auth))
                    } else {
                        manager.auth_for_request().await
                    }
                })
            };
            tokio::time::timeout(Duration::from_secs(/*secs*/ 2), pause.loaded.notified())
                .await
                .unwrap();
            let winner = if same_arc {
                initial
            } else {
                credential(Kind::Chatgpt, "account-b", "winner")
            };
            if same_arc {
                manager.set_external_auth(source).await.unwrap();
            } else {
                manager
                    .set_external_auth(Source::new(winner.clone()))
                    .await
                    .unwrap();
            }
            let before = snapshot(&manager);
            let revisions = *manager.auth_change_state_receiver().borrow();
            pause.release.notify_one();
            assert!(
                tokio::time::timeout(Duration::from_secs(/*secs*/ 2), pending)
                    .await
                    .unwrap()
                    .unwrap()
                    .is_err()
            );
            assert_eq!(
                (
                    snapshot(&manager),
                    *manager.auth_change_state_receiver().borrow()
                ),
                (before, revisions)
            );
            let (auth, _factory) = manager
                .auth_for_request_with_http_client_factory()
                .await
                .unwrap()
                .unwrap();
            assert_eq!(auth, winner);
        }
    }
}

#[tokio::test]
async fn successful_request_factory_is_bound_to_the_returned_credentials_owner() {
    let home = tempfile::tempdir().unwrap();
    let initial = credential(Kind::Chatgpt, "account-a", "initial");
    let mut manager =
        AuthManager::from_auth_for_testing_with_home(initial.clone(), home.path().into());
    let controller = antex_http_client::NetworkPolicyController::default();
    let policy = controller.policy();
    let factory = manager
        .http_client_factory()
        .with_network_policy(policy.clone());
    Arc::get_mut(&mut manager).unwrap().auth_route_config =
        AuthRouteConfig::from_http_client_factory(factory);
    manager
        .set_external_auth(Source::new(initial.clone()))
        .await
        .unwrap();
    assert!(controller.publish(
        policy.revision(),
        antex_http_client::DestinationPolicy::Unrestricted
    ));
    let (auth, factory) = manager
        .auth_for_request_with_http_client_factory()
        .await
        .unwrap()
        .unwrap();
    let endpoint = "https://example.com/".parse().unwrap();
    assert_eq!(auth, initial);
    assert!(factory.network_policy().acquire(&endpoint).is_ok());
    let replacement = credential(Kind::Chatgpt, "account-b", "replacement");
    manager
        .set_external_auth(Source::new(replacement.clone()))
        .await
        .unwrap();
    assert!(controller.publish(
        policy.revision(),
        antex_http_client::DestinationPolicy::Unrestricted
    ));
    assert_eq!(
        factory.network_policy().acquire(&endpoint).unwrap_err(),
        antex_http_client::NetworkPolicyDenied::Revoked
    );
    let (auth, factory) = manager
        .auth_for_request_with_http_client_factory()
        .await
        .unwrap()
        .unwrap();
    assert_eq!(auth, replacement);
    assert!(factory.network_policy().acquire(&endpoint).is_ok());
}
