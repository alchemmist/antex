//! Private-file publication without truncating an existing credential on failure.
use std::fs;
use std::fs::OpenOptions;
use std::io;
use std::io::Write;
use std::path::Path;
use std::path::PathBuf;

pub(crate) struct PendingFile(PathBuf);

impl PendingFile {
    pub(crate) fn write(path: &Path, contents: &[u8]) -> io::Result<Self> {
        let parent = path
            .parent()
            .ok_or_else(|| io::Error::other("Missing storage directory"))?;
        fs::create_dir_all(parent)?;
        let temporary = parent.join(format!(
            ".antex-auth-{}.tmp",
            crate::oauth::generate_state()
        ));
        let mut options = OpenOptions::new();
        options.create_new(true).write(true);
        #[cfg(unix)]
        {
            use std::os::unix::fs::OpenOptionsExt;
            options.mode(0o600);
        }
        let mut file = options.open(&temporary)?;
        let pending = Self(temporary);
        file.write_all(contents)?;
        file.sync_all()?;
        Ok(pending)
    }

    pub(crate) fn replace(self, path: &Path) -> io::Result<()> {
        fs::rename(&self.0, path)
    }

    /// Publish once, without exposing partially written data to concurrent logins.
    pub(crate) fn create(self, path: &Path) -> io::Result<()> {
        fs::hard_link(&self.0, path)
    }
}

#[cfg(test)]
#[path = "atomic_file_tests.rs"]
mod tests;

impl Drop for PendingFile {
    fn drop(&mut self) {
        let _ = fs::remove_file(&self.0);
    }
}
