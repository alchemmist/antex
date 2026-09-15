use std::sync::Arc;

use serde::Serialize;
use serde_json::Value;
use tokio::io::AsyncBufReadExt;
use tokio::io::AsyncWriteExt;
use tokio::io::BufReader;
use tokio::sync::Mutex;

#[derive(Serialize)]
struct WorkflowResponse<T> {
    id: u64,
    ok: bool,
    result: Option<T>,
    error: Option<String>,
}

pub(super) async fn respond_ok(
    stdin: &mut tokio::process::ChildStdin,
    id: u64,
    result: impl Serialize,
) -> Result<(), String> {
    write_response(
        stdin,
        &WorkflowResponse {
            id,
            ok: true,
            result: Some(result),
            error: None,
        },
    )
    .await
}

pub(super) async fn respond_error(
    stdin: &mut tokio::process::ChildStdin,
    id: u64,
    error: impl Into<String>,
) -> Result<(), String> {
    write_response(
        stdin,
        &WorkflowResponse::<Value> {
            id,
            ok: false,
            result: None,
            error: Some(error.into()),
        },
    )
    .await
}

async fn write_response(
    stdin: &mut tokio::process::ChildStdin,
    response: &impl Serialize,
) -> Result<(), String> {
    let mut encoded = serde_json::to_vec(response)
        .map_err(|err| format!("failed to encode workflow response: {err}"))?;
    encoded.push(b'\n');
    stdin
        .write_all(&encoded)
        .await
        .map_err(|err| format!("failed to write workflow response: {err}"))
}

pub(super) async fn capture_stderr(
    stderr: impl tokio::io::AsyncRead + Unpin,
    tail: Arc<Mutex<String>>,
) -> std::io::Result<()> {
    let mut lines = BufReader::new(stderr).lines();
    while let Some(line) = lines.next_line().await? {
        tracing::debug!(target: "codex_workflow", "{line}");
        let mut tail = tail.lock().await;
        tail.push_str(&line);
        tail.push('\n');
        if tail.len() > 32_768 {
            let split = tail.len().saturating_sub(16_384);
            let split = (split..tail.len())
                .find(|index| tail.is_char_boundary(*index))
                .unwrap_or(split);
            tail.drain(..split);
        }
    }
    Ok(())
}
