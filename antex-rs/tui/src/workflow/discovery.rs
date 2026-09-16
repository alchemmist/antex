use std::collections::BTreeMap;
use std::path::Path;
use std::path::PathBuf;

use tokio::fs;

use super::BUILTIN_GITHUB_BOT_PR_WORKFLOW_ID;
use super::BUILTIN_PR_BABYSITTER_WORKFLOW_ID;
use super::BUILTIN_RUFF_WORKFLOW_ID;
use super::WorkflowDefinition;
use super::python_host::describe_workflow;

const BUILTIN_RUFF_WORKFLOW: &str = include_str!("builtin_ruff.py");
const BUILTIN_GITHUB_BOT_PR_WORKFLOW: &str = include_str!("builtin_github_bot_pr_maintenance.py");
const BUILTIN_PR_BABYSITTER_WORKFLOW: &str = include_str!("builtin_pr_babysitter.py");
const BUILTIN_WORKFLOWS: [(&str, u32, &str); 3] = [
    (BUILTIN_RUFF_WORKFLOW_ID, 1, BUILTIN_RUFF_WORKFLOW),
    (
        BUILTIN_GITHUB_BOT_PR_WORKFLOW_ID,
        1,
        BUILTIN_GITHUB_BOT_PR_WORKFLOW,
    ),
    (
        BUILTIN_PR_BABYSITTER_WORKFLOW_ID,
        1,
        BUILTIN_PR_BABYSITTER_WORKFLOW,
    ),
];

pub(crate) async fn discover_workflows(
    codex_home: &Path,
    cwd: &Path,
) -> (Vec<WorkflowDefinition>, Vec<String>) {
    let mut candidates = Vec::new();
    let mut diagnostics = Vec::new();

    for (id, version, source) in BUILTIN_WORKFLOWS {
        match materialize_builtin_workflow(codex_home, id, version, source).await {
            Ok(path) => candidates.push((path, "built-in".to_string(), 0u8)),
            Err(err) => diagnostics.push(err),
        }
    }
    collect_python_files(
        &codex_home.join("workflows"),
        "user",
        /*priority*/ 1,
        &mut candidates,
        &mut diagnostics,
    )
    .await;
    collect_python_files(
        &cwd.join(".antex/workflows"),
        "project",
        /*priority*/ 2,
        &mut candidates,
        &mut diagnostics,
    )
    .await;

    let mut definitions = BTreeMap::<String, (u8, WorkflowDefinition)>::new();
    for (path, source, priority) in candidates {
        match describe_workflow(&path).await {
            Ok(manifest) => {
                let definition = WorkflowDefinition {
                    manifest: manifest.clone(),
                    script_path: path,
                    source,
                };
                match definitions.get(&manifest.id) {
                    Some((existing_priority, _)) if *existing_priority > priority => {}
                    _ => {
                        definitions.insert(manifest.id, (priority, definition));
                    }
                }
            }
            Err(err) => diagnostics.push(format!("{}: {err}", path.display())),
        }
    }

    let mut definitions = definitions
        .into_values()
        .map(|(_, definition)| definition)
        .collect::<Vec<_>>();
    definitions.sort_by(|left, right| left.manifest.title.cmp(&right.manifest.title));
    (definitions, diagnostics)
}

async fn materialize_builtin_workflow(
    codex_home: &Path,
    id: &str,
    version: u32,
    source: &str,
) -> Result<PathBuf, String> {
    let cache_dir = codex_home.join("workflow-cache");
    fs::create_dir_all(&cache_dir)
        .await
        .map_err(|err| format!("failed to create workflow cache: {err}"))?;
    let path = cache_dir.join(format!("{id}-v{version}.py"));
    let current = fs::read_to_string(&path).await.ok();
    if current.as_deref() != Some(source) {
        fs::write(&path, source)
            .await
            .map_err(|err| format!("failed to materialize built-in workflow `{id}`: {err}"))?;
    }
    Ok(path)
}

async fn collect_python_files(
    directory: &Path,
    source: &str,
    priority: u8,
    candidates: &mut Vec<(PathBuf, String, u8)>,
    diagnostics: &mut Vec<String>,
) {
    let mut entries = match fs::read_dir(directory).await {
        Ok(entries) => entries,
        Err(err) if err.kind() == std::io::ErrorKind::NotFound => return,
        Err(err) => {
            diagnostics.push(format!("failed to read {}: {err}", directory.display()));
            return;
        }
    };
    loop {
        match entries.next_entry().await {
            Ok(Some(entry)) => {
                let path = entry.path();
                let is_python = path.extension().is_some_and(|extension| extension == "py");
                let hidden = path
                    .file_name()
                    .and_then(|name| name.to_str())
                    .is_some_and(|name| name.starts_with('_') || name.starts_with('.'));
                if is_python && !hidden {
                    candidates.push((path, source.to_string(), priority));
                }
            }
            Ok(None) => break,
            Err(err) => {
                diagnostics.push(format!("failed to scan {}: {err}", directory.display()));
                break;
            }
        }
    }
}

#[cfg(test)]
#[path = "discovery_tests.rs"]
mod tests;
