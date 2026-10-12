//! New operations bind current auth without reviving retained MCP authority.

use std::collections::HashMap;
use std::sync::Arc;
use std::sync::atomic::AtomicBool;
use std::sync::atomic::Ordering;

use antex_config::McpServerTransportConfig;
use antex_login::AntexAuth;
use antex_login::ExternalAuth;
use antex_login::ExternalAuthFuture;
use antex_login::ExternalAuthRefreshContext;
use antex_mcp::CODEX_APPS_MCP_SERVER_NAME;
use antex_mcp::EffectiveMcpServer;
use antex_mcp::McpResourceClient;
use antex_mcp::McpRuntime;
use antex_mcp::McpServerRegistration;
use antex_mcp::ResolvedMcpCatalog;
use anyhow::Result;
use core_test_support::apps_test_server::AppsTestServer;
use core_test_support::responses::start_mock_server;
use pretty_assertions::assert_eq;
use serde_json::Value;
use serde_json::json;
use tokio::sync::Semaphore;
use tokio::time::timeout;

use crate::event_stream_tests::Fixture;
use crate::event_stream_tests::StaticAuth;
use crate::event_stream_tests::WAIT;

fn auth(token: &str) -> Result<AntexAuth> {
    Ok(AntexAuth::from_external_chatgpt_tokens(
        token, "account", /*chatgpt_plan_type*/ None,
    )?)
}

async fn apps_runtime(fixture: &Fixture, base_url: &str) -> Arc<McpRuntime> {
    let mut input = fixture.runtime_input.clone();
    let mut server = antex_mcp::hosted_plugin_runtime_mcp_server_config(
        base_url, /*apps_mcp_product_sku*/ None, /*originator*/ None,
    );
    let McpServerTransportConfig::StreamableHttp {
        bearer_token_env_var,
        ..
    } = &mut server.transport
    else {
        panic!("hosted runtime must use HTTP");
    };
    *bearer_token_env_var = None;
    let mut catalog = ResolvedMcpCatalog::builder();
    catalog.register(McpServerRegistration::from_hosted_apps(
        "test",
        /*contribution_order*/ 0,
        server.clone(),
    ));
    Arc::make_mut(&mut input.config).mcp_server_catalog = catalog.build();
    input.mcp_servers = HashMap::from([(
        CODEX_APPS_MCP_SERVER_NAME.to_string(),
        EffectiveMcpServer::from_host_config(server),
    )]);
    // The server was contributed after ConfigBuilder ran. Publish its normal
    // threadless permission profile before capturing executable tool bindings.
    input.config = Arc::new(input.config.for_threadless_operations(&input.mcp_servers));
    let runtime = Arc::new(McpRuntime::new(input).await);
    assert!(
        runtime
            .latest_wait_for_server_ready(CODEX_APPS_MCP_SERVER_NAME, WAIT)
            .await
    );
    runtime
}

#[tokio::test]
async fn new_latest_call_rebinds_but_old_prepared_call_and_upload_stay_rejected() -> Result<()> {
    let fixture = Fixture::new().await?;
    let server = start_mock_server().await;
    let apps = AppsTestServer::mount_hosted_plugin_runtime_searchable(&server).await?;
    let runtime = apps_runtime(&fixture, &apps.chatgpt_base_url).await;
    let binding = runtime
        .current_binding_for_call(CODEX_APPS_MCP_SERVER_NAME)
        .await
        .expect("ready binding");
    assert!(
        binding
            .config()
            .permission_profile_for_server(CODEX_APPS_MCP_SERVER_NAME)
            .is_some()
    );
    let prepared = binding
        .prepare_call(CODEX_APPS_MCP_SERVER_NAME, "calendar_create_event")
        .expect("prepared calendar tool");
    fixture
        .auth
        .set_external_auth(Arc::new(StaticAuth(auth("header.e30.second")?)))
        .await?;
    assert!(prepared.ensure_openai_file_upload_owner().is_err());
    assert!(
        prepared
            .call(
                /*arguments*/ None, /*meta*/ None, /*timeout*/ None
            )
            .await
            .is_err()
    );

    let result = runtime
        .latest_call_tool(
            CODEX_APPS_MCP_SERVER_NAME,
            "calendar_create_event",
            /*environment_id*/ None,
            Some(json!({"title": "Lunch", "starts_at": "2026-06-18T12:00:00Z"})),
            /*meta*/ None,
            /*requested_timeout*/ None,
            /*wait_for_server*/ true,
        )
        .await?;
    assert_eq!(result.is_error, Some(false));
    // Creating a new transport must not replace the provider on the old one.
    assert!(prepared.ensure_openai_file_upload_owner().is_err());
    assert!(
        prepared
            .call(
                /*arguments*/ None, /*meta*/ None, /*timeout*/ None
            )
            .await
            .is_err()
    );
    let requests = server.received_requests().await.expect("captured requests");
    let calls = requests
        .iter()
        .filter(|request| {
            serde_json::from_slice::<Value>(&request.body)
                .ok()
                .is_some_and(|body| body["method"] == "tools/call")
        })
        .collect::<Vec<_>>();
    assert_eq!(calls.len(), 1);
    assert_eq!(
        calls[0].headers["authorization"],
        "Bearer header.e30.second"
    );
    runtime.shutdown().await;
    fixture.runtime.shutdown().await;
    Ok(())
}

