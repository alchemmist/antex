use std::path::PathBuf;
use std::process::Stdio;
use std::sync::Arc;
use std::time::Duration;
use std::time::Instant;

use serde_json::Value;
use tokio::io::AsyncBufReadExt;
use tokio::io::BufReader;
use tokio::process::Command;
use tokio::sync::Mutex;
use tokio::sync::mpsc;
use tokio::sync::watch;
use tokio::task::JoinSet;

use super::StoredWorkflowRun;
use super::WorkflowControl;
use super::WorkflowRunRequest;
use super::WorkflowRunStatus;
use super::WorkflowUpdate;
use super::executor::AgentActivity;
use super::executor::AgentExecutionContext;
use super::executor::AgentRequest;
use super::executor::AgentResult;
use super::executor::execute_agent;
use super::executor::execute_shell;
use super::python_host::PYTHON_HOST;
use super::python_host::python_program;
use super::runtime_io::capture_stderr;
use super::runtime_io::respond_error;
use super::runtime_io::respond_ok;
use super::runtime_protocol::MAX_PROTOCOL_LINE_BYTES;
use super::runtime_protocol::PROTOCOL_VERSION;
use super::runtime_protocol::WorkflowRequest;

const MAX_BATCH_SIZE: usize = 64;

pub(crate) async fn run_workflow(
    request: WorkflowRunRequest,
    mut control: watch::Receiver<WorkflowControl>,
    updates: mpsc::UnboundedSender<WorkflowUpdate>,
) {
    let mut runtime = Runtime::new(request, control.clone(), updates);
    runtime.send(WorkflowUpdate::Started {
        run_id: runtime.run.run_id.clone(),
        title: runtime.run.manifest.title.clone(),
    });
    let timeout_seconds = runtime.run.manifest.guardrails.timeout_seconds;
    let result = tokio::time::timeout(
        Duration::from_secs(timeout_seconds),
        runtime.run_python(&mut control),
    )
    .await;
    match result {
        Ok(Ok(result)) => runtime.finish_completed(result).await,
        Ok(Err(error)) => runtime.finish_interrupted_or_failed(error).await,
        Err(_) => {
            runtime
                .finish_interrupted_or_failed(format!(
                    "workflow timed out after {timeout_seconds}s"
                ))
                .await;
        }
    }
}

struct Runtime {
    run: StoredWorkflowRun,
    workspace: PathBuf,
    antex_exe: PathBuf,
    control: watch::Receiver<WorkflowControl>,
    updates: mpsc::UnboundedSender<WorkflowUpdate>,
    agent_calls: u32,
    shell_calls: u32,
    progress: Option<(String, Option<u64>, Option<u64>)>,
}

impl Runtime {
    fn new(
        request: WorkflowRunRequest,
        control: watch::Receiver<WorkflowControl>,
        updates: mpsc::UnboundedSender<WorkflowUpdate>,
    ) -> Self {
        let agent_calls = request.run.agent_calls;
        let shell_calls = request.run.shell_calls;
        Self {
            run: request.run,
            workspace: request.workspace,
            antex_exe: request.antex_exe,
            control,
            updates,
            agent_calls,
            shell_calls,
            progress: None,
        }
    }

