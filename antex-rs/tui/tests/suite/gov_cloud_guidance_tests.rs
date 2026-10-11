//! Verify startup acknowledgment, quit without saving, and restart suppression.

use super::*;
use pretty_assertions::assert_eq;

#[test]
fn gov_cloud_startup_persists_only_acknowledgment() -> Result<()> {
    let cwd = antex_utils_cargo_bin::repo_root()?;
    let args = [
        "--no-daemon",
        "-c",
        "features.api_key_model_discovery=false",
    ];
    for action in ["quit", "acknowledge"] {
        let home = tempfile::tempdir()?;
        write_test_config(home.path(), &cwd)?;
        let path = home.path().join("config.toml");
        let config = std::fs::read_to_string(&path)?.replace(
            "model_provider = \"openai\"",
            "model_provider = \"amazon-bedrock\"",
        );
        let config = format!(
            "{config}\n[model_providers.amazon-bedrock]\nbase_url = 'https://bedrock-mantle.us-gov-west-1.api.aws/openai/v1'\n"
        );
        std::fs::write(&path, &config)?;
        let mut terminal = PtyAntex::start(&cwd, home, &args)?;
        terminal.wait_for_screen("Using Antex with AWS GovCloud")?;
        let original = std::fs::read_to_string(&path)?;
        let deadline = Instant::now() + STARTUP_TIMEOUT;
        loop {
            // Startup drains pending keys after drawing the guidance; retry until it handles one.
            terminal.write_input(if action == "quit" { b"q" } else { b"\r" })?;
            terminal.read_output(Duration::from_millis(/*millis*/ 50))?;
            if action == "quit" {
                if let Some(status) = terminal.child.try_wait()? {
                    ensure!(status.success(), "quit exited with {status}");
                    break;
                }
            } else if terminal.screen_contains("alchemmist antex") {
                break;
            }
            ensure!(Instant::now() < deadline, "{action} did not complete");
        }
        let saved = std::fs::read_to_string(&path)?;
        if action == "acknowledge" {
            let config: toml::Value = toml::from_str(&saved)?;
            assert_eq!(
                config["notice"]["hide_gov_cloud_guidance"].as_bool(),
                Some(true)
            );
            let home = std::mem::replace(&mut terminal._antex_home, tempfile::tempdir()?);
            drop(terminal);
            let mut restarted = PtyAntex::start(&cwd, home, &args)?;
            // The startup draft says "loading". With Bedrock model discovery disabled,
            // the ready header shows the configured slug, not OpenAI's catalog label.
            let model = config["model"].as_str().expect("configured model");
            restarted.wait_for_screen(model)?;
            assert!(!restarted.screen_contains("Using Antex with AWS GovCloud"));
        } else {
            assert_eq!(saved, original);
        }
    }
    Ok(())
}
