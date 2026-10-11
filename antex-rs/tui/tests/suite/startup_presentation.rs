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
        &["--no-alt-screen", "-c", "unknown_startup_probe=true"],
        "PREVIOUS SHELL OUTPUT\r\n$ antex\r\n",
    )?;
    terminal.wait_for_screen("1 warning")?;
    ensure!(
        terminal.screen_contains("PREVIOUS SHELL OUTPUT") && terminal.screen_contains("$ antex"),
        "startup warning erased shell output: {}",
        terminal.screen_contents(),
    );
    terminal.write_input(b"\x1bOQ")?; // F2 opens the upstream warning inspector.
    terminal.wait_for_screen("unknown_startup_probe")?;
    Ok(())
}

#[test]
fn startup_mascot_finishes_animation_after_backend_is_ready() -> Result<()> {
    let workspace = tempfile::tempdir()?;
    let workspace = workspace.path().canonicalize()?;
    let home = tempfile::tempdir()?;
    write_test_config(home.path(), &workspace)?;
    // Motion starts at the first loading-header render, not when wait_for_startup returns.
    // Keep a launch-relative clock across that wait, just as production keeps the same
    // motion handle across backend initialization. The unit tests cover its exact 8 x 180ms
    // duration; this PTY test covers visible motion surviving the ready-state handoff.
    let launched = Instant::now();
    let mut terminal = PtyAntex::start_with_history(
        &workspace,
        home,
        &["-c", "tui.animations=true"],
        "PREVIOUS SHELL OUTPUT\r\n$ antex\r\n",
    )?;
    terminal.wait_for_startup()?;
    let started = Instant::now();
    let startup_wait = launched.elapsed();
    let mut previous = mascot_contents(&terminal);
    // The loading header says "loading". A configured model in the session header
    // establishes readiness, unlike wait_for_startup's generic Antex title check.
    let mut backend_ready = terminal.screen_contains("GPT-5.6-Terra");
    let mut changed_after_ready = false;
    let mut last_change = Duration::ZERO;
    while started.elapsed() < Duration::from_secs(2) {
        terminal.read_output(Duration::from_millis(20))?;
        let mascot = mascot_contents(&terminal);
        if !mascot.is_empty() && mascot != previous {
            // Do not count the initial appearance as animation. In particular, slow
            // startup followed by a static mascot must not satisfy the duration check.
            if !previous.is_empty() {
                last_change = launched.elapsed();
                changed_after_ready |= backend_ready;
            }
            previous = mascot;
        }
        backend_ready |= terminal.screen_contains("GPT-5.6-Terra");
    }
    ensure!(
        backend_ready && changed_after_ready,
        "no mascot transition after backend readiness (startup wait {startup_wait:?}): {}",
        terminal.screen_contents(),
    );
    ensure!(
        last_change >= Duration::from_secs(1),
        "mascot stopped at {last_change:?} since launch (startup wait {startup_wait:?}): {}",
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
