//! Direct subscription refresh stays separate from the legacy Codex token authority.
use super::AntexAuth;
use super::AuthManager;
use super::ChatgptAuth;
use super::RefreshTokenError;
use super::validate_auth_restrictions;
use chrono::Utc;

impl AuthManager {
    pub(super) async fn refresh_direct(&self, auth: &ChatgptAuth) -> Result<(), RefreshTokenError> {
        let allowed = self.allowed_login_methods();
        validate_auth_restrictions(
            Some(&allowed),
            self.effective_chatgpt_workspaces().as_deref(),
            &AntexAuth::ChatgptDirect(auth.clone()),
        )
        .map_err(|_| {
            std::io::Error::new(
                std::io::ErrorKind::PermissionDenied,
                "Direct ChatGPT authentication is restricted by policy",
            )
        })?;
        let client = auth.client().clone();
        self.refresh_managed_with(
            &AntexAuth::ChatgptDirect(auth.clone()),
            move |original| async move {
                let tokens = original
                    .tokens
                    .as_ref()
                    .ok_or_else(|| std::io::Error::other("Missing direct credentials"))?;
                let refreshed = crate::direct_oauth::refresh(&client, tokens).await?;
                refreshed_document(original, refreshed).map_err(Into::into)
            },
        )
        .await
    }
}

pub(super) fn refreshed_document(
    original: super::AuthDotJson,
    refreshed: crate::TokenData,
) -> std::io::Result<super::AuthDotJson> {
    crate::direct_oauth::validate_tokens(&refreshed)?;
    let tokens = original
        .tokens
        .as_ref()
        .ok_or_else(|| std::io::Error::other("Missing tokens"))?;
    if refreshed.account_id != tokens.account_id
        || refreshed.id_token.chatgpt_user_id != tokens.id_token.chatgpt_user_id
        || refreshed.direct.as_ref().map(|d| &d.client_id)
            != tokens.direct.as_ref().map(|d| &d.client_id)
    {
        return Err(std::io::Error::other(
            "Direct ChatGPT refresh changed account identity",
        ));
    }
    let mut updated = original.clone();
    updated.tokens = Some(refreshed);
    updated.last_refresh = Some(Utc::now());
    Ok(updated)
}

#[cfg(test)]
#[path = "direct_tests.rs"]
mod tests;
