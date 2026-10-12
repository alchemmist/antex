//! A pending browser attempt may publish only while it is still live and its
//! original credential snapshot is unchanged. Cancellation and commit compete
//! at an atomic state transition *under* the storage mutation lease.
use std::io;
use std::sync::Arc;
use std::sync::atomic::AtomicU8;
use std::sync::atomic::Ordering;
use tokio::time::Instant;

use super::storage::AuthStorageBackend;
use super::storage::create_auth_storage;
use crate::AuthDotJson;
use crate::ServerOptions;

const PENDING: u8 = 0;
const CANCELED: u8 = 1;
const COMMITTING: u8 = 2;

#[derive(Debug, Default)]
pub(crate) struct LoginAttempt(AtomicU8);

impl LoginAttempt {
    /// True means cancellation won (or already won); false means publication
    /// already linearized and cancellation is too late to undo that commit.
    pub(crate) fn cancel(&self) -> bool {
        self.0
            .compare_exchange(PENDING, CANCELED, Ordering::AcqRel, Ordering::Acquire)
            .is_ok()
            || self.0.load(Ordering::Acquire) == CANCELED
    }

    fn commit(&self, deadline: Instant, write: impl FnOnce() -> io::Result<()>) -> io::Result<()> {
        if Instant::now() >= deadline {
            self.cancel();
        }
        self.0
            .compare_exchange(PENDING, COMMITTING, Ordering::AcqRel, Ordering::Acquire)
            .map_err(|_| canceled())?;
        if Instant::now() >= deadline {
            self.0.store(CANCELED, Ordering::Release);
            return Err(canceled());
        }
        write()
    }
}

struct AttemptLifetime(Arc<LoginAttempt>);

impl Drop for AttemptLifetime {
    fn drop(&mut self) {
        self.0.cancel();
    }
}

pub(crate) struct Publication {
    storage: Arc<dyn AuthStorageBackend>,
    original: Option<AuthDotJson>,
    revision: Option<String>,
}

impl Publication {
    pub(crate) fn capture(opts: &ServerOptions) -> io::Result<Self> {
        let storage = create_auth_storage(
            opts.codex_home.clone(),
            opts.cli_auth_credentials_store_mode,
            opts.auth_keyring_backend_kind,
        );
        let original = storage
            .load()
            .map_err(|_| io::Error::other("Could not read authentication state"))?;
        let revision = storage.revision()?;
        Ok(Self {
            storage,
            original,
            revision,
        })
    }

    pub(crate) async fn publish(
        self,
        document: AuthDotJson,
        attempt: Arc<LoginAttempt>,
        shutdown: &tokio::sync::Notify,
        deadline: Instant,
    ) -> io::Result<()> {
        // Task abort/future drop must also invalidate a queued write, not only
        // explicit shutdown notifications and the deadline branches below.
        let _lifetime = AttemptLifetime(attempt.clone());
        let transaction = tokio::select! {
            biased;
            _ = shutdown.notified() => { attempt.cancel(); return Err(canceled()); }
            _ = tokio::time::sleep_until(deadline) => { attempt.cancel(); return Err(canceled()); }
            result = super::storage_transaction::begin(self.storage) => result?,
        };
        let commit_attempt = attempt.clone();
        let originator = antex_otel::auth_storage::AuthStorageOriginator::current();
        let mut task = tokio::task::spawn_blocking(move || {
            originator.sync_scope(|| {
                // Both the snapshot check and the live-attempt check are under the lease.
                if transaction.load()? != self.original || transaction.revision()? != self.revision
                {
                    return Err(io::Error::other("Authentication changed during login"));
                }
                commit_attempt.commit(deadline, || transaction.save(&document))
            })
        });
        tokio::select! {
            biased;
            _ = shutdown.notified() => {
                if attempt.cancel() { return Err(canceled()); }
            }
            _ = tokio::time::sleep_until(deadline) => {
                if attempt.cancel() { return Err(canceled()); }
            }
            result = &mut task => return result.map_err(|_| io::Error::other("Login publication task failed"))?
                .map_err(|_| io::Error::other("Could not publish direct ChatGPT credentials")),
        }
        // Publication won the atomic transition. Do not report a cancellation
        // while a write that already linearized is in progress.
        task.await
            .map_err(|_| io::Error::other("Login publication task failed"))?
            .map_err(|_| io::Error::other("Could not publish direct ChatGPT credentials"))
    }
}

fn canceled() -> io::Error {
    io::Error::other("Login canceled or expired before publication")
}

#[cfg(test)]
#[path = "login_publication_tests.rs"]
mod tests;
