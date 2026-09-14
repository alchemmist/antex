# Python workflows (fork feature)

Python workflows are restartable orchestration programs for repetitive tasks that are too large
for one model context. Open the picker with `/workflow`. Project workflows live in
`.codex/workflows/*.py`; personal workflows live in `~/.codex/workflows/*.py`. A project workflow
with the same `id` overrides a personal or built-in workflow.

Each file exposes a `WORKFLOW` manifest and `run(ctx)`:

```python
WORKFLOW = {
    "id": "my-cleanup",
    "title": "My cleanup",
    "description": "Fix one independent issue per agent.",
    "fields": [
        {
            "id": "scope",
            "label": "Scope",
            "type": "text",
            "default": "src",
            "required": True,
        },
        {
            "id": "parallelism",
            "label": "Parallel agents",
            "type": "integer",
            "min": 1,
            "max": 8,
            "default": 4,
        },
        {
            "id": "verify",
            "label": "Verify when finished",
            "type": "boolean",
            "default": True,
        },
    ],
    "guardrails": {
        "max_agent_calls": 1000,
        "max_shell_calls": 1000,
        "max_parallel_agents": 8,
        "timeout_seconds": 43200,
    },
}


def run(ctx):
    report = ctx.shell(["my-linter", "--json", ctx.params["scope"]])
    issues = parse_report(report["stdout"])
    prompts = [prompt_for(issue) for issue in issues]
    results = ctx.agent_batch(prompts, parallelism=ctx.params["parallelism"])
    ctx.checkpoint({"completed": len(results)})
    ctx.progress("Finished one wave", current=len(results), total=len(prompts))
    return {"fixed": sum(result["success"] for result in results)}
```

The context API is synchronous on purpose: Python describes order and branching while Codex owns
the async execution machinery.

- `ctx.params`: answers collected by the TUI from manifest fields.
- `ctx.state`: the last JSON checkpoint, or an empty object for a new run.
- `ctx.progress(message, current=None, total=None)`: update the TUI status row.
- `ctx.shell(argv, cwd=None, timeout_seconds=None, env=None)`: run a bounded command. `argv` must be
  a list; string shell evaluation is deliberately not implicit.
- `ctx.agent(prompt, model=None, reasoning_effort=None, developer_instructions=None, forbid_quality_graph_ignore=False, cwd=None, timeout_seconds=None)`: run one ephemeral Codex agent.
- `ctx.agent_batch(prompts, parallelism=None, model=None, reasoning_effort=None, developer_instructions=None, forbid_quality_graph_ignore=False, cwd=None, timeout_seconds=None)`: run
  independent ephemeral agents concurrently. A prompt can be a string or a dictionary containing
  `prompt`, `model`, `reasoning_effort`, `developer_instructions`, `forbid_quality_graph_ignore`, `cwd`, and `timeout_seconds`.
Both agent methods also accept `approval_mode="inherit"` (the default) or `"auto-review"`.
Auto-review runs the child with `--approve-for-me`: approval requests go to the automatic reviewer
and the sandbox is workspace-write, overriding the requested `sandbox`. A reviewer denial remains
a denial; this mode does not disable approval checks. Bot maintenance selects it only for merge
agents working on non-archived repositories.

- `ctx.checkpoint(json_value)`: persist at most 1 MiB of restart state.
- `ctx.log(message)`: write a diagnostic to the Codex log without corrupting the protocol stream.

Text, integer, boolean, and select fields are supported. The manifest controls labels, descriptions,
defaults, ranges, and choices, so each workflow owns its setup UX without owning terminal rendering.

`/workflow pause` stops the current process at the nearest host action and retains its checkpoint.
`/workflow stop` cancels it but also keeps the checkpoint for diagnosis or an explicit resume.
`/workflow resume` lists paused, failed, cancelled, and runs interrupted by a previous Codex exit.

Agent calls invoke the same locally installed `codex exec` binary with the same `CODEX_HOME`, login,
and subscription. Each invocation is ephemeral and starts with a small context. Rust enforces the
manifest guardrails, bounds protocol/output sizes, prevents workflow actions from changing their cwd
outside the current workspace, and snapshots the Python source into every run directory.

Workflow files are trusted local code. Python itself is not sandboxed and can call arbitrary Python
or operating-system APIs, so Rust guardrails constrain the `ctx` API but are not a security boundary.
Only install or run workflows whose source you trust.

Bot PR maintenance returns a compact summary with a `status` of `completed` or
`needs_user` and a `report_path`. Its full per-repository results remain in the
checkpoint. The adjacent `workflow.report.md` lists merged/fixed PRs, skipped
candidates, execution errors, and human blockers with a PR link and next action.
Confirmed human blockers do not fail the workflow or trigger another repair
attempt. Invalid reports and unresolved execution errors still fail the run, with
the report saved before the error is returned. Existing run snapshots keep their
original behavior; start a new run to use an updated workflow.

Model fields use `"type": "model"` to display a searchable list from the current
Codex model catalog. Optional model fields include “Use current Codex model”,
which stores an empty string. Required fields require an explicit selection.
A configured workflow default remains selectable even if absent from the catalog.
For existing workflows, text fields named `model` or ending in `_model` use the
same picker automatically. Other text fields and explicit select options are
unchanged. Selecting a workflow model does not change the main chat's model.
