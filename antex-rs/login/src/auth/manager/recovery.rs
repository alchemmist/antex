//! Bind recovery to the failed request's owner, not whoever is signed in when
//! a 401 eventually arrives. The public guard also protects outer retry loops.
use super::*;

#[path = "cloud_config_recovery.rs"]
pub(super) mod cloud_config;

pub(super) struct RecoveryOwner {
    generation: u64,
    host_token_owner_generation: u64,
    mode: Option<AuthMode>,
    registration: Option<String>,
    account: Option<String>,
    user: Option<String>,
    external: Option<Arc<dyn ExternalAuth>>,
    external_source_epoch: u64,
}

impl RecoveryOwner {
    pub(super) fn capture(manager: &AuthManager) -> Self {
        Self::capture_with_auth(manager).0
    }

    pub(super) fn capture_with_auth(manager: &AuthManager) -> (Self, Option<AntexAuth>) {
        // Source changes and cache revisions are published under this lock.
        let cached = manager
            .inner
            .read()
            .unwrap_or_else(std::sync::PoisonError::into_inner);
        let auth = cached.auth.as_ref();
        let owner = Self {
            generation: manager.auth_change_state_tx.borrow().owner_generation,
            host_token_owner_generation: cached.host_token_owner_generation,
            mode: auth.map(AntexAuth::api_auth_mode),
            registration: auth.and_then(registration),
            account: auth.and_then(AntexAuth::get_account_id),
            user: auth.and_then(AntexAuth::get_chatgpt_user_id),
            external: manager.external_auth_provider(),
            external_source_epoch: manager.external_source_epoch.load(Ordering::Relaxed),
        };
        (owner, cached.auth.clone())
    }

    pub(super) fn external_source(&self) -> Option<Arc<dyn ExternalAuth>> {
        self.external.clone()
    }

    pub(super) fn accepts_external_result(&self, auth: &AntexAuth) -> bool {
        self.matches_auth(Some(auth)) || self.is_opaque_bearer(Some(auth))
    }

    fn is_opaque_bearer(&self, auth: Option<&AntexAuth>) -> bool {
        self.external.is_some()
            && matches!(self.mode, None | Some(AuthMode::ApiKey))
            && auth.is_some_and(|auth| auth.api_auth_mode() == AuthMode::ApiKey)
            && self.registration.is_none()
            && self.account.is_none()
            && self.user.is_none()
            && auth.and_then(registration).is_none()
            && auth.and_then(AntexAuth::get_account_id).is_none()
            && auth.and_then(AntexAuth::get_chatgpt_user_id).is_none()
    }

    fn is_external_chatgpt_rotation(&self, auth: Option<&AntexAuth>) -> bool {
        // The host owns external ChatGPT credential rotation. A known account
        // plus unchanged optional user identity is enough within ONE source
        // installation; do not relax managed grants, Headers, or other modes.
        self.external.is_some()
            && self.mode == Some(AuthMode::ChatgptAuthTokens)
            && self
                .account
                .as_ref()
                .is_some_and(|account| !account.trim().is_empty())
            && self.matches_auth(auth)
    }

    pub(super) fn matches_auth(&self, auth: Option<&AntexAuth>) -> bool {
        self.mode == auth.map(AntexAuth::api_auth_mode)
            && self.registration == auth.and_then(registration)
            && self.account == auth.and_then(AntexAuth::get_account_id)
            && self.user == auth.and_then(AntexAuth::get_chatgpt_user_id)
    }

    pub(super) fn check(&self, manager: &AuthManager) -> Result<(), RefreshTokenError> {
        let cached = manager.inner.read().map_err(|_| owner_changed())?;
        let generation = manager.auth_change_state_tx.borrow().owner_generation;
        self.check_locked(manager, cached.auth.as_ref(), generation)
    }

    fn check_source_locked(&self, manager: &AuthManager) -> Result<(), RefreshTokenError> {
        let current_source = manager.external_auth_provider();
        let same_source = match (&self.external, &current_source) {
            (None, None) => true,
            (Some(original), Some(current)) => Arc::ptr_eq(original, current),
            (None, Some(_)) | (Some(_), None) => false,
        };
        // Called with the cache lock held, as are explicit source/epoch writes.
        // Arc identity alone cannot distinguish clearing/reinstalling the same
        // provider from a credential rotation within its original installation.
        if !same_source
            || self.external_source_epoch != manager.external_source_epoch.load(Ordering::Relaxed)
        {
            return Err(owner_changed());
        }
        Ok(())
    }

