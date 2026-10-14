import json
import os
import tempfile
import unittest
from pathlib import Path

from test_install_modes import snapshot_tree
from test_install_sh import VERSION
from test_install_sh import create_package_release
from test_install_sh import process_start_time
from test_install_sh import run_installer_in


class InstallerProvenanceTests(unittest.TestCase):
    def test_legacy_updater_cannot_replace_latest_foreground_install(self):
        for record_name in ("app-server-updater.pid", "daemon-updater.pid"):
            for guarded in (False, True):
                with (
                    self.subTest(record=record_name, guarded=guarded),
                    tempfile.TemporaryDirectory() as directory,
                ):
                    root = Path(directory)
                    archive, checksums, metadata = create_package_release(root)
                    options = dict(
                        metadata_json=metadata,
                        archive_path=archive,
                        checksum_path=checksums,
                        force_macos=True,
                    )
                    installed, requests = run_installer_in(root, "latest", **options)
                    self.assertEqual(installed.returncode, 0, installed.stderr)
                    home = root / "antex-home"
                    record = home / "app-server-daemon" / record_name
                    record.parent.mkdir()
                    record.write_text(
                        json.dumps(
                            {
                                "pid": os.getpid(),
                                "processStartTime": process_start_time(),
                            }
                        )
                    )
                    pending = home / "packages/standalone/releases/.staging.pending"
                    pending.mkdir()
                    (pending / "sentinel").write_text("concurrent install")
                    (home / "history.jsonl").write_text("preserved history")
                    protected = (home, root / "install-bin", root / "home")
                    before = [snapshot_tree(path) for path in protected]
                    rejected, after_requests = run_installer_in(
                        root,
                        "latest",
                        old_updater_parent_pid=os.getpid(),
                        update_guard_from_release=(
                            f"{VERSION}-aarch64-apple-darwin" if guarded else None
                        ),
                        **options,
                    )
                    self.assertNotEqual(rejected.returncode, 0)
                    self.assertIn("Legacy daemon updater", rejected.stderr)
                    self.assertEqual(after_requests, requests)
                    self.assertEqual(
                        [snapshot_tree(path) for path in protected], before
                    )


if __name__ == "__main__":
    unittest.main()
