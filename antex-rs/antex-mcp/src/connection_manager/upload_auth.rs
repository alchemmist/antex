//! File uploads retain operation ownership, not just account-scoped headers.
use antex_api::AuthError;
use antex_api::SharedAuthProvider;
use antex_login::AntexAuth;
use antex_login::AuthManager;
use antex_login::UnauthorizedRecovery;
use std::sync::Arc;

pub(crate) struct FileUploadAuth {
    pub(crate) provider: SharedAuthProvider,
    owner: Result<Option<UnauthorizedRecovery>, &'static str>,
}

impl FileUploadAuth {
    pub(super) fn capture(auth: &AntexAuth, manager: Option<&Arc<AuthManager>>) -> Self {
        let Some(manager) = manager else {
            return Self {
                provider: antex_model_provider::auth_provider_from_auth(auth),
                owner: Ok(None),
            };
        };
        // Capture before startup awaits, then verify the startup anchor under
        // that lifetime. Never bind a replacement first seen after approval.
        let owner = manager.unauthorized_recovery();
        let matches_startup = manager.auth_cached().is_some_and(|current| {
            current.api_auth_mode() == auth.api_auth_mode()
                && current.get_account_id() == auth.get_account_id()
                && current.get_chatgpt_user_id() == auth.get_chatgpt_user_id()
                && current.is_workspace_account() == auth.is_workspace_account()
                && current.is_fedramp_account() == auth.is_fedramp_account()
                && current.get_token().ok() == auth.get_token().ok()
        });
        let owner = if matches_startup && owner.ensure_same_owner().is_ok() {
            Ok(Some(owner))
        } else {
            Err("file upload authentication no longer matches the MCP startup owner")
        };
        Self {
            provider: antex_model_provider::auth_provider_from_auth_manager(
                Arc::clone(manager),
                auth,
            ),
            owner,
        }
    }

    pub(crate) fn ensure_same_owner(&self) -> Result<(), AuthError> {
        match &self.owner {
            Ok(Some(owner)) => owner
                .ensure_same_owner()
                .map_err(|error| AuthError::Build(error.to_string())),
            Ok(None) => Ok(()),
            Err(message) => Err(AuthError::Build((*message).to_string())),
        }
    }
}
