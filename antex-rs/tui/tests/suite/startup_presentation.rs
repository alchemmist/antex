use super::focus_palette::PtyAntex;
use super::focus_palette::write_test_config;
use anyhow::Result;
use anyhow::ensure;
use std::time::Duration;
use std::time::Instant;

#[test]
fn startup_warning_preserves_shell_output_above_the_command() -> Result<()> {
    let workspace = tempfile::tempdir()?;
    let workspace = workspace.path().canonicalize()?;
    let home = tempfile::tempdir()?;
    write_test_config(home.path(), &workspace)?;
    let mut terminal = PtyAntex::start_with_history(
        &workspace,
        home,
        &["-c", "unknown_startup_probe=true"],
        "PREVIOUS SHELL OUTPUT\r\n$ antex\r\n",
    )?;
    terminal.wait_for_screen("startup issue")?;
    ensure!(
        terminal.screen_contains("PREVIOUS SHELL OUTPUT") && terminal.screen_contains("$ antex"),
        "startup warning erased shell output: {}",
        terminal.screen_contents(),
    );
    Ok(())
}

#[test]
fn startup_mascot_finishes_animation_after_backend_is_ready() -> Result<()> {
    let workspace = tempfile::tempdir()?;
    let workspace = workspace.path().canonicalize()?;
    let home = tempfile::tempdir()?;
    write_test_config(home.path(), &workspace)?;
    let mut terminal = PtyAntex::start_with_history(
        &workspace,
        home,
        &["-c", "tui.animations=true"],
        "PREVIOUS SHELL OUTPUT\r\n$ antex\r\n",
    )?;
    terminal.wait_for_startup()?;
    let started = Instant::now();
    let mut previous = String::new();
    let mut last_change = Duration::ZERO;
    while started.elapsed() < Duration::from_secs(2) {
        terminal.read_output(Duration::from_millis(20))?;
        let mascot = mascot_contents(&terminal);
        if !mascot.is_empty() && mascot != previous {
            last_change = started.elapsed();
            previous = mascot;
        }
    }
    ensure!(
        last_change >= Duration::from_secs(1),
        "mascot stopped after {last_change:?}: {}",
        terminal.screen_contents(),
    );
    terminal.ensure_running()?;
    Ok(())
}

fn mascot_contents(terminal: &PtyAntex) -> String {
    terminal
        .screen_contents()
        .lines()
        .map(|line| line.chars().take(13).collect::<String>())
        .filter(|line| line.contains(['▀', '▄', '█']))
        .take(6)
        .collect::<Vec<_>>()
        .join("\n")
}

#[test]
fn startup_mascot_obeys_disabled_animations() -> Result<()> {
    let workspace = tempfile::tempdir()?;
    let workspace = workspace.path().canonicalize()?;
    let home = tempfile::tempdir()?;
    write_test_config(home.path(), &workspace)?;
    let mut terminal = PtyAntex::start(&workspace, home, &["-c", "tui.animations=false"])?;
    terminal.wait_for_screen("▀▀▄▄▀▀▀▀▀▄▄▀▀")?;
    let before = mascot_contents(&terminal);
    let started = Instant::now();
    while started.elapsed() < Duration::from_secs(2) {
        terminal.read_output(Duration::from_millis(20))?;
        let current = mascot_contents(&terminal);
        if !current.is_empty() {
            pretty_assertions::assert_eq!(current, before);
        }
    }
    Ok(())
}
