import importlib.util
import json
import pathlib
import tempfile
import unittest

PATH = pathlib.Path(__file__).with_name("builtin_github_bot_pr_maintenance.py")
SPEC = importlib.util.spec_from_file_location("workflow", PATH)
MODULE = importlib.util.module_from_spec(SPEC)
SPEC.loader.exec_module(MODULE)


class Context:
    def __init__(self, report):
        self.params = {
            "owner": "owner",
            "action": "merge",
            "merge_method": "merge",
            "parallelism": 1,
            "max_repositories": 10,
        }
        self.state = {
            "repositories": [{"owner": "owner", "name": "repo"}],
            "next_index": 0,
            "results": [],
        }
        self.report = report
        self.requests = []
        self.retry_report = None
        self.retry_error = None

    def progress(self, *args, **kwargs):
        pass

    def checkpoint(self, state):
        self.state = state

    def agent(self, prompt, **kwargs):
        self.requests.append({"prompt": prompt, **kwargs})
        if self.retry_error:
            raise RuntimeError(self.retry_error)
        return {
            "success": True,
            "message": json.dumps(self.retry_report or self.report),
        }

    def agent_batch(self, requests, **kwargs):
        self.requests.extend(requests)
        return [{"success": True, "message": json.dumps(self.report)}]


