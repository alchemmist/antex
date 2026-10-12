//! Actual sampler/lease/prewarm paths. No replacement dispatch or retry loop.
use super::*;
use antex_login::ExternalAuth;
use antex_login::ExternalAuthFuture;
use antex_login::ExternalAuthRefreshContext;
use antex_login::RefreshTokenError;
use antex_protocol::error::RefreshTokenFailedError;
use antex_protocol::error::RefreshTokenFailedReason;
use pretty_assertions::assert_eq;
use std::future::Future;

#[derive(Clone)]
enum Outcome {
    Auth(AntexAuth),
    Transient,
    Permanent,
}

struct Source {
    current: Mutex<Outcome>,
    after_routing: Mutex<Option<Outcome>>,
    routing_manager: Mutex<Option<std::sync::Weak<AuthManager>>>,
    calls: AtomicUsize,
    fail_at: AtomicUsize,
}

impl Source {
    fn new(auth: AntexAuth) -> Arc<Self> {
        Arc::new(Self {
            current: Mutex::new(Outcome::Auth(auth)),
            after_routing: Mutex::new(None),
            routing_manager: Mutex::new(None),
            calls: AtomicUsize::new(0),
            fail_at: AtomicUsize::new(usize::MAX),
        })
    }
}

impl ExternalAuth for Source {
    fn resolve(&self) -> ExternalAuthFuture<'_, AntexAuth> {
        Box::pin(async {
            let call = self.calls.fetch_add(/*val*/ 1, Ordering::SeqCst) + 1;
            let mut current = self.current.lock().unwrap();
            if call == self.fail_at.load(Ordering::SeqCst) {
                *current = Outcome::Transient;
            }
            match current.clone() {
                Outcome::Auth(auth) => Ok(auth),
                Outcome::Transient => Err(std::io::Error::other("synthetic external unavailable")),
                Outcome::Permanent => Err(std::io::Error::new(
                    std::io::ErrorKind::PermissionDenied,
                    "synthetic revoked",
                )),
            }
        })
    }

    fn refresh(&self, _context: ExternalAuthRefreshContext) -> ExternalAuthFuture<'_, AntexAuth> {
        ExternalAuth::resolve(self)
    }

    fn classify_error(&self, error: std::io::Error) -> RefreshTokenError {
        if error.kind() == std::io::ErrorKind::PermissionDenied {
            RefreshTokenError::Permanent(RefreshTokenFailedError::new(
                RefreshTokenFailedReason::Other,
                "synthetic revoked",
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
    ) -> ModelProviderFuture<'_, std::io::Result<Option<WorkspaceRouting>>> {
        Box::pin(async {
            if let Some(next) = self.after_routing.lock().unwrap().take() {
                *self.current.lock().unwrap() = next;
            }
            let manager = self
                .routing_manager
                .lock()
                .unwrap()
                .as_ref()
                .and_then(std::sync::Weak::upgrade);
            if let Some(manager) = manager {
                manager
                    .auth_for_request()
                    .await
                    .map_err(std::io::Error::from)?;
            }
            Ok(None)
        })
    }
}

async fn idle_websockets() -> responses::WebSocketTestServer {
    responses::start_websocket_server_with_headers(vec![
        responses::WebSocketConnectionConfig {
            requests: vec![vec![
                responses::ev_output_text_delta("low"),
                responses::ev_completed("score")
            ]],
            response_headers: Vec::new(),
            accept_delay: None,
            close_after_requests: false,
        };
        INITIAL_WEBSOCKET_CONNECTIONS
    ])
    .await
}

fn sparse_auth(account: &str, token: &str) -> AntexAuth {
    AntexAuth::from_external_chatgpt_tokens(token, account, /*chatgpt_plan_type*/ None).unwrap()
}

#[tokio::test]
async fn failed_sources_block_initial_prewarm_and_idle_pool_evidence() -> Result<()> {
    skip_if_no_network!(Ok(()));
    for warm in [false, true] {
        for failure in [
            Outcome::Transient,
            Outcome::Permanent,
            Outcome::Auth(sparse_auth("workspace-b", "header.e30.rejected")),
        ] {
            let http = responses::start_mock_server().await;
            let response = responses::mount_sse_once(
                &http,
                responses::sse(vec![
                    responses::ev_output_text_delta("low"),
                    responses::ev_completed("score"),
                ]),
            )
            .await;
            let ws = idle_websockets().await;
            let gateway = Gateway::new(&http.uri(), ws.uri()).await?;
            gateway
                .allowed_opens
                .store(INITIAL_WEBSOCKET_CONNECTIONS, Ordering::SeqCst);
            let home = tempfile::tempdir()?;
            let initial = sparse_auth("workspace-a", "header.e30.initial");
            let manager =
                AuthManager::from_auth_for_testing_with_home(initial.clone(), home.path().into());
            let source = Source::new(initial.clone());
            manager.set_external_auth(source.clone()).await?;
            let mut config = sampler_config(format!("{}/backend-api/codex", gateway.url));
            config.provider =
                create_model_provider(config.provider.info().clone(), Some(manager.clone()));
            let sampler = LunaSampler::new(config);
            if warm {
                sampler.prewarm().await;
            }
            *source.current.lock().unwrap() = failure;
            if !warm {
                sampler.prewarm().await;
                assert!(
                    sampler
                        .connections
                        .idle_connections
                        .lock()
                        .unwrap()
                        .is_empty()
                );
                assert_eq!(gateway.opens.load(Ordering::SeqCst), 0);
            }
            for _ in 0..2 {
                assert!(sampler.sample(sample_request("blocked")).await.is_err());
                assert_eq!(manager.auth_cached(), Some(initial.clone()));
                assert!(response.requests().is_empty());
                assert!(ws.connections().iter().all(Vec::is_empty));
            }
        }
    }
    Ok(())
}

#[tokio::test]
async fn failed_resolution_after_retryable_dispatch_never_sends_a_second_request() -> Result<()> {
    skip_if_no_network!(Ok(()));
    for failure in [
        Outcome::Transient,
        Outcome::Permanent,
        Outcome::Auth(sparse_auth("workspace-b", "header.e30.rejected")),
    ] {
        for status in [401, 500] {
            let http = responses::start_mock_server().await;
            let home = tempfile::tempdir()?;
            let initial = sparse_auth("workspace-a", "header.e30.initial");
            let manager =
                AuthManager::from_auth_for_testing_with_home(initial.clone(), home.path().into());
            let source = Source::new(initial.clone());
            manager.set_external_auth(source.clone()).await?;
            let source_on_dispatch = source.clone();
            let failure = failure.clone();
            wiremock::Mock::given(wiremock::matchers::method("POST"))
                .respond_with(move |_: &wiremock::Request| {
                    *source_on_dispatch.current.lock().unwrap() = failure.clone();
                    wiremock::ResponseTemplate::new(status)
                })
                .mount(&http)
                .await;
            let mut config = sampler_config(format!("{}/backend-api/codex", http.uri()));
            config.provider =
                create_model_provider(config.provider.info().clone(), Some(manager.clone()));
            let sampler = LunaSampler::new(config);
            let error = sampler.sample(sample_request("retry")).await.unwrap_err();
            assert!(matches!(error, LunaSamplerError::Provider(_)), "{error}");
            let requests = http.received_requests().await.unwrap();
            assert_eq!(
                requests
                    .iter()
                    .filter(|request| request.method.as_str() == "POST")
                    .count(),
                1
            );
            assert_eq!(manager.auth_cached(), Some(initial));
        }
    }
    Ok(())
}

#[tokio::test]
async fn equal_source_reinstall_invalidates_idle_sockets_and_blocked_admission() -> Result<()> {
    skip_if_no_network!(Ok(()));
    for admission in [false, true] {
        let http = responses::start_mock_server().await;
        let response = responses::mount_sse_once(
            &http,
            responses::sse(vec![
                responses::ev_output_text_delta("low"),
                responses::ev_completed("score"),
            ]),
        )
        .await;
        let ws = idle_websockets().await;
        let gateway = Gateway::new(&http.uri(), ws.uri()).await?;
        gateway
            .allowed_opens
            .store(INITIAL_WEBSOCKET_CONNECTIONS, Ordering::SeqCst);
        let home = tempfile::tempdir()?;
        let initial = sparse_auth("workspace-a", "header.e30.initial");
        let manager =
            AuthManager::from_auth_for_testing_with_home(initial.clone(), home.path().into());
        let source = Source::new(initial);
        manager.set_external_auth(source.clone()).await?;
        let mut config = sampler_config(format!("{}/backend-api/codex", gateway.url));
        config.provider =
            create_model_provider(config.provider.info().clone(), Some(manager.clone()));
        let sampler = LunaSampler::new(config);
        sampler.prewarm().await;
        let revisions = *manager.auth_change_state_receiver().borrow();
        if admission {
            let permits = sampler
                .connections
                .classifications
                .clone()
                .acquire_many_owned(MAX_CONCURRENT_REQUESTS as u32)
                .await?;
            let mut sample = Box::pin(sampler.sample(sample_request("admission")));
            std::future::poll_fn(|cx| {
                assert!(sample.as_mut().poll(cx).is_pending());
                std::task::Poll::Ready(())
            })
            .await;
            manager.set_external_auth(source).await?;
            let error = tokio::time::timeout(Duration::from_secs(/*secs*/ 2), sample)
                .await?
                .unwrap_err();
            assert!(error.to_string().contains("account changed"));
            drop(permits);
            assert!(response.requests().is_empty());
        } else {
            manager.set_external_auth(source).await?;
            assert_eq!(
                sampler.sample(sample_request("new-operation")).await?,
                "low"
            );
            assert_eq!(response.requests().len(), 1);
        }
        assert_eq!(*manager.auth_change_state_receiver().borrow(), revisions);
        assert!(ws.connections().iter().all(Vec::is_empty));
    }
    Ok(())
}

#[tokio::test]
async fn sparse_same_source_rotation_during_routing_preserves_original_operation() -> Result<()> {
    skip_if_no_network!(Ok(()));
    let http = responses::start_mock_server().await;
    let response = responses::mount_sse_once(
        &http,
        responses::sse(vec![
            responses::ev_output_text_delta("low"),
            responses::ev_completed("score"),
        ]),
    )
    .await;
    let home = tempfile::tempdir()?;
    let initial = sparse_auth("workspace-a", "header.e30.initial");
    let manager = AuthManager::from_auth_for_testing_with_home(initial.clone(), home.path().into());
    let source = Source::new(initial);
    manager.set_external_auth(source.clone()).await?;
    let original = manager.unauthorized_recovery();
    *source.after_routing.lock().unwrap() = Some(Outcome::Auth(sparse_auth(
        "workspace-a",
        "header.e30.rotated",
    )));
    *source.routing_manager.lock().unwrap() = Some(Arc::downgrade(&manager));
    let resolver: Arc<dyn WorkspaceRoutingResolver> = source;
    manager.set_workspace_routing_resolver(Arc::downgrade(&resolver));
    let mut config = sampler_config(format!("{}/backend-api/codex", http.uri()));
    config.provider = create_model_provider(config.provider.info().clone(), Some(manager.clone()));
    assert_eq!(
        LunaSampler::new(config)
            .sample(sample_request("rotation"))
            .await?,
        "low"
    );
    original.ensure_same_owner()?;
    assert!(
        manager
            .auth_change_state_receiver()
            .borrow()
            .owner_generation
            > original.owner_generation()
    );
    assert_eq!(
        response.single_request().header("authorization"),
        Some("Bearer header.e30.rotated".into())
    );
    Ok(())
}

#[tokio::test]
async fn request_kind_and_explicit_cyber_resolution_fail_closed_after_setup() -> Result<()> {
    skip_if_no_network!(Ok(()));
    for successful_lookups in [4, 5] {
        let http = responses::start_mock_server().await;
        let response = responses::mount_sse_once(
            &http,
            responses::sse(vec![responses::ev_completed("unused")]),
        )
        .await;
        let home = tempfile::tempdir()?;
        let initial = sparse_auth("workspace-a", "header.e30.initial");
        let manager =
            AuthManager::from_auth_for_testing_with_home(initial.clone(), home.path().into());
        let source = Source::new(initial.clone());
        manager.set_external_auth(source.clone()).await?;
        // Real producer order: provider mode, routing mode, post-routing validation,
        // scoped credentials, request kind, explicit cyber. Fail the latter two.
        source.fail_at.store(
            source.calls.load(Ordering::SeqCst) + successful_lookups + 1,
            Ordering::SeqCst,
        );
        let mut config = sampler_config(format!("{}/backend-api/codex", http.uri()));
        config.provider =
            create_model_provider(config.provider.info().clone(), Some(manager.clone()));
        let sampler = LunaSampler::new(config);
        // Existing production cooldown keeps background resolution out of this
        // call-order test; classification still uses the real HTTP lease path.
        *sampler.connections.retry_after.lock().unwrap() = Some(Instant::now() + CONNECT_COOLDOWN);
        let mut request = sample_request("late-resolution");
        request.cyber_access_program =
            Some(antex_protocol::turn_input::CyberAccessProgram::DaybreakBlue);
        assert!(sampler.sample(request).await.is_err());
        assert_eq!(
            source.calls.load(Ordering::SeqCst),
            source.fail_at.load(Ordering::SeqCst)
        );
        assert_eq!(manager.auth_cached(), Some(initial));
        assert!(response.requests().is_empty());
    }
    Ok(())
}

#[tokio::test]
async fn equal_source_aba_cancels_pending_dispatch_without_anonymous_or_stale_retry() -> Result<()>
{
    skip_if_no_network!(Ok(()));
    for clear in [false, true] {
        let http = responses::start_mock_server().await;
        let response = responses::mount_response_once(
            &http,
            wiremock::ResponseTemplate::new(/*s*/ 500)
                .set_delay(Duration::from_secs(/*secs*/ 60)),
        )
        .await;
        let home = tempfile::tempdir()?;
        let initial = sparse_auth("workspace-a", "header.e30.initial");
        let manager =
            AuthManager::from_auth_for_testing_with_home(initial.clone(), home.path().into());
        let source = Source::new(initial);
        manager.set_external_auth(source.clone()).await?;
        let mut config = sampler_config(format!("{}/backend-api/codex", http.uri()));
        config.provider =
            create_model_provider(config.provider.info().clone(), Some(manager.clone()));
        let sampler = LunaSampler::new(config);
        let sample = sampler.sample(sample_request("pending-dispatch"));
        tokio::pin!(sample);
        tokio::time::timeout(Duration::from_secs(/*secs*/ 2), async {
            while response.requests().is_empty() {
                tokio::select! {
                    result = &mut sample => panic!("sample completed before replacement: {result:?}"),
                    () = tokio::task::yield_now() => {}
                }
            }
        }).await?;
        let revisions = *manager.auth_change_state_receiver().borrow();
        if clear {
            manager.clear_external_auth();
        }
        manager.set_external_auth(source).await?;
        if !clear {
            assert_eq!(*manager.auth_change_state_receiver().borrow(), revisions);
        }
        let error = tokio::time::timeout(Duration::from_secs(/*secs*/ 2), sample)
            .await?
            .unwrap_err();
        assert!(error.to_string().contains("account changed"));
        assert_eq!(response.requests().len(), 1);
    }
    Ok(())
}
