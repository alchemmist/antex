use anyhow::Result;
use tempfile::TempDir;

const THREAD_ID: &str = "123e4567-e89b-12d3-a456-426614174000";

#[test]
fn queue_rejects_empty_message() -> Result<()> {
    let home = TempDir::new()?;
    let output = std::process::Command::new(antex_utils_cargo_bin::cargo_bin("antex")?)
        .env("ANTEX_HOME", home.path())
        .args(["queue", "--thread", THREAD_ID, "--message", ""])
        .output()?;
    assert!(!output.status.success());
    Ok(())
}

#[test]
fn queue_rejects_image_attachments() -> Result<()> {
    let home = TempDir::new()?;
    let output = std::process::Command::new(antex_utils_cargo_bin::cargo_bin("antex")?)
        .env("ANTEX_HOME", home.path())
        .args([
            "queue",
            "--thread",
            THREAD_ID,
            "--message",
            "do the thing",
            "--image",
            "screenshot.png",
        ])
        .output()?;
    assert!(!output.status.success());
    assert!(String::from_utf8(output.stderr)?.contains("does not support image attachments"));
    Ok(())
}
