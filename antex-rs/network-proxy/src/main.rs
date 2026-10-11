#![deny(clippy::print_stdout, clippy::print_stderr)]

use antex_network_proxy::ConfigReloader;
use antex_network_proxy::ConfigReloaderFuture;
use antex_network_proxy::ConfigState;
use antex_network_proxy::NetworkMode;
use antex_network_proxy::NetworkProxy;
use antex_network_proxy::NetworkProxyConfig;
use antex_network_proxy::NetworkProxyConstraints;
use antex_network_proxy::NetworkProxyState;
use antex_network_proxy::Platform;
use antex_network_proxy::build_config_state;
use anyhow::Context as _;
use anyhow::Result;
use anyhow::ensure;
use clap::Parser;
use serde::Deserialize;
use std::path::PathBuf;
use std::sync::Arc;
use tracing_subscriber::EnvFilter;

const MAX_CONFIG_BYTES: usize = 1024 * 1024;

#[derive(Debug, Parser)]
#[command(name = "antex-network-proxy", about = "Codex network policy proxy")]
struct Args {
    /// Standalone JSON configuration containing a `network` object.
    #[arg(long, value_name = "PATH")]
    config: PathBuf,
}

#[derive(Debug, Deserialize)]
#[serde(deny_unknown_fields)]
struct StandaloneConfig {
    network: NetworkProxyConfig,
}

fn parse_standalone_config(bytes: &[u8]) -> Result<StandaloneConfig> {
    let mut deserializer = serde_json::Deserializer::from_slice(bytes);
    let mut ignored_fields = Vec::new();
    let mut config: StandaloneConfig = serde_ignored::deserialize(&mut deserializer, |path| {
        ignored_fields.push(path.to_string());
    })?;
    deserializer.end()?;
    ensure!(
        ignored_fields.is_empty(),
        "unknown fields in network proxy config: {}",
        ignored_fields.join(", ")
    );
    config.network.mitm |=
        config.network.mode == NetworkMode::Limited || !config.network.mitm_hooks.is_empty();
    Ok(config)
}

struct StaticConfigReloader {
    state: ConfigState,
}

impl ConfigReloader for StaticConfigReloader {
    fn source_label(&self) -> String {
        "standalone network proxy config".to_string()
    }

    fn maybe_reload(&self) -> ConfigReloaderFuture<'_, Option<ConfigState>> {
        Box::pin(async { Ok(None) })
    }

    fn reload_now(&self) -> ConfigReloaderFuture<'_, ConfigState> {
        Box::pin(async { Ok(self.state.clone()) })
    }
}

#[tokio::main]
async fn main() -> Result<()> {
    tracing_subscriber::fmt()
        .with_env_filter(EnvFilter::from_default_env())
        .init();
    let args = Args::parse();
    let bytes = std::fs::read(&args.config).with_context(|| {
        format!(
            "failed to read network proxy config: {}",
            args.config.display()
        )
    })?;
    ensure!(
        bytes.len() <= MAX_CONFIG_BYTES,
        "network proxy config exceeds {MAX_CONFIG_BYTES} bytes"
    );
    let config = parse_standalone_config(&bytes)
        .with_context(|| format!("invalid network proxy config: {}", args.config.display()))?;
    ensure!(
        config.network.enabled,
        "standalone network proxy requires network.enabled = true"
    );

    let config_state = build_config_state(
        config.network,
        NetworkProxyConstraints::default(),
        Platform::native(),
    )
    .context("failed to initialize network proxy policy")?;
    let reloader = Arc::new(StaticConfigReloader {
        state: config_state.clone(),
    });
    let state = Arc::new(NetworkProxyState::with_reloader(config_state, reloader));
    NetworkProxy::builder()
        .state(state)
        .managed_by_antex(/*managed_by_antex*/ false)
        .build()
        .await?
        .run()
        .await?
        .wait()
        .await
}

#[cfg(test)]
#[path = "main_tests.rs"]
mod tests;
