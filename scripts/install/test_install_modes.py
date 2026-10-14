"""Retired installer callers must fail before touching any installation state."""

import os
import subprocess
import tempfile
import unittest
from pathlib import Path


class InstallerModeTests(unittest.TestCase):
    def test_retired_modes_reject_before_mutation(self):
        modes = (
            "ANTEX_INSTALL_DAEMON_ONLY",
            "ANTEX_INSTALL_DEFER_SELECTION",
            "ANTEX_INSTALL_IF_CURRENT",
        )
        for mode in modes:
            for value in ("1", "true"):
                for existing in (False, True):
                    with (
                        self.subTest(mode=mode, value=value, existing=existing),
                        tempfile.TemporaryDirectory() as temporary,
                    ):
                        root = Path(temporary)
                        home = root / "home"
                        home.mkdir()
                        scratch = root / "tmp"
                        scratch.mkdir()
                        if existing:
                            for name in (
                                "bin/antex",
                                "home/.profile",
                                "antex/config.toml",
                                "antex/history.jsonl",
                                "antex/app-server-daemon/settings.json",
                                "antex/packages/app-server-daemon/current/sentinel",
                                "antex/packages/standalone/current/sentinel",
                                "antex/packages/standalone/auto-update-version",
                                "antex/packages/standalone/releases/.staging.pending/sentinel",
                            ):
                                path = root / name
                                path.parent.mkdir(parents=True, exist_ok=True)
                                path.write_text(f"preserve {name}\n")
                        environment = os.environ | {
                            "HOME": str(home),
                            "USERPROFILE": str(home),
                            "LOCALAPPDATA": str(home),
                            "TMPDIR": str(scratch),
                            "TEMP": str(scratch),
                            "TMP": str(scratch),
                            "ANTEX_HOME": str(root / "antex"),
                            "ANTEX_INSTALL_DIR": str(root / "bin"),
                            "ANTEX_NON_INTERACTIVE": "1",
                            "ANTEX_RELEASE": "latest",
                            "ANTEX_INSTALL_IF_LATEST": "1",
                            "ANTEX_UPDATE_FROM_RELEASE": "old-release",
                            **dict.fromkeys(modes, "0"),
                            mode: value,
                        }
                        if os.name == "nt":
                            command = [
                                "powershell.exe",
                                "-NoProfile",
                                "-NonInteractive",
                                "-ExecutionPolicy",
                                "Bypass",
                                "-File",
                                str(Path(__file__).resolve().with_name("install.ps1")),
                            ]
                        else:
                            command = [
                                "/bin/sh",
                                str(Path(__file__).resolve().with_name("install.sh")),
                            ]
                        before = snapshot_tree(root)
                        result = subprocess.run(
                            command,
                            env=environment,
                            cwd=root,
                            capture_output=True,
                            text=True,
                            timeout=10,
                        )
                        self.assertNotEqual(result.returncode, 0)
                        self.assertIn(mode, result.stderr)
                        self.assertIn("is no longer supported", result.stderr)
                        self.assertEqual(snapshot_tree(root), before)


def snapshot_tree(root):
    return {
        str(path.relative_to(root)): (
            ("link", os.readlink(path))
            if path.is_symlink()
            else ("directory",)
            if path.is_dir()
            else ("file", path.read_bytes())
        )
        for path in root.rglob("*")
    }


if __name__ == "__main__":
    unittest.main()
