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
    // Keep lifecycle diagnostics in this test's temporary home, never on the PTY screen.
    // The coordinator can enable only antex_tui::startup_mascot=trace via RUST_LOG.
    let diagnostic_dir = home.path().join("mascot-diagnostic");
    let diagnostic_log = diagnostic_dir.join("antex-tui.log");
    let log_dir_override = format!(
        "log_dir={}",
        toml::Value::String(diagnostic_dir.display().to_string())
    );
    // Motion starts at the first loading-header render, not when wait_for_startup returns.
    // Keep a launch-relative clock across that wait, just as production keeps the same
    // motion handle across backend initialization. The unit tests cover its exact 8 x 180ms
    // duration; this PTY test covers visible motion surviving the ready-state handoff.
    let launched = Instant::now();
    let mut terminal = PtyAntex::start_with_history(
        &workspace,
        home,
        &["-c", "tui.animations=true", "-c", &log_dir_override],
        "PREVIOUS SHELL OUTPUT\r\n$ antex\r\n",
    )?;
    terminal.wait_for_startup()?;
    let started = Instant::now();
    let startup_wait = launched.elapsed();
    let mut previous = mascot_contents(terminal.screen());
    // The loading header says "loading". A configured model in the session header
    // establishes readiness, unlike wait_for_startup's generic Antex title check.
    let mut backend_ready = terminal.screen_contains("GPT-5.6-Terra");
    let mut observations = vec![(launched.elapsed(), backend_ready, previous.clone())];
    let mut changed_after_ready = false;
    let mut last_change = Duration::ZERO;
    while started.elapsed() < Duration::from_secs(2) {
        terminal.read_output(Duration::from_millis(20))?;
        let mascot = mascot_contents(terminal.screen());
        let changed = !mascot.is_empty() && mascot != previous;
        let ready_now = terminal.screen_contains("GPT-5.6-Terra");
        if (changed || (!backend_ready && ready_now)) && observations.len() < 32 {
            observations.push((launched.elapsed(), ready_now, mascot.clone()));
        }
        if changed {
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
    let motion_diagnostics = std::fs::read_to_string(&diagnostic_log)
        .map(|log| {
            let handoff = log
                .lines()
                .filter(|line| {
                    line.contains("antex_tui::startup_mascot:")
                        && !line.contains("startup mascot frame lookup")
                        && !line.contains("startup mascot refresh")
                })
                .take(32)
                .collect::<Vec<_>>()
                .join("\n");
            let frames = log
                .lines()
                .filter(|line| {
                    line.contains("antex_tui::startup_mascot:")
                        && line.contains("startup mascot frame lookup")
                })
                .rev()
                .take(32)
                .collect::<Vec<_>>()
                .join("\n");
            let refreshes = log
                .lines()
                .filter(|line| {
                    line.contains("antex_tui::startup_mascot:")
                        && line.contains("startup mascot refresh")
                })
                .rev()
                .take(32)
                .collect::<Vec<_>>()
                .join("\n");
            if handoff.is_empty() && frames.is_empty() && refreshes.is_empty() {
                return "no mascot lifecycle records captured; enable \
                        RUST_LOG=antex_tui::startup_mascot=trace for the coordinator diagnostic gate"
                    .to_string();
            }
            format!(
                "handoff:\n{handoff}\nlatest frame lookups first:\n{frames}\n\
                 latest refreshes first:\n{refreshes}"
            )
        })
        .unwrap_or_else(|error| format!("mascot diagnostic log unavailable: {error}"));
    ensure!(
        backend_ready && changed_after_ready,
        "no mascot transition after backend readiness (startup wait {startup_wait:?}; observations {observations:?}; lifecycle records:\n{motion_diagnostics}): {}",
        terminal.screen_contents(),
    );
    ensure!(
        last_change >= Duration::from_secs(1),
        "mascot stopped at {last_change:?} since launch (startup wait {startup_wait:?}; observations {observations:?}; lifecycle records:\n{motion_diagnostics}): {}",
        terminal.screen_contents(),
    );
    terminal.ensure_running()?;
    Ok(())
}

#[derive(Clone, Debug, Eq, PartialEq)]
struct MascotPixel {
    column: u16,
    glyph: String,
    foreground: vt100::Color,
    background: vt100::Color,
}

fn mascot_contents(screen: &vt100::Screen) -> Vec<Vec<MascotPixel>> {
    // Half-block pixels encode two colors. In particular, the blink changes the
    // eye's background color without changing its glyph. Ignore surrounding text
    // and blank-cell styling so only changes to the mascot satisfy the assertion.
    (0..screen.size().0)
        .map(|row| {
            (0..13)
                .filter_map(|column| {
                    let cell = screen.cell(row, column)?;
                    let glyph = cell.contents();
                    if !glyph.contains(['▀', '▄', '█']) {
                        return None;
                    }
                    Some(MascotPixel {
                        column,
                        glyph: glyph.to_owned(),
                        foreground: cell.fgcolor(),
                        background: cell.bgcolor(),
                    })
                })
                .collect::<Vec<_>>()
        })
        .filter(|row| !row.is_empty())
        .take(6)
        .collect()
}

#[test]
fn mascot_observer_detects_color_only_motion_not_metadata_or_repaints() {
    let mut parser =
        vt100::Parser::new(/*rows*/ 8, /*cols*/ 80, /*scrollback_len*/ 0);
    parser.process(b"\x1b[38;2;120;60;30m\x1b[48;2;20;100;100m\xe2\x96\x80");
    let rest = mascot_contents(parser.screen());
    let rest_text = parser.screen().contents();
    assert!(!rest.is_empty());

    // A half-block eye blinks by changing only its lower pixel (background).
    parser.process(b"\x1b[H\x1b[48;2;60;30;10m\xe2\x96\x80");
    pretty_assertions::assert_eq!(parser.screen().contents(), rest_text);
    let blink = mascot_contents(parser.screen());
    pretty_assertions::assert_ne!(blink, rest);

    // Repainting the same pose or changing ready-header text is not motion.
    parser.process(b"\x1b[H\xe2\x96\x80\x1b[1;20HGPT-5.6-Terra");
    pretty_assertions::assert_eq!(mascot_contents(parser.screen()), blink);
    parser.process(b"\x1b[H\x1b[48;2;20;100;100m\xe2\x96\x80");
    pretty_assertions::assert_eq!(mascot_contents(parser.screen()), rest);
}

#[test]
fn startup_mascot_obeys_disabled_animations() -> Result<()> {
    let workspace = tempfile::tempdir()?;
    let workspace = workspace.path().canonicalize()?;
    let home = tempfile::tempdir()?;
    write_test_config(home.path(), &workspace)?;
    let mut terminal = PtyAntex::start(&workspace, home, &["-c", "tui.animations=false"])?;
    terminal.wait_for_screen("▀▀▄▄▀▀▀▀▀▄▄▀▀")?;
    let before = mascot_contents(terminal.screen());
    let started = Instant::now();
    while started.elapsed() < Duration::from_secs(2) {
        terminal.read_output(Duration::from_millis(20))?;
        let current = mascot_contents(terminal.screen());
        if !current.is_empty() {
            pretty_assertions::assert_eq!(current, before);
        }
    }
    Ok(())
}
