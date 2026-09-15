import argparse
import json
from pathlib import Path
import sys
import tempfile

ROOT = Path(__file__).resolve().parents[1]
sys.path.insert(0, str(ROOT / "sdk/python/src"))
sys.path.insert(0, str(ROOT / "sdk/python/tests"))

from antex_sdk import Antex, ApprovalMode, Sandbox
from app_server_harness import AppServerHarness, ev_completed, ev_response_created, sse


def main():
    parser = argparse.ArgumentParser()
    parser.add_argument("binary", type=Path)
    args = parser.parse_args()
    with tempfile.TemporaryDirectory(prefix="antex-runtime-smoke-") as directory:
        with AppServerHarness(Path(directory)) as harness:
            config = harness.app_server_config()
            config.antex_bin = str(args.binary.resolve())
            with (harness.codex_home / "config.toml").open("a") as output:
                output.write("\n[features]\ncode_mode = true\ncode_mode_host = true\n")
            harness.responses.enqueue_sse(
                sse(
                    [
                        ev_response_created("smoke-tools"),
                        {
                            "type": "response.output_item.done",
                            "item": {
                                "type": "custom_tool_call",
                                "call_id": "smoke-exec",
                                "name": "exec",
                                "input": 'const result = await tools.exec_command({cmd: "printf ANTEX_PAIR_SMOKE_OK", max_output_tokens: 100}); if (result.exit_code !== 0 || result.output !== "ANTEX_PAIR_SMOKE_OK") throw new Error("shell smoke failed: " + JSON.stringify(result)); text("ANTEX_PAIR_SMOKE_OK");',
                            },
                        },
                        ev_completed("smoke-tools"),
                    ]
                )
            )
            harness.responses.enqueue_assistant_message(
                "smoke complete", response_id="smoke-done"
            )
            with Antex(config=config) as client:
                thread = client.thread_start(
                    approval_mode=ApprovalMode.deny_all,
                    sandbox=Sandbox.read_only,
                )
                result = thread.turn("Run the smoke command.").run()
                assert result.final_response == "smoke complete", result
            requests = harness.responses.requests()
            assert len(requests) == 2, len(requests)
            outputs = [
                item
                for item in requests[1].input()
                if item.get("type") == "custom_tool_call_output"
            ]
            assert len(outputs) == 1, outputs
            text = json.dumps(outputs[0])
            assert "ANTEX_PAIR_SMOKE_OK" in text, outputs
    print(
        "Antex runtime passed: app-server, code-mode host, JavaScript, real shell tool, response"
    )


if __name__ == "__main__":
    main()
