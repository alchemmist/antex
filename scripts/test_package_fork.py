import hashlib
import importlib.util
import json
import os
import subprocess
import sys
import tarfile
import tempfile
import unittest
from pathlib import Path
from unittest.mock import patch

SPEC = importlib.util.spec_from_file_location(
    "package_fork", Path(__file__).with_name("package-fork.py")
)
MODULE = importlib.util.module_from_spec(SPEC)
SPEC.loader.exec_module(MODULE)
COMMIT = "a" * 40


class ForkPackageTests(unittest.TestCase):
    def setUp(self):
        directory = tempfile.TemporaryDirectory()
        self.addCleanup(directory.cleanup)
        self.root = Path(directory.name)
        self.output = self.root / "package"
        self.input = self.root / "executable"
        self.input.write_bytes(b"prebuilt executable")
        self.input.chmod(0o755)

    def arguments(self, target, *extra):
        return [
            "package-fork.py",
            "--target",
            target,
            "--commit",
            COMMIT,
            "--output",
            str(self.output),
            "--entrypoint-bin",
            str(self.input),
            "--code-mode-host-bin",
            str(self.input),
            "--rg-bin",
            str(self.input),
            "--bwrap-bin",
            str(self.input),
            *extra,
        ]

    def test_canonical_package_archives_and_checksums(self):
        for target in ("aarch64-apple-darwin", "x86_64-unknown-linux-gnu"):
            with self.subTest(target=target):
                self.output = self.root / target
                with patch.object(sys, "argv", self.arguments(target)):
                    MODULE.main()
                expected = {
                    "bin/antex": self.input.read_bytes(),
                    "bin/antex-code-mode-host": self.input.read_bytes(),
                    "antex-path/rg": self.input.read_bytes(),
                    "antex-package.json": (
                        self.output / "antex-package.json"
                    ).read_bytes(),
                }
                if "linux" in target:
                    expected["antex-resources/bwrap"] = self.input.read_bytes()
                actual = {
                    path.relative_to(self.output).as_posix(): path.read_bytes()
                    for path in self.output.rglob("*")
                    if path.is_file()
                }
                self.assertEqual(actual, expected)
                metadata = json.loads(actual["antex-package.json"])
                self.assertEqual(
                    metadata,
                    {
                        "layoutVersion": 1,
                        "version": (MODULE.ROOT / "FORK_VERSION").read_text().strip()
                        + "+"
                        + COMMIT,
                        "upstreamVersion": MODULE.read_workspace_version(),
                        "target": target,
                        "variant": "antex",
                        "entrypoint": "bin/antex",
                        "resourcesDir": "antex-resources",
                        "pathDir": "antex-path",
                    },
                )
                for stem in ("antex", "antex-package"):
                    archive = self.root / f"{stem}-{target}.tar.gz"
                    with tarfile.open(archive) as contents:
                        archived = {
                            member.name.removeprefix("./"): contents.extractfile(
                                member
                            ).read()
                            for member in contents.getmembers()
                            if member.isfile()
                        }
                    self.assertEqual(archived, expected)
                    digest = hashlib.sha256(archive.read_bytes()).hexdigest()
                    self.assertEqual(
                        archive.with_suffix(".gz.sha256").read_text(),
                        f"{digest}  {archive.name}\n",
                    )

    def test_release_validates_version_and_preserves_inputs_when_stripping(self):
        target = "x86_64-unknown-linux-gnu"
        run = subprocess.run
        calls = []

        def run_packaging(command, **kwargs):
            calls.append(command)
            if command[0] != "strip":
                return run(command, **kwargs)
            for name in command[1:]:
                Path(name).write_bytes(b"stripped executable")

        version = (MODULE.ROOT / "FORK_VERSION").read_text().strip()
        with (
            patch.object(sys, "argv", self.arguments(target, "--release")),
            patch.dict(os.environ, {"GITHUB_REF": f"refs/tags/v{version}"}),
            patch.object(MODULE.subprocess, "run", side_effect=run_packaging),
            patch.object(
                MODULE.subprocess,
                "check_output",
                return_value=f"antex {MODULE.read_workspace_version()}\n",
            ),
        ):
            MODULE.main()
        self.assertEqual(self.input.read_bytes(), b"prebuilt executable")
        self.assertEqual(
            (self.output / "bin/antex").read_bytes(), b"stripped executable"
        )
        self.assertEqual(
            (self.output / "bin/antex-code-mode-host").read_bytes(),
            b"stripped executable",
        )
        self.assertEqual(
            json.loads((self.output / "antex-package.json").read_text())["version"],
            version,
        )
        self.assertEqual(
            len([command for command in calls if command[0] == "strip"]), 1
        )

    def test_release_rejects_mismatched_entrypoint_version(self):
        version = (MODULE.ROOT / "FORK_VERSION").read_text().strip()
        with (
            patch.object(
                sys, "argv", self.arguments("x86_64-unknown-linux-gnu", "--release")
            ),
            patch.dict(os.environ, {"GITHUB_REF": f"refs/tags/v{version}"}),
            patch.object(
                MODULE.subprocess, "check_output", return_value="antex 999.0.0\n"
            ),
            self.assertRaisesRegex(ValueError, "Cargo workspace version"),
        ):
            MODULE.main()
        self.assertFalse(self.output.exists())

    def test_rejects_mismatched_release_tag_before_assembly(self):
        with (
            patch.object(
                sys, "argv", self.arguments("aarch64-apple-darwin", "--release")
            ),
            patch.dict(os.environ, {"GITHUB_REF": "refs/tags/v999.0.0"}),
            patch.object(MODULE.subprocess, "run") as run,
            self.assertRaises(SystemExit),
        ):
            MODULE.main()
        run.assert_not_called()

    def test_rejects_short_commit_before_assembly(self):
        with (
            patch.object(
                sys,
                "argv",
                self.arguments("aarch64-apple-darwin", "--commit", "abcdef"),
            ),
            patch.object(MODULE.subprocess, "run") as run,
            self.assertRaises(SystemExit),
        ):
            MODULE.main()
        run.assert_not_called()

    def test_preserves_existing_output(self):
        self.output.mkdir()
        marker = self.output / "existing"
        marker.write_bytes(b"keep")
        with (
            patch.object(sys, "argv", self.arguments("aarch64-apple-darwin")),
            self.assertRaises(SystemExit),
        ):
            MODULE.main()
        self.assertEqual(marker.read_bytes(), b"keep")


if __name__ == "__main__":
    unittest.main()
