import hashlib
import importlib.util
import json
import os
import platform
import subprocess
import tarfile
import tempfile
import unittest
from pathlib import Path

SPEC = importlib.util.spec_from_file_location(
    "install_fork_package", Path(__file__).with_name("install-fork-package.py")
)
MODULE = importlib.util.module_from_spec(SPEC)
SPEC.loader.exec_module(MODULE)


class PackageInstallationTests(unittest.TestCase):
    def setUp(self):
        directory = tempfile.TemporaryDirectory()
        self.addCleanup(directory.cleanup)
        self.root = Path(directory.name)
        self.source = self.root / "package"
        self.destination = self.root / "bin"
        self.source.mkdir()
        self.target = "x86_64-unknown-linux-gnu"
        self.files = [
            "bin/antex",
            "bin/antex-code-mode-host",
            "antex-path/rg",
            "antex-resources/zsh/bin/zsh",
            "antex-resources/bwrap",
            "antex-resources/voice/bin/antex-voice-host",
            "antex-resources/voice/lib/libgstreamer-1.0.so.0",
        ]
        for name in self.files:
            path = self.source / name
            path.parent.mkdir(parents=True, exist_ok=True)
            path.write_bytes(name.encode())
            path.chmod(0o755)
        (self.source / "antex-package.json").write_text(
            json.dumps(
                {
                    "layoutVersion": 1,
                    "version": "0.1.4",
                    "variant": "antex",
                    "target": self.target,
                    "entrypoint": "bin/antex",
                    "resourcesDir": "antex-resources",
                    "pathDir": "antex-path",
                }
            )
        )
        (self.source / "antex-resources/voice/manifest.json").write_text(
            json.dumps(
                {
                    "sha256": {
                        name: hashlib.sha256(
                            (self.source / name).read_bytes()
                        ).hexdigest()
                        for name in self.files
                    },
                }
            )
        )

    def test_preserves_resources_and_physical_package_layout_after_upgrade(self):
        self.destination.mkdir()
        (self.destination / "antex").write_bytes(b"previous flat binary")
        MODULE.install_package(self.source, self.destination, self.target)
        first = (self.destination / "antex").resolve().parent.parent
        MODULE.install_package(self.source, self.destination, self.target)
        second = (self.destination / "antex").resolve().parent.parent
        self.assertNotEqual(first, second)
        self.assertEqual(
            {name: (second / name).read_bytes() for name in self.files},
            {name: (self.source / name).read_bytes() for name in self.files},
        )
        self.assertEqual((first / "bin/antex").read_bytes(), b"bin/antex")
        self.assertEqual(
            (self.destination / "antex-code-mode-host").resolve(),
            second / "bin/antex-code-mode-host",
        )

    def test_rejects_damaged_runtime_before_changing_installation(self):
        MODULE.install_package(self.source, self.destination, self.target)
        installed = (self.destination / "antex").resolve()
        (self.source / self.files[-1]).write_bytes(b"corrupted")
        with self.assertRaisesRegex(ValueError, "checksum mismatch"):
            MODULE.install_package(self.source, self.destination, self.target)
        self.assertEqual((self.destination / "antex").resolve(), installed)

    @unittest.skipUnless(
        platform.system() == "Linux" and platform.machine() == "x86_64",
        "Linux release fixture",
    )
    def test_make_installs_complete_release_archive(self):
        release = self.root / "release"
        release.mkdir()
        archive = release / f"antex-{self.target}.tar.gz"
        with tarfile.open(archive, "w:gz") as output:
            output.add(self.source, arcname=".")
        archive.with_suffix(".gz.sha256").write_text(
            hashlib.sha256(archive.read_bytes()).hexdigest()
            + "  "
            + archive.name
            + "\n"
        )
        subprocess.run(
            ["make", "install-linux"],
            cwd=Path(__file__).resolve().parent.parent,
            env=os.environ
            | {
                "ANTEX_RELEASE_BASE_URL": release.as_uri(),
                "ANTEX_INSTALL_DIR": str(self.destination),
            },
            check=True,
        )
        package = (self.destination / "antex").resolve().parent.parent
        self.assertEqual(
            (package / self.files[-1]).read_bytes(), self.files[-1].encode()
        )


if __name__ == "__main__":
    unittest.main()
