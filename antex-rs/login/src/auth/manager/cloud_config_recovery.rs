//! Compatibility for the fixed, payload-free cloud-config GET only. Never use
//! this recovery for sampling, compaction, or other caller-supplied payloads.
use super::*;

/// Bounded cloud-config recovery. Only its first managed-legacy storage reload
/// may rebind a known user within the original known workspace. This is not a
/// general request-owner guard and cannot be converted into one.
pub struct CloudConfigRecovery {
    recovery: UnauthorizedRecovery,
}

impl CloudConfigRecovery {
    pub fn new(manager: Arc<AuthManager>, auth: &AntexAuth) -> Result<Self, RefreshTokenError> {
        let recovery = UnauthorizedRecovery::new(manager);
        if !recovery.owner.matches_auth(Some(auth)) {
            return Err(account_mismatch());
        }
        recovery.ensure_same_owner()?;
        Ok(Self { recovery })
    }

    pub fn ensure_same_owner(&self) -> Result<(), RefreshTokenError> {
        self.recovery.ensure_same_owner()
    }

    pub fn has_next(&self) -> bool {
        self.recovery.has_next()
    }

    /// Consume the existing recovery budget and return a guarded auth snapshot,
    /// without a second proactive reload that could select a different owner.
    pub async fn next(&mut self) -> Result<Option<AntexAuth>, RefreshTokenError> {
        self.ensure_same_owner()?;
        if matches!(self.recovery.step, UnauthorizedRecoveryStep::Reload)
            && self.recovery.owner.mode == Some(AuthMode::Chatgpt)
            && self.recovery.owner.external.is_none()
        {
            self.reload_legacy_metadata().await?;
        } else {
            self.recovery.next().await?;
        }
        let manager = &self.recovery.manager;
        let cached = manager.inner.read().map_err(|_| owner_changed())?;
        self.recovery.owner.check_locked(
            manager,
            cached.auth.as_ref(),
            manager.auth_change_state_tx.borrow().owner_generation,
        )?;
        Ok(cached.auth.clone())
    }

    async fn reload_legacy_metadata(&mut self) -> Result<(), RefreshTokenError> {
        let manager = &self.recovery.manager;
        let owner = &mut self.recovery.owner;
        owner.check(manager)?;
        let Some(AntexAuth::Chatgpt(mut auth)) = manager.auth_cached() else {
            return Err(account_mismatch());
        };
        // Read only the original managed backend. Do not consult environment,
        // ephemeral overrides, or a newly installed external provider.
        let stored = auth.storage().load()?.ok_or_else(account_mismatch)?;
        #[cfg(test)]
        let pause = manager.proactive_load_pause.lock().unwrap().take();
        #[cfg(test)]
        if let Some(pause) = pause {
            pause.wait().await;
        }
        if stored.resolved_mode() != AuthMode::Chatgpt
            || stored
                .tokens
                .as_ref()
                .is_none_or(|tokens| tokens.direct.is_some())
        {
            return Err(account_mismatch());
        }
        let original_claim_account = auth
            .current_token_data()
            .and_then(|tokens| tokens.id_token.chatgpt_account_id);
        let loaded_claim_account = stored
            .tokens
            .as_ref()
            .and_then(|tokens| tokens.id_token.chatgpt_account_id.clone());
        auth.state.auth_dot_json = Arc::new(Mutex::new(Some(stored)));
        let loaded = AntexAuth::Chatgpt(auth);
        let known = |value: Option<&str>| value.is_some_and(|value| !value.trim().is_empty());
        if !known(owner.account.as_deref())
            || !known(owner.user.as_deref())
            || !known(loaded.get_chatgpt_user_id().as_deref())
            || owner.account != loaded.get_account_id()
            || owner.registration != registration(&loaded)
            || original_claim_account != loaded_claim_account
        {
            return Err(account_mismatch());
        }
        validate_auth_restrictions(
            Some(&manager.allowed_login_methods()),
            manager.effective_chatgpt_workspaces().as_deref(),
            &loaded,
        )
        .map_err(|_| account_mismatch())?;

        // R2: A loaded before a wait cannot overwrite a newer cached B. Check
        // under the write lock BEFORE publishing. Rebind only this metadata
        // guard in the same critical section; generic guards retain old owner.
        let mut cached = manager.inner.write().map_err(|_| owner_changed())?;
        owner.check_locked(
            manager,
            cached.auth.as_ref(),
            manager.auth_change_state_tx.borrow().owner_generation,
        )?;
        let user = loaded.get_chatgpt_user_id();
        manager.publish_cached_auth(&mut cached, Some(loaded));
        owner.user = user;
        owner.generation = manager.auth_change_state_tx.borrow().owner_generation;
        self.recovery.step = UnauthorizedRecoveryStep::RefreshToken;
        Ok(())
    }
}

fn account_mismatch() -> RefreshTokenError {
    RefreshTokenError::Permanent(RefreshTokenFailedError::new(
        RefreshTokenFailedReason::Other,
        REFRESH_TOKEN_ACCOUNT_MISMATCH_MESSAGE,
    ))
}

#[cfg(test)]
#[path = "cloud_config_recovery_tests.rs"]
mod tests;
