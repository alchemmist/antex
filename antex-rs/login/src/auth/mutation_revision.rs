//! Nonsecret mutation revision also invalidates a pending login when logout
//! succeeds against an already-empty store (None -> None).
use super::storage_ownership::LockScope;
use std::collections::HashMap;
use std::io;
use std::path::PathBuf;
use std::sync::LazyLock;
use std::sync::Mutex;

static MEMORY: LazyLock<Mutex<HashMap<PathBuf, String>>> = LazyLock::new(Mutex::default);
const FILE: &str = "auth-mutation-revision";

pub(super) fn read(scope: &LockScope) -> io::Result<Option<String>> {
    if scope.ephemeral {
        let key = scope
            .home
            .canonicalize()
            .unwrap_or_else(|_| scope.home.clone());
        return Ok(MEMORY
            .lock()
            .map_err(|_| io::Error::other("Auth revision unavailable"))?
            .get(&key)
            .cloned());
    }
    match std::fs::read_to_string(scope.home.join(FILE)) {
        Ok(value) => Ok(Some(value)),
        Err(error) if error.kind() == io::ErrorKind::NotFound => Ok(None),
        Err(error) => Err(error),
    }
}

/// Called only under mutation ownership, before a write/delete. Invalidate even
/// if the following operation fails, but never modify the existing credential.
pub(super) fn advance(scope: &LockScope) -> io::Result<()> {
    let value = crate::oauth::generate_state();
    if scope.ephemeral {
        let key = scope
            .home
            .canonicalize()
            .unwrap_or_else(|_| scope.home.clone());
        MEMORY
            .lock()
            .map_err(|_| io::Error::other("Auth revision unavailable"))?
            .insert(key, value);
        return Ok(());
    }
    let path = scope.home.join(FILE);
    crate::atomic_file::PendingFile::write(&path, value.as_bytes())?.replace(&path)
}