    async fn run_python(
        &mut self,
        control: &mut watch::Receiver<WorkflowControl>,
    ) -> Result<Value, String> {
        self.run
            .set_status(WorkflowRunStatus::Running, None)
            .await?;
        let mut child = Command::new(python_program())
            .arg("-u")
            .arg("-c")
            .arg(PYTHON_HOST)
            .arg("run")
            .arg(self.run.script_path())
            .arg(self.run.params_path())
            .arg(self.run.state_path())
            .current_dir(&self.workspace)
            .stdin(Stdio::piped())
            .stdout(Stdio::piped())
            .stderr(Stdio::piped())
            .kill_on_drop(true)
            .spawn()
            .map_err(|err| format!("failed to start Python workflow: {err}"))?;
        let mut stdin = child
            .stdin
            .take()
            .ok_or_else(|| "failed to open Python workflow stdin".to_string())?;
        let stdout = child
            .stdout
            .take()
            .ok_or_else(|| "failed to capture Python workflow stdout".to_string())?;
        let stderr = child
            .stderr
            .take()
            .ok_or_else(|| "failed to capture Python workflow stderr".to_string())?;
        let stderr_tail = Arc::new(Mutex::new(String::new()));
        let stderr_task = tokio::spawn(capture_stderr(stderr, stderr_tail.clone()));
        let mut lines = BufReader::new(stdout).lines();
        let outcome = loop {
            tokio::select! {
                line = lines.next_line() => {
                    let line = line.map_err(|err| format!("failed to read workflow protocol: {err}"))?;
                    let Some(line) = line else {
                        let tail = stderr_tail.lock().await.clone();
                        break Err(if tail.trim().is_empty() {
                            "Python workflow exited without a completion message".to_string()
                        } else {
                            format!("Python workflow exited unexpectedly: {}", tail.trim())
                        });
                    };
                    if line.len() > MAX_PROTOCOL_LINE_BYTES {
                        break Err("workflow protocol message exceeds 2 MiB".to_string());
                    }
                    let message: WorkflowRequest = serde_json::from_str(&line)
                        .map_err(|err| format!("invalid workflow protocol message: {err}"))?;
                    if message.protocol_version() != PROTOCOL_VERSION {
                        break Err(format!(
                            "unsupported workflow protocol version {}",
                            message.protocol_version()
                        ));
                    }
                    if let Some(result) = self.handle_request(message, &mut stdin).await? { break Ok(result) }
                }
                changed = control.changed() => {
                    if changed.is_err() || *control.borrow() != WorkflowControl::Run {
                        let _ = child.kill().await;
                        break Err("workflow interrupted".to_string());
                    }
                }
            }
        };
        drop(stdin);
        let _ = child.wait().await;
        let _ = stderr_task.await;
        outcome
    }

    async fn handle_request(
        &mut self,
        message: WorkflowRequest,
        stdin: &mut tokio::process::ChildStdin,
    ) -> Result<Option<Value>, String> {
        match message {
            WorkflowRequest::Progress {
                id,
                message,
                current,
                total,
                ..
            } => {
                self.progress = Some((message.clone(), current, total));
                self.send(WorkflowUpdate::Progress {
                    run_id: self.run.run_id.clone(),
                    message,
                    current,
                    total,
                });
                respond_ok(stdin, id, Value::Null).await?;
            }
            WorkflowRequest::Shell {
                id,
                argv,
                cwd,
                timeout_seconds,
                env,
                ..
            } => {
                if self.shell_calls.saturating_add(1) > self.run.manifest.guardrails.max_shell_calls
                {
                    respond_error(stdin, id, "workflow shell-call limit reached").await?;
                } else {
                    self.shell_calls = self.shell_calls.saturating_add(1);
                    self.run
                        .record_calls(/*agent_delta*/ 0, /*shell_delta*/ 1)
                        .await?;
                    match execute_shell(
                        argv,
                        cwd,
                        env,
                        timeout_seconds,
                        &self.workspace,
                        self.control.clone(),
                    )
                    .await
                    {
                        Ok(result) => respond_ok(stdin, id, result).await?,
                        Err(err) => respond_error(stdin, id, err).await?,
                    }
                }
            }
            WorkflowRequest::Agent {
                id,
                prompt,
                model,
                reasoning_effort,
                developer_instructions,
                forbid_quality_graph_ignore,
                sandbox,
                approval_mode,
                cwd,
                timeout_seconds,
                ..
            } => {
                let request = AgentRequest {
                    prompt,
                    model,
                    reasoning_effort,
                    developer_instructions,
                    forbid_quality_graph_ignore,
                    sandbox,
                    approval_mode,
                    cwd,
                    timeout_seconds,
                };
                let results = self
                    .run_agent_batch(
                        vec![request],
                        /*parallelism*/ Some(1),
                        /*default_model*/ None,
                        /*default_cwd*/ None,
                        /*default_timeout_seconds*/ None,
                    )
                    .await;
                match results {
                    Ok(mut results) => respond_ok(stdin, id, results.remove(0)).await?,
                    Err(err) => respond_error(stdin, id, err).await?,
                }
            }
            WorkflowRequest::AgentBatch {
                id,
                requests,
                parallelism,
                model,
                cwd,
                timeout_seconds,
                ..
            } => match self
                .run_agent_batch(requests, parallelism, model, cwd, timeout_seconds)
                .await
            {
                Ok(results) => respond_ok(stdin, id, results).await?,
                Err(err) => respond_error(stdin, id, err).await?,
            },
            WorkflowRequest::Checkpoint { id, state, .. } => {
                match self.run.checkpoint(state).await {
                    Ok(()) => {
                        self.send(WorkflowUpdate::Checkpointed {
                            run_id: self.run.run_id.clone(),
                        });
                        respond_ok(stdin, id, Value::Null).await?;
                    }
                    Err(err) => respond_error(stdin, id, err).await?,
                }
            }
            WorkflowRequest::Completed { result, .. } => {
                return Ok(Some(result.unwrap_or(Value::Null)));
            }
            WorkflowRequest::Failed { error, .. } => return Err(error),
        }
        Ok(None)
    }

