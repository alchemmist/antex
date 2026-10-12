//! A new latest-call operation may open a new transport, never reauthorize an old one.

use std::sync::Arc;

use antex_config::McpServerAuth;
use antex_config::McpServerTransportConfig;
use antex_login::UnauthorizedRecovery;
use anyhow::Result;
use anyhow::bail;

use super::McpConnectionSet;
use crate::CODEX_APPS_MCP_SERVER_NAME;
use crate::elicitation::ElicitationRequestRouter;
use crate::runtime::McpPublicationGate;
use crate::runtime::McpRuntimeInput;
use crate::runtime::McpStartupPolicy;

pub(super) struct NewCallConnection {
    input: McpRuntimeInput,
    elicitation_router: ElicitationRequestRouter,
    pub(super) transport_owner: Arc<UnauthorizedRecovery>,
}

impl NewCallConnection {
    pub(super) fn capture(
        input: &McpRuntimeInput,
        elicitation_router: ElicitationRequestRouter,
    ) -> Option<Self> {
        let manager = input.auth_manager.as_ref()?;
        let auth = input.auth.as_ref()?;
        let server = input.mcp_servers.get(CODEX_APPS_MCP_SERVER_NAME)?;
        if !auth.uses_antex_backend()
            || !server.enabled()
            || !server.config().is_local_environment()
            || !matches!(server.config().auth, McpServerAuth::ChatGpt)
            || !matches!(
                &server.config().transport,
                McpServerTransportConfig::StreamableHttp {
                    bearer_token_env_var: None,
                    ..
                }
            )
        {
            return None;
        }
        let transport_owner = Arc::new(manager.unauthorized_recovery());
        let mut input = input.clone();
        input
            .mcp_servers
            .retain(|name, _| name == CODEX_APPS_MCP_SERVER_NAME);
        input.startup_policy = McpStartupPolicy::Eager;
        // This operation-private connection does not republish runtime startup status.
        input.tx_event = None;
        Some(Self {
            input,
            elicitation_router,
            transport_owner,
        })
    }
}

impl McpConnectionSet {
    /// Only the explicitly new latest-call path uses this. Frozen bindings and
    /// PreparedMcpCall retain their original clients and upload authority.
    pub(crate) async fn connections_for_new_call(
        self: &Arc<Self>,
        server: &str,
        wait_for_server: bool,
    ) -> Result<(Arc<Self>, Option<Arc<UnauthorizedRecovery>>)> {
        let Some(connection) = self
            .new_call_connection
            .as_ref()
            .filter(|_| server == CODEX_APPS_MCP_SERVER_NAME)
        else {
            return Ok((Arc::clone(self), None));
        };
        let manager = connection
            .input
            .auth_manager
            .as_ref()
            .expect("managed MCP auth");
        // Capture once, before acquisition/startup awaits; never rebind this operation.
        let owner = Arc::new(manager.unauthorized_recovery());
        // Sparse source-less host tokens have a deliberately broader retained
        // header lifetime than an operation. Reusing that provider could send
        // this operation after an ambiguous token reload while queued in RMCP.
        let needs_operation_transport = !manager.has_external_auth()
            && manager.auth_cached().as_ref().is_some_and(|auth| {
                auth.is_external_chatgpt_tokens() && auth.get_chatgpt_user_id().is_none()
            });
        if !needs_operation_transport
            && connection
                .transport_owner
                .ensure_same_header_provider_owner()
                .is_ok()
        {
            owner.ensure_same_owner()?;
            return Ok((Arc::clone(self), Some(owner)));
        }
        if !wait_for_server {
            bail!("MCP server '{server}' is not connected with current authentication");
        }
        let auth = manager.auth_for_request().await?;
        owner.ensure_same_owner()?;
        let (Some(expected), Some(current)) = (connection.input.auth.as_ref(), auth.as_ref())
        else {
            bail!("MCP tool call authentication is unavailable");
        };
        if expected.api_auth_mode() != current.api_auth_mode()
            || !current.uses_antex_backend()
            || expected
                .get_account_id()
                .is_none_or(|account| account.trim().is_empty())
            || expected.get_account_id() != current.get_account_id()
            || expected.get_chatgpt_user_id() != current.get_chatgpt_user_id()
            || expected.is_workspace_account() != current.is_workspace_account()
            || expected.is_fedramp_account() != current.is_fedramp_account()
        {
            bail!("MCP tool call account changed");
        }
        let provider = crate::operation_auth::auth_for_operation(
            Arc::clone(manager),
            current,
            Arc::clone(&owner),
        );
        owner.ensure_same_owner()?;
        let mut input = connection.input.clone();
        input.auth = auth;
        // Do not mutate or shut down the old connection: retained/in-flight calls
        // must continue to fail its old source-bound provider, not inherit this one.
        let connections = Arc::new(
            Self::new_with_call_auth(
                /*previous*/ None,
                McpPublicationGate::already_published(),
                input,
                connection.elicitation_router.clone(),
                Some(provider),
            )
            .await,
        );
        owner.ensure_same_owner()?;
        Ok((connections, Some(owner)))
    }
}
