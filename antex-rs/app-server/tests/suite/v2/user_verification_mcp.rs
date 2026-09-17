//! Exercises the MCP-to-core ceremony without depending on biometric hardware.

use std::sync::Arc;
use std::time::Duration;

use antex_config::Constrained;
use antex_core::StartThreadOptions;
use antex_core::config::Config;
use antex_extension_api::ExtensionFuture;
use antex_extension_api::ExtensionRegistryBuilder;
use antex_extension_api::McpServerContribution;
use antex_extension_api::McpServerContributionContext;
use antex_extension_api::McpServerContributor;
use antex_mcp::CODEX_APPS_MCP_SERVER_NAME;
use antex_protocol::approvals::ElicitationRequest;
use antex_protocol::mcp::ClientMcpExtensions;
use antex_protocol::mcp::OPENAI_ELICITATION_EXTENSION_ID;
use antex_protocol::models::PermissionProfile;
use antex_protocol::protocol::AskForApproval;
use antex_protocol::protocol::ElicitationAction;
use antex_protocol::protocol::EventMsg;
use antex_protocol::protocol::Op;
use anyhow::Result;
use axum::Router;
use core_test_support::apps_test_server::apps_enabled_builder;
use core_test_support::responses::start_mock_server;
use core_test_support::wait_for_event;
use pretty_assertions::assert_eq;
use rmcp::ServerHandler;
use rmcp::model::CallToolRequestParams;
use rmcp::model::CallToolResponse;
use rmcp::model::CallToolResult;
use rmcp::model::ContentBlock;
use rmcp::model::CustomRequest;
use rmcp::model::ServerCapabilities;
use rmcp::model::ServerInfo;
use rmcp::model::ServerRequest;
use rmcp::service::RequestContext;
use rmcp::service::RoleServer;
use rmcp::transport::StreamableHttpServerConfig;
use rmcp::transport::StreamableHttpService;
use rmcp::transport::streamable_http_server::session::local::LocalSessionManager;
use serde_json::Value;
use serde_json::json;
use tokio::net::TcpListener;
use tokio::sync::Mutex;
use tokio::time::timeout;
use tokio_util::task::AbortOnDropHandle;

#[derive(Clone)]
struct VerificationServer {
    response: Arc<Mutex<Option<Value>>>,
}

struct HostedVerificationServer(antex_config::McpServerConfig);

impl McpServerContributor<Config> for HostedVerificationServer {
    fn id(&self) -> &'static str {
        "user-verification-integration-test"
    }

    fn contribute<'a>(
        &'a self,
        _context: McpServerContributionContext<'a, Config>,
    ) -> ExtensionFuture<'a, Vec<McpServerContribution>> {
        Box::pin(async move {
            vec![McpServerContribution::HostedApps {
                config: Box::new(self.0.clone()),
            }]
        })
    }
}

impl ServerHandler for VerificationServer {
    fn get_info(&self) -> ServerInfo {
        ServerInfo::new(ServerCapabilities::builder().enable_tools().build())
    }

    async fn call_tool(
        &self,
        _request: CallToolRequestParams,
        context: RequestContext<RoleServer>,
    ) -> Result<CallToolResponse, rmcp::ErrorData> {
        assert_eq!(
            context
                .client_capabilities()
                .and_then(|capabilities| capabilities.extensions)
                .and_then(|extensions| extensions.get(OPENAI_ELICITATION_EXTENSION_ID).cloned())
                .map(Value::Object),
            Some(json!({"userVerification": {}})),
        );
        let result = context
            .peer
            .send_request(ServerRequest::CustomRequest(CustomRequest::new(
                "openai/elicitation/create",
                Some(json!({
                    "mode": "openai/userVerification",
                    "title": "Approve purchase",
                    "description": "Pay $200 to Example Store",
                    "challenge": "AAECA_7_",
                })),
            )))
            .await
            .map_err(|error| {
                rmcp::ErrorData::internal_error(error.to_string(), /*data*/ None)
            })?;
        *self.response.lock().await = Some(serde_json::to_value(result).map_err(|error| {
            rmcp::ErrorData::internal_error(error.to_string(), /*data*/ None)
        })?);
        Ok(CallToolResult::success(vec![ContentBlock::text("verified")]).into())
    }
}

