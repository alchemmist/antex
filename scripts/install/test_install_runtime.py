import json
import tempfile
import unittest
from pathlib import Path

from test_install_sh import INSTALL_SCRIPT
from test_install_sh import VERSION
from test_install_sh import create_package_release
from test_install_sh import run_installer_in


class ReleaseRuntimeTests(unittest.TestCase):
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
