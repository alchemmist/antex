use std::cmp::Reverse;
use std::path::Path;
use std::path::PathBuf;

use chrono::DateTime;
use chrono::Utc;
use serde::Deserialize;
use serde::Serialize;
use serde_json::Map;
use serde_json::Value;
use tokio::fs;
use uuid::Uuid;

use super::WorkflowDefinition;
use super::WorkflowManifest;

const RUN_METADATA_FILE: &str = "run.json";
const RUN_SCRIPT_FILE: &str = "workflow.py";
const PARAMS_FILE: &str = "params.json";
const STATE_FILE: &str = "state.json";
const MAX_STORED_RUNS: usize = 100;

#[derive(Clone, Copy, Debug, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub(crate) enum WorkflowRunStatus {
    Running,
    Paused,
    Failed,
    Cancelled,
    Completed,
}

impl WorkflowRunStatus {
    pub(crate) fn resumable(self) -> bool {
        matches!(
            self,
            Self::Running | Self::Paused | Self::Failed | Self::Cancelled
        )
    }
}

#[derive(Clone, Debug, PartialEq, Serialize, Deserialize)]
pub(crate) struct StoredWorkflowRun {
    pub(crate) run_id: String,
    pub(crate) manifest: WorkflowManifest,
    pub(crate) source: String,
    pub(crate) created_at: DateTime<Utc>,
    pub(crate) updated_at: DateTime<Utc>,
    pub(crate) status: WorkflowRunStatus,
    pub(crate) params: Map<String, Value>,
    #[serde(default)]
    pub(crate) state: Value,
    #[serde(default)]
    pub(crate) error: Option<String>,
    #[serde(default)]
    pub(crate) agent_calls: u32,
    #[serde(default)]
    pub(crate) shell_calls: u32,
    #[serde(default)]
    pub(crate) last_activity: Option<String>,
    #[serde(default)]
    pub(crate) last_activity_at: Option<DateTime<Utc>>,
    pub(crate) run_dir: PathBuf,
}

impl StoredWorkflowRun {
    pub(crate) fn script_path(&self) -> PathBuf {
        self.run_dir.join(RUN_SCRIPT_FILE)
    }

    pub(crate) fn params_path(&self) -> PathBuf {
        self.run_dir.join(PARAMS_FILE)
    }

    pub(crate) fn state_path(&self) -> PathBuf {
        self.run_dir.join(STATE_FILE)
    }

    fn metadata_path(&self) -> PathBuf {
        self.run_dir.join(RUN_METADATA_FILE)
    }

    pub(crate) async fn persist(&self) -> Result<(), String> {
        write_json(&self.metadata_path(), self).await?;
        write_json(&self.params_path(), &self.params).await?;
        write_json(&self.state_path(), &self.state).await
    }

    pub(crate) async fn checkpoint(&mut self, state: Value) -> Result<(), String> {
        let encoded = serde_json::to_vec(&state)
            .map_err(|err| format!("failed to encode workflow checkpoint: {err}"))?;
        if encoded.len() > 1_048_576 {
            return Err("workflow checkpoint exceeds 1 MiB".to_string());
        }
        self.state = state;
        self.updated_at = Utc::now();
        fs::write(self.state_path(), encoded)
            .await
            .map_err(|err| format!("failed to write workflow checkpoint: {err}"))?;
        write_json(&self.metadata_path(), self).await
    }

    pub(crate) async fn set_status(
        &mut self,
        status: WorkflowRunStatus,
        error: Option<String>,
    ) -> Result<(), String> {
        self.status = status;
        self.error = error;
        self.updated_at = Utc::now();
        write_json(&self.metadata_path(), self).await
    }

    pub(crate) async fn record_calls(
        &mut self,
        agent_delta: u32,
        shell_delta: u32,
    ) -> Result<(), String> {
        self.agent_calls = self.agent_calls.saturating_add(agent_delta);
        self.shell_calls = self.shell_calls.saturating_add(shell_delta);
        self.updated_at = Utc::now();
        write_json(&self.metadata_path(), self).await
    }

    pub(crate) async fn record_activity(&mut self, activity: String) -> Result<(), String> {
        let now = Utc::now();
        self.last_activity = Some(activity);
        self.last_activity_at = Some(now);
        self.updated_at = now;
        write_json(&self.metadata_path(), self).await
    }
}

pub(crate) async fn create_run(
    codex_home: &Path,
    definition: WorkflowDefinition,
    params: Map<String, Value>,
) -> Result<StoredWorkflowRun, String> {
    let now = Utc::now();
    let run_id = format!(
        "{}-{}",
        now.format("%Y%m%d-%H%M%S"),
        &Uuid::new_v4().simple().to_string()[..8]
    );
    let run_dir = codex_home.join("workflow-runs").join(&run_id);
    fs::create_dir_all(&run_dir)
        .await
        .map_err(|err| format!("failed to create workflow run directory: {err}"))?;
    fs::copy(&definition.script_path, run_dir.join(RUN_SCRIPT_FILE))
        .await
        .map_err(|err| format!("failed to snapshot workflow source: {err}"))?;
    let run = StoredWorkflowRun {
        run_id,
        manifest: definition.manifest,
        source: definition.source,
        created_at: now,
        updated_at: now,
        status: WorkflowRunStatus::Running,
        params,
        state: Value::Object(Map::new()),
        error: None,
        agent_calls: 0,
        shell_calls: 0,
        last_activity: None,
        last_activity_at: None,
        run_dir,
    };
    run.persist().await?;
    Ok(run)
}

pub(crate) async fn list_resumable_runs(
    codex_home: &Path,
) -> Result<Vec<StoredWorkflowRun>, String> {
    let root = codex_home.join("workflow-runs");
    let mut entries = match fs::read_dir(&root).await {
        Ok(entries) => entries,
        Err(err) if err.kind() == std::io::ErrorKind::NotFound => return Ok(Vec::new()),
        Err(err) => return Err(format!("failed to read {}: {err}", root.display())),
    };
    let mut runs = Vec::new();
    while let Some(entry) = entries
        .next_entry()
        .await
        .map_err(|err| format!("failed to scan workflow runs: {err}"))?
    {
        let metadata_path = entry.path().join(RUN_METADATA_FILE);
        let Ok(bytes) = fs::read(&metadata_path).await else {
            continue;
        };
        let Ok(mut run) = serde_json::from_slice::<StoredWorkflowRun>(&bytes) else {
            continue;
        };
        run.run_dir = entry.path();
        if run.status.resumable() {
            runs.push(run);
        }
    }
    runs.sort_by_key(|run| Reverse(run.updated_at));
    runs.truncate(MAX_STORED_RUNS);
    Ok(runs)
}

async fn write_json(path: &Path, value: &impl Serialize) -> Result<(), String> {
    let bytes = serde_json::to_vec_pretty(value)
        .map_err(|err| format!("failed to encode {}: {err}", path.display()))?;
    fs::write(path, bytes)
        .await
        .map_err(|err| format!("failed to write {}: {err}", path.display()))
}

#[cfg(test)]
#[path = "state_tests.rs"]
mod tests;
