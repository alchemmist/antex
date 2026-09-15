//! Live reloads the TUI syntax theme when user config changes on disk.

use std::io::ErrorKind;
use std::path::Path;
use std::path::PathBuf;
use std::sync::Arc;
use std::time::Duration;

use antex_config::CONFIG_TOML_FILE;
use antex_file_watcher::DebouncedWatchReceiver;
use antex_file_watcher::FileWatcher;
use antex_file_watcher::WatchPath;
use serde::Deserialize;
use tokio::sync::oneshot;

use crate::app_event::AppEvent;
use crate::app_event_sender::AppEventSender;

const THEME_CONFIG_DEBOUNCE: Duration = Duration::from_millis(75);

#[derive(Deserialize)]
struct ThemeConfig {
    tui: Option<ThemeConfigTui>,
}

#[derive(Deserialize)]
struct ThemeConfigTui {
    theme: Option<String>,
}

pub(super) fn spawn(
    codex_home: PathBuf,
    initial_theme: Option<String>,
    app_event_tx: AppEventSender,
) {
    let _ready = spawn_with_ready(codex_home, initial_theme, app_event_tx);
}

fn spawn_with_ready(
    codex_home: PathBuf,
    initial_theme: Option<String>,
    app_event_tx: AppEventSender,
) -> oneshot::Receiver<()> {
    let (ready_tx, ready_rx) = oneshot::channel();
    tokio::spawn(async move {
        let file_watcher = match FileWatcher::new() {
            Ok(file_watcher) => Arc::new(file_watcher),
            Err(err) => {
                tracing::warn!("failed to initialize syntax theme config watcher: {err}");
                return;
            }
        };
        let (subscriber, rx) = file_watcher.add_subscriber();
        let _registration = subscriber.register_paths(vec![WatchPath {
            path: codex_home.clone(),
            recursive: false,
        }]);
        let config_path = codex_home.join(CONFIG_TOML_FILE);
        let mut current_theme = initial_theme;
        let mut rx = DebouncedWatchReceiver::new(rx, THEME_CONFIG_DEBOUNCE);
        let _ = ready_tx.send(());

        while let Some(event) = rx.recv().await {
            if !event_targets_config(&event.paths, &codex_home, &config_path) {
                continue;
            }

            let next_theme = match read_theme(&config_path).await {
                Ok(theme) => theme,
                Err(err) if err.kind() == ErrorKind::NotFound => None,
                Err(err) => {
                    tracing::warn!(
                        path = %config_path.display(),
                        "failed to reload syntax theme from config: {err}"
                    );
                    continue;
                }
            };
            if next_theme == current_theme {
                continue;
            }

            current_theme = next_theme.clone();
            app_event_tx.send(AppEvent::SyntaxThemeConfigChanged { name: next_theme });
        }
    });
    ready_rx
}

fn event_targets_config(paths: &[PathBuf], codex_home: &Path, config_path: &Path) -> bool {
    paths
        .iter()
        .any(|path| path == codex_home || path == config_path)
}

async fn read_theme(config_path: &Path) -> std::io::Result<Option<String>> {
    let contents = tokio::fs::read_to_string(config_path).await?;
    parse_theme(&contents).map_err(std::io::Error::other)
}

fn parse_theme(contents: &str) -> Result<Option<String>, toml::de::Error> {
    let config: ThemeConfig = toml::from_str(contents)?;
    Ok(config.tui.and_then(|tui| tui.theme))
}

#[cfg(test)]
#[path = "theme_watcher_tests.rs"]
mod tests;
