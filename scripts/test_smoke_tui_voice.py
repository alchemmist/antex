import os
import signal
import subprocess
import sys
import tempfile
import unittest
from pathlib import Path

SCRIPT = Path(__file__).with_name("smoke-tui-voice.py")


@unittest.skipUnless(os.name == "posix", "PTY smoke test")
class VoiceSmokeTests(unittest.TestCase):
    def run_probe(
        self,
        *,
        ignore_term,
        recognize_voice,
        exit_before_startup=False,
        deny_group_signal=False,
    ):
        with tempfile.TemporaryDirectory() as directory:
            root = Path(directory)
            marker = root / "child.pid"
            binary = root / "antex"
            binary.write_text(
                f"#!{sys.executable}\n"
                "import os, signal, sys, time\n"
                "from pathlib import Path\n"
                "signal.signal(signal.SIGHUP, signal.SIG_IGN)\n"
                f"Path({str(marker)!r}).write_text(str(os.getpid()))\n"
                + (
                    "signal.signal(signal.SIGTERM, signal.SIG_IGN)\n"
                    if ignore_term
                    else ""
                )
                + ("sys.exit(2)\n" if exit_before_startup else "")
                + 'print("gpt-6-astra", flush=True)\n'
                + "command = sys.stdin.readline().strip()\n"
                + 'assert command == "/voice settings", command\n'
                + (
                    'print("Select voice for the next voice conversation", flush=True)\n'
                    if recognize_voice
                    else "print(\"Unrecognized command '/voice'\", flush=True)\n"
                )
                + "while True: time.sleep(1)\n"
            )
            binary.chmod(0o755)
            command = [sys.executable, str(SCRIPT), str(binary)]
            if deny_group_signal:
                command = [
                    sys.executable,
                    "-c",
                    "import os, runpy, sys\n"
                    "def deny(*args): raise PermissionError(1, 'Operation not permitted')\n"
                    "os.killpg = deny\n"
                    "sys.argv = sys.argv[1:]\n"
                    "runpy.run_path(sys.argv[0], run_name='__main__')\n",
                    str(SCRIPT),
                    str(binary),
                ]
            process = subprocess.Popen(
                command,
                stdout=subprocess.PIPE,
                stderr=subprocess.PIPE,
                text=True,
            )
            try:
                try:
                    stdout, stderr = process.communicate(timeout=8)
                except subprocess.TimeoutExpired:
                    self.fail(
                        "voice smoke hung instead of completing bounded PTY cleanup"
                    )
                return process.returncode, stdout, stderr
            finally:
                if process.poll() is None:
                    if marker.exists():
                        try:
                            os.kill(int(marker.read_text()), signal.SIGKILL)
                        except ProcessLookupError:
                            pass
                    try:
                        process.communicate(timeout=2)
                    except subprocess.TimeoutExpired:
                        process.kill()
                        process.communicate(timeout=2)

    def test_picker_passes_when_group_signal_is_denied(self):
        code, stdout, stderr = self.run_probe(
            ignore_term=True, recognize_voice=True, deny_group_signal=True
        )
        self.assertEqual(code, 0, stderr)
        self.assertIn("voice picker", stdout)

    def test_early_exit_reports_missing_picker(self):
        code, _, stderr = self.run_probe(
            ignore_term=False, recognize_voice=False, exit_before_startup=True
        )
        self.assertNotEqual(code, 0)
        self.assertIn("did not open the voice picker", stderr)

    def test_picker_passes_with_normal_termination(self):
        code, stdout, stderr = self.run_probe(ignore_term=False, recognize_voice=True)
        self.assertEqual(code, 0, stderr)
        self.assertIn("voice picker", stdout)

    def test_picker_passes_when_cli_ignores_sigterm(self):
        code, stdout, stderr = self.run_probe(ignore_term=True, recognize_voice=True)
        self.assertEqual(code, 0, stderr)
        self.assertIn("voice picker", stdout)

    def test_rejected_voice_command_is_reported_when_cli_ignores_sigterm(self):
        code, _, stderr = self.run_probe(ignore_term=True, recognize_voice=False)
        self.assertNotEqual(code, 0)
        self.assertIn("does not recognize /voice", stderr)


if __name__ == "__main__":
    unittest.main()
