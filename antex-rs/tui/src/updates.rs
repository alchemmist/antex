#![cfg(not(debug_assertions))]

use crate::legacy_core::config::Config;
use crate::npm_registry;
use crate::npm_registry::NpmPackageInfo;
use crate::update_action;
use crate::update_action::UpdateAction;
use crate::update_versions::extract_version_from_tag;
use crate::update_versions::is_newer;
use crate::update_versions::is_source_build_version;
use crate::updates_cache::VersionInfo;
use crate::updates_cache::fork_version_filepath;
use crate::updates_cache::read_version_info;
use crate::updates_cache::version_filepath;
use antex_http_client::ClientRouteClass;
use antex_http_client::HttpClientFactory;
use antex_http_client::RouteAwareClientPool;
use antex_login::default_client::default_headers;
use chrono::Duration;
use chrono::Utc;
use serde::Deserialize;
use std::path::Path;

use crate::history_cell::StartupUpdates;
use crate::history_cell::VersionUpdate;
use crate::version::ANTEX_CLI_VERSION;

pub fn get_upgrade_version(config: &Config) -> Option<String> {
    if !config.check_for_update_on_startup || is_source_build_version(ANTEX_CLI_VERSION) {
        return None;
    }

    let action = update_action::get_update_action();
    let version_file = version_filepath(config);
    let info = read_version_info(&version_file).ok();

    if match &info {
        None => true,
        Some(info) => info.last_checked_at < Utc::now() - Duration::hours(20),
    } {
        let http_client_factory = config.http_client_factory();
        // Refresh the cached latest version in the background so TUI startup
        // isn’t blocked by a network call. The UI reads the previously cached
        // value (if any) for this run; the next run shows the banner if needed.
        tokio::spawn(async move {
            check_for_update(&version_file, action, http_client_factory)
                .await
                .inspect_err(|e| tracing::error!("Failed to update version: {e}"))
        });
    }

    info.and_then(|info| {
        if is_newer(&info.latest_version, ANTEX_CLI_VERSION).unwrap_or(false) {
            Some(info.latest_version)
        } else {
            None
        }
    })
}

// We use the latest version from the cask if installation is via homebrew - homebrew does not immediately pick up the latest release and can lag behind.
const HOMEBREW_CASK_API_URL: &str = "https://formulae.brew.sh/api/cask/codex.json";
const LATEST_RELEASE_URL: &str = "https://api.github.com/repos/openai/codex/releases/latest";
const FORK_LATEST_RELEASE_URL: &str =
    "https://api.github.com/repos/alchemmist/codex/releases/latest";
const FORK_VERSION: &str = match option_env!("ALCHEMMIST_FORK_VERSION") {
    Some(version) => version,
    None => include_str!("../../../FORK_VERSION"),
};

#[derive(Deserialize, Debug, Clone)]
struct ReleaseInfo {
    tag_name: String,
}

#[derive(Deserialize, Debug, Clone)]
struct HomebrewCaskInfo {
    version: String,
}

pub fn get_startup_updates(config: &Config) -> StartupUpdates {
    StartupUpdates {
        fork: get_fork_upgrade_version(config)
            .map(|latest| VersionUpdate::new(FORK_VERSION.trim(), latest)),
        upstream: get_upgrade_version(config)
            .map(|latest| VersionUpdate::new(ANTEX_CLI_VERSION, latest)),
    }
}

fn get_fork_upgrade_version(config: &Config) -> Option<String> {
    if !config.check_for_update_on_startup || FORK_VERSION == "dev" {
        return None;
    }

    let version_file = fork_version_filepath(config);
    let info = read_version_info(&version_file).ok();
    if match &info {
        None => true,
        Some(info) => info.last_checked_at < Utc::now() - Duration::hours(20),
    } {
        let http_client_factory = config.http_client_factory();
        tokio::spawn(async move {
            check_fork_for_update(&version_file, http_client_factory)
                .await
                .inspect_err(|err| tracing::error!("Failed to update fork version: {err}"))
        });
    }

    info.and_then(|info| {
        if is_newer(&info.latest_version, FORK_VERSION.trim()).unwrap_or(false) {
            Some(info.latest_version)
        } else {
            None
        }
    })
}

