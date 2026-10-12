//! Keeps an automatic user turn (or standalone request) bound to its original owner.

use antex_login::RefreshTokenError;
use antex_login::UnauthorizedRecovery;
use std::ops::Deref;
use std::ops::DerefMut;
use std::sync::Arc;

use super::ClientRouting;
use super::CurrentClientSetup;
use super::ModelClient;
use super::ModelClientSession;
use antex_protocol::error::CodexErr;
use antex_protocol::error::Result;

pub(super) fn ensure_same_owner(recovery: Option<&UnauthorizedRecovery>) -> Result<()> {
    let Some(recovery) = recovery else {
        return Ok(());
    };
    recovery.ensure_same_owner().map_err(|error| match error {
        RefreshTokenError::Policy(error) => CodexErr::Fatal(error.to_string()),
        RefreshTokenError::Permanent(failure) => CodexErr::RefreshTokenFailed(failure),
        RefreshTokenError::Transient(error) => CodexErr::Io(error),
    })
}

// Nested sampling/stream/fallback calls inherit the enclosing operation's binding.
// Drop restores the prior scope on success, failure and cancellation; a later user
// turn on the same session is free to bind the newly selected account.
struct Operation<'a> {
    session: &'a mut ModelClientSession,
    previous: Option<Arc<UnauthorizedRecovery>>,
}

impl Deref for Operation<'_> {
    type Target = ModelClientSession;

    fn deref(&self) -> &Self::Target {
        self.session
    }
}

impl DerefMut for Operation<'_> {
    fn deref_mut(&mut self) -> &mut Self::Target {
        self.session
    }
}

impl Drop for Operation<'_> {
    fn drop(&mut self) {
        self.session.operation_owner = self.previous.take();
    }
}

impl ModelClientSession {
    /// Bind an operation, including the entire automatic loop when called by run_turn.
    /// Standalone sampling/compaction binds freshly; nested scopes inherit the owner.
    /// This recovery is only an owner guard; each attempt retains its own 401 budget.
    pub(crate) fn sampling_operation(&mut self) -> impl DerefMut<Target = Self> + '_ {
        let previous = self.operation_owner.clone();
        if self.operation_owner.is_none() {
            self.operation_owner = self
                .client
                .auth_manager()
                .map(|manager| Arc::new(manager.unauthorized_recovery()));
        }
        Operation {
            session: self,
            previous,
        }
    }

    /// Give automatic local compaction independent transport/incremental state without
    /// allowing it to capture a newer login than its parent turn. Only the immutable
    /// owner guard is shared, never a request's mutable unauthorized-recovery budget.
    pub(crate) fn new_session_with_operation_owner(&self) -> Self {
        let mut child = self.client.new_session();
        child.operation_owner = self.operation_owner.clone();
        child
    }

    pub(crate) fn ensure_operation_owner(&self) -> Result<()> {
        ensure_same_owner(self.operation_owner.as_deref())
    }
}

impl ModelClient {
    pub(super) async fn current_client_setup_for_request(
        &self,
        routing: ClientRouting,
        recovery: Option<&UnauthorizedRecovery>,
    ) -> Result<CurrentClientSetup> {
        ensure_same_owner(recovery)?;
        // External bearers use their stable provider as the opaque owner;
        // rotation may change the credential generation. The recovery guards
        // still reject changes to an identified external account or its source.
        let external = self
            .state
            .provider
            .auth_manager()
            .is_some_and(|manager| manager.has_external_auth());
        let owner = if external {
            self.auth_owner_generation()
        } else {
            recovery
                .map(UnauthorizedRecovery::owner_generation)
                .or_else(|| self.auth_owner_generation())
        };
        let setup = self.current_client_setup_for_owner(routing, owner).await;
        // Check even when setup failed: an owner switch is a permanent request
        // failure, not a transient setup error that may retry under a new login.
        ensure_same_owner(recovery)?;
        setup
    }
}
