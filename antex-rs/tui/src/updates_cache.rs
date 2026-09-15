use crate::legacy_core::config::Config;
use chrono::DateTime;
use chrono::Utc;
use serde::Deserialize;
use serde::Serialize;
use std::path::Path;
use std::path::PathBuf;

#[derive(Serialize, Deserialize, Debug, Clone)]
pub(crate) struct VersionInfo {
    pub(crate) latest_version: String,
    // ISO-8601 timestamp (RFC3339)
    pub(crate) last_checked_at: DateTime<Utc>,
    #[serde(default)]
    pub(crate) dismissed_version: Option<String>,
}

const VERSION_FILENAME: &str = "version.json";
#[cfg(not(debug_assertions))]
const FORK_VERSION_FILENAME: &str = "fork-version.json";

pub(crate) fn version_filepath(config: &Config) -> PathBuf {
    config.codex_home.join(VERSION_FILENAME).into_path_buf()
}

#[cfg(not(debug_assertions))]
pub(crate) fn fork_version_filepath(config: &Config) -> PathBuf {
    config
        .codex_home
        .join(FORK_VERSION_FILENAME)
        .into_path_buf()
}

pub(crate) fn read_version_info(version_file: &Path) -> anyhow::Result<VersionInfo> {
    let contents = std::fs::read_to_string(version_file)?;
    Ok(serde_json::from_str(&contents)?)
}