async fn check_fork_for_update(
    version_file: &Path,
    http_client_factory: HttpClientFactory,
) -> anyhow::Result<()> {
    let client_pool = RouteAwareClientPool::with_chatgpt_cloudflare_cookies(
        http_client_factory,
        ClientRouteClass::Other,
    )
    .with_legacy_custom_ca_fallback();
    let latest_version = fetch_release_version(&client_pool, FORK_LATEST_RELEASE_URL, "v").await?;
    let info = VersionInfo {
        latest_version,
        last_checked_at: Utc::now(),
        dismissed_version: None,
    };
    let json_line = format!("{}\n", serde_json::to_string(&info)?);
    if let Some(parent) = version_file.parent() {
        tokio::fs::create_dir_all(parent).await?;
    }
    tokio::fs::write(version_file, json_line).await?;
    Ok(())
}

async fn check_for_update(
    version_file: &Path,
    action: Option<UpdateAction>,
    http_client_factory: HttpClientFactory,
) -> anyhow::Result<()> {
    let client_pool = RouteAwareClientPool::with_chatgpt_cloudflare_cookies(
        http_client_factory,
        ClientRouteClass::Other,
    )
    .with_legacy_custom_ca_fallback();
    let latest_version = match action {
        Some(UpdateAction::Daemon(_)) => return Ok(()),
        Some(UpdateAction::BrewUpgrade) => {
            let HomebrewCaskInfo { version } = client_pool
                .get(HOMEBREW_CASK_API_URL)
                .headers(default_headers())
                .send()
                .await?
                .error_for_status()?
                .json::<HomebrewCaskInfo>()
                .await?;
            version
        }
        Some(UpdateAction::NpmGlobalLatest)
        | Some(UpdateAction::BunGlobalLatest)
        | Some(UpdateAction::VitePlusGlobalLatest)
        | Some(UpdateAction::PnpmGlobalLatest) => {
            let latest_version = fetch_latest_github_release_version(&client_pool).await?;
            let package_info = client_pool
                .get(npm_registry::PACKAGE_URL)
                .headers(default_headers())
                .send()
                .await?
                .error_for_status()?
                .json::<NpmPackageInfo>()
                .await?;
            npm_registry::ensure_version_ready(&package_info, &latest_version)?;
            latest_version
        }
        Some(UpdateAction::StandaloneUnix) | Some(UpdateAction::StandaloneWindows) | None => {
            fetch_latest_github_release_version(&client_pool).await?
        }
    };

    // Preserve any previously dismissed version if present.
    let prev_info = read_version_info(version_file).ok();
    let info = VersionInfo {
        latest_version,
        last_checked_at: Utc::now(),
        dismissed_version: prev_info.and_then(|p| p.dismissed_version),
    };

    let json_line = format!("{}\n", serde_json::to_string(&info)?);
    if let Some(parent) = version_file.parent() {
        tokio::fs::create_dir_all(parent).await?;
    }
    tokio::fs::write(version_file, json_line).await?;
    Ok(())
}

async fn fetch_latest_github_release_version(
    client_pool: &RouteAwareClientPool,
) -> anyhow::Result<String> {
    fetch_release_version(client_pool, LATEST_RELEASE_URL, "rust-v").await
}

async fn fetch_release_version(
    client_pool: &RouteAwareClientPool,
    release_url: &str,
    tag_prefix: &str,
) -> anyhow::Result<String> {
    let ReleaseInfo {
        tag_name: latest_tag_name,
    } = client_pool
        .get(release_url)
        .headers(default_headers())
        .send()
        .await?
        .error_for_status()?
        .json::<ReleaseInfo>()
        .await?;
    extract_version_from_tag(&latest_tag_name, tag_prefix)
}
