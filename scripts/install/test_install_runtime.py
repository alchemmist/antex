import json
import os
import select
import signal
import time
import tempfile
import unittest
from pathlib import Path

from test_install_sh import INSTALL_SCRIPT
from test_install_sh import VERSION
from test_install_sh import create_package_release
from test_install_sh import run_installer_in


class ReleaseRuntimeTests(unittest.TestCase):
    @unittest.skipUnless(os.name == "posix", "terminal-based installer")
    def test_piped_installer_can_launch_on_the_terminal(self):
        import pty

        with tempfile.TemporaryDirectory() as directory:
            root = Path(directory)
            archive, checksums, metadata = create_package_release(
                root,
                extra_files={
                    "bin/antex": f'#!/bin/sh\nif [ "$1" = --version ]; then echo antex-cli {VERSION}; exit 0; fi\n[ -t 0 ] && [ -t 1 ] && echo INTERACTIVE_ANTEX_STARTED\n'
                },
            )
            pid, terminal = pty.fork()
            if pid == 0:
                result, _ = run_installer_in(
                    root,
                    "latest",
                    metadata_json=metadata,
                    archive_path=archive,
                    checksum_path=checksums,
                    force_macos=True,
                    script_input=INSTALL_SCRIPT.read_text(),
                    interactive=True,
                )
                print(result.stdout, result.stderr, flush=True)
                os._exit(result.returncode)
            status = None
            output = b""
            answered = False
            deadline = time.monotonic() + 15
            try:
                while time.monotonic() < deadline:
                    if select.select([terminal], [], [], 0.1)[0]:
                        try:
                            output += os.read(terminal, 65536)
                        except OSError:
                            pass
                    if not answered and b"Start Antex now?" in output:
                        os.write(terminal, b"y\n")
                        answered = True
                    finished, code = os.waitpid(pid, os.WNOHANG)
                    if finished:
                        status = code
                        while select.select([terminal], [], [], 0)[0]:
                            try:
                                data = os.read(terminal, 65536)
                            except OSError:
                                break
                            if not data:
                                break
                            output += data
                        break
                self.assertEqual(status, 0, output.decode(errors="replace"))
                self.assertIn(b"INTERACTIVE_ANTEX_STARTED", output)
            finally:
                if status is None:
                    os.killpg(pid, signal.SIGTERM)
                    os.waitpid(pid, 0)
                os.close(terminal)

    def test_unrunnable_package_preserves_existing_installation(self):
        with tempfile.TemporaryDirectory() as directory:
            root = Path(directory)
            archive, checksums, metadata = create_package_release(
                root,
                extra_files={
                    "bin/antex": "#!/bin/sh\necho missing-system-library >&2\nexit 127\n",
                    "antex-package.json": json.dumps({"version": VERSION}),
                },
            )
            command = root / "install-bin/antex"
            command.parent.mkdir()
            command.write_text("previous installation")
            result, _ = run_installer_in(
                root,
                "latest",
                metadata_json=metadata,
                archive_path=archive,
                checksum_path=checksums,
                force_macos=True,
                script_input=INSTALL_SCRIPT.read_text(),
            )
            self.assertNotEqual(result.returncode, 0)
            self.assertIn("missing-system-library", result.stderr)
            self.assertEqual(command.read_text(), "previous installation")
            self.assertFalse((root / "antex-home/packages/standalone/current").exists())


if __name__ == "__main__":
    unittest.main()
