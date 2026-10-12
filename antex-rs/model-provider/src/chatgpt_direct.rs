//! Direct subscription grants are Responses-only, not Codex backend credentials.
use std::sync::Arc;

use antex_api::AuthError;
use antex_api::AuthHeadersFuture;
use antex_api::AuthProvider;
use antex_api::AuthProviderFuture;
use antex_http_client::Request;
use antex_login::AntexAuth;
use antex_login::AuthManager;
use http::HeaderMap;
use http::HeaderValue;
use http::Method;

pub(crate) fn grant_id(auth: &AntexAuth) -> Option<String> {
    if !matches!(auth, AntexAuth::ChatgptDirect(_)) {
        return None;
    }
    auth.get_token_data()
        .ok()?
        .direct
        .map(|direct| direct.client_id)
}

pub(crate) struct DirectAuthProvider {
    expected_auth: AntexAuth,
    manager: Option<Arc<AuthManager>>,
    owner: Option<antex_login::UnauthorizedRecovery>,
}

impl DirectAuthProvider {
    pub(crate) fn new(expected_auth: AntexAuth, manager: Option<Arc<AuthManager>>) -> Self {
        let owner = manager
            .as_ref()
            .map(antex_login::AuthManager::unauthorized_recovery);
        Self {
            expected_auth,
            manager,
            owner,
        }
    }
}

impl AuthProvider for DirectAuthProvider {
    // Do not expose these credentials to header-only backend/MCP/WebSocket consumers.
    fn add_auth_headers(&self, _headers: &mut HeaderMap) {}

    fn resolve_auth_headers(&self) -> AuthHeadersFuture<'_> {
        Box::pin(async {
            Err(AuthError::Build(
                "Direct ChatGPT authentication requires a public Responses request".to_string(),
            ))
        })
    }

    fn apply_auth(&self, mut request: Request) -> AuthProviderFuture<'_> {
        Box::pin(async move {
            let allowed = url::Url::parse(&request.url).ok().is_some_and(|url| {
                url.scheme() == "https"
                    && url.host_str() == Some("api.openai.com")
                    && url.port_or_known_default() == Some(443)
                    && url.username().is_empty()
                    && url.password().is_none()
                    && url.path() == "/v1/responses"
                    && url.fragment().is_none()
                    && url.query().is_none()
            });
            if !allowed || request.method != Method::POST {
                return Err(AuthError::Build(
                    "Direct ChatGPT credentials may only be used with the public Responses API"
                        .to_string(),
                ));
            }
            if let Some(owner) = &self.owner {
                owner
                    .ensure_same_owner()
                    .map_err(|error| AuthError::Build(error.to_string()))?;
            }
            let auth = match &self.manager {
                Some(manager) => manager
                    .auth_for_request()
                    .await
                    .map_err(crate::request_auth::attachment_error)?,
                None => Some(self.expected_auth.clone()),
            }
            .filter(|auth| {
                matches!(auth, AntexAuth::ChatgptDirect(_))
                    && grant_id(auth).is_some()
                    && grant_id(auth) == grant_id(&self.expected_auth)
            })
            .ok_or_else(|| {
                AuthError::Transient("Direct ChatGPT sign-in changed or is unavailable".to_string())
            })?;
            if let Some(owner) = &self.owner {
                owner
                    .ensure_same_owner()
                    .map_err(|error| AuthError::Build(error.to_string()))?;
            }
            let token = auth.get_token().map_err(|_| {
                AuthError::Transient("Direct ChatGPT access token is unavailable".to_string())
            })?;
            let mut bearer = HeaderValue::from_str(&format!("Bearer {token}"))
                .map_err(|_| AuthError::Build("Invalid direct ChatGPT access token".to_string()))?;
            bearer.set_sensitive(true);
            request.headers.remove("chatgpt-account-id");
            request.headers.remove("x-openai-fedramp");
            request.headers.remove("x-oai-attestation");
            request.headers.insert(http::header::AUTHORIZATION, bearer);
            Ok(request)
        })
    }
}

#[cfg(test)]
#[path = "chatgpt_direct_tests.rs"]
mod tests;
