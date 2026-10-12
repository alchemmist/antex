//! Typed refresh decisions without exposing issuer bodies, URLs, or token values.
use crate::RefreshTokenError;
use crate::auth::RefreshTokenFailedError;
use crate::auth::RefreshTokenFailedReason;
use crate::oauth::OAuthError;

pub(crate) fn classify(error: OAuthError) -> RefreshTokenError {
    match error {
        OAuthError::Transport(antex_http_client::HttpError::Policy(denied)) => denied.into(),
        OAuthError::Rejected(rejection) => {
            if let Some(antex_http_client::HttpError::Policy(denied)) = rejection.body_read_error {
                return denied.into();
            }
            let code = rejection.detail.error_code().map(str::to_ascii_lowercase);
            let reason = match code.as_deref() {
                Some("refresh_token_expired") => Some(RefreshTokenFailedReason::Expired),
                Some("refresh_token_reused") => Some(RefreshTokenFailedReason::Exhausted),
                Some("refresh_token_invalidated" | "refresh_token_revoked") => {
                    Some(RefreshTokenFailedReason::Revoked)
                }
                Some("invalid_grant") => Some(RefreshTokenFailedReason::Other),
                _ => None,
            };
            if matches!(rejection.status.as_u16(), 400 | 401 | 403)
                && let Some(reason) = reason
            {
                return RefreshTokenError::Permanent(RefreshTokenFailedError::new(
                    reason,
                    "Direct ChatGPT credentials are no longer valid; reconnect ChatGPT",
                ));
            }
            transient()
        }
        OAuthError::Transport(_) | OAuthError::InvalidResponse => transient(),
    }
}

#[cfg(test)]
#[path = "direct_refresh_error_tests.rs"]
mod tests;

fn transient() -> RefreshTokenError {
    std::io::Error::other("Direct ChatGPT token refresh failed; try again").into()
}
