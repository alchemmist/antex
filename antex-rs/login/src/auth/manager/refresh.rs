//! All managed grants share refresh-consumption ownership, independently of the
//! short mutation transaction used to publish. Login/logout can always supersede
//! an in-flight exchange; its result is committed against its ORIGINAL document.
use super::*;
use crate::auth::storage_transaction;

pub(super) fn same_grant_owner(a: &AuthDotJson, b: &AuthDotJson) -> bool {
    if a.resolved_mode() != b.resolved_mode() {
        return false;
    }
    let (Some(a), Some(b)) = (&a.tokens, &b.tokens) else {
        return false;
    };
    let same_registration = match (&a.direct, &b.direct) {
        (Some(a), Some(b)) => a.client_id == b.client_id,
        (None, None) => true,
        _ => false,
    };
    same_registration
        && a.account_id == b.account_id
        && a.id_token.chatgpt_user_id == b.id_token.chatgpt_user_id
        && a.id_token.chatgpt_account_id == b.id_token.chatgpt_account_id
}

impl AuthManager {
    // A private authority-operation seam keeps caching/recovery identical for
    // production fixed endpoints and controlled hermetic HTTP authorities.
    pub(super) async fn refresh_with<F, Fut>(&self, operation: F) -> Result<(), RefreshTokenError>
    where
        F: FnOnce(Option<AntexAuth>) -> Fut,
        Fut: Future<Output = Result<(), RefreshTokenError>>,
    {
        let (owner, attempted) = recovery::RecoveryOwner::capture_with_auth(self);
        if let Some(error) = attempted
            .as_ref()
            .and_then(|auth| self.refresh_failure_for_auth(auth))
        {
            return Err(RefreshTokenError::Permanent(error));
        }
        let result = operation(attempted.clone()).await;
        if let Some(auth) = attempted.as_ref()
            && let Err(RefreshTokenError::Permanent(error)) = &result
        {
            self.record_refresh_failure_with_owner(&owner, auth, error);
        }
        result
    }

    pub(super) async fn refresh_managed_with<F, Fut>(
        &self,
        attempted: &AntexAuth,
        exchange: F,
    ) -> Result<(), RefreshTokenError>
    where
        F: FnOnce(AuthDotJson) -> Fut + Send + 'static,
        Fut: Future<Output = Result<AuthDotJson, RefreshTokenError>> + Send + 'static,
    {
        let auth = match attempted {
            AntexAuth::Chatgpt(auth) | AntexAuth::ChatgptDirect(auth) => auth,
            _ => return Err(std::io::Error::other("Not a managed OAuth grant").into()),
        };
        let original = auth
            .current_auth_json()
            .ok_or_else(|| std::io::Error::other("Missing OAuth credentials"))?;
        let owner = recovery::RecoveryOwner::capture(self);
        owner.check(self)?;
        // This lease is shared by independently constructed managers and (except
        // ephemeral auth) by independent processes using the same auth home.
        let lease = storage_transaction::refresh_lease(auth.storage().clone()).await?;
        owner.check(self)?;
        if let Some(failure) = self.refresh_failure_for_auth(attempted) {
            return Err(RefreshTokenError::Permanent(failure));
        }
        let stored = auth.storage().load()?;
        if stored.as_ref() != Some(&original) {
            if stored
                .as_ref()
                .is_some_and(|stored| same_grant_owner(&original, stored))
            {
                // Another holder consumed the old token and persisted its rotation.
                // Never consume it again, even for an explicit unauthorized refresh.
                return self.accept_managed_rotation(auth, &original, &owner);
            }
            return Err(recovery::owner_changed());
        }
        let storage = auth.storage().clone();
        let job_original = original.clone();
        // Dispatch transfers the entire transaction (not just its final write)
        // to the runtime. Dropping this caller's JoinHandle does not abort it:
        // an authority may already have consumed its single-use refresh token.
        // No AuthManager is captured, so an abandoned waiter cannot update any
        // manager's cache. Login/logout still win via the storage CAS below.
        let job = async move {
            let updated =
                tokio::time::timeout(Duration::from_secs(60), exchange(job_original.clone()))
                    .await
                    .map_err(|_| std::io::Error::other("OAuth refresh exchange timed out"))??;
            if !same_grant_owner(&job_original, &updated) {
                return Err(recovery::owner_changed());
            }
            let transaction = storage_transaction::begin(storage).await?;
            let originator = antex_otel::auth_storage::AuthStorageOriginator::current();
            tokio::task::spawn_blocking(move || {
                originator.sync_scope(|| {
                    let _lease = lease;
                    transaction.replace_if_matches(&job_original, &updated)
                })
            })
            .await
            .map_err(|_| std::io::Error::other("OAuth publication task failed"))?
            .map_err(|_| {
                std::io::Error::other("Credentials changed or could not be saved during refresh")
            })?;
            Ok::<(), RefreshTokenError>(())
        };
        tokio::spawn(antex_otel::auth_storage::AuthStorageOriginator::current().scope(job))
            .await
            .map_err(|_| std::io::Error::other("OAuth refresh task failed"))??;
        self.accept_managed_rotation(auth, &original, &owner)
    }