    async fn run_agent_batch(
        &mut self,
        requests: Vec<AgentRequest>,
        parallelism: Option<usize>,
        default_model: Option<String>,
        default_cwd: Option<String>,
        default_timeout_seconds: Option<u64>,
    ) -> Result<Vec<AgentResult>, String> {
        if requests.is_empty() || requests.len() > MAX_BATCH_SIZE {
            return Err(format!(
                "agent batch must contain between 1 and {MAX_BATCH_SIZE} requests"
            ));
        }
        let request_count = u32::try_from(requests.len()).unwrap_or(u32::MAX);
        if self.agent_calls.saturating_add(request_count)
            > self.run.manifest.guardrails.max_agent_calls
        {
            return Err("workflow agent-call limit reached".to_string());
        }
        self.agent_calls = self.agent_calls.saturating_add(request_count);
        self.run
            .record_calls(request_count, /*shell_delta*/ 0)
            .await?;
        let parallelism = parallelism
            .unwrap_or(1)
            .clamp(1, self.run.manifest.guardrails.max_parallel_agents)
            .min(requests.len());
        self.send(WorkflowUpdate::AgentBatchStarted {
            run_id: self.run.run_id.clone(),
            count: requests.len(),
            parallelism,
            phase: self
                .progress
                .as_ref()
                .map(|(message, _, _)| message.clone()),
            phase_current: self.progress.as_ref().and_then(|(_, current, _)| *current),
            phase_total: self.progress.as_ref().and_then(|(_, _, total)| *total),
        });

        let semaphore = Arc::new(tokio::sync::Semaphore::new(parallelism));
        let mut tasks = JoinSet::new();
        let (activity_tx, mut activity_rx) = mpsc::unbounded_channel::<AgentActivity>();
        let total = requests.len();
        for (index, request) in requests.into_iter().enumerate() {
            let semaphore = semaphore.clone();
            let antex_exe = self.antex_exe.clone();
            let workspace = self.workspace.clone();
            let control = self.control.clone();
            let default_model = default_model.clone();
            let default_cwd = default_cwd.clone();
            let activity_tx = activity_tx.clone();
            tasks.spawn(async move {
                let permit = semaphore
                    .acquire_owned()
                    .await
                    .map_err(|_| "agent batch was closed".to_string())?;
                let result = execute_agent(
                    request,
                    AgentExecutionContext {
                        default_model,
                        default_cwd,
                        default_timeout_seconds,
                        antex_exe: &antex_exe,
                        workspace: &workspace,
                        control,
                        agent_index: index,
                        activity_tx,
                    },
                )
                .await;
                drop(permit);
                Ok::<_, String>((index, result))
            });
        }
        drop(activity_tx);
        let mut completed = 0usize;
        let mut results = vec![None; total];
        let mut live_activity = vec![None::<(String, Instant)>; total];
        let mut heartbeat = tokio::time::interval(Duration::from_secs(5));
        heartbeat.set_missed_tick_behavior(tokio::time::MissedTickBehavior::Skip);
        heartbeat.tick().await;
        let mut last_activity_persisted_at = Instant::now();
        while completed < total {
            tokio::select! {
                Some(joined) = tasks.join_next() => {
                    let (index, result) =
                        joined.map_err(|err| format!("agent batch task failed: {err}"))??;
                    let result = result.unwrap_or_else(|error| AgentResult {
                        success: false,
                        exit_code: None,
                        message: String::new(),
                        error,
                        input_tokens: 0,
                        cached_input_tokens: 0,
                        output_tokens: 0,
                    });
                    completed = completed.saturating_add(1);
                    live_activity[index] = None;
                    self.send(WorkflowUpdate::AgentFinished {
                        run_id: self.run.run_id.clone(),
                        completed,
                        total,
                        success: result.success,
                        phase: self
                            .progress
                            .as_ref()
                            .map(|(message, _, _)| message.clone()),
                        phase_current: self.progress.as_ref().and_then(|(_, current, _)| *current),
                        phase_total: self.progress.as_ref().and_then(|(_, _, total)| *total),
                    });
                    results[index] = Some(result);
                }
                Some(activity) = activity_rx.recv() => {
                    let persisted = format!(
                        "Agent {}/{}: {}",
                        activity.agent_index.saturating_add(1),
                        total,
                        activity.message
                    );
                    if let Err(err) = self.run.record_activity(persisted).await {
                        tracing::warn!(%err, "failed to persist workflow agent activity");
                    }
                    last_activity_persisted_at = Instant::now();
                    live_activity[activity.agent_index] =
                        Some((activity.message.clone(), Instant::now()));
                    self.send_agent_activity(
                        activity.agent_index,
                        total,
                        activity.message,
                        /*idle_seconds*/ 0,
                    );
                }
                _ = heartbeat.tick() => {
                    let latest = live_activity
                        .iter()
                        .enumerate()
                        .filter_map(|(index, activity)| {
                            activity.as_ref().map(|(message, at)| (index, message, *at))
                        })
                        .min_by_key(|(_, _, at)| at.elapsed());
                    if let Some((index, message, at)) = latest {
                        let idle_seconds = at.elapsed().as_secs();
                        self.send_agent_activity(
                            index,
                            total,
                            message.clone(),
                            idle_seconds,
                        );
                        if last_activity_persisted_at.elapsed() >= Duration::from_secs(30) {
                            if let Err(err) = self.run
                                .record_activity(format!(
                                    "Agent {}/{}: {} · {idle_seconds}s since activity",
                                    index.saturating_add(1),
                                    total,
                                    message
                                ))
                                .await
                            {
                                tracing::warn!(%err, "failed to persist workflow heartbeat");
                            }
                            last_activity_persisted_at = Instant::now();
                        }
                    }
                }
            }
        }
        results
            .into_iter()
            .map(|result| result.ok_or_else(|| "agent batch lost a result".to_string()))
            .collect()
    }

