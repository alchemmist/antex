import os
import signal
import subprocess
import sys
import tempfile
import unittest
from pathlib import Path

SCRIPT = Path(__file__).with_name("smoke-tui-startup.py")


@unittest.skipUnless(os.name == "posix", "PTY smoke test")
class StartupSmokeTests(unittest.TestCase):
    def run_probe(
        self, *, ignore_term=False, early_exit=False, deny_group_signal=False
    ):
        with tempfile.TemporaryDirectory() as directory:
            root = Path(directory)
            marker = root / "child.pid"
            stopped = root / "daemon-stopped"
            binary = root / "antex"
            binary.write_text(
                f"#!{sys.executable}\n"
                "import os, signal, sys, time\n"
                "from pathlib import Path\n"
                "if sys.argv[1:] == ['app-server', 'daemon', 'stop']:\n"
                f"    Path({str(stopped)!r}).write_text(os.environ['ANTEX_HOME'])\n"
                "    sys.exit(0)\n"
                "assert sys.argv[1:] == ['--no-alt-screen'], sys.argv\n"
                "assert 'ANTEX_APP_SERVER_URL' not in os.environ\n"
                "signal.signal(signal.SIGHUP, signal.SIG_IGN)\n"
                f"Path({str(marker)!r}).write_text(str(os.getpid()))\n"
                + (
                    "signal.signal(signal.SIGTERM, signal.SIG_IGN)\n"
                    if ignore_term
                    else ""
                )
                + ("sys.exit(2)\n" if early_exit else "")
                + 'print("Ask Antex to do anything", flush=True)\n'
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
                env=os.environ | {"ANTEX_APP_SERVER_URL": "ws://127.0.0.1:9"},
            )
            try:
                stdout, stderr = process.communicate(timeout=8)
                self.assertTrue(stopped.is_file(), stderr)
                temporary_home = Path(stopped.read_text())
                self.assertTrue(temporary_home.name.startswith("antex-tui-startup-"))
                self.assertFalse(temporary_home.exists())
                with self.assertRaises(ProcessLookupError):
                    os.kill(int(marker.read_text()), 0)
                return process.returncode, stdout, stderr
            finally:
                if process.poll() is None:
                    if marker.exists():
                        try:
                            os.kill(int(marker.read_text()), signal.SIGKILL)
                        except ProcessLookupError:
                            pass
                    process.kill()
                    process.communicate(timeout=2)

    def test_normal_startup_and_daemon_cleanup(self):
        code, stdout, stderr = self.run_probe()
        self.assertEqual(code, 0, stderr)
        self.assertIn("normal startup reached the composer", stdout)

    def test_startup_cleans_up_child_ignoring_sigterm(self):
        code, _, stderr = self.run_probe(ignore_term=True)
        self.assertEqual(code, 0, stderr)

    def test_startup_cleans_up_when_group_signal_is_denied(self):
        code, _, stderr = self.run_probe(ignore_term=True, deny_group_signal=True)
        self.assertEqual(code, 0, stderr)

    def test_early_exit_reports_missing_composer_and_stops_daemon(self):
        code, _, stderr = self.run_probe(early_exit=True)
        self.assertNotEqual(code, 0)
        self.assertIn("did not reach the composer", stderr)


if __name__ == "__main__":
    unittest.main()