class Tests(unittest.TestCase):
    def setUp(self):
        folder = tempfile.TemporaryDirectory()
        self.addCleanup(folder.cleanup)
        old_path = MODULE.__file__
        self.addCleanup(setattr, MODULE, "__file__", old_path)
        MODULE.__file__ = str(pathlib.Path(folder.name) / "workflow.py")

    def test_human_blocker_completes_with_actionable_report(self):
        report = self.report()
        report["needs_user"] = [
            {
                "number": 453,
                "reason": "Required review missing",
                "next_action": "Ask a repository reviewer to approve PR #453",
            }
        ]
        ctx = Context(report)
        result = MODULE.run(ctx)
        self.assertEqual(
            (result["status"], result["needs_user"], result["failed"]),
            ("needs_user", 1, 0),
        )
        self.assertEqual(len(ctx.requests), 1)
        content = pathlib.Path(result["report_path"]).read_text()
        self.assertIn("https://github.com/owner/repo/pull/453", content)
        self.assertIn(report["needs_user"][0]["next_action"], content)
        self.assertEqual(ctx.state["outcome"]["status"], "needs_user")

    def test_recovery_handoff_preserves_successes(self):
        report = self.report()
        report.update(
            candidates=2, merged=[1], failed=[{"number": 2, "reason": "unknown"}]
        )
        ctx = Context(report)
        ctx.retry_report = self.report()
        ctx.retry_report["needs_user"] = [
            {
                "number": 2,
                "reason": "Branch protection",
                "next_action": "Obtain the required review",
            }
        ]
        result = MODULE.run(ctx)
        self.assertEqual(
            (result["merged"], result["needs_user"], result["failed"]), (1, 1, 0)
        )

    def test_handoff_requires_next_action(self):
        report = self.report()
        report["needs_user"] = [{"number": 2, "reason": "blocked"}]
        with self.assertRaisesRegex(RuntimeError, "next_action"):
            MODULE.run(Context(report))

    def report(self, **kwargs):
        return dict(
            repository="owner/repo",
            candidates=1,
            merged=[],
            fixed=[],
            skipped=[],
            failed=[],
            needs_user=[],
            summary="report",
            **kwargs,
        )

    def test_failed_run_is_not_completed(self):
        report = self.report()
        report["failed"] = [
            {"number": 453, "reason": "Workflow graph validation failed"}
        ]
        ctx = Context(report)
        with self.assertRaisesRegex(RuntimeError, "Workflow graph validation failed"):
            MODULE.run(ctx)
        self.assertEqual(ctx.state["results"], [report])

    def test_missing_candidate_is_invalid(self):
        with self.assertRaisesRegex(RuntimeError, "candidate"):
            MODULE.run(Context(self.report()))

    def test_failure_requires_reason(self):
        report = self.report()
        report["failed"] = [453]
        with self.assertRaisesRegex(RuntimeError, "reason"):
            MODULE.run(Context(report))

    def test_duplicate_candidate_is_invalid(self):
        report = self.report()
        report["candidates"] = 2
        report["merged"] = [453, 453]
        with self.assertRaisesRegex(RuntimeError, "candidate"):
            MODULE.run(Context(report))

    def test_failed_checkpoint_stays_failed_on_resume(self):
        report = self.report()
        report["failed"] = [{"number": 453, "reason": "MCP approval denied"}]
        ctx = Context(report)
        with self.assertRaisesRegex(RuntimeError, "MCP approval denied"):
            MODULE.run(ctx)
        with self.assertRaisesRegex(RuntimeError, "MCP approval denied"):
            MODULE.run(ctx)

    def test_review_only_skip_completes(self):
        report = self.report()
        report["skipped"] = [{"number": 453, "reason": "review-only mode"}]
        ctx = Context(report)
        ctx.params["action"] = "review"
        result = MODULE.run(ctx)
        self.assertEqual((result["merged"], result["skipped"]), (0, 1))

    def test_approval_review_is_scoped_to_mutating_agents(self):
        for action, archived, expected in [
            ("merge", False, "auto-review"),
            ("review", False, "inherit"),
            ("merge", True, "inherit"),
        ]:
            with self.subTest(action=action, archived=archived):
                report = self.report()
                report["skipped"] = [{"number": 453, "reason": "blocked"}]
                ctx = Context(report)
                ctx.params["action"] = action
                ctx.state["repositories"][0]["archived"] = archived
                MODULE.run(ctx)
                self.assertEqual(ctx.requests[0]["approval_mode"], expected)

    def test_retries_only_failed_candidates_and_preserves_successes(self):
        report = self.report()
        report.update(
            candidates=2,
            merged=[1],
            fixed=[1],
            failed=[{"number": 2, "reason": "mergeability unknown"}],
        )
        ctx = Context(report)
        ctx.retry_report = self.report()
        ctx.retry_report["merged"] = [2]
        result = MODULE.run(ctx)
        self.assertEqual(
            (result["merged"], result["fixed"], result["failed"]), (2, 1, 0)
        )
        self.assertEqual(len(ctx.requests), 2)
        self.assertIn("Retry only these PR numbers: [2]", ctx.requests[1]["prompt"])

    def test_retry_cannot_report_an_unrequested_pr(self):
        report = self.report()
        report["failed"] = [{"number": 2, "reason": "mergeability unknown"}]
        ctx = Context(report)
        ctx.retry_report = self.report()
        ctx.retry_report["merged"] = [99]
        with self.assertRaisesRegex(RuntimeError, "retry"):
            MODULE.run(ctx)
        self.assertEqual(ctx.state["results"][0]["merged"], [])

    def test_retry_is_bounded(self):
        report = self.report()
        report["failed"] = [{"number": 2, "reason": "logs unavailable"}]
        ctx = Context(report)
        with self.assertRaises(RuntimeError):
            MODULE.run(ctx)
        self.assertEqual(len(ctx.requests), 2)

    def test_retry_transport_error_preserves_completed_work(self):
        report = self.report()
        report.update(
            candidates=2, merged=[1], failed=[{"number": 2, "reason": "unknown"}]
        )
        ctx = Context(report)
        ctx.retry_error = "agent timed out"
        with self.assertRaisesRegex(RuntimeError, "retry failed: agent timed out"):
            MODULE.run(ctx)
        self.assertEqual(ctx.state["results"][0]["merged"], [1])

    def test_review_retry_does_not_enable_mutation_approvals(self):
        report = self.report()
        report["failed"] = [{"number": 2, "reason": "unknown"}]
        ctx = Context(report)
        ctx.params["action"] = "review"
        with self.assertRaises(RuntimeError):
            MODULE.run(ctx)
        self.assertEqual(ctx.requests[1]["approval_mode"], "inherit")

    def test_execution_error_also_saves_report(self):
        report = self.report()
        report["failed"] = [{"number": 2, "reason": "API unavailable"}]
        ctx = Context(report)
        with self.assertRaisesRegex(RuntimeError, "API unavailable"):
            MODULE.run(ctx)
        self.assertEqual(ctx.state["outcome"]["status"], "failed")
        self.assertIn(
            "API unavailable",
            pathlib.Path(ctx.state["outcome"]["report_path"]).read_text(),
        )

    def test_legacy_report_without_handoffs_is_accepted(self):
        report = self.report()
        del report["needs_user"]
        report["merged"] = [1]
        self.assertEqual(MODULE.run(Context(report))["status"], "completed")

    def test_invalid_report_is_repaired_without_repeating_mutations(self):
        report = self.report()
        report.update(candidates=2, merged=[1])
        ctx = Context(report)
        ctx.retry_report = self.report()
        ctx.retry_report["merged"] = [1]
        result = MODULE.run(ctx)
        self.assertEqual(result["merged"], 1)
        self.assertEqual(len(ctx.requests), 2)
        self.assertEqual(ctx.requests[1]["sandbox"], "read-only")
        raw = pathlib.Path(MODULE.__file__).parent / "agent-reports/owner-repo.txt"
        self.assertEqual(json.loads(raw.read_text()), report)

    def test_report_repair_cannot_invent_merges(self):
        report = self.report()
        report["candidates"] = 2
        ctx = Context(report)
        ctx.retry_report = self.report()
        ctx.retry_report["merged"] = [99]
        with self.assertRaisesRegex(RuntimeError, "changed recorded merged"):
            MODULE.run(ctx)

    def test_report_repair_cannot_drop_candidates(self):
        report = self.report()
        report.update(
            candidates=3,
            skipped=[
                {"number": 1, "reason": "closed"},
                {"number": 2, "reason": "closed"},
            ],
        )
        ctx = Context(report)
        ctx.retry_report = self.report()
        ctx.retry_report["skipped"] = [{"number": 1, "reason": "closed"}]
        with self.assertRaisesRegex(RuntimeError, "dropped recorded candidates"):
            MODULE.run(ctx)

    def test_human_draft_skip_is_not_a_bot_candidate(self):
        report = self.report()
        report.update(
            candidates=0,
            skipped=[
                {
                    "number": 1,
                    "reason": "draft PR authored by human account alchemmist, not a bot",
                }
            ],
        )
        ctx = Context(report)
        result = MODULE.run(ctx)
        self.assertEqual(
            (result["status"], result["merged"], result["skipped"], result["failed"]),
            ("completed", 0, 1, 0),
        )
        self.assertEqual(len(ctx.requests), 1)
        self.assertEqual(ctx.state["results"][0]["candidates"], 0)

    def test_bot_candidates_can_coexist_with_excluded_prs(self):
        report = self.report()
        report.update(
            candidates=1, merged=[2], skipped=[{"number": 1, "reason": "human PR"}]
        )
        ctx = Context(report)
        result = MODULE.run(ctx)
        self.assertEqual((result["merged"], result["skipped"]), (1, 1))
        self.assertEqual(len(ctx.requests), 1)

    def test_non_skipped_outcome_cannot_exceed_candidate_count(self):
        report = self.report()
        report.update(candidates=0, merged=[2])
        with self.assertRaisesRegex(RuntimeError, "candidate"):
            MODULE.run(Context(report))

    def test_successful_merge(self):
        report = self.report()
        report["merged"] = [453]
        self.assertEqual(MODULE.run(Context(report))["merged"], 1)


if __name__ == "__main__":
    unittest.main()
