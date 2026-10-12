//! Request headers for a newly bound MCP operation, not a reusable auth lifetime.

use std::sync::Arc;

use antex_api::AuthError;
use antex_api::AuthHeadersFuture;
use antex_api::AuthProvider;
use antex_api::SharedAuthProvider;
use antex_login::AntexAuth;
use antex_login::AuthManager;
use antex_login::UnauthorizedRecovery;
use http::HeaderMap;

struct OperationAuth {
    provider: SharedAuthProvider,
    owner: Arc<UnauthorizedRecovery>,
}

pub(crate) fn auth_for_operation(
    manager: Arc<AuthManager>,
    expected: &AntexAuth,
    owner: Arc<UnauthorizedRecovery>,
) -> SharedAuthProvider {
    Arc::new(OperationAuth {
        provider: antex_model_provider::auth_provider_from_auth_manager(manager, expected),
        owner,
    })
}

impl AuthProvider for OperationAuth {
    fn add_auth_headers(&self, headers: &mut HeaderMap) {
        if self.owner.ensure_same_owner().is_ok() {
            let resolved = self.provider.to_auth_headers();
            if self.owner.ensure_same_owner().is_ok() {
                headers.extend(resolved);
            }
        }
    }

    fn resolve_auth_headers(&self) -> AuthHeadersFuture<'_> {
        Box::pin(async move {
            self.owner
                .ensure_same_owner()
                .map_err(|error| AuthError::Build(error.to_string()))?;
            let headers = self.provider.resolve_auth_headers().await?;
            self.owner
                .ensure_same_owner()
                .map_err(|error| AuthError::Build(error.to_string()))?;
            Ok(headers)
        })
    }
}
