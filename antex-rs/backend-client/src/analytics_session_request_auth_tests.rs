//! Distinguish request authority failures from an actual server rejection.
use super::*;
use antex_http_client::OutboundProxyPolicy;
use antex_login::ExternalAuth;
use antex_login::ExternalAuthFuture;
use antex_login::ExternalAuthRefreshContext;
use pretty_assertions::assert_eq;
use std::sync::atomic::AtomicBool;
use std::sync::atomic::AtomicUsize;
use std::sync::atomic::Ordering;
use wiremock::Mock;
use wiremock::MockServer;
use wiremock::ResponseTemplate;
use wiremock::matchers::method;

struct Source {
    auth: AntexAuth,
    unavailable: AtomicBool,
    refreshes: AtomicUsize,
}

impl ExternalAuth for Source {
    fn resolve(&self) -> ExternalAuthFuture<'_, AntexAuth> {
        Box::pin(async {
            if self.unavailable.load(Ordering::SeqCst) {
                Err(std::io::Error::other("synthetic source unavailable"))
            } else {
                Ok(self.auth.clone())
            }
        })
    }

    fn refresh(&self, _context: ExternalAuthRefreshContext) -> ExternalAuthFuture<'_, AntexAuth> {
        Box::pin(async {
            self.refreshes.fetch_add(/*val*/ 1, Ordering::SeqCst);
            // Recovery succeeds, but the next request acquisition cannot resolve.
            self.unavailable.store(true, Ordering::SeqCst);
            Ok(self.auth.clone())
        })
    }
}

async fn session(server: &MockServer) -> (AnalyticsSession, Arc<Source>) {
    let auth = AntexAuth::from_external_chatgpt_tokens(
        "e30.eyJleHAiOjQxMDI0NDQ4MDAsImh0dHBzOi8vYXBpLm9wZW5haS5jb20vYXV0aCI6eyJjaGF0Z3B0X3VzZXJfaWQiOiJ1c2VyLWEifX0.test",
        "account-a",
        Some("plus"),
    )
    .unwrap();
    let auth_manager = AuthManager::from_auth_for_testing_with_home(
        auth.clone(),
        std::env::temp_dir().join(format!("analytics-consumer-{}", server.address().port())),
    );
    let source = Arc::new(Source {
        auth: auth.clone(),
        unavailable: AtomicBool::new(false),
        refreshes: AtomicUsize::new(/*v*/ 0),
    });
    auth_manager
        .set_external_auth(source.clone())
        .await
        .unwrap();
    let client = Client::new_without_redirects(
        server.uri(),
        HttpClientFactory::new(OutboundProxyPolicy::ReqwestDefault),
    )
    .with_auth_provider(antex_model_provider::auth_provider_from_auth_manager(
        auth_manager.clone(),
        &auth,
    ));
    (
        AnalyticsSession {
            client,
            auth_manager,
            account: AnalyticsAccount {
                id: "account-a".into(),
                email: None,
                plan_type: None,
            },
            auth,
        },
        source,
    )
}

#[tokio::test]
async fn initial_auth_failure_has_no_http_status_and_never_dispatches() {
    let server = MockServer::start().await;
    let (session, source) = session(&server).await;
    source.unavailable.store(true, Ordering::SeqCst);
    let error = session
        .request(|client| async move { client.get_user_settings().await })
        .await
        .unwrap_err();
    assert_eq!(error.status(), None);
    assert!(server.received_requests().await.unwrap().is_empty());
    assert_eq!(source.refreshes.load(Ordering::SeqCst), 0);
    assert_eq!(
        session.auth_manager.auth_cached(),
        Some(session.auth.clone())
    );
}

#[tokio::test]
async fn revalidation_failure_after_successful_recovery_keeps_real_401() {
    let server = MockServer::start().await;
    let expected_requests = 1;
    Mock::given(method("GET"))
        .respond_with(ResponseTemplate::new(/*s*/ 401))
        .expect(expected_requests)
        .mount(&server)
        .await;
    let (session, source) = session(&server).await;
    let error = session
        .request(|client| async move { client.get_user_settings().await })
        .await
        .unwrap_err();
    assert_eq!(error.status().map(|status| status.as_u16()), Some(401));
    assert_eq!(source.refreshes.load(Ordering::SeqCst), 1);
    assert_eq!(server.received_requests().await.unwrap().len(), 1);
    // Identity metadata remains usable; it is not request authority.
    session.ensure_identity().await.unwrap();
    assert!(session.auth_manager.auth_for_request().await.is_err());
    server.verify().await;
}

#[tokio::test]
async fn source_reinstallation_after_http_cannot_publish_or_retry() {
    for status in [200, 401] {
        let server = MockServer::start().await;
        let expected_requests = 1;
        Mock::given(method("GET"))
            .respond_with(
                ResponseTemplate::new(status)
                    .set_body_json(serde_json::json!({ "commit_attribution_enabled": true })),
            )
            .expect(expected_requests)
            .mount(&server)
            .await;
        let (session, source) = session(&server).await;
        let error = session
            .request(|client| {
                let manager = session.auth_manager.clone();
                let source = source.clone();
                async move {
                    let result = client.get_user_settings().await;
                    // Identical credentials and Arc still create a new epoch.
                    manager.set_external_auth(source).await.unwrap();
                    result
                }
            })
            .await
            .unwrap_err();
        assert_eq!(
            error.status().map(|status| status.as_u16()),
            (status == 401).then_some(401)
        );
        assert_eq!(source.refreshes.load(Ordering::SeqCst), 0);
        assert_eq!(server.received_requests().await.unwrap().len(), 1);
        server.verify().await;
    }
}
