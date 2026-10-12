//! An immutable operation/source lifetime, independent of the 401 retry budget.
//! Watch events only wake the source/epoch-aware guard; revisions are not identity.
use super::LunaSamplerError;
use antex_login::AuthChangeState;
use antex_login::UnauthorizedRecovery;
use antex_model_provider::SharedModelProvider;
use std::sync::Arc;
use tokio::sync::watch;

pub(super) struct RequestOwner {
    guard: Option<UnauthorizedRecovery>,
    changes: Option<watch::Receiver<AuthChangeState>>,
}

impl RequestOwner {
    pub(super) fn capture(provider: &SharedModelProvider) -> Arc<Self> {
        let manager = provider.auth_manager();
        let changes = manager
            .as_ref()
            .map(|manager| manager.auth_change_state_receiver());
        Arc::new(Self {
            guard: manager.map(|manager| manager.unauthorized_recovery()),
            changes,
        })
    }

    pub(super) fn check(&self) -> Result<(), LunaSamplerError> {
        if let Some(guard) = &self.guard {
            guard
                .ensure_same_owner()
                .map_err(|error| LunaSamplerError::Provider(std::io::Error::from(error).into()))?;
        }
        Ok(())
    }

    pub(super) async fn changed(&self) {
        let Some(mut changes) = self.changes.clone() else {
            return std::future::pending().await;
        };
        loop {
            changes.borrow_and_update();
            if self.check().is_err() || changes.changed().await.is_err() {
                return;
            }
        }
    }
}
