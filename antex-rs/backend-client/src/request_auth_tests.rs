//! Exercise the real backend dispatch boundary with a retained dynamic auth provider.
use super::*;
use antex_login::AuthHeaders;
use antex_login::AuthManager;
use antex_login::ExternalAuth;
use antex_login::ExternalAuthFuture;
use antex_login::ExternalAuthRefreshContext;
use antex_login::RefreshTokenError;
use antex_protocol::error::RefreshTokenFailedError;
use antex_protocol::error::RefreshTokenFailedReason;
use pretty_assertions::assert_eq;
use std::io;
use std::sync::Arc;
use std::sync::Mutex;

struct Source {
    auth: AntexAuth,
    failure: Mutex<Option<io::ErrorKind>>,
}

impl ExternalAuth for Source {
    fn resolve(&self) -> ExternalAuthFuture<'_, AntexAuth> {
        Box::pin(async {
            match *self.failure.lock().unwrap() {
                Some(kind) => Err(io::Error::new(kind, "synthetic source unavailable")),
                None => Ok(self.auth.clone()),
            }
        })
    }

    fn refresh(&self, _context: ExternalAuthRefreshContext) -> ExternalAuthFuture<'_, AntexAuth> {
        self.resolve()
    }

    fn classify_error(&self, error: io::Error) -> RefreshTokenError {
        if error.kind() == io::ErrorKind::PermissionDenied {
            RefreshTokenError::Permanent(RefreshTokenFailedError::new(
                RefreshTokenFailedReason::Other,
                "synthetic source revoked",
            ))
        } else {
            RefreshTokenError::Transient(error)
        }
    }
}

#[tokio::test]
async fn retained_backend_client_rejects_failed_source_in_both_dispatch_paths() {
    for failure in [io::ErrorKind::Other, io::ErrorKind::PermissionDenied] {
        let server = wiremock::MockServer::start().await;
        wiremock::Mock::given(wiremock::matchers::method("GET"))
            .respond_with(
                wiremock::ResponseTemplate::new(/*s*/ 200)
                    .set_body_json(serde_json::json!({ "commit_attribution_enabled": true })),
            )
            .mount(&server)
            .await;
        let initial = AntexAuth::Headers(AuthHeaders::new(
            [
                (
                    "authorization".parse().unwrap(),
                    "Bearer synthetic-working".parse().unwrap(),
                ),
                (
                    "chatgpt-account-id".parse().unwrap(),
                    "account-a".parse().unwrap(),
                ),
            ]
            .into_iter()
            .collect(),
        ));
        let manager = AuthManager::from_auth_for_testing(initial.clone());
        let source = Arc::new(Source {
            auth: initial.clone(),
            failure: Mutex::new(None),
        });
        manager.set_external_auth(source.clone()).await.unwrap();
        let client = Client::new(
            server.uri(),
            HttpClientFactory::new(OutboundProxyPolicy::ReqwestDefault),
        )
        .with_auth_provider(antex_model_provider::auth_provider_from_auth_manager(
            manager.clone(),
            &initial,
        ));
        let revisions = *manager.auth_change_state_receiver().borrow();
        *source.failure.lock().unwrap() = Some(failure);
        for _ in 0..2 {
            assert_eq!(manager.auth().await, Some(initial.clone()));
            assert!(client.get_user_settings().await.is_err());
            assert!(
                client
                    .list_tasks(
                        /*limit*/ None, /*task_filter*/ None,
                        /*environment_id*/ None, /*cursor*/ None
                    )
                    .await
                    .is_err()
            );
            assert!(server.received_requests().await.unwrap().is_empty());
            assert_eq!(
                (
                    manager.auth_cached(),
                    *manager.auth_change_state_receiver().borrow()
                ),
                (Some(initial.clone()), revisions)
            );
        }
        // Recovery requires a fresh installation after a permanent failure. The
        // old client must not adopt that installation's otherwise equal identity.
        *source.failure.lock().unwrap() = None;
        manager.set_external_auth(source).await.unwrap();
        assert!(client.get_user_settings().await.is_err());
        assert!(server.received_requests().await.unwrap().is_empty());
        let fresh = Client::new(
            server.uri(),
            HttpClientFactory::new(OutboundProxyPolicy::ReqwestDefault),
        )
        .with_auth_provider(antex_model_provider::auth_provider_from_auth_manager(
            manager, &initial,
        ));
        assert!(
            fresh
                .get_user_settings()
                .await
                .unwrap()
                .commit_attribution_enabled
        );
        let requests = server.received_requests().await.unwrap();
        assert_eq!(requests.len(), 1);
        assert_eq!(
            requests[0]
                .headers
                .get(http::header::AUTHORIZATION)
                .unwrap(),
            "Bearer synthetic-working"
        );
    }
}
