use std::path::PathBuf;

use pretty_assertions::assert_eq;
use tempfile::TempDir;
use tokio::sync::mpsc::unbounded_channel;
use tokio::time::timeout;

use super::*;

#[test]
fn parses_only_tui_theme_from_full_config() {
    let config = r#"
model = "gpt-5"

[tui]
animations = true
theme = "gruvbox-light"

[projects."/tmp/project"]
trust_level = "trusted"
"#;

    assert_eq!(
        parse_theme(config).expect("theme config should parse"),
        Some("gruvbox-light".to_string()),
    );
}

#[test]
fn missing_tui_theme_selects_automatic_theme() {
    assert_eq!(
        parse_theme("model = \"gpt-5\"\n").expect("config without tui should parse"),
        None,
    );
    assert_eq!(
        parse_theme("[tui]\nanimations = true\n").expect("config without a theme should parse"),
        None,
    );
}

#[test]
fn config_change_filter_accepts_file_and_parent_events() {
    let codex_home = PathBuf::from("/tmp/antex-home");
    let config_path = codex_home.join(CONFIG_TOML_FILE);

    assert_eq!(
        (
            event_targets_config(
                std::slice::from_ref(&config_path),
                &codex_home,
                &config_path
            ),
            event_targets_config(std::slice::from_ref(&codex_home), &codex_home, &config_path,),
            event_targets_config(
                &[codex_home.join("history.jsonl")],
                &codex_home,
                &config_path,
            ),
        ),
        (true, true, false),
    );
}

#[tokio::test]
async fn atomically_replaced_config_emits_theme_change() {
    let codex_home = TempDir::new().expect("temporary codex home");
    let config_path = codex_home.path().join(CONFIG_TOML_FILE);
    tokio::fs::write(&config_path, "[tui]\ntheme = \"gruvbox-dark\"\n")
        .await
        .expect("write initial config");
    let (event_tx, mut event_rx) = unbounded_channel();
    let ready = spawn_with_ready(
        codex_home.path().to_path_buf(),
        Some("gruvbox-dark".to_string()),
        AppEventSender::new(event_tx),
    );
    timeout(Duration::from_secs(2), ready)
        .await
        .expect("watcher startup timeout")
        .expect("watcher should start");

    let replacement_path = codex_home.path().join("config.toml.new");
    tokio::fs::write(&replacement_path, "[tui]\ntheme = \"gruvbox-light\"\n")
        .await
        .expect("write replacement config");
    tokio::fs::rename(replacement_path, config_path)
        .await
        .expect("atomically replace config");

    let event = timeout(Duration::from_secs(2), event_rx.recv())
        .await
        .expect("theme change event timeout")
        .expect("theme change event");
    assert!(matches!(
        event,
        AppEvent::SyntaxThemeConfigChanged { name }
            if name.as_deref() == Some("gruvbox-light")
    ));
}
