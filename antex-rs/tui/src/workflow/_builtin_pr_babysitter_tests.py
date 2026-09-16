import importlib.util
import json
import pathlib
import subprocess
import sys
import tempfile


def load_workflow(path):
    spec = importlib.util.spec_from_file_location("pr_babysitter", path)
    module = importlib.util.module_from_spec(spec)
    spec.loader.exec_module(module)
    module.time.sleep = lambda _seconds: None
    return module


class FakeContext:
    def __init__(self, module, monitor_results, worker_results=None):
        self.module = module
        self.monitor_results = iter(monitor_results)
        self.worker_results = iter(worker_results or [])
        self.params = {
            "pull_request": "https://github.com/acme/repo/pull/7",
            "poll_interval_seconds": 30,
            "monitor_model": "gpt-5.6-luna",
            "monitor_reasoning": "low",
            "worker_model": "gpt-5.6-sol",
            "worker_reasoning": "high",
            "stable_green_polls": 1,
            "max_worker_failures": 3,
        }
        self.state = {}
        self.checkpoints = []
        self.agent_calls = []
        self.shell_calls = []

    def agent(self, prompt, **kwargs):
        self.agent_calls.append((prompt, kwargs))
        if prompt.startswith("Inspect pull request"):
            assert kwargs["sandbox"] == "read-only"
            value = next(self.monitor_results)
        else:
            assert kwargs["developer_instructions"] == self.module.WORKER_POLICY
            assert kwargs["forbid_quality_graph_ignore"] is True
            assert kwargs["cwd"] == "/tmp/pr-7"
            assert kwargs["timeout_seconds"] == 1800
            assert kwargs["sandbox"] == "danger-full-access"
            assert "Do not create another worktree" in prompt
            value = next(self.worker_results)
        return {"success": True, "message": self.module.json.dumps(value), "error": ""}

    def shell(self, argv, cwd=None, timeout_seconds=None, env=None):
        self.shell_calls.append((argv, cwd, timeout_seconds, env))
        return {
            "exit_code": 0,
            "stdout": self.module.json.dumps(
                {"path": "/tmp/pr-7", "head_sha": "a" * 40, "dirty": True}
            ),
            "stderr": "",
        }

    def checkpoint(self, state):
        self.state = dict(state)
        self.checkpoints.append(dict(state))

    def progress(self, _message, current=None, total=None):
        return (current, total)


def snapshot(ci_status="green", failed_checks=None, feedback=None, head_sha=None):
    failed_checks = failed_checks or []
    feedback = feedback or []
    failed = len(failed_checks)
    return {
        "repository": "acme/repo",
        "number": 7,
        "url": "https://github.com/acme/repo/pull/7",
        "state": "open",
        "head_sha": head_sha or "a" * 40,
        "head_ref": "feature/pr-7",
        "mergeable": True,
        "review_decision": "approved",
        "ci_status": ci_status,
        "checks": {"passed": 3 - failed, "pending": 0, "failed": failed, "total": 3},
        "failed_checks": failed_checks,
        "unresolved_feedback": feedback,
    }


def test_prepare_checkout(module):
    with tempfile.TemporaryDirectory() as temporary:
        root = pathlib.Path(temporary)
        origin = root / "origin.git"
        seed = root / "seed"
        workspace = root / "workspace"
        subprocess.run(
            ["git", "init", "--bare", origin], check=True, capture_output=True
        )
        subprocess.run(["git", "init", seed], check=True, capture_output=True)
        subprocess.run(
            ["git", "config", "user.email", "test@example.com"],
            cwd=seed,
            check=True,
        )
        subprocess.run(["git", "config", "user.name", "Test"], cwd=seed, check=True)
        (seed / "file.txt").write_text("content\n")
        subprocess.run(["git", "add", "file.txt"], cwd=seed, check=True)
        subprocess.run(["git", "commit", "-m", "initial"], cwd=seed, check=True)
        head_sha = subprocess.run(
            ["git", "rev-parse", "HEAD"],
            cwd=seed,
            check=True,
            text=True,
            capture_output=True,
        ).stdout.strip()
        subprocess.run(["git", "remote", "add", "origin", origin], cwd=seed, check=True)
        subprocess.run(["git", "push", "origin", "HEAD:main"], cwd=seed, check=True)
        subprocess.run(
            ["git", "update-ref", "refs/pull/7/head", head_sha],
            cwd=origin,
            check=True,
        )
        subprocess.run(
            ["git", "clone", origin, workspace], check=True, capture_output=True
        )
        result = subprocess.run(
            [
                sys.executable,
                "-c",
                module.PREPARE_CHECKOUT_SCRIPT,
                json.dumps({"number": 7, "head_sha": head_sha}),
            ],
            cwd=workspace,
            check=False,
            text=True,
            capture_output=True,
        )
        assert result.returncode == 0, result.stderr
        checkout = json.loads(result.stdout)
        checkout_path = pathlib.Path(checkout["path"])
        assert checkout["head_sha"] == head_sha
        assert checkout["dirty"] is False
        assert (checkout_path / ".git").is_file()
        assert checkout_path.parent.name == "codex-workflow-checkouts"
        (checkout_path / "file.txt").write_text("changed\n")
        repeated = subprocess.run(
            [
                sys.executable,
                "-c",
                module.PREPARE_CHECKOUT_SCRIPT,
                json.dumps({"number": 7, "head_sha": head_sha}),
            ],
            cwd=workspace,
            check=False,
            text=True,
            capture_output=True,
        )
        assert repeated.returncode == 0, repeated.stderr
        repeated_checkout = json.loads(repeated.stdout)
        assert repeated_checkout["path"] == checkout["path"]
        assert repeated_checkout["head_sha"] == checkout["head_sha"]
        assert repeated_checkout["dirty"] is True


def main():
    module = load_workflow(pathlib.Path(sys.argv[1]))
    test_prepare_checkout(module)

    ready_context = FakeContext(module, [snapshot()])
    ready = module.run(ready_context)
    assert ready["status"] == "ready"
    assert len(ready_context.agent_calls) == 1
    assert ready_context.agent_calls[0][1]["model"] == "gpt-5.6-luna"
    assert ready_context.agent_calls[0][1]["reasoning_effort"] == "low"

    failure = {"id": "check-1", "name": "tests", "summary": "failed", "url": "url"}
    worker_context = FakeContext(
        module,
        [snapshot("failing", [failure]), snapshot(head_sha="b" * 40)],
        [
            {
                "status": "changed",
                "summary": "fixed tests",
                "commit": "b" * 40,
                "handled_ids": ["check-1"],
                "needs_user_reason": "",
            }
        ],
    )
    repaired = module.run(worker_context)
    assert repaired["status"] == "ready"
    assert repaired["worker_calls"] == 1
    assert len(worker_context.agent_calls) == 3
    assert len(worker_context.shell_calls) == 1
    assert worker_context.shell_calls[0][2] == 300

    unchanged_context = FakeContext(
        module,
        [snapshot("failing", [failure]), snapshot("failing", [failure])],
        [
            {
                "status": "changed",
                "summary": "claimed a push",
                "commit": "c" * 40,
                "handled_ids": ["check-1"],
                "needs_user_reason": "",
            }
        ],
    )
    unchanged_context.params["max_worker_failures"] = 1
    unchanged = module.run(unchanged_context)
    assert unchanged["status"] == "needs_user"
    assert "head SHA did not move" in unchanged["reason"]

    assert module.forbidden_quality_graph_text({"message": "/qg finding ignore"})
    assert not module.forbidden_quality_graph_text({"message": "fixed root cause"})


if __name__ == "__main__":
    main()