#[tokio::test]
async fn sparse_host_latest_call_uses_an_operation_private_transport() -> Result<()> {
    let fixture = Fixture::new().await?;
    assert!(!fixture.auth.has_external_auth());
    assert_eq!(
        fixture.auth.auth_cached().unwrap().get_chatgpt_user_id(),
        None
    );
    let server = start_mock_server().await;
    let apps = AppsTestServer::mount_hosted_plugin_runtime_searchable(&server).await?;
    let runtime = apps_runtime(&fixture, &apps.chatgpt_base_url).await;
    let initial_requests = server.received_requests().await.unwrap();
    let initializations = |requests: &[wiremock::Request]| {
        requests
            .iter()
            .filter(|request| {
                serde_json::from_slice::<Value>(&request.body)
                    .ok()
                    .is_some_and(|body| body["method"] == "initialize")
            })
            .count()
    };
    assert_eq!(initializations(&initial_requests), 1);
    let result = runtime
        .latest_call_tool(
            CODEX_APPS_MCP_SERVER_NAME,
            "calendar_create_event",
            /*environment_id*/ None,
            Some(json!({"title": "Lunch", "starts_at": "2026-06-18T12:00:00Z"})),
            /*meta*/ None,
            /*requested_timeout*/ None,
            /*wait_for_server*/ true,
        )
        .await?;
    assert_eq!(result.is_error, Some(false));
    // Even without a source replacement, sparse retained headers are not an
    // operation boundary. The fresh transport receives the strict adapter.
    assert_eq!(
        initializations(&server.received_requests().await.unwrap()),
        2
    );
    runtime.shutdown().await;
    fixture.runtime.shutdown().await;
    Ok(())
}

struct FailingAuth {
    auth: AntexAuth,
    fail: AtomicBool,
}

impl ExternalAuth for FailingAuth {
    fn resolve(&self) -> ExternalAuthFuture<'_, AntexAuth> {
        Box::pin(async {
            if self.fail.load(Ordering::SeqCst) {
                Err(std::io::Error::other("source unavailable"))
            } else {
                Ok(self.auth.clone())
            }
        })
    }

    fn refresh(&self, _: ExternalAuthRefreshContext) -> ExternalAuthFuture<'_, AntexAuth> {
        self.resolve()
    }
}

#[tokio::test]
async fn new_call_and_event_connection_do_not_dispatch_when_current_source_fails() -> Result<()> {
    let mut fixture = Fixture::new().await?;
    let server = start_mock_server().await;
    let apps = AppsTestServer::mount_hosted_plugin_runtime_searchable(&server).await?;
    let runtime = apps_runtime(&fixture, &apps.chatgpt_base_url).await;
    let opener = McpResourceClient::new(Arc::clone(&fixture.runtime)).event_stream_opener()?;
    let before = server
        .received_requests()
        .await
        .expect("captured requests")
        .len();
    let source = Arc::new(FailingAuth {
        auth: auth("header.e30.second")?,
        fail: AtomicBool::new(false),
    });
    fixture.auth.set_external_auth(source.clone()).await?;
    source.fail.store(true, Ordering::SeqCst);
    assert!(
        runtime
            .latest_call_tool(
                CODEX_APPS_MCP_SERVER_NAME,
                "calendar_create_event",
                /*environment_id*/ None,
                /*arguments*/ None,
                /*meta*/ None,
                /*requested_timeout*/ None,
                /*wait_for_server*/ true,
            )
            .await
            .is_err()
    );
    assert!(
        opener
            .open("test.event", &json!({}), /*request_meta*/ None)
            .await
            .is_err()
    );
    assert_eq!(
        server
            .received_requests()
            .await
            .expect("captured requests")
            .len(),
        before
    );
    assert_eq!(
        fixture.opened.try_recv().err(),
        Some(tokio::sync::mpsc::error::TryRecvError::Empty)
    );
    runtime.shutdown().await;
    fixture.runtime.shutdown().await;
    Ok(())
}

