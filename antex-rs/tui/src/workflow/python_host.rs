use std::path::Path;
use std::process::Stdio;
use std::time::Duration;

use tokio::process::Command;

use super::WorkflowManifest;

pub(super) const PYTHON_HOST: &str = include_str!("python_host.py");
const DESCRIBE_TIMEOUT: Duration = Duration::from_secs(8);
const MAX_DESCRIBE_OUTPUT: usize = 1_048_576;

pub(super) fn python_program() -> String {
    std::env::var("ANTEX_WORKFLOW_PYTHON").unwrap_or_else(|_| "python3".to_string())
}

pub(super) async fn describe_workflow(path: &Path) -> Result<WorkflowManifest, String> {
    let child = Command::new(python_program())
        .arg("-u")
        .arg("-c")
        .arg(PYTHON_HOST)
        .arg("describe")
        .arg(path)
        .stdin(Stdio::null())
        .stdout(Stdio::piped())
        .stderr(Stdio::piped())
        .kill_on_drop(true)
        .spawn()
        .map_err(|err| format!("failed to start Python: {err}"))?;
    let output = tokio::time::timeout(DESCRIBE_TIMEOUT, child.wait_with_output())
        .await
        .map_err(|_| "workflow description timed out".to_string())?
        .map_err(|err| format!("failed to read Python output: {err}"))?;
    if !output.status.success() {
        let stderr = String::from_utf8_lossy(&output.stderr);
        return Err(format!("Python rejected the workflow: {}", stderr.trim()));
    }
    if output.stdout.len() > MAX_DESCRIBE_OUTPUT {
        return Err("workflow manifest exceeds 1 MiB".to_string());
    }
    let manifest: WorkflowManifest = serde_json::from_slice(&output.stdout)
        .map_err(|err| format!("invalid workflow manifest JSON: {err}"))?;
    manifest.validate()?;
    Ok(manifest)
}