    // Only a still-live waiter reaches this cache update. Read the pinned source
    // and publish under an owner precondition; never use an unguarded manager
    // reload that could overwrite a concurrently installed account.
    fn accept_managed_rotation(
        &self,
        auth: &ChatgptAuth,
        original: &AuthDotJson,
        owner: &recovery::RecoveryOwner,
    ) -> Result<(), RefreshTokenError> {
        owner.check(self)?;
        let stored = auth.storage().load()?.ok_or_else(recovery::owner_changed)?;
        if !same_grant_owner(original, &stored) {
            return Err(recovery::owner_changed());
        }
        let mut reloaded = auth.clone();
        reloaded.state.auth_dot_json = Arc::new(Mutex::new(Some(stored)));
        let reloaded = match original.resolved_mode() {
            AuthMode::Chatgpt => AntexAuth::Chatgpt(reloaded),
            AuthMode::ChatgptDirect => AntexAuth::ChatgptDirect(reloaded),
            _ => return Err(recovery::owner_changed()),
        };
        if !owner.matches_auth(Some(&reloaded)) {
            return Err(recovery::owner_changed());
        }
        validate_auth_restrictions(
            Some(&self.allowed_login_methods()),
            self.effective_chatgpt_workspaces().as_deref(),
            &reloaded,
        )
        .map_err(|_| {
            std::io::Error::new(
                std::io::ErrorKind::PermissionDenied,
                "Refreshed authentication is restricted by policy",
            )
        })?;
        self.set_cached_auth_with_owner(Some(reloaded), Some(owner))?;
        Ok(())
    }

    pub(super) async fn refresh_and_persist_chatgpt_token(
        &self,
        auth: &ChatgptAuth,
    ) -> Result<(), RefreshTokenError> {
        let client = auth.client().clone();
        self.refresh_managed_with(
            &AntexAuth::Chatgpt(auth.clone()),
            move |original| async move {
                let refresh_token = original
                    .tokens
                    .as_ref()
                    .ok_or_else(|| std::io::Error::other("Missing OAuth credentials"))?
                    .refresh_token
                    .clone();
                let response = request_chatgpt_token_refresh(refresh_token, &client).await?;
                refreshed_legacy(original, response)
            },
        )
        .await
    }
}

pub(super) fn refreshed_legacy(
    mut original: AuthDotJson,
    response: RefreshResponse,
) -> Result<AuthDotJson, RefreshTokenError> {
    if original.resolved_mode() != AuthMode::Chatgpt {
        return Err(std::io::Error::other("Legacy refresh requires a legacy grant").into());
    }
    let tokens = original
        .tokens
        .as_mut()
        .ok_or_else(|| std::io::Error::other("Missing OAuth credentials"))?;
    if tokens.direct.is_some() {
        return Err(std::io::Error::other("Not a legacy grant").into());
    }
    if let Some(jwt) = response.id_token {
        tokens.id_token = parse_chatgpt_jwt_claims(&jwt)
            .map_err(|_| std::io::Error::other("Invalid refreshed ID token"))?;
    }
    if let Some(access) = response.access_token {
        tokens.access_token = access;
    }
    if let Some(refresh) = response.refresh_token {
        tokens.refresh_token = refresh;
    }
    original.last_refresh = Some(Utc::now());
    Ok(original)
}

#[cfg(test)]
#[path = "canceled_refresh_tests.rs"]
mod canceled_tests;

#[cfg(test)]
#[path = "refresh_tests.rs"]
mod tests;
