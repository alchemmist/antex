use std::path::PathBuf;

use antex_utils_absolute_path::AbsolutePathBuf;

/// Paths and sandbox settings initialized when creating an executor.
#[derive(Clone, Debug, Eq, PartialEq)]
pub struct ExecServerRuntimePaths {
    /// Stable path to the Antex executable used to launch hidden helper modes.
    pub antex_self_exe: AbsolutePathBuf,
    /// Path to the Linux sandbox helper alias used when the platform sandbox
    /// needs to re-enter Antex by argv0.
    pub antex_linux_sandbox_exe: Option<AbsolutePathBuf>,
    /// User-config opt-out of writable-root symlink checks beneath this host's home.
    #[cfg(target_os = "macos")]
    pub allowed_symlinked_antex_home: Option<AbsolutePathBuf>,
}

impl ExecServerRuntimePaths {
    pub fn from_optional_paths(
        antex_self_exe: Option<PathBuf>,
        antex_linux_sandbox_exe: Option<PathBuf>,
    ) -> std::io::Result<Self> {
        let antex_self_exe = antex_self_exe.ok_or_else(|| {
            std::io::Error::new(
                std::io::ErrorKind::InvalidInput,
                "Codex executable path is not configured",
            )
        })?;
        Self::new(antex_self_exe, antex_linux_sandbox_exe)
    }

    pub fn new(
        antex_self_exe: PathBuf,
        antex_linux_sandbox_exe: Option<PathBuf>,
    ) -> std::io::Result<Self> {
        Ok(Self {
            antex_self_exe: absolute_path(antex_self_exe)?,
            antex_linux_sandbox_exe: antex_linux_sandbox_exe.map(absolute_path).transpose()?,
            #[cfg(target_os = "macos")]
            allowed_symlinked_antex_home: None,
        })
    }

    /// Applies the symlink opt-in resolved by the execution host's config loader.
    #[cfg(target_os = "macos")]
    pub fn with_allowed_symlinked_antex_home(
        mut self,
        allowed_symlinked_antex_home: Option<AbsolutePathBuf>,
    ) -> Self {
        self.allowed_symlinked_antex_home = allowed_symlinked_antex_home;
        self
    }
}

fn absolute_path(path: PathBuf) -> std::io::Result<AbsolutePathBuf> {
    AbsolutePathBuf::from_absolute_path(path.as_path())
        .map_err(|err| std::io::Error::new(std::io::ErrorKind::InvalidInput, err))
}
