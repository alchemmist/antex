//! External results stay bound to their producing source until publication.
//! Lock order: optional ephemeral mutation lease, cache write, source write.
//! No cache/source lock is held while awaiting a provider or mutation ownership.
use super::*;
use crate::auth::storage_transaction;

#[cfg(test)]
#[path = "external_publication_tests.rs"]
mod tests;

enum Publication {
    CurrentSource,
    ReplaceSource,
}

impl AuthManager {
    pub(super) async fn install_external_auth(
        &self,
        source: Arc<dyn ExternalAuth>,
    ) -> Result<(), RefreshTokenError> {
        let owner = recovery::RecoveryOwner::capture(self);
        // Keep the working source/cache pair intact until replacement resolves,
        // validates and can commit. Failed replacement never clears working auth.
        let auth = self.resolve_external_auth(source.as_ref()).await?;
        self.publish_external(&owner, source, auth, Publication::ReplaceSource)
            .await?;
        Ok(())
    }

    pub(super) async fn reload_external(
        &self,
        owner: &recovery::RecoveryOwner,
        cached: Option<&AntexAuth>,
    ) -> Result<bool, RefreshTokenError> {
        owner.check(self)?;
        let source = owner
            .external_source()
            .ok_or_else(recovery::owner_changed)?;
        if let Some(failure) = cached.and_then(|auth| self.refresh_failure_for_auth(auth)) {
            return Err(RefreshTokenError::Permanent(failure));
        }
        match self.resolve_external_auth(source.as_ref()).await {
            Ok(auth) => {
                self.publish_external(owner, source, auth, Publication::CurrentSource)
                    .await
            }
            Err(error) => {
                if let (Some(auth), RefreshTokenError::Permanent(failure)) = (cached, &error) {
                    self.record_refresh_failure_with_owner(owner, auth, failure);
                }
                // A failure or rejected identity is not a request to clear the
                // last working credential. Initial lazy failure still has None.
                Err(error)
            }
        }
    }

    pub(super) async fn refresh_external_auth(
        &self,
        reason: ExternalAuthRefreshReason,
    ) -> Result<(), RefreshTokenError> {
        let (owner, previous) = recovery::RecoveryOwner::capture_with_auth(self);
        let source = owner
            .external_source()
            .ok_or_else(recovery::owner_changed)?;
        let context = ExternalAuthRefreshContext {
            reason,
            previous_account_id: previous.as_ref().and_then(AntexAuth::get_account_id),
        };
        let refreshed = source
            .refresh(context)
            .await
            .map_err(|error| source.classify_error(error))?;
        self.validate_external_auth(&refreshed, source.as_ref())?;
        self.publish_external(&owner, source, refreshed, Publication::CurrentSource)
            .await?;
        Ok(())
    }

    async fn publish_external(
        &self,
        owner: &recovery::RecoveryOwner,
        source: Arc<dyn ExternalAuth>,
        auth: AntexAuth,
        publication: Publication,
    ) -> Result<bool, RefreshTokenError> {
        owner.check(self)?;
        if matches!(publication, Publication::CurrentSource)
            && (!owner
                .external_source()
                .is_some_and(|original| Arc::ptr_eq(&original, &source))
                || !owner.accepts_external_result(&auth))
        {
            return Err(recovery::owner_changed());
        }
        let document = if auth.is_external_chatgpt_tokens() {
            Some(
                auth.get_current_auth_json()
                    .ok_or_else(|| std::io::Error::other("External auth state is missing"))?,
            )
        } else {
            None
        };
        let transaction = if document.is_some() {
            Some(
                storage_transaction::begin(create_auth_storage(
                    self.codex_home.clone(),
                    AuthCredentialsStoreMode::Ephemeral,
                    AuthKeyringBackendKind::default(),
                ))
                .await?,
            )
        } else {
            None
        };
        let mut cached = self.inner.write().map_err(|_| recovery::owner_changed())?;
        owner.check_locked(
            self,
            cached.auth.as_ref(),
            self.auth_change_state_tx.borrow().owner_generation,
        )?;
        // Every source writer takes the cache lock first. The source cannot
        // change between the owner check and either the storage or cache write.
        let mut installed = self
            .external_auth
            .write()
            .map_err(|_| recovery::owner_changed())?;
        validate_auth_restrictions(
            Some(&self.allowed_login_methods()),
            self.effective_chatgpt_workspaces().as_deref(),
            &auth,
        )
        .map_err(|_| {
            std::io::Error::new(
                std::io::ErrorKind::PermissionDenied,
                "External authentication is restricted by policy",
            )
        })?;
        if let (Some(transaction), Some(document)) = (&transaction, &document) {
            transaction.save(document)?;
        }
        if matches!(publication, Publication::ReplaceSource) {
            self.external_source_epoch.fetch_add(1, Ordering::Relaxed);
            *installed = Some(source);
            cached.permanent_refresh_failure = None;
        }
        let changed = self.publish_cached_auth(&mut cached, Some(auth));
        if matches!(publication, Publication::ReplaceSource) {
            // Wake lifetime guards even for equal-token/same-Arc reinstall.
            // Public credential and owner revisions retain their existing meaning.
            self.auth_change_state_tx.send_modify(|_| {});
        }
        Ok(changed)
    }

    pub fn clear_external_auth(&self) {
        if self.workload_identity_selected {
            return;
        }
        if let Ok(mut cached) = self.inner.write()
            && let Ok(mut source) = self.external_auth.write()
        {
            // Even clearing an empty slot invalidates a pending installation.
            // Failed replacements and automatic rotations never advance this.
            self.external_source_epoch.fetch_add(1, Ordering::Relaxed);
            if source.take().is_some() {
                self.publish_cached_auth(&mut cached, /*new_auth*/ None);
            }
            self.auth_change_state_tx.send_modify(|_| {});
        }
    }
}
