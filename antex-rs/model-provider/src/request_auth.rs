//! Request-only auth selection. Explicit provider credentials and local providers
//! must not make an unrelated ambient login source authoritative.
use antex_login::AntexAuth;
use antex_login::AuthManager;
use antex_login::RefreshTokenError;
use antex_model_provider_info::ModelProviderInfo;
use antex_protocol::error::CodexErr;

pub(crate) fn uses_manager(info: &ModelProviderInfo) -> bool {
    info.auth.is_some()
        || (info.requires_openai_auth
            && info.env_key.is_none()
            && info.experimental_bearer_token.is_none()
            && info.aws.is_none())
}

pub(crate) fn request_error(error: RefreshTokenError) -> CodexErr {
    match error {
        RefreshTokenError::Permanent(error) => CodexErr::RefreshTokenFailed(error),
        RefreshTokenError::Transient(error) => CodexErr::Io(error),
        RefreshTokenError::Policy(error) => CodexErr::Fatal(error.to_string()),
    }
}

pub(crate) fn attachment_error(error: RefreshTokenError) -> antex_api::AuthError {
    match error {
        RefreshTokenError::Transient(error) => antex_api::AuthError::Transient(error.to_string()),
        RefreshTokenError::Permanent(error) => antex_api::AuthError::Build(error.to_string()),
        RefreshTokenError::Policy(error) => antex_api::AuthError::Build(error.to_string()),
    }
}

pub(crate) async fn resolve(
    info: &ModelProviderInfo,
    manager: Option<&AuthManager>,
) -> antex_protocol::error::Result<Option<AntexAuth>> {
    if uses_manager(info)
        && let Some(manager) = manager
    {
        return manager.auth_for_request().await.map_err(request_error);
    }
    Ok(None)
}
