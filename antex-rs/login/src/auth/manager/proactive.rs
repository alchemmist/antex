//! Proactive reload is bound before semaphore wait and before source loading.
//! Unrelated file changes still require explicit reload; this only conditions
//! publication on the original in-memory owner remaining current.
use super::*;

pub(super) fn skipped_refresh_error() -> RefreshTokenError {
    RefreshTokenError::Permanent(RefreshTokenFailedError::new(
        RefreshTokenFailedReason::Other,
        REFRESH_TOKEN_ACCOUNT_MISMATCH_MESSAGE.to_string(),
    ))
}

impl AuthManager {
    /// Reload the original owner's credentials, refreshing at the authority only
    /// when that guarded reload is unchanged.
    pub async fn refresh_token(&self) -> Result<(), RefreshTokenError> {
        let (owner, previous) = recovery::RecoveryOwner::capture_with_auth(self);
        match self
            .refresh_token_for_owner(&owner, previous.as_ref())
            .await?
        {
            ReloadOutcome::ReloadedChanged | ReloadOutcome::ReloadedNoChange => Ok(()),
            ReloadOutcome::Skipped => Err(skipped_refresh_error()),
        }
    }

    pub(super) async fn refresh_token_for_owner(
        &self,
        owner: &recovery::RecoveryOwner,
        previous: Option<&AntexAuth>,
    ) -> Result<ReloadOutcome, RefreshTokenError> {
        let _refresh_guard = self
            .refresh_lock
            .acquire()
            .await
            .map_err(|_| recovery::owner_changed())?;
        owner.check(self)?;
        if previous
            .is_some_and(|auth| auth.is_api_key_auth() || auth.is_personal_access_token_auth())
        {
            return Ok(ReloadOutcome::ReloadedNoChange);
        }
        let outcome = self.reload_matching_owner(owner, previous).await?;
        if matches!(outcome, ReloadOutcome::ReloadedNoChange) {
            owner.check(self)?;
            self.refresh_token_from_authority_impl().await?;
        }
        // A skipped local reload is not a failed authority exchange. Keep that
        // distinction typed so request acquisition can preserve legacy cached
        // grants without swallowing permanent refresh or policy failures.
        Ok(outcome)
    }

    pub(super) async fn reload_matching_owner(
        &self,
        owner: &recovery::RecoveryOwner,
        previous: Option<&AntexAuth>,
    ) -> Result<ReloadOutcome, RefreshTokenError> {
        owner.check(self)?;
        if owner.external_source().is_some() {
            self.reload_external(owner, previous).await?;
            owner.check(self)?;
            return Ok(
                if Self::auths_equal_for_refresh(previous, self.auth_cached().as_ref()) {
                    ReloadOutcome::ReloadedNoChange
                } else {
                    ReloadOutcome::ReloadedChanged
                },
            );
        }
        let expected_account = previous.and_then(AntexAuth::get_account_id);
        let direct = previous.is_some_and(|auth| matches!(auth, AntexAuth::ChatgptDirect(_)));
        if !direct && expected_account.is_none() {
            return Ok(ReloadOutcome::Skipped);
        }
        let loaded = self.load_auth().await;
        // Test-only barrier observes an actual loaded document, not a substitute
        // loader or publication implementation. Explicit reload is not paused.
        #[cfg(test)]
        let pause = self.proactive_load_pause.lock().unwrap().take();
        #[cfg(test)]
        if let Some(pause) = pause {
            pause.wait().await;
        }
        let matches = if direct {
            loaded
                .as_ref()
                .is_some_and(|new| matches!(new, AntexAuth::ChatgptDirect(_)))
                && previous
                    .and_then(AntexAuth::get_current_auth_json)
                    .as_ref()
                    .zip(
                        loaded
                            .as_ref()
                            .and_then(AntexAuth::get_current_auth_json)
                            .as_ref(),
                    )
                    .is_some_and(|(before, after)| refresh::same_grant_owner(before, after))
        } else {
            loaded.as_ref().and_then(AntexAuth::get_account_id) == expected_account
                && owner.matches_auth(loaded.as_ref())
        };
        if !matches {
            return Ok(ReloadOutcome::Skipped);
        }
        let changed = !Self::auths_equal_for_refresh(previous, loaded.as_ref());
        self.set_cached_auth_with_owner(loaded, Some(owner))?;
        Ok(if changed {
            ReloadOutcome::ReloadedChanged
        } else {
            ReloadOutcome::ReloadedNoChange
        })
    }
}