    pub(super) fn check_locked(
        &self,
        manager: &AuthManager,
        auth: Option<&AntexAuth>,
        generation: u64,
    ) -> Result<(), RefreshTokenError> {
        self.check_source_locked(manager)?;
        // A lazy command provider starts without credentials. Only that same
        // captured source may materialize an identity-free API bearer and rotate
        // it. This does not allow None -> another mode or a new provider Arc,
        // even if that provider happens to return the identical token string.
        if self.is_opaque_bearer(auth)
            || self.is_external_chatgpt_rotation(auth)
            || (generation == self.generation && self.matches_auth(auth))
        {
            Ok(())
        } else {
            Err(owner_changed())
        }
    }

    pub(super) async fn reload(&self, manager: &AuthManager) -> Result<bool, RefreshTokenError> {
        self.check(manager)?;
        if self.external.is_some() {
            let before = manager.auth_cached();
            manager.reload_external(self, before.as_ref()).await?;
            self.check(manager)?;
            return Ok(!AuthManager::auths_equal_for_refresh(
                before.as_ref(),
                manager.auth_cached().as_ref(),
            ));
        }
        let loaded = manager.load_auth().await;
        self.check(manager)?;
        if !self.matches_auth(loaded.as_ref()) {
            return Err(owner_changed());
        }
        let changed =
            !AuthManager::auths_equal_for_refresh(manager.auth_cached().as_ref(), loaded.as_ref());
        manager.set_cached_auth_with_owner(loaded, Some(self))?;
        self.check(manager)?;
        Ok(changed)
    }
}

// ChatgptAuthTokens is the legacy host-owned Ephemeral token mode, not a
// managed OAuth grant. Never infer this contract from workspace equality alone.
pub(super) fn same_host_token_owner(
    previous: Option<&AntexAuth>,
    current: Option<&AntexAuth>,
) -> bool {
    let (Some(previous), Some(current)) = (previous, current) else {
        return false;
    };
    matches!(
        (previous, current),
        (
            AntexAuth::ChatgptAuthTokens(_),
            AntexAuth::ChatgptAuthTokens(_)
        )
    ) && previous
        .get_account_id()
        .as_ref()
        .is_some_and(|account| !account.trim().is_empty())
        && previous.get_account_id() == current.get_account_id()
        && previous.get_chatgpt_user_id() == current.get_chatgpt_user_id()
}

fn registration(auth: &AntexAuth) -> Option<String> {
    if auth.api_auth_mode() != AuthMode::ChatgptDirect {
        return None;
    }
    auth.get_token_data()
        .ok()
        .and_then(|tokens| tokens.direct)
        .map(|direct| direct.client_id)
}

pub(super) fn owner_changed() -> RefreshTokenError {
    RefreshTokenError::Permanent(RefreshTokenFailedError::new(
        RefreshTokenFailedReason::Other,
        "Authentication owner changed (account changed or authentication source replaced); restart the request after signing in",
    ))
}

impl UnauthorizedRecovery {
    /// Check before every retry setup and after setup, before dispatch. Keep this
    /// recovery object for the original operation; do not recapture it per retry.
    pub fn ensure_same_owner(&self) -> Result<(), RefreshTokenError> {
        self.owner.check(&self.manager)
    }

    /// Check a retained account-scoped header provider, NOT an operation/retry.
    /// In addition to strict ownership, this follows legacy Ephemeral host-token
    /// reloads with a known account and unchanged optional user. A separately
    /// tracked lifetime rejects observed logout/account/user/mode ABA, and the
    /// source epoch still rejects equal-token or same-Arc source replacement.
    /// Actual request acquisition must additionally use `auth_for_request`.
    pub fn ensure_same_header_provider_owner(&self) -> Result<(), RefreshTokenError> {
        let cached = self.manager.inner.read().map_err(|_| owner_changed())?;
        self.owner.check_source_locked(&self.manager)?;
        if self.owner.external.is_none()
            && self.owner.mode == Some(AuthMode::ChatgptAuthTokens)
            && self
                .owner
                .account
                .as_ref()
                .is_some_and(|account| !account.trim().is_empty())
            && self.owner.host_token_owner_generation == cached.host_token_owner_generation
            && self.owner.matches_auth(cached.auth.as_ref())
        {
            return Ok(());
        }
        self.owner.check_locked(
            &self.manager,
            cached.auth.as_ref(),
            self.manager.auth_change_state_tx.borrow().owner_generation,
        )
    }

    /// Original operation's owner revision, for binding request/client setup.
    pub fn owner_generation(&self) -> u64 {
        self.owner.generation
    }
}

impl AuthManager {
    pub(super) async fn refresh_for_recovery(
        &self,
        owner: &RecoveryOwner,
    ) -> Result<(), RefreshTokenError> {
        owner.check(self)?;
        let _guard = self
            .refresh_lock
            .acquire()
            .await
            .map_err(|_| owner_changed())?;
        owner.check(self)?;
        self.refresh_token_from_authority_impl().await?;
        owner.check(self)
    }
}

#[cfg(test)]
#[path = "recovery_tests.rs"]
mod tests;
