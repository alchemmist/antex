//! Failed active-source resolution must not produce anonymous or stale request auth.
//! These tests exercise production producers and an in-memory Responses transport.

use crate::AgentIdentitySessionFallback;
use crate::ProviderAuthScope;
use crate::WorkspaceRoutingContext;
use crate::auth_provider_from_auth_manager;
use crate::create_model_provider;
use antex_api::ResponsesApiRequest;
use antex_api::ResponsesClient;
use antex_api::ResponsesOptions;
use antex_http_client::HttpTransport;
use antex_http_client::Request;
use antex_http_client::Response;
use antex_http_client::StreamResponse;
use antex_http_client::TransportError;
use antex_login::AgentIdentityAuthPolicy;
use antex_login::AntexAuth;
use antex_login::AuthManager;
use antex_login::ExternalAuth;
use antex_login::ExternalAuthFuture;
use antex_login::ExternalAuthRefreshContext;
use antex_login::RefreshTokenError;
use antex_login::WorkspaceRouting;
use antex_login::WorkspaceRoutingRequest;
use antex_login::WorkspaceRoutingResolver;
use antex_model_provider_info::ModelProviderInfo;
use antex_protocol::error::RefreshTokenFailedError;
use antex_protocol::error::RefreshTokenFailedReason;
use antex_protocol::protocol::SessionSource;
use pretty_assertions::assert_eq;
use std::io;
use std::sync::Arc;
use std::sync::Mutex;

#[path = "request_auth_consumer_tests.rs"]
mod request_auth_consumers;

#[derive(Clone)]
enum Resolution {
    Auth(AntexAuth),
    TransientFailure,
    PermanentFailure,
}

struct Source {
    current: Mutex<Resolution>,
    after_routing: Resolution,
}

impl ExternalAuth for Source {
    fn resolve(&self) -> ExternalAuthFuture<'_, AntexAuth> {
        Box::pin(async {
            match self.current.lock().unwrap().clone() {
                Resolution::Auth(auth) => Ok(auth),
                Resolution::TransientFailure => {
                    Err(io::Error::other("external source unavailable"))
                }
                Resolution::PermanentFailure => Err(io::Error::new(
                    io::ErrorKind::PermissionDenied,
                    "external source revoked",
                )),
            }
        })
    }

    fn refresh(&self, _context: ExternalAuthRefreshContext) -> ExternalAuthFuture<'_, AntexAuth> {
        ExternalAuth::resolve(self)
    }

    fn classify_error(&self, error: io::Error) -> RefreshTokenError {
        if error.kind() == io::ErrorKind::PermissionDenied {
            RefreshTokenError::Permanent(RefreshTokenFailedError::new(
                RefreshTokenFailedReason::Other,
                "external source revoked",
            ))
        } else {
            RefreshTokenError::Transient(error)
        }
    }
}

impl WorkspaceRoutingResolver for Source {
    fn resolve(
        &self,
        _request: WorkspaceRoutingRequest,
    ) -> crate::ModelProviderFuture<'_, io::Result<Option<WorkspaceRouting>>> {
        Box::pin(async {
            *self.current.lock().unwrap() = self.after_routing.clone();
            Ok(None)
        })
    }
}

fn chatgpt_auth(account: &str, signature: &str) -> AntexAuth {
    let token =
        "header.eyJodHRwczovL2FwaS5vcGVuYWkuY29tL2F1dGgiOnsiY2hhdGdwdF91c2VyX2lkIjoidXNlci1hIn19";
    AntexAuth::from_external_chatgpt_tokens(
        &format!("{token}.{signature}"),
        account,
        /*chatgpt_plan_type*/ None,
    )
    .unwrap()
}

fn scope() -> ProviderAuthScope {
    ProviderAuthScope {
        agent_identity_policy: AgentIdentityAuthPolicy::JwtOnly,
        session_source: SessionSource::Cli,
        agent_identity_session_fallback: AgentIdentitySessionFallback::default(),
    }
}

