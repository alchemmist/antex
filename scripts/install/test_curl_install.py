import platform
import tempfile
import unittest
from pathlib import Path

from test_install_sh import INSTALL_SCRIPT
from test_install_sh import VERSION
from test_install_sh import create_package_release
from test_install_sh import run_installer_in


class CurlInstallationTests(unittest.TestCase):
    def test_stdin_installs_complete_package_outside_checkout(self):
        targets = ["aarch64-apple-darwin"]
        if platform.system() == "Linux" and platform.machine() == "x86_64":
            targets.append("x86_64-unknown-linux-gnu")
        resources = {
            "antex-resources/zsh/bin/zsh": "bundled shell",
            "antex-resources/voice/bin/antex-voice-host": "voice helper",
            "antex-resources/voice/lib/runtime": "voice runtime",
        }
        for target in targets:
            with (
                self.subTest(target=target),
                tempfile.TemporaryDirectory(prefix="antex curl ") as directory,
            ):
                root = Path(directory)
                archive, checksums, metadata = create_package_release(
                    root, target=target, extra_files=resources
                )
                result, requests = run_installer_in(
                    root,
                    "latest",
                    metadata_json=metadata,
                    archive_path=archive,
                    checksum_path=checksums,
                    force_macos=target.endswith("darwin"),
                    script_input=INSTALL_SCRIPT.read_text(),
                )
                self.assertEqual(result.returncode, 0, result.stderr)
                package = (root / "install-bin/antex").resolve().parent.parent
                self.assertEqual(
                    {name: (package / name).read_text() for name in resources},
                    resources,
                )
                self.assertTrue((package / "bin/antex-code-mode-host").is_file())
                self.assertTrue((package / "antex-path/rg").is_file())
                if "linux" in target:
                    self.assertTrue((package / "antex-resources/bwrap").is_file())
                self.assertEqual(
                    requests,
                    [
                        "https://api.github.com/repos/alchemmist/antex/releases/latest",
                        f"https://github.com/alchemmist/antex/releases/download/v{VERSION}/antex-package_SHA256SUMS",
                        f"https://github.com/alchemmist/antex/releases/download/v{VERSION}/antex-package-{target}.tar.gz",
                    ],
                )

    def test_existing_codex_data_allows_installation_before_migration(self):
        with tempfile.TemporaryDirectory() as directory:
            root = Path(directory)
            legacy = root / "home/.codex/config.toml"
            legacy.parent.mkdir(parents=True)
            legacy.write_text("existing configuration")
            archive, checksums, metadata = create_package_release(root)
            result, _ = run_installer_in(
                root,
                "latest",
                metadata_json=metadata,
                archive_path=archive,
                checksum_path=checksums,
                force_macos=True,
                script_input=INSTALL_SCRIPT.read_text(),
            )
            self.assertEqual(result.returncode, 0, result.stderr)
            self.assertTrue((root / "install-bin/antex").is_file())
            self.assertFalse((root / "antex-home").exists())
            self.assertEqual(legacy.read_text(), "existing configuration")
            self.assertIn("antex migrate", result.stdout)
            bootstrap = (root / "install-bin/antex").resolve()
            config = root / "antex-home/config.toml"
            config.parent.mkdir()
            config.write_text("migrated configuration")
            result, _ = run_installer_in(
                root,
                "latest",
                metadata_json=metadata,
                archive_path=archive,
                checksum_path=checksums,
                force_macos=True,
                script_input=INSTALL_SCRIPT.read_text(),
            )
            self.assertEqual(result.returncode, 0, result.stderr)
            installed = (root / "install-bin/antex").resolve()
            self.assertTrue(
                installed.is_relative_to(
                    (root / "antex-home/packages/standalone").resolve()
                )
            )
            self.assertTrue(bootstrap.is_file())
            self.assertEqual(config.read_text(), "migrated configuration")
            self.assertEqual(legacy.read_text(), "existing configuration")

    def test_stdin_accepts_pinned_release_argument(self):
        with tempfile.TemporaryDirectory() as directory:
            root = Path(directory)
            archive, checksums, metadata = create_package_release(root)
            result, requests = run_installer_in(
                root,
                "latest",
                metadata_json=metadata,
                archive_path=archive,
                checksum_path=checksums,
                force_macos=True,
                script_input=INSTALL_SCRIPT.read_text(),
                script_args=("--release", VERSION),
            )
            self.assertEqual(result.returncode, 0, result.stderr)
            self.assertEqual(
                requests[0],
                f"https://api.github.com/repos/alchemmist/antex/releases/tags/v{VERSION}",
            )
            self.assertFalse(
                (root / "antex-home/packages/standalone/auto-update-version").exists()
            )

    def test_stdin_rejects_corrupt_download_before_replacing_command(self):
        with tempfile.TemporaryDirectory() as directory:
            root = Path(directory)
            archive, checksums, metadata = create_package_release(root)
            archive.write_bytes(b"corrupt download")
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
            self.assertIn("checksum", result.stderr)
            self.assertEqual(command.read_text(), "previous installation")


if __name__ == "__main__":
    unittest.main()