struct PausedAuth {
    auth: AntexAuth,
    pause: AtomicBool,
    entered: Semaphore,
    release: Semaphore,
}

impl ExternalAuth for PausedAuth {
    fn resolve(&self) -> ExternalAuthFuture<'_, AntexAuth> {
        Box::pin(async {
            if self.pause.load(Ordering::SeqCst) {
                self.entered.add_permits(1);
                self.release
                    .acquire()
                    .await
                    .expect("release resolution")
                    .forget();
            }
            Ok(self.auth.clone())
        })
    }

    fn refresh(&self, _: ExternalAuthRefreshContext) -> ExternalAuthFuture<'_, AntexAuth> {
        self.resolve()
    }
}

#[tokio::test]
async fn source_replacement_during_new_call_acquisition_has_zero_http_dispatch() -> Result<()> {
    let fixture = Fixture::new().await?;
    let server = start_mock_server().await;
    let apps = AppsTestServer::mount_hosted_plugin_runtime_searchable(&server).await?;
    let runtime = apps_runtime(&fixture, &apps.chatgpt_base_url).await;
    let before = server
        .received_requests()
        .await
        .expect("captured requests")
        .len();
    let source = Arc::new(PausedAuth {
        auth: auth("header.e30.second")?,
        pause: AtomicBool::new(false),
        entered: Semaphore::new(/*permits*/ 0),
        release: Semaphore::new(/*permits*/ 0),
    });
    fixture.auth.set_external_auth(source.clone()).await?;
    source.pause.store(true, Ordering::SeqCst);
    let calling = tokio::spawn({
        let runtime = Arc::clone(&runtime);
        async move {
            runtime
                .latest_call_tool(
                    CODEX_APPS_MCP_SERVER_NAME,
                    "calendar_create_event",
                    /*environment_id*/ None,
                    /*arguments*/ None,
                    /*meta*/ None,
                    /*requested_timeout*/ None,
                    /*wait_for_server*/ true,
                )
                .await
        }
    });
    timeout(WAIT, source.entered.acquire()).await??.forget();
    fixture
        .auth
        .set_external_auth(Arc::new(StaticAuth(auth("header.e30.third")?)))
        .await?;
    source.release.add_permits(1);
    assert!(timeout(WAIT, calling).await??.is_err());
    assert_eq!(
        server
            .received_requests()
            .await
            .expect("captured requests")
            .len(),
        before
    );
    runtime.shutdown().await;
    fixture.runtime.shutdown().await;
    Ok(())
}

#[tokio::test]
async fn source_replacement_during_open_rejects_old_operation_but_next_open_succeeds() -> Result<()>
{
    let mut fixture = Fixture::new().await?;
    let opener = McpResourceClient::new(Arc::clone(&fixture.runtime)).event_stream_opener()?;
    let source = Arc::new(PausedAuth {
        auth: auth("header.e30.second")?,
        pause: AtomicBool::new(false),
        entered: Semaphore::new(/*permits*/ 0),
        release: Semaphore::new(/*permits*/ 0),
    });
    fixture.auth.set_external_auth(source.clone()).await?;
    source.pause.store(true, Ordering::SeqCst);
    let opening = tokio::spawn({
        let opener = opener.clone();
        async move {
            opener
                .open("test.event", &json!({}), /*request_meta*/ None)
                .await
        }
    });
    timeout(WAIT, source.entered.acquire()).await??.forget();
    fixture
        .auth
        .set_external_auth(Arc::new(StaticAuth(auth("header.e30.third")?)))
        .await?;
    source.release.add_permits(1);
    assert!(timeout(WAIT, opening).await??.is_err());
    assert_eq!(
        fixture.opened.try_recv().err(),
        Some(tokio::sync::mpsc::error::TryRecvError::Empty)
    );

    let mut stream = opener
        .open("test.event", &json!({}), /*request_meta*/ None)
        .await?;
    let remote = timeout(WAIT, fixture.opened.recv())
        .await?
        .expect("new stream");
    assert_eq!(remote.headers["authorization"], "Bearer header.e30.third");
    let notification = remote.notify("notifications/events/active");
    assert_eq!(timeout(WAIT, stream.recv()).await??, Some(notification));
    drop(stream);
    timeout(WAIT, remote.notifications.closed()).await?;
    fixture.runtime.shutdown().await;
    Ok(())
}
