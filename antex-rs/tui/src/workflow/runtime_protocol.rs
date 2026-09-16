use std::collections::HashMap;

use serde::Deserialize;
use serde_json::Value;

use super::executor::AgentApprovalMode;
use super::executor::AgentRequest;
use super::executor::AgentSandbox;

pub(super) const PROTOCOL_VERSION: u32 = 1;
pub(super) const MAX_PROTOCOL_LINE_BYTES: usize = 2_097_152;

#[derive(Debug, Deserialize)]
#[serde(tag = "type", rename_all = "snake_case")]
pub(super) enum WorkflowRequest {
    Progress {
        protocol_version: u32,
        id: u64,
        message: String,
        current: Option<u64>,
        total: Option<u64>,
    },
    Shell {
        protocol_version: u32,
        id: u64,
        argv: Vec<String>,
        cwd: Option<String>,
        timeout_seconds: Option<u64>,
        #[serde(default)]
        env: HashMap<String, String>,
    },
    Agent {
        protocol_version: u32,
        id: u64,
        prompt: String,
        model: Option<String>,
        #[serde(default)]
        reasoning_effort: Option<String>,
        #[serde(default)]
        developer_instructions: Option<String>,
        #[serde(default)]
        forbid_quality_graph_ignore: bool,
        #[serde(default)]
        sandbox: AgentSandbox,
        #[serde(default)]
        approval_mode: AgentApprovalMode,
        cwd: Option<String>,
        timeout_seconds: Option<u64>,
    },
    AgentBatch {
        protocol_version: u32,
        id: u64,
        requests: Vec<AgentRequest>,
        parallelism: Option<usize>,
        model: Option<String>,
        cwd: Option<String>,
        timeout_seconds: Option<u64>,
    },
    Checkpoint {
        protocol_version: u32,
        id: u64,
        state: Value,
    },
    Completed {
        protocol_version: u32,
        result: Option<Value>,
    },
    Failed {
        protocol_version: u32,
        error: String,
    },
}

impl WorkflowRequest {
    pub(super) fn protocol_version(&self) -> u32 {
        match self {
            Self::Progress {
                protocol_version, ..
            }
            | Self::Shell {
                protocol_version, ..
            }
            | Self::Agent {
                protocol_version, ..
            }
            | Self::AgentBatch {
                protocol_version, ..
            }
            | Self::Checkpoint {
                protocol_version, ..
            }
            | Self::Completed {
                protocol_version, ..
            }
            | Self::Failed {
                protocol_version, ..
            } => *protocol_version,
        }
    }
}
