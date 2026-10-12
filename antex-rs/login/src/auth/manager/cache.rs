//! Publish a cache update and its revisions atomically with an optional owner
//! precondition. A recovery reload must not overwrite a concurrently installed B.
use super::*;

impl AuthManager {
    pub(super) fn set_cached_auth_with_owner(
        &self,
        new_auth: Option<AntexAuth>,
        owner: Option<&recovery::RecoveryOwner>,
    ) -> Result<bool, RefreshTokenError> {
        let mut guard = self.inner.write().map_err(|_| recovery::owner_changed())?;
        if let Some(owner) = owner {
            owner.check_locked(
                self,
                guard.auth.as_ref(),
                self.auth_change_state_tx.borrow().owner_generation,
            )?;
        }
        Ok(self.publish_cached_auth(&mut guard, new_auth))
    }

    // Caller holds the cache write lock. Source writers hold this lock first,
    // then the source lock, so readers never observe B paired with A's token.
    pub(super) fn publish_cached_auth(
        &self,
        guard: &mut CachedAuth,
        new_auth: Option<AntexAuth>,
    ) -> bool {
        let previous = guard.auth.as_ref();
        let changed = !Self::auths_equal(previous, new_auth.as_ref());
        let auth_changed_for_refresh = !Self::auths_equal_for_refresh(previous, new_auth.as_ref());
        let owner_changed = auth_changed_for_refresh && !same_owner(previous, new_auth.as_ref());
        if auth_changed_for_refresh && !recovery::same_host_token_owner(previous, new_auth.as_ref())
        {
            guard.host_token_owner_generation += 1;
        }
        if owner_changed {
            self.auth_route_config
                .application_network_policy()
                .invalidate();
        }
        if auth_changed_for_refresh {
            guard.permanent_refresh_failure = None;
        }
        tracing::info!("Reloaded auth, changed: {changed}");
        guard.auth = new_auth;
        if auth_changed_for_refresh {
            self.auth_change_state_tx.send_modify(|state| {
                state.generation += 1;
                if owner_changed {
                    state.owner_generation += 1;
                }
            });
            self.auth_change_tx.send_modify(|revision| *revision += 1);
        }
        changed
    }

    pub(super) fn record_refresh_failure_with_owner(
        &self,
        owner: &recovery::RecoveryOwner,
        attempted: &AntexAuth,
        error: &RefreshTokenFailedError,
    ) {
        if let Ok(mut cached) = self.inner.write()
            && owner
                .check_locked(
                    self,
                    cached.auth.as_ref(),
                    self.auth_change_state_tx.borrow().owner_generation,
                )
                .is_ok()
            && Self::auths_equal_for_refresh(Some(attempted), cached.auth.as_ref())
        {
            cached.permanent_refresh_failure = Some(AuthScopedRefreshFailure {
                auth: attempted.clone(),
                error: error.clone(),
            });
        }
    }
}
