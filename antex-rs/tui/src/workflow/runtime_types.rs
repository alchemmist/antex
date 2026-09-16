use std::path::PathBuf;

use serde_json::Value;

use super::StoredWorkflowRun;

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub(crate) enum WorkflowControl {
    Run,
    Pause,
    Cancel,
}

#[derive(Clone, Debug)]
pub(crate) struct WorkflowRunRequest {
    pub(crate) run: StoredWorkflowRun,
    pub(crate) workspace: PathBuf,
    pub(crate) antex_exe: PathBuf,
}

#[derive(Clone, Debug, PartialEq)]
pub(crate) enum WorkflowUpdate {
    Started {
        run_id: String,
        title: String,
    },
    Progress {
        run_id: String,
        message: String,
        current: Option<u64>,
        total: Option<u64>,
    },
    AgentBatchStarted {
        run_id: String,
        count: usize,
        parallelism: usize,
        phase: Option<String>,
        phase_current: Option<u64>,
        phase_total: Option<u64>,
    },
    AgentFinished {
        run_id: String,
        completed: usize,
        total: usize,
        success: bool,
        phase: Option<String>,
        phase_current: Option<u64>,
        phase_total: Option<u64>,
    },
    AgentActivity {
        run_id: String,
        agent: usize,
        total: usize,
        message: String,
        idle_seconds: u64,
        phase: Option<String>,
        phase_current: Option<u64>,
        phase_total: Option<u64>,
    },
    Checkpointed {
        run_id: String,
    },
    Completed {
        run_id: String,
        title: String,
        result: Value,
        agent_calls: u32,
        shell_calls: u32,
    },
    Paused {
        run_id: String,
        title: String,
    },
    Cancelled {
        run_id: String,
        title: String,
    },
    Failed {
        run_id: String,
        title: String,
        error: String,
    },
}

impl WorkflowUpdate {
    pub(crate) fn run_id(&self) -> &str {
        match self {
            Self::Started { run_id, .. }
            | Self::Progress { run_id, .. }
            | Self::AgentBatchStarted { run_id, .. }
            | Self::AgentFinished { run_id, .. }
            | Self::AgentActivity { run_id, .. }
            | Self::Checkpointed { run_id }
            | Self::Completed { run_id, .. }
            | Self::Paused { run_id, .. }
            | Self::Cancelled { run_id, .. }
            | Self::Failed { run_id, .. } => run_id,
        }
    }

    pub(crate) fn is_terminal(&self) -> bool {
        matches!(
            self,
            Self::Completed { .. }
                | Self::Paused { .. }
                | Self::Cancelled { .. }
                | Self::Failed { .. }
        )
    }
}
