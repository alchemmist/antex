use std::collections::HashMap;
use std::path::Path;
use std::path::PathBuf;
use std::process::Stdio;
use std::time::Duration;

use antex_config::McpServerActivation;
use tokio::io::AsyncReadExt;

use crate::server::EffectiveMcpServer;

pub(crate) async fn apply_activation(
    servers: &mut HashMap<String, EffectiveMcpServer>,
    cwd: &Path,
) {
    if !servers
        .values()
        .any(|server| server.enabled() && server.config().activation.is_some())
    {
        return;
    }
    let cwd = tokio::fs::canonicalize(cwd).await.ok();
    let needs_arc = servers.values().any(|server| {
        server.enabled()
            && matches!(
                server.config().activation,
                Some(McpServerActivation::Arcadia {})
            )
    });
    let arcadia = if needs_arc && let Some(cwd) = &cwd {
        arcadia_root(cwd).await.is_some()
    } else {
        false
    };
    for server in servers.values_mut().filter(|server| server.enabled()) {
        let active = match &server.config().activation {
            None => true,
            Some(McpServerActivation::Arcadia {}) => arcadia,
            Some(McpServerActivation::Directories { roots }) => {
                let mut active = false;
                if let Some(cwd) = &cwd {
                    for root in roots {
                        if let Ok(root) = tokio::fs::canonicalize(root.as_path()).await
                            && cwd.starts_with(root)
                        {
                            active = true;
                            break;
                        }
                    }
                }
                active
            }
        };
        if !active {
            server.disable_for_activation();
        }
    }
}

async fn arcadia_root(cwd: &Path) -> Option<PathBuf> {
    tokio::time::timeout(Duration::from_secs(/*secs*/ 4), async {
        let mut child = tokio::process::Command::new("arc")
            .arg("root")
            .current_dir(cwd)
            .stdin(Stdio::null())
            .stdout(Stdio::piped())
            .stderr(Stdio::null())
            .kill_on_drop(true)
            .spawn()
            .ok()?;
        let mut output = Vec::new();
        child
            .stdout
            .take()?
            .take(/*limit*/ 4097)
            .read_to_end(&mut output)
            .await
            .ok()?;
        if output.len() > 4096 || !child.wait().await.ok()?.success() {
            return None;
        }
        verified_arcadia_root(cwd, &output).await
    })
    .await
    .ok()
    .flatten()
}

async fn verified_arcadia_root(cwd: &Path, output: &[u8]) -> Option<PathBuf> {
    let root = Path::new(std::str::from_utf8(output).ok()?.trim());
    if !root.is_absolute() || root.parent().is_none() {
        return None;
    }
    let root = tokio::fs::canonicalize(root).await.ok()?;
    cwd.starts_with(&root).then_some(root)
}

#[cfg(test)]
#[path = "activation_tests.rs"]
mod tests;
