use anyhow::Result;
use predicates::str::contains;
use std::path::Path;
use tempfile::TempDir;

fn antex_command(codex_home: &Path) -> Result<assert_cmd::Command> {
    let mut cmd = assert_cmd::Command::new(antex_utils_cargo_bin::cargo_bin("antex")?);
    cmd.env("ANTEX_HOME", codex_home);
    Ok(cmd)
}

#[cfg(debug_assertions)]
#[tokio::test]
async fn update_does_not_start_interactive_prompt() -> Result<()> {
    let codex_home = TempDir::new()?;

    antex_command(codex_home.path())?
        .arg("update")
        .assert()
        .failure()
        .stderr(contains("`antex update` is not available in debug builds"));

    Ok(())
}