#[tokio::test(flavor = "multi_thread", worker_threads = 4)]
async fn user_verification_mcp_round_trip_requires_proof_in_full_access() -> Result<()> {
    let received = Arc::new(Mutex::new(None));
    let server = VerificationServer {
        response: Arc::clone(&received),
    };
    let service = StreamableHttpService::new(
        move || Ok(server.clone()),
        Arc::new(LocalSessionManager::default()),
        StreamableHttpServerConfig::default(),
    );
    let listener = TcpListener::bind("127.0.0.1:0").await?;
    let base_url = format!("http://{}", listener.local_addr()?);
    let router = Router::new().nest_service("/api/codex/ps/mcp", service);
    let _server = AbortOnDropHandle::new(tokio::spawn(async move {
        axum::serve(listener, router).await.unwrap();
    }));
    let model_server = start_mock_server().await;
    let mut extensions = ExtensionRegistryBuilder::new();
    let plugin_service_config = serde_json::from_value(json!({
        "url": format!("{base_url}/api/codex/ps/mcp"),
    }))?;
    extensions.mcp_server_contributor(Arc::new(HostedVerificationServer(plugin_service_config)));
    let test = apps_enabled_builder(base_url)
        .with_extensions(Arc::new(extensions.build()))
        .with_config(|config| {
            config.permissions.approval_policy = Constrained::allow_any(AskForApproval::Never);
            config
                .permissions
                .set_permission_profile(PermissionProfile::Disabled)
                .unwrap();
        })
        .build_with_auto_env(&model_server)
        .await?;
    // Supply the trusted host capability through the public core API. The production
    // app-server only activates it for supported in-process UIs with biometric hardware.
    let thread = test
        .thread_manager
        .start_thread(StartThreadOptions {
            client_mcp_extensions: ClientMcpExtensions::new([(
                OPENAI_ELICITATION_EXTENSION_ID.to_string(),
                json!({"userVerification": {}}),
            )]),
            environments: Some(vec![test.executor_environment().selection().clone()]),
            ..StartThreadOptions::new(test.config.clone())
        })
        .await?
        .thread;
    let caller = Arc::clone(&thread);
    let call = AbortOnDropHandle::new(tokio::spawn(async move {
        caller
            .call_mcp_tool(
                CODEX_APPS_MCP_SERVER_NAME,
                "verify_action",
                /*arguments*/ None,
                /*meta*/ None,
            )
            .await
    }));
    let EventMsg::ElicitationRequest(request) = wait_for_event(&thread, |event| {
        matches!(event, EventMsg::ElicitationRequest(_))
    })
    .await
    else {
        unreachable!()
    };
    assert_eq!(request.server_name, CODEX_APPS_MCP_SERVER_NAME);
    assert_eq!(
        request.request,
        ElicitationRequest::UserVerification {
            title: "Approve purchase".into(),
            description: "Pay $200 to Example Store".into(),
            challenge: "AAECA_7_".into(),
        }
    );
    assert!(received.lock().await.is_none());
    assert!(!call.is_finished());
    let proof = json!({"credentialId": "AQID", "signature": "BAUG"});
    thread
        .submit(Op::ResolveElicitation {
            server_name: request.server_name,
            request_id: request.id,
            decision: ElicitationAction::Accept,
            content: Some(proof.clone()),
            meta: Some(json!({"untrusted": "discarded"})),
        })
        .await?;
    assert_eq!(
        timeout(Duration::from_secs(/*secs*/ 10), call).await???,
        antex_protocol::mcp::CallToolResult {
            content: vec![json!({"type": "text", "text": "verified"})],
            structured_content: None,
            is_error: Some(false),
            meta: None,
        },
    );
    assert_eq!(
        *received.lock().await,
        Some(json!({"action": "accept", "content": proof})),
    );
    thread.shutdown_and_wait().await?;
    test.antex.shutdown_and_wait().await?;
    Ok(())
}
