//! Local analytics authentication, server plan discovery, and account/user identity checks.
//! Each new session fetches its plan once; token plan claims do not select reports.

use crate::Client;
use crate::RequestError;
use antex_http_client::HttpClientFactory;
use antex_login::AntexAuth;
use antex_login::AuthManager;
use antex_login::AuthManagerConfig;
use antex_protocol::account::PlanType;
use std::sync::Arc;

/// Non-secret account metadata associated with an analytics session.
#[derive(Clone, Debug)]
pub struct AnalyticsAccount {
    pub id: String,
    pub email: Option<String>,
    pub plan_type: Option<PlanType>,
}

/// A backend client bound to its initial ChatGPT account and user, with a server plan snapshot.
pub struct AnalyticsSession {
    client: Client,
    auth_manager: Arc<AuthManager>,
    auth: AntexAuth,
    account: AnalyticsAccount,
}

impl AnalyticsSession {
    /// Load local ChatGPT credentials and the current server plan using the configured HTTP policy.
    pub async fn from_config(
        config: &impl AuthManagerConfig,
        http_client_factory: HttpClientFactory,
    ) -> Result<Self, String> {
        let auth_manager =
            AuthManager::shared_from_config(config, /*enable_codex_api_key_env*/ false)
                .await
                .map_err(|_| {
                    "Couldn't load local sign-in. Sign in with ChatGPT and retry.".to_string()
                })?;
        let auth = auth_manager
            .auth_for_request()
            .await
            .map_err(|error| error.to_string())?
            .filter(|auth| auth.uses_antex_backend() && auth.is_chatgpt_auth())
            .ok_or("Sign in locally with ChatGPT to view Analytics.")?;
        let (Some(id), Some(_)) = (auth.get_account_id(), auth.get_chatgpt_user_id()) else {
            return Err("Analytics requires a ChatGPT account and user identity.".into());
        };
        let account = AnalyticsAccount {
            id,
            email: auth.get_account_email(),
            plan_type: None,
        };
        let client = Client::new_without_redirects(config.chatgpt_base_url(), http_client_factory)
            .with_auth_provider(antex_model_provider::auth_provider_from_auth_manager(
                Arc::clone(&auth_manager),
                &auth,
            ));
        let mut session = Self {
            client,
            auth_manager,
            auth,
            account,
        };
        let accounts = session
            .request(|client| async move { client.get_accounts_check().await })
            .await;
        // Preserve account-switch guidance even when it interrupts account discovery.
        session.ensure_identity().await?;
        let accounts = accounts.map_err(|error| {
            if error.is_unauthorized() {
                "Sign in again to load Analytics."
            } else {
                "Couldn't load account plan. Press R to retry Analytics."
            }
        })?;
        session.account.plan_type = Some(
            accounts
                .accounts
                .into_iter()
                .find(|account| account.id == session.account.id)
                .and_then(|account| account.plan_type)
                .ok_or("Couldn't load account plan. Press R to retry Analytics.")?,
        );
        Ok(session)
    }

    /// Return the account metadata captured when this session was opened.
    pub fn account(&self) -> &AnalyticsAccount {
        &self.account
    }
    /// Reject responses or cached data after a local account or user change.
    pub async fn ensure_identity(&self) -> Result<(), String> {
        self.auth_manager.reload().await;
        // This checks identity metadata, not permission to send a request.
        // A failed refresh can leave the same identity signed in but unusable.
        let current = self.auth_manager.auth_cached();
        if current.is_none_or(|auth| {
            !auth.uses_antex_backend()
                || auth.get_account_id() != self.auth.get_account_id()
                || auth.get_chatgpt_user_id() != self.auth.get_chatgpt_user_id()
        }) {
            return Err("Account changed. Press R to refresh Analytics.".into());
        }
        Ok(())
    }

    /// Run an account-scoped request with bounded unauthorized recovery.
    pub async fn request<T, F>(&self, request: impl Fn(Client) -> F) -> Result<T, RequestError>
    where
        F: std::future::Future<Output = Result<T, RequestError>>,
    {
        let mut recovery = self.auth_manager.unauthorized_recovery();
        let mut unauthorized = None;
        loop {
            let ready = async {
                recovery
                    .ensure_same_owner()
                    .map_err(|error| error.to_string())?;
                self.ensure_identity().await?;
                self.auth_manager
                    .auth_for_request()
                    .await
                    .map_err(|error| error.to_string())?
                    .ok_or_else(|| "Sign in again to load Analytics.".to_string())?;
                recovery
                    .ensure_same_owner()
                    .map_err(|error| error.to_string())
            }
            .await;
            if let Err(error) = ready {
                // Only an observed HTTP 401 supplies a status. Initial auth
                // failures must remain status-less and must not dispatch.
                return Err(
                    unauthorized.unwrap_or_else(|| RequestError::Other(anyhow::anyhow!(error)))
                );
            }
            let result = request(self.client.clone()).await;
            if result.as_ref().is_err_and(RequestError::is_unauthorized) {
                if recovery.has_next() && recovery.next().await.is_ok() {
                    unauthorized = result.err();
                    continue;
                }
                return result;
            }
            let still_owned = async {
                self.ensure_identity().await?;
                recovery
                    .ensure_same_owner()
                    .map_err(|error| error.to_string())
            }
            .await;
            if let Err(error) = still_owned {
                return Err(
                    unauthorized.unwrap_or_else(|| RequestError::Other(anyhow::anyhow!(error)))
                );
            }
            return result;
        }
    }
}

#[cfg(test)]
#[path = "analytics_session_request_auth_tests.rs"]
mod request_auth_tests;

#[cfg(test)]
#[path = "analytics_session_tests.rs"]
mod tests;
