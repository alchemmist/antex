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
        self,
        *,
        ignore_term=False,
        early_exit=False,
        deny_group_signal=False,
        startup_timeout=False,
        connect_legacy=False,
    ):
        with tempfile.TemporaryDirectory() as directory:
            root = Path(directory)
            marker = root / "child.pid"
            home_marker = root / "frontend-home"
            binary = root / "antex"
            binary.write_text(
                f"#!{sys.executable}\n"
                "import os, signal, sys, time\n"
                "from pathlib import Path\n"
                "assert sys.argv[1:] == ['--no-alt-screen'], sys.argv\n"
                "assert 'ANTEX_APP_SERVER_URL' not in os.environ\n"
                "signal.signal(signal.SIGHUP, signal.SIG_IGN)\n"
                f"Path({str(marker)!r}).write_text(str(os.getpid()))\n"
                f"Path({str(home_marker)!r}).write_text(os.environ['ANTEX_HOME'])\n"
                + (
                    "signal.signal(signal.SIGTERM, signal.SIG_IGN)\n"
                    if ignore_term
                    else ""
                )
                + (
                    "import socket\n"
                    "legacy = socket.socket(socket.AF_UNIX, socket.SOCK_STREAM)\n"
                    "legacy.connect(str(Path(os.environ['ANTEX_HOME']) / 'app-server-control/app-server-control.sock'))\n"
                    if connect_legacy
                    else ""
                )
                + ("sys.exit(2)\n" if early_exit else "")
                + (
                    ""
                    if startup_timeout
                    else 'print("Ask Antex to do anything", flush=True)\n'
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
            if startup_timeout:
                command.extend(["--timeout", "0.25"])
            process = subprocess.Popen(
                command,
                stdout=subprocess.PIPE,
                stderr=subprocess.PIPE,
                text=True,
                env=os.environ | {"ANTEX_APP_SERVER_URL": "ws://127.0.0.1:9"},
            )
            try:
                stdout, stderr = process.communicate(timeout=8)
                self.assertTrue(home_marker.is_file(), stderr)
                temporary_home = Path(home_marker.read_text())
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

    def test_normal_embedded_startup_and_frontend_cleanup(self):
        code, stdout, stderr = self.run_probe()
        self.assertEqual(code, 0, stderr)
        self.assertIn("normal embedded startup reached the composer", stdout)

    def test_legacy_socket_connection_is_rejected_and_frontend_reaped(self):
        code, _, stderr = self.run_probe(connect_legacy=True)
        self.assertNotEqual(code, 0)
        self.assertIn("foreground startup connected to a legacy daemon socket", stderr)

    def test_startup_cleans_up_child_ignoring_sigterm(self):
        code, _, stderr = self.run_probe(ignore_term=True)
        self.assertEqual(code, 0, stderr)

    def test_startup_cleans_up_when_group_signal_is_denied(self):
        code, _, stderr = self.run_probe(ignore_term=True, deny_group_signal=True)
        self.assertEqual(code, 0, stderr)

    def test_timeout_reports_missing_composer_and_cleans_up_frontend(self):
        code, _, stderr = self.run_probe(startup_timeout=True, ignore_term=True)
        self.assertNotEqual(code, 0)
        self.assertIn("did not reach the composer", stderr)

    def test_early_exit_reports_missing_composer_and_reaps_frontend(self):
        code, _, stderr = self.run_probe(early_exit=True)
        self.assertNotEqual(code, 0)
        self.assertIn("did not reach the composer", stderr)


if __name__ == "__main__":
    unittest.main()