    fn send_agent_activity(
        &self,
        agent_index: usize,
        total: usize,
        message: String,
        idle_seconds: u64,
    ) {
        self.send(WorkflowUpdate::AgentActivity {
            run_id: self.run.run_id.clone(),
            agent: agent_index.saturating_add(1),
            total,
            message,
            idle_seconds,
            phase: self
                .progress
                .as_ref()
                .map(|(message, _, _)| message.clone()),
            phase_current: self.progress.as_ref().and_then(|(_, current, _)| *current),
            phase_total: self.progress.as_ref().and_then(|(_, _, total)| *total),
        });
    }

    async fn finish_completed(&mut self, result: Value) {
        let _ = self
            .run
            .set_status(WorkflowRunStatus::Completed, None)
            .await;
        self.send(WorkflowUpdate::Completed {
            run_id: self.run.run_id.clone(),
            title: self.run.manifest.title.clone(),
            result,
            agent_calls: self.agent_calls,
            shell_calls: self.shell_calls,
        });
    }

    async fn finish_interrupted_or_failed(&mut self, error: String) {
        let (status, update) = match *self.control.borrow() {
            WorkflowControl::Pause => (
                WorkflowRunStatus::Paused,
                WorkflowUpdate::Paused {
                    run_id: self.run.run_id.clone(),
                    title: self.run.manifest.title.clone(),
                },
            ),
            WorkflowControl::Cancel => (
                WorkflowRunStatus::Cancelled,
                WorkflowUpdate::Cancelled {
                    run_id: self.run.run_id.clone(),
                    title: self.run.manifest.title.clone(),
                },
            ),
            WorkflowControl::Run => (
                WorkflowRunStatus::Failed,
                WorkflowUpdate::Failed {
                    run_id: self.run.run_id.clone(),
                    title: self.run.manifest.title.clone(),
                    error: error.clone(),
                },
            ),
        };
        let stored_error = (status == WorkflowRunStatus::Failed).then_some(error);
        let _ = self.run.set_status(status, stored_error).await;
        self.send(update);
    }

    fn send(&self, update: WorkflowUpdate) {
        let _ = self.updates.send(update);
    }
}

#[cfg(test)]
#[path = "runtime_tests.rs"]
mod tests;
