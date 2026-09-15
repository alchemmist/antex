//! Python-authored workflows for long-running, repetitive agent tasks.
//!
//! Python owns workflow-specific control flow and input declarations. Rust owns discovery,
//! validation, persistence, process execution, and the TUI integration.

mod discovery;
mod executor;
mod protocol;
mod python_host;
mod runtime;
mod runtime_io;
mod runtime_protocol;
mod runtime_types;
mod state;

pub(crate) use discovery::discover_workflows;
pub(crate) use protocol::WorkflowDefinition;
pub(crate) use protocol::WorkflowField;
pub(crate) use protocol::WorkflowFieldKind;
pub(crate) use protocol::WorkflowManifest;
pub(crate) use runtime::run_workflow;
pub(crate) use runtime_types::WorkflowControl;
pub(crate) use runtime_types::WorkflowRunRequest;
pub(crate) use runtime_types::WorkflowUpdate;
pub(crate) use state::StoredWorkflowRun;
pub(crate) use state::WorkflowRunStatus;
pub(crate) use state::create_run;
pub(crate) use state::list_resumable_runs;

pub(crate) const BUILTIN_RUFF_WORKFLOW_ID: &str = "ruff-cleanup";
pub(crate) const BUILTIN_GITHUB_BOT_PR_WORKFLOW_ID: &str = "github-bot-pr-maintenance";
pub(crate) const BUILTIN_PR_BABYSITTER_WORKFLOW_ID: &str = "pr-babysitter";
