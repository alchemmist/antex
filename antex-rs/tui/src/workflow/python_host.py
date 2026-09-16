"""Private bridge between a Python workflow and the Codex Rust host.

Workflow files intentionally do not import this module. They expose a WORKFLOW
dictionary and a run(ctx) function; this bridge supplies the context at runtime.
"""

import asyncio
import contextlib
import importlib.util
import inspect
import json
import pathlib
import sys
import traceback


PROTOCOL_VERSION = 1


def _load(path):
    spec = importlib.util.spec_from_file_location("codex_user_workflow", path)
    if spec is None or spec.loader is None:
        raise RuntimeError(f"cannot load workflow at {path}")
    module = importlib.util.module_from_spec(spec)
    with contextlib.redirect_stdout(sys.stderr):
        spec.loader.exec_module(module)
    return module


def _manifest(module):
    manifest = getattr(module, "WORKFLOW", None)
    if callable(manifest):
        with contextlib.redirect_stdout(sys.stderr):
            manifest = manifest()
    if not isinstance(manifest, dict):
        raise TypeError("workflow must expose a WORKFLOW dictionary or function")
    return manifest


class Context:
    def __init__(self, params, state, protocol_out):
        self.params = params
        self.state = state
        self._out = protocol_out
        self._next_id = 1

    def _request(self, kind, **payload):
        request_id = self._next_id
        self._next_id += 1
        message = {
            "protocol_version": PROTOCOL_VERSION,
            "id": request_id,
            "type": kind,
            **payload,
        }
        self._out.write(json.dumps(message, ensure_ascii=False) + "\n")
        self._out.flush()
        line = sys.stdin.readline()
        if not line:
            raise RuntimeError("Codex workflow host closed the protocol stream")
        response = json.loads(line)
        if response.get("id") != request_id:
            raise RuntimeError("Codex workflow host returned a mismatched response")
        if not response.get("ok", False):
            raise RuntimeError(response.get("error", "workflow action failed"))
        return response.get("result")

    def progress(self, message, current=None, total=None):
        return self._request(
            "progress", message=str(message), current=current, total=total
        )

    def shell(self, argv, cwd=None, timeout_seconds=None, env=None):
        return self._request(
            "shell",
            argv=list(argv),
            cwd=cwd,
            timeout_seconds=timeout_seconds,
            env=env or {},
        )

    def agent(
        self,
        prompt,
        model=None,
        reasoning_effort=None,
        developer_instructions=None,
        forbid_quality_graph_ignore=False,
        sandbox="workspace-write",
        cwd=None,
        timeout_seconds=None,
        approval_mode="inherit",
    ):
        return self._request(
            "agent",
            prompt=str(prompt),
            model=model,
            reasoning_effort=reasoning_effort,
            developer_instructions=developer_instructions,
            forbid_quality_graph_ignore=bool(forbid_quality_graph_ignore),
            sandbox=sandbox,
            approval_mode=approval_mode,
            cwd=cwd,
            timeout_seconds=timeout_seconds,
        )

    def agent_batch(
        self,
        prompts,
        parallelism=None,
        model=None,
        reasoning_effort=None,
        developer_instructions=None,
        forbid_quality_graph_ignore=False,
        sandbox="workspace-write",
        cwd=None,
        timeout_seconds=None,
        approval_mode="inherit",
    ):
        requests = []
        for prompt in prompts:
            if isinstance(prompt, str):
                requests.append({"prompt": prompt})
            elif isinstance(prompt, dict):
                requests.append(dict(prompt))
            else:
                raise TypeError("agent_batch prompts must be strings or dictionaries")
        for request in requests:
            request.setdefault("reasoning_effort", reasoning_effort)
            request.setdefault("developer_instructions", developer_instructions)
            request.setdefault(
                "forbid_quality_graph_ignore", bool(forbid_quality_graph_ignore)
            )
            request.setdefault("sandbox", sandbox)
            request.setdefault("approval_mode", approval_mode)
        return self._request(
            "agent_batch",
            requests=requests,
            parallelism=parallelism,
            model=model,
            cwd=cwd,
            timeout_seconds=timeout_seconds,
        )

    def checkpoint(self, state):
        self.state = state
        return self._request("checkpoint", state=state)

    def log(self, message):
        print(str(message), file=sys.stderr, flush=True)


def _describe(path, protocol_out):
    module = _load(path)
    json.dump(_manifest(module), protocol_out, ensure_ascii=False)
    protocol_out.write("\n")
    protocol_out.flush()


def _run(path, params_path, state_path, protocol_out):
    module = _load(path)
    run = getattr(module, "run", None)
    if not callable(run):
        raise TypeError("workflow must expose run(ctx)")
    params = json.loads(pathlib.Path(params_path).read_text(encoding="utf-8"))
    state = json.loads(pathlib.Path(state_path).read_text(encoding="utf-8"))
    ctx = Context(params, state, protocol_out)
    with contextlib.redirect_stdout(sys.stderr):
        result = run(ctx)
        if inspect.isawaitable(result):
            result = asyncio.run(result)
    protocol_out.write(
        json.dumps(
            {
                "protocol_version": PROTOCOL_VERSION,
                "type": "completed",
                "result": result,
            },
            ensure_ascii=False,
        )
        + "\n"
    )
    protocol_out.flush()


def main():
    protocol_out = sys.stdout
    try:
        if len(sys.argv) < 3:
            raise RuntimeError("expected describe|run and workflow path")
        command = sys.argv[1]
        path = sys.argv[2]
        if command == "describe":
            _describe(path, protocol_out)
        elif command == "run":
            if len(sys.argv) != 5:
                raise RuntimeError("run expects workflow, params, and state paths")
            _run(path, sys.argv[3], sys.argv[4], protocol_out)
        else:
            raise RuntimeError(f"unknown bridge command: {command}")
    except BaseException as exc:
        traceback.print_exc(file=sys.stderr)
        if len(sys.argv) > 1 and sys.argv[1] == "run":
            protocol_out.write(
                json.dumps(
                    {
                        "protocol_version": PROTOCOL_VERSION,
                        "type": "failed",
                        "error": str(exc),
                    },
                    ensure_ascii=False,
                )
                + "\n"
            )
            protocol_out.flush()
        return 1
    return 0


if __name__ == "__main__":
    raise SystemExit(main())
