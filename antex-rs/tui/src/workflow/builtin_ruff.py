"""Built-in example: repair a large Ruff backlog with bounded atomic agents."""

import json
import shlex


WORKFLOW = {
    "id": "ruff-cleanup",
    "title": "Ruff cleanup",
    "description": "Fix a large Ruff backlog in small, restartable agent batches.",
    "version": 1,
    "fields": [
        {
            "id": "scope",
            "label": "Scope",
            "description": "File or directory passed to Ruff and used as the agent working scope.",
            "type": "text",
            "required": True,
            "default": ".",
            "placeholder": "src",
        },
        {
            "id": "ruff_command",
            "label": "Ruff command",
            "description": "Command prefix; JSON output flags are added automatically.",
            "type": "text",
            "required": True,
            "default": "ruff check",
            "placeholder": "uv run ruff check",
        },
        {
            "id": "batch_size",
            "label": "Issues per agent",
            "description": "Maximum number of violations from one file assigned to an agent.",
            "type": "integer",
            "min": 1,
            "max": 100,
            "default": 20,
        },
        {
            "id": "parallelism",
            "label": "Parallel agents",
            "description": "Agents work on different files to avoid edit conflicts.",
            "type": "integer",
            "min": 1,
            "max": 8,
            "default": 4,
        },
        {
            "id": "model",
            "label": "Model override",
            "description": "Leave empty to use the configured Codex model.",
            "type": "text",
            "default": "",
            "placeholder": "gpt-5-codex-mini",
        },
        {
            "id": "max_failures",
            "label": "Failure limit",
            "description": "Stop before repeated agent failures can burn the whole allowance.",
            "type": "integer",
            "min": 1,
            "max": 100,
            "default": 10,
        },
        {
            "id": "max_passes",
            "label": "Ruff passes",
            "description": "Maximum scan/fix cycles before the workflow stops.",
            "type": "integer",
            "min": 1,
            "max": 1000,
            "default": 100,
        },
        {
            "id": "verify",
            "label": "Final verification",
            "description": "Run Ruff once more and fail if violations remain.",
            "type": "boolean",
            "default": True,
        },
    ],
    "guardrails": {
        "max_agent_calls": 10000,
        "max_shell_calls": 2000,
        "max_parallel_agents": 8,
        "timeout_seconds": 86400,
    },
}


def _scan(ctx, command, scope):
    argv = [*shlex.split(command), scope, "--output-format", "json"]
    result = ctx.shell(argv)
    if result["exit_code"] not in (0, 1):
        raise RuntimeError(
            f"Ruff exited with {result['exit_code']}: {result['stderr'][-2000:]}"
        )
    try:
        return json.loads(result["stdout"] or "[]")
    except json.JSONDecodeError as exc:
        raise RuntimeError(
            "Ruff did not return JSON; check the configured command"
        ) from exc


def _issue_key(issue):
    location = issue.get("location", {})
    return (
        issue.get("filename", ""),
        issue.get("code", ""),
        location.get("row", 0),
        location.get("column", 0),
    )


def _group_requests(issues, batch_size):
    by_file = {}
    for issue in issues:
        by_file.setdefault(issue.get("filename", "<unknown>"), []).append(issue)
    requests = []
    for filename, file_issues in sorted(by_file.items()):
        file_issues.sort(key=_issue_key)
        for offset in range(0, len(file_issues), batch_size):
            batch = file_issues[offset : offset + batch_size]
            compact = [
                {
                    "code": issue.get("code"),
                    "message": issue.get("message"),
                    "location": issue.get("location"),
                    "end_location": issue.get("end_location"),
                }
                for issue in batch
            ]
            prompt = f"""Fix only the Ruff violations listed below in {filename}.

Do not add noqa comments, ignores, per-file-ignores, exclusions, or weaken Ruff configuration.
Do not make unrelated refactors. Preserve behavior. Inspect the file, apply the smallest correct
edits, and run a targeted Ruff check for this file before finishing.

Violations:
{json.dumps(compact, ensure_ascii=False, indent=2)}
"""
            requests.append((filename, {"prompt": prompt}))
    return requests


def run(ctx):
    scope = ctx.params["scope"]
    command = ctx.params["ruff_command"]
    batch_size = ctx.params["batch_size"]
    parallelism = ctx.params["parallelism"]
    model = ctx.params.get("model") or None
    max_failures = ctx.params["max_failures"]
    max_passes = ctx.params["max_passes"]
    verify = ctx.params["verify"]
    state = ctx.state or {"pass": 0, "agent_calls": 0, "failures": 0}

    for pass_number in range(state.get("pass", 0) + 1, max_passes + 1):
        issues = _scan(ctx, command, scope)
        if not issues:
            ctx.progress("Ruff reports no remaining violations", current=1, total=1)
            ctx.checkpoint({**state, "pass": pass_number, "remaining": 0})
            return {"passes": pass_number, "agent_calls": state["agent_calls"]}

        requests = _group_requests(issues, batch_size)
        # One wave intentionally contains at most one request per file. That keeps parallel
        # agents from racing on the same file; the next Ruff scan refreshes line positions.
        wave = []
        seen_files = set()
        for filename, request in requests:
            if filename not in seen_files:
                seen_files.add(filename)
                wave.append(request)

        ctx.progress(
            f"Pass {pass_number}: {len(issues)} violations, {len(wave)} file agents",
            current=state["agent_calls"],
            total=state["agent_calls"] + len(wave),
        )
        results = ctx.agent_batch(
            wave,
            parallelism=parallelism,
            model=model,
        )
        failures = sum(1 for result in results if not result.get("success"))
        state = {
            "pass": pass_number,
            "agent_calls": state["agent_calls"] + len(results),
            "failures": state["failures"] + failures,
            "remaining_before_pass": len(issues),
        }
        ctx.checkpoint(state)
        if state["failures"] >= max_failures:
            raise RuntimeError(
                f"stopped after {state['failures']} failed agent calls; inspect the run and resume"
            )

    remaining = _scan(ctx, command, scope) if verify else []
    if remaining:
        raise RuntimeError(
            f"reached {max_passes} passes with {len(remaining)} Ruff violations remaining"
        )
    return {"passes": max_passes, "agent_calls": state["agent_calls"]}
