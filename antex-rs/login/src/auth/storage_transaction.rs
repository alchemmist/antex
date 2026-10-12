//! Mutation transactions and asynchronous lease acquisition for auth lifecycle operations.
use std::io;
use std::sync::Arc;

use super::storage::AuthDotJson;
use super::storage::AuthStorageBackend;
use super::storage_ownership::Lease;
use super::storage_ownership::LockScope;
use super::storage_ownership::Ownership;
use super::storage_ownership::POLL_INTERVAL;
use super::storage_ownership::WAIT_LIMIT;

#[derive(Debug)]
pub(super) struct TransactionalStorage {
    inner: Arc<dyn AuthStorageBackend>,
    scope: LockScope,
}

#[derive(Debug)]
struct Transaction {
    inner: Arc<dyn AuthStorageBackend>,
    _lease: Lease,
    scope: LockScope,
}

impl TransactionalStorage {
    pub(super) fn wrap(
        inner: Arc<dyn AuthStorageBackend>,
        scope: LockScope,
    ) -> Arc<dyn AuthStorageBackend> {
        Arc::new(Self { inner, scope })
    }
}

impl AuthStorageBackend for TransactionalStorage {
    fn load(&self) -> io::Result<Option<AuthDotJson>> {
        self.inner.load()
    }
    fn save(&self, auth: &AuthDotJson) -> io::Result<()> {
        let _lease = self.scope.acquire_sync(Ownership::Mutation)?;
        super::mutation_revision::advance(&self.scope)?;
        self.inner.save(auth)
    }
    fn delete(&self) -> io::Result<bool> {
        let _lease = self.scope.acquire_sync(Ownership::Mutation)?;
        super::mutation_revision::advance(&self.scope)?;
        self.inner.delete()
    }
    fn replace_if_matches(&self, expected: &AuthDotJson, updated: &AuthDotJson) -> io::Result<()> {
        let lease = self.scope.acquire_sync(Ownership::Mutation)?;
        Transaction {
            inner: self.inner.clone(),
            _lease: lease,
            scope: self.scope.clone(),
        }
        .replace_if_matches(expected, updated)
    }
    fn revision(&self) -> io::Result<Option<String>> {
        super::mutation_revision::read(&self.scope)
    }
    fn try_transaction(&self) -> io::Result<Option<Arc<dyn AuthStorageBackend>>> {
        Ok(self.scope.try_acquire(Ownership::Mutation)?.map(|lease| {
            Arc::new(Transaction {
                inner: self.inner.clone(),
                _lease: lease,
                scope: self.scope.clone(),
            }) as Arc<dyn AuthStorageBackend>
        }))
    }
    fn try_refresh_lease(&self) -> io::Result<Option<Lease>> {
        self.scope.try_acquire(Ownership::Refresh)
    }
}

impl AuthStorageBackend for Transaction {
    fn load(&self) -> io::Result<Option<AuthDotJson>> {
        self.inner.load()
    }
    fn save(&self, auth: &AuthDotJson) -> io::Result<()> {
        super::mutation_revision::advance(&self.scope)?;
        self.inner.save(auth)
    }
    fn delete(&self) -> io::Result<bool> {
        super::mutation_revision::advance(&self.scope)?;
        self.inner.delete()
    }
    fn revision(&self) -> io::Result<Option<String>> {
        super::mutation_revision::read(&self.scope)
    }
}

pub(super) async fn begin(
    storage: Arc<dyn AuthStorageBackend>,
) -> io::Result<Arc<dyn AuthStorageBackend>> {
    wait_for(move || storage.try_transaction()).await
}

pub(super) async fn refresh_lease(storage: Arc<dyn AuthStorageBackend>) -> io::Result<Lease> {
    wait_for(move || storage.try_refresh_lease()).await
}

async fn wait_for<T: Send + 'static>(
    attempt: impl Fn() -> io::Result<Option<T>> + Send + Sync + 'static,
) -> io::Result<T> {
    let attempt = Arc::new(attempt);
    let wait = async move {
        loop {
            let attempt = attempt.clone();
            // Only a nonblocking acquisition may outlive cancellation. No queued task
            // contains a write; its result/lease is dropped when the receiver is gone.
            let acquired = tokio::task::spawn_blocking(move || attempt())
                .await
                .map_err(|_| io::Error::other("Auth ownership task failed"))??;
            if let Some(acquired) = acquired {
                return Ok(acquired);
            }
            tokio::time::sleep(POLL_INTERVAL).await;
        }
    };
    tokio::time::timeout(WAIT_LIMIT, wait)
        .await
        .map_err(|_| super::storage_ownership::timed_out())?
}
