//! Independent mutation and refresh leases. Waiting never parks a Tokio worker;
//! abandoning acquisition can only drop a lease, never run a delayed credential write.
use std::collections::HashMap;
use std::fs::File;
use std::fs::OpenOptions;
use std::io;
use std::path::PathBuf;
use std::sync::Arc;
use std::sync::LazyLock;
use std::sync::Mutex;
use std::sync::Weak;
use std::sync::atomic::AtomicBool;
use std::sync::atomic::Ordering;
use std::time::Duration;
use std::time::Instant;

#[derive(Clone, Copy, Debug, Hash, PartialEq, Eq)]
pub(super) enum Ownership {
    Mutation,
    Refresh,
}

#[derive(Clone, Debug)]
pub(super) struct LockScope {
    pub home: PathBuf,
    pub ephemeral: bool,
}

#[derive(Debug)]
pub(super) enum Lease {
    File(File),
    Memory(Arc<AtomicBool>),
}

impl Drop for Lease {
    fn drop(&mut self) {
        match self {
            Self::Memory(held) => held.store(false, Ordering::Release),
            Self::File(file) => {
                let _ = file.unlock();
            }
        }
    }
}

type MemoryLocks = HashMap<(PathBuf, Ownership), Weak<AtomicBool>>;
static MEMORY_LOCKS: LazyLock<Mutex<MemoryLocks>> = LazyLock::new(Mutex::default);
pub(super) const WAIT_LIMIT: Duration = Duration::from_secs(60);
pub(super) const POLL_INTERVAL: Duration = Duration::from_millis(10);

impl LockScope {
    pub(super) fn try_acquire(&self, ownership: Ownership) -> io::Result<Option<Lease>> {
        if self.ephemeral {
            let key = (
                self.home
                    .canonicalize()
                    .unwrap_or_else(|_| self.home.clone()),
                ownership,
            );
            let held = {
                let mut locks = MEMORY_LOCKS
                    .lock()
                    .map_err(|_| io::Error::other("Auth ownership registry unavailable"))?;
                locks.retain(|_, entry| entry.strong_count() > 0);
                let held = locks.get(&key).and_then(Weak::upgrade).unwrap_or_default();
                locks.insert(key, Arc::downgrade(&held));
                held
            };
            return Ok(
                if held
                    .compare_exchange(false, true, Ordering::Acquire, Ordering::Relaxed)
                    .is_ok()
                {
                    Some(Lease::Memory(held))
                } else {
                    None
                },
            );
        }
        std::fs::create_dir_all(&self.home)?;
        let name = match ownership {
            Ownership::Mutation => "auth-mutation.lock",
            Ownership::Refresh => "auth-refresh.lock",
        };
        let mut options = OpenOptions::new();
        options.create(true).truncate(false).read(true).write(true);
        #[cfg(unix)]
        {
            use std::os::unix::fs::OpenOptionsExt;
            options.mode(0o600);
        }
        let file = options.open(self.home.join(name))?;
        match file.try_lock() {
            Ok(()) => Ok(Some(Lease::File(file))),
            Err(std::fs::TryLockError::WouldBlock) => Ok(None),
            Err(std::fs::TryLockError::Error(error)) => Err(error),
        }
    }

    pub(super) fn acquire_sync(&self, ownership: Ownership) -> io::Result<Lease> {
        let deadline = Instant::now() + WAIT_LIMIT;
        loop {
            if let Some(lease) = self.try_acquire(ownership)? {
                return Ok(lease);
            }
            if Instant::now() >= deadline {
                return Err(timed_out());
            }
            std::thread::sleep(POLL_INTERVAL);
        }
    }
}

pub(super) fn timed_out() -> io::Error {
    io::Error::new(
        io::ErrorKind::TimedOut,
        "Timed out waiting for authentication ownership",
    )
}