#[derive(Clone, Copy, Debug)]
enum RequestBoundary {
    Provider,
    Responses,
    Auth,
    Scoped,
    ManagedHeaders,
}

#[tokio::test]
async fn request_producers_reject_active_external_failures_without_discarding_working_cache() {
    for failure in [
        Resolution::TransientFailure,
        Resolution::PermanentFailure,
        Resolution::Auth(chatgpt_auth("workspace-b", "other-account")),
    ] {
        for boundary in [
            RequestBoundary::Provider,
            RequestBoundary::Responses,
            RequestBoundary::Auth,
            RequestBoundary::Scoped,
            RequestBoundary::ManagedHeaders,
        ] {
            let home = tempfile::tempdir().unwrap();
            let initial = chatgpt_auth("workspace-a", "initial");
            let manager = AuthManager::from_auth_for_testing_with_home(
                initial.clone(),
                home.path().to_owned(),
            );
            let source = Arc::new(Source {
                current: Mutex::new(Resolution::Auth(initial.clone())),
                after_routing: failure.clone(),
            });
            manager.set_external_auth(source.clone()).await.unwrap();
            let owner = manager.unauthorized_recovery();
            let revision = *manager.auth_change_receiver().borrow();
            let provider = create_model_provider(
                ModelProviderInfo::create_openai_provider(/*base_url*/ None),
                Some(manager.clone()),
            );
            let managed_headers = auth_provider_from_auth_manager(manager.clone(), &initial);
            *source.current.lock().unwrap() = failure.clone();
            // A cached permanent failure must also reject the next request, not
            // turn reload_external's no-op into a successful stale credential.
            for _ in 0..2 {
                let rejected = match boundary {
                    RequestBoundary::Provider => provider.api_provider().await.is_err(),
                    RequestBoundary::Responses => provider
                        .responses_api_provider(&WorkspaceRoutingContext::new(
                            "https://chatgpt.com/backend-api".into(),
                        ))
                        .await
                        .is_err(),
                    RequestBoundary::Auth => provider.api_auth().await.is_err(),
                    RequestBoundary::Scoped => provider.api_auth_for_scope(scope()).await.is_err(),
                    RequestBoundary::ManagedHeaders => {
                        managed_headers.resolve_auth_headers().await.is_err()
                    }
                };
                assert!(
                    rejected,
                    "{boundary:?} must fail before constructing request credentials"
                );
                assert_eq!(
                    (
                        manager.auth_cached(),
                        *manager.auth_change_receiver().borrow()
                    ),
                    (Some(initial.clone()), revision),
                );
                owner.ensure_same_owner().unwrap();
            }
        }
    }
}

#[derive(Clone)]
struct CaptureRequest(Arc<Mutex<Vec<Request>>>);

impl HttpTransport for CaptureRequest {
    async fn execute(&self, _request: Request) -> Result<Response, TransportError> {
        panic!("Responses must use the streaming transport");
    }

    async fn stream(&self, request: Request) -> Result<StreamResponse, TransportError> {
        self.0.lock().unwrap().push(request);
        Err(TransportError::Network("injected transport stop".into()))
    }
}

