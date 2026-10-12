//! Request resolution is fallible: an installed source failing is not absence.
//! Keep the pre-await source/epoch binding through the final locked snapshot.
use super::*;

#[cfg(test)]
#[path = "request_auth_diagnostic_tests.rs"]
mod diagnostic_tests;

impl AuthManager {
    /// Resolve request credentials without silently using a failed external source's cache.
    pub async fn auth_for_request(&self) -> Result<Option<AntexAuth>, RefreshTokenError> {
        self.auth_for_request_with_http_client_factory()
            .await
            .map(|snapshot| snapshot.map(|(auth, _)| auth))
    }

    /// Resolve credentials and capture their account-bound factory under the same
    /// cache lock. A replacement during resolution cannot supply either half.
    pub async fn auth_for_request_with_http_client_factory(
        &self,
    ) -> Result<Option<(AntexAuth, HttpClientFactory)>, RefreshTokenError> {
        let (owner, previous) = recovery::RecoveryOwner::capture_with_auth(self);
        if owner.external_source().is_some() {
            self.reload_external(&owner, previous.as_ref()).await?;
        } else if let Some(auth) = previous.as_ref()
            && Self::should_refresh_proactively(auth)
        {
            match self.refresh_token_for_owner(&owner, Some(auth)).await {
                Ok(ReloadOutcome::ReloadedChanged | ReloadOutcome::ReloadedNoChange) => {}
                Ok(ReloadOutcome::Skipped) if matches!(auth, AntexAuth::Chatgpt(_)) => {
                    // Legacy storage is not an active credential resolver. A
                    // missing/different disk grant does not revoke cached A;
                    // only an explicit reload adopts it. No authority exchange
                    // was attempted, and no loaded credential was published.
                    // Final locked checks below still reject cache/source ABA,
                    // policy restrictions and recorded authority failures.
                }
                Ok(ReloadOutcome::Skipped) => return Err(proactive::skipped_refresh_error()),
                Err(RefreshTokenError::Transient(error))
                    if matches!(auth, AntexAuth::Chatgpt(_))
                        && error.kind() != std::io::ErrorKind::PermissionDenied =>
                {
                    tracing::warn!(
                        "Temporary proactive refresh failure; retaining same-owner legacy credentials"
                    );
                }
                Err(error) => return Err(error),
            }
        }
        let cached = self.inner.read().map_err(|_| recovery::owner_changed())?;
        owner.check_locked(
            self,
            cached.auth.as_ref(),
            self.auth_change_state_tx.borrow().owner_generation,
        )?;
        if let Some(auth) = cached.auth.as_ref() {
            validate_auth_restrictions(
                Some(&self.allowed_login_methods()),
                self.effective_chatgpt_workspaces().as_deref(),
                auth,
            )
            .map_err(|_| {
                std::io::Error::new(
                    std::io::ErrorKind::PermissionDenied,
                    "Request authentication is restricted by policy",
                )
            })?;
            if let Some(failure) = cached.permanent_refresh_failure.as_ref()
                && Self::auths_equal_for_refresh(Some(auth), Some(&failure.auth))
            {
                return Err(RefreshTokenError::Permanent(failure.error.clone()));
            }
        }
        Ok(cached
            .auth
            .clone()
            .map(|auth| (auth, self.http_client_factory())))
    }

    /// Legacy best-effort account/status lookup, not request authority.
    ///
    /// Preserves the last working cached account when active resolution fails.
    /// Never use this compatibility API to acquire request credentials, export
    /// tokens, or decide authorization; use `auth_for_request` instead.
    #[instrument(level = "trace", skip_all)]
    pub async fn auth(&self) -> Option<AntexAuth> {
        match self.auth_for_request().await {
            Ok(auth) => auth,
            Err(_) => self.auth_cached(),
        }
    }

    /// Fail-closed optional request pair, unlike the metadata-only `auth()`.
    /// Prefer the fallible counterpart to distinguish resolution failure from absence.
    pub async fn auth_with_http_client_factory(&self) -> Option<(AntexAuth, HttpClientFactory)> {
        self.auth_for_request_with_http_client_factory()
            .await
            .ok()
            .flatten()
    }
}
