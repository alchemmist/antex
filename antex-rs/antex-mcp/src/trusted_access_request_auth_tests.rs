use super::*;
use pretty_assertions::assert_eq;
use std::sync::atomic::AtomicUsize;
use std::sync::atomic::Ordering;

struct CountedSequence {
    sequence: SequencedExternalAuth,
    resolutions: AtomicUsize,
}

impl ExternalAuth for CountedSequence {
    fn resolve(&self) -> ExternalAuthFuture<'_, AntexAuth> {
        self.resolutions.fetch_add(/*val*/ 1, Ordering::SeqCst);
        self.sequence.resolve()
    }

    fn refresh(&self, _context: ExternalAuthRefreshContext) -> ExternalAuthFuture<'_, AntexAuth> {
        self.resolve()
    }
}

struct FailingSource {
    current: Mutex<Option<AntexAuth>>,
}

impl ExternalAuth for FailingSource {
    fn resolve(&self) -> ExternalAuthFuture<'_, AntexAuth> {
        Box::pin(async {
            self.current
                .lock()
                .unwrap()
                .clone()
                .ok_or_else(|| std::io::Error::other("synthetic source unavailable"))
        })
    }

    fn refresh(&self, _context: ExternalAuthRefreshContext) -> ExternalAuthFuture<'_, AntexAuth> {
        self.resolve()
    }
}

#[tokio::test]
async fn publication_checks_owner_without_resolving_another_credential() {
    let initial = chatgpt_auth("account-a");
    let refreshed = AntexAuth::from_external_chatgpt_tokens(
        "header.e30.refreshed",
        "account-a",
        /*chatgpt_plan_type*/ None,
    )
    .unwrap();
    let source = Arc::new(CountedSequence {
        sequence: SequencedExternalAuth(Mutex::new(VecDeque::from([
            initial.clone(),
            refreshed,
            header_auth(),
        ]))),
        resolutions: AtomicUsize::new(/*v*/ 0),
    });
    let client = Arc::new(RecordingHttpClient::new(
        /*status*/ 200,
        cyber_response("inactive", json!([])),
    ));
    let home = tempfile::tempdir().unwrap();
    let manager = AuthManager::from_auth_for_testing_with_home(initial.clone(), home.path().into());
    let context = TrustedAccessContext::new(
        initial,
        manager,
        "https://chatgpt.com/backend-api".to_string(),
        client.clone(),
    );
    context
        .auth_manager
        .set_external_auth(source.clone())
        .await
        .unwrap();

    assert_eq!(
        context.add_context(/*meta*/ None).await,
        Some(expected_metadata("not_granted", json!([])))
    );
    assert_eq!(source.resolutions.load(Ordering::SeqCst), 2);
    assert_eq!(
        *source.sequence.0.lock().unwrap(),
        VecDeque::from([header_auth()])
    );
    let requests = client.requests.lock().unwrap();
    assert_eq!(requests.len(), 1);
    assert!(requests[0].headers.iter().any(|header| {
        header.name == "authorization" && header.value == "Bearer header.e30.refreshed"
    }));
}

#[tokio::test]
async fn publication_rejects_source_reinstallation_and_owner_aba_during_get() {
    for change in [
        "same-source reinstall",
        "new source",
        "source ABA",
        "account ABA",
    ] {
        let initial = chatgpt_auth("account-a");
        let source = Arc::new(StaticExternalAuth(initial.clone()));
        let mut client = RecordingHttpClient::new(
            /*status*/ 200,
            cyber_response(
                "active",
                json!([{ "level": "tac2", "source": "individual" }]),
            ),
        );
        client.response_gate = Some((Notify::new(), Notify::new()));
        let client = Arc::new(client);
        let home = tempfile::tempdir().unwrap();
        let manager =
            AuthManager::from_auth_for_testing_with_home(initial.clone(), home.path().into());
        let context = TrustedAccessContext::new(
            initial.clone(),
            manager,
            "https://chatgpt.com/backend-api".to_string(),
            client.clone(),
        );
        let manager = &context.auth_manager;
        manager.set_external_auth(source.clone()).await.unwrap();
        let (metadata, ()) = tokio::join!(context.add_context(/*meta*/ None), async {
            let (requested, release) = client.response_gate.as_ref().unwrap();
            requested.notified().await;
            match change {
                "same-source reinstall" => {
                    manager.set_external_auth(source.clone()).await.unwrap();
                }
                "new source" => {
                    manager
                        .set_external_auth(Arc::new(StaticExternalAuth(initial.clone())))
                        .await
                        .unwrap();
                }
                "source ABA" => {
                    manager.clear_external_auth();
                    manager.set_external_auth(source.clone()).await.unwrap();
                }
                "account ABA" => {
                    manager
                        .set_external_auth(Arc::new(StaticExternalAuth(chatgpt_auth("account-b"))))
                        .await
                        .unwrap();
                    manager.set_external_auth(source.clone()).await.unwrap();
                }
                _ => unreachable!("test case"),
            }
            release.notify_one();
        });
        assert_eq!(
            metadata,
            Some(expected_metadata("unknown", json!([]))),
            "{change}"
        );
        assert_eq!(client.requests.lock().unwrap().len(), 1, "{change}");
        assert_eq!(manager.auth_cached(), Some(initial), "{change}");
    }
}

#[tokio::test]
async fn failed_source_cannot_export_cached_trusted_access_grants() {
    for replacement in [None, Some(chatgpt_auth("account-b"))] {
        let home = tempfile::tempdir().unwrap();
        let initial = chatgpt_auth("account-a");
        let manager =
            AuthManager::from_auth_for_testing_with_home(initial.clone(), home.path().into());
        let source = Arc::new(FailingSource {
            current: Mutex::new(Some(initial.clone())),
        });
        manager.set_external_auth(source.clone()).await.unwrap();
        let revisions = *manager.auth_change_state_receiver().borrow();
        let client = Arc::new(RecordingHttpClient::new(
            /*status*/ 200,
            cyber_response(
                "active",
                json!([{ "level": "tac2", "source": "individual" }]),
            ),
        ));
        let context = TrustedAccessContext::new(
            initial.clone(),
            manager.clone(),
            "https://chatgpt.com/backend-api".to_string(),
            client.clone(),
        );
        *source.current.lock().unwrap() = replacement;
        for _ in 0..2 {
            assert_eq!(manager.auth().await, Some(initial.clone()));
            assert!(manager.auth_for_request().await.is_err());
            assert_eq!(
                context
                    .add_context(Some(expected_metadata(
                        "granted",
                        json!([
                            { "level": "tac2", "source": "user" }
                        ])
                    )))
                    .await,
                Some(expected_metadata("unknown", json!([])))
            );
            assert!(client.requests.lock().unwrap().is_empty());
            assert_eq!(
                (
                    manager.auth_cached(),
                    *manager.auth_change_state_receiver().borrow()
                ),
                (Some(initial.clone()), revisions)
            );
        }
        *source.current.lock().unwrap() = Some(initial);
        assert_eq!(
            context.add_context(/*meta*/ None).await,
            Some(expected_metadata(
                "granted",
                json!([{ "level": "tac2", "source": "user" }])
            ))
        );
        assert_eq!(client.requests.lock().unwrap().len(), 1);
    }
}