#[tokio::test]
async fn routing_time_external_resolution_only_dispatches_successful_same_workspace_rotation() {
    let rotated = chatgpt_auth("workspace-a", "rotated");
    for (next, expected_dispatches) in [
        (Resolution::Auth(rotated.clone()), 1),
        (Resolution::Auth(chatgpt_auth("workspace-b", "rejected")), 0),
        (Resolution::TransientFailure, 0),
        (Resolution::PermanentFailure, 0),
    ] {
        for scoped in [false, true] {
            let home = tempfile::tempdir().unwrap();
            let initial = chatgpt_auth("workspace-a", "initial");
            let manager = AuthManager::from_auth_for_testing_with_home(
                initial.clone(),
                home.path().to_owned(),
            );
            let source = Arc::new(Source {
                current: Mutex::new(Resolution::Auth(initial.clone())),
                after_routing: next.clone(),
            });
            manager.set_external_auth(source.clone()).await.unwrap();
            let owner = manager.unauthorized_recovery();
            let resolver: Arc<dyn WorkspaceRoutingResolver> = source;
            manager.set_workspace_routing_resolver(Arc::downgrade(&resolver));
            let provider = create_model_provider(
                ModelProviderInfo::create_openai_provider(/*base_url*/ None),
                Some(manager.clone()),
            );
            let requests = Arc::new(Mutex::new(Vec::new()));
            let result: antex_protocol::error::Result<()> = async {
                let route = provider
                    .responses_api_provider(&WorkspaceRoutingContext::new(
                        "https://chatgpt.com/backend-api".into(),
                    ))
                    .await?;
                let auth = if scoped {
                    provider.api_auth_for_scope(scope()).await?.auth
                } else {
                    provider.api_auth().await?
                };
                let mut api_provider = route.provider;
                api_provider.retry.max_attempts = 0;
                let client =
                    ResponsesClient::new(CaptureRequest(requests.clone()), api_provider, auth);
                client
                    .stream_request(
                        ResponsesApiRequest {
                            model: "test-classifier".into(),
                            input: Vec::new(),
                            tools: None,
                            tool_choice: "none".into(),
                            parallel_tool_calls: false,
                            reasoning: None,
                            store: false,
                            stream: true,
                            stream_options: None,
                            include: Vec::new(),
                            service_tier: None,
                            prompt_cache_key: None,
                            text: None,
                            client_metadata: None,
                            access_programs: None,
                        },
                        ResponsesOptions::default(),
                    )
                    .await
                    .map_err(antex_api::map_api_error)?;
                Ok(())
            }
            .await;
            // The transport deliberately errors too: an error alone cannot prove
            // that authentication failed closed. Count actual transport entries.
            assert!(result.is_err());
            let requests = requests.lock().unwrap();
            assert_eq!(requests.len(), expected_dispatches);
            let expected = if expected_dispatches == 1 {
                &rotated
            } else {
                &initial
            };
            assert_eq!(manager.auth_cached(), Some(expected.clone()));
            owner.ensure_same_owner().unwrap();
            if let Some(request) = requests.first() {
                assert_eq!(request.headers["chatgpt-account-id"], "workspace-a");
                assert_eq!(
                    request.headers[http::header::AUTHORIZATION]
                        .to_str()
                        .unwrap(),
                    format!("Bearer {}", rotated.get_token().unwrap()),
                );
            }
        }
    }
}

#[tokio::test]
async fn failed_explicit_replacement_leaves_old_source_usable_for_new_requests() {
    let home = tempfile::tempdir().unwrap();
    let initial = chatgpt_auth("workspace-a", "initial");
    let manager =
        AuthManager::from_auth_for_testing_with_home(initial.clone(), home.path().to_owned());
    let source = Arc::new(Source {
        current: Mutex::new(Resolution::Auth(initial.clone())),
        after_routing: Resolution::Auth(initial.clone()),
    });
    manager.set_external_auth(source.clone()).await.unwrap();
    let owner = manager.unauthorized_recovery();
    let replacement = Arc::new(Source {
        current: Mutex::new(Resolution::TransientFailure),
        after_routing: Resolution::TransientFailure,
    });
    assert!(manager.set_external_auth(replacement).await.is_err());
    let rotated = chatgpt_auth("workspace-a", "rotated");
    *source.current.lock().unwrap() = Resolution::Auth(rotated.clone());
    let provider = create_model_provider(
        ModelProviderInfo::create_openai_provider(/*base_url*/ None),
        Some(manager.clone()),
    );
    let headers = provider
        .api_auth_for_scope(scope())
        .await
        .unwrap()
        .auth
        .resolve_auth_headers()
        .await
        .unwrap();
    assert_eq!(
        headers,
        crate::auth_provider_from_auth(&rotated).to_auth_headers()
    );
    assert_eq!(manager.auth_cached(), Some(rotated));
    owner.ensure_same_owner().unwrap();
}
