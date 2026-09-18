#!/usr/bin/env python3

import hashlib
import json
import os
import platform
from pathlib import Path
import subprocess
import tarfile
import tempfile
import textwrap
import unittest


INSTALL_SCRIPT = Path(__file__).with_name("install.sh")
VERSION = "0.142.5"
MISMATCH_VERSION = "0.145.0"


class InstallShTest(unittest.TestCase):
    @unittest.skipUnless(
        platform.system() == "Linux" and platform.machine() == "x86_64",
        "GNU Linux fixture",
    )
    def test_gnu_release_uses_complete_package_layout(self) -> None:
        with tempfile.TemporaryDirectory() as temporary:
            root = Path(temporary)
            archive, checksums, metadata = create_package_release(
                root, target="x86_64-unknown-linux-gnu"
            )
            result, requests = run_installer_in(
                root,
                VERSION,
                archive_path=archive,
                checksum_path=checksums,
                metadata_json=metadata,
            )
            self.assertEqual(result.returncode, 0, result.stderr)
            current = root / "antex-home/packages/standalone/current"
            self.assertTrue((current / "antex-path/rg").is_file())
            self.assertTrue(
                any(
                    url.endswith("antex-package-x86_64-unknown-linux-gnu.tar.gz")
                    for url in requests
                )
            )

    def test_metadata_fetch_failure_is_not_reported_as_missing_assets(self) -> None:
        result, requests = run_installer(VERSION, metadata_failure=True)

        self.assertNotEqual(result.returncode, 0)
        self.assertEqual(
            requests,
            [f"https://api.github.com/repos/alchemmist/antex/releases/tags/v{VERSION}"],
        )
        self.assertIn(
            f"Could not fetch GitHub release metadata for Antex {VERSION}",
            result.stderr,
        )
        self.assertNotIn("Could not find Antex package", result.stderr)

    def test_exact_release_opt_out_uses_github_metadata_once(self) -> None:
        result, requests = run_installer(VERSION, use_mirror=False)

        self.assertNotEqual(result.returncode, 0)
        self.assertEqual(
            requests,
            [
                "https://api.github.com/repos/alchemmist/antex/releases/tags/"
                f"v{VERSION}",
                "https://github.com/alchemmist/antex/releases/download/"
                f"v{VERSION}/antex-package_SHA256SUMS",
            ],
        )
        self.assertIn(f"Resolved version: {VERSION}", result.stdout)

    def test_alpha_hotfix_release_is_valid(self) -> None:
        version = "0.145.0-alpha.23.1"
        result, requests = run_installer(version)

        self.assertNotEqual(result.returncode, 0)
        self.assertEqual(
            requests,
            [
                "https://api.github.com/repos/alchemmist/antex/releases/tags/"
                f"v{version}",
                "https://github.com/alchemmist/antex/releases/download/"
                f"v{version}/antex-package_SHA256SUMS",
            ],
        )
        self.assertIn(f"Resolved version: {version}", result.stdout)

    def test_latest_release_reuses_version_metadata(self) -> None:
        result, requests = run_installer("latest")

        self.assertNotEqual(result.returncode, 0)
        self.assertEqual(
            requests,
            [
                "https://api.github.com/repos/alchemmist/antex/releases/latest",
                "https://github.com/alchemmist/antex/releases/download/"
                f"v{VERSION}/antex-package_SHA256SUMS",
            ],
        )
        self.assertIn(f"Resolved version: {VERSION}", result.stdout)

    def test_compact_metadata_is_independent_of_field_order(self) -> None:
        result, requests = run_installer(
            "latest", metadata_json=release_metadata(compact=True, reorder=True)
        )

        self.assertNotEqual(result.returncode, 0)
        self.assertEqual(
            requests,
            [
                "https://api.github.com/repos/alchemmist/antex/releases/latest",
                "https://github.com/alchemmist/antex/releases/download/"
                f"v{VERSION}/antex-package_SHA256SUMS",
            ],
        )
        self.assertIn(f"Resolved version: {VERSION}", result.stdout)

    def test_json_like_strings_and_nested_fields_do_not_define_assets(self) -> None:
        result, requests = run_installer(
            VERSION, metadata_json=legacy_release_metadata_with_decoys()
        )

        self.assertNotEqual(result.returncode, 0)
        self.assertEqual(len(requests), 2)
        self.assertIn("/antex-npm-", requests[1])
        self.assertNotIn("antex-package_SHA256SUMS", requests[1])

    def test_macos_install_exposes_code_mode_host_beside_antex(self) -> None:
        with tempfile.TemporaryDirectory() as temp_dir:
            root = Path(temp_dir)
            archive_path, checksum_path, metadata_json = create_package_release(root)

            result, _requests = run_installer_in(
                root,
                VERSION,
                metadata_json=metadata_json,
                archive_path=archive_path,
                checksum_path=checksum_path,
                force_macos=True,
            )

            self.assertEqual(result.returncode, 0, result.stderr)
            install_bin = root / "install-bin"
            current = root / "antex-home" / "packages" / "standalone" / "current"
            antex_path = install_bin / "antex"
            host_path = install_bin / "antex-code-mode-host"
            self.assertEqual(os.readlink(antex_path), str(current / "bin" / "antex"))
            self.assertEqual(
                os.readlink(host_path),
                str(current / "bin" / "antex-code-mode-host"),
            )
            self.assertTrue(os.access(host_path, os.X_OK))

    def test_flat_release_keeps_fork_version_and_installs_matching_host(self) -> None:
        with tempfile.TemporaryDirectory() as temp_dir:
            root = Path(temp_dir)
            create_package_release(root)
            version = "0.0.18"
            asset = "antex-aarch64-apple-darwin.tar.gz"
            archive_path = root / asset
            with tarfile.open(archive_path, "w:gz") as archive:
                for name in ("antex", "antex-code-mode-host"):
                    archive.add(root / "package" / "bin" / name, arcname=name)
            metadata = json.dumps(
                {
                    "tag_name": f"v{version}",
                    "assets": [
                        {
                            "name": asset,
                            "digest": "sha256:"
                            + hashlib.sha256(archive_path.read_bytes()).hexdigest(),
                        }
                    ],
                }
            )
            for _ in range(2):
                result, requests = run_installer_in(
                    root,
                    version,
                    metadata_json=metadata,
                    archive_path=archive_path,
                    force_macos=True,
                )
                self.assertEqual(result.returncode, 0, result.stderr)
            current = root / "antex-home" / "packages" / "standalone" / "current"
            self.assertEqual(
                json.loads((current / "antex-package.json").read_text()),
                {"version": version},
            )
            self.assertTrue((root / "install-bin" / "antex-code-mode-host").is_file())
            self.assertEqual(
                requests,
                [
                    f"https://api.github.com/repos/alchemmist/antex/releases/tags/v{version}",
                    f"https://github.com/alchemmist/antex/releases/download/v{version}/{asset}",
                    f"https://api.github.com/repos/alchemmist/antex/releases/tags/v{version}",
                ],
            )

    def test_configured_mirror_installs_verified_package(self) -> None:
        with tempfile.TemporaryDirectory() as temp_dir:
            root = Path(temp_dir)
            archive_path, checksum_path, metadata_json = create_package_release(root)

            result, requests = run_installer_in(
                root,
                "latest",
                metadata_json=metadata_json,
                archive_path=archive_path,
                checksum_path=checksum_path,
                force_macos=True,
                use_mirror=True,
            )

            self.assertEqual(result.returncode, 0, result.stderr)
            self.assertEqual(
                requests,
                [
                    "https://releases.example.com/antex/channels/latest",
                    f"https://releases.example.com/antex/releases/{VERSION}/antex-package_SHA256SUMS",
                    f"https://releases.example.com/antex/releases/{VERSION}/antex-package-aarch64-apple-darwin.tar.gz",
                ],
            )

    def test_daemon_install_preserves_visible_cli_and_profile(self) -> None:
        with tempfile.TemporaryDirectory() as temp_dir:
            root = Path(temp_dir).resolve()
            archive, checksum, metadata = create_package_release(root)
            options = dict(
                metadata_json=metadata,
                archive_path=archive,
                checksum_path=checksum,
                force_macos=True,
            )
            pinned, _ = run_installer_in(root, VERSION, **options)
            self.assertEqual(pinned.returncode, 0, pinned.stderr)
            visible = root / "install-bin/antex"
            original = visible.resolve()
            pending = visible.with_name(".antex.12345")
            pending.symlink_to(original)
            profile = root / "home/.profile"
            profile.write_text("unchanged profile\n")

            installed, _ = run_installer_in(root, "latest", daemon_only=True, **options)
            self.assertEqual(installed.returncode, 0, installed.stderr)
            daemon = root / "antex-home/packages/app-server-daemon"
            release_name = f"{VERSION}-aarch64-apple-darwin"
            self.assertEqual(
                (daemon / "current").resolve(), daemon / "releases" / release_name
            )
            self.assertEqual((daemon / "auto-update-version").read_text(), release_name)
            options["daemon_only"] = True
            local = daemon / "releases/local-development"
            local.mkdir()
            (daemon / "current").unlink()
            (daemon / "current").symlink_to(local)
            marker = daemon / "auto-update-version"
            marker.unlink()

            scheduled, _ = run_installer_in(
                root, "latest", update_guard_from_release=local.name, **options
            )
            self.assertEqual(scheduled.returncode, 0, scheduled.stderr)
            self.assertEqual((daemon / "current").resolve(), local)
            self.assertFalse(marker.exists())

            raced, _ = run_installer_in(
                root,
                "latest",
                update_guard_from_release="a-different-release",
                manual_update=True,
                **options,
            )
            self.assertNotEqual(raced.returncode, 0, raced.stderr)
            self.assertEqual((daemon / "current").resolve(), local)
            self.assertFalse(marker.exists())

            binary = daemon / "releases" / release_name / "bin/antex"
            contents = binary.read_text()
            binary.write_text(
                '#!/bin/sh\n[ "$1" = "--version" ] || exit 2\n'
                f"echo codex-cli {VERSION}\n"
            )
            incompatible, _ = run_installer_in(
                root,
                "latest",
                update_guard_from_release=local.name,
                manual_update=True,
                **options,
            )
            self.assertNotEqual(incompatible.returncode, 0)
            self.assertIn("does not support daemon-owned packages", incompatible.stderr)
            self.assertEqual((daemon / "current").resolve(), local)
            self.assertFalse(marker.exists())
            binary.write_text(contents)

            restored, _ = run_installer_in(
                root,
                "latest",
                update_guard_from_release=local.name,
                manual_update=True,
                **options,
            )
            self.assertEqual(restored.returncode, 0, restored.stderr)
            self.assertEqual(
                (daemon / "current").resolve(), daemon / "releases" / release_name
            )
            self.assertEqual(marker.read_text(), release_name)
            self.assertEqual(visible.resolve(), original)
            self.assertTrue(pending.is_symlink())
            self.assertEqual(profile.read_text(), "unchanged profile\n")
            self.assertFalse(
                (root / "antex-home/packages/standalone/auto-update-version").exists()
            )

    def test_explicit_release_pins_even_the_current_latest_version(self) -> None:
        with tempfile.TemporaryDirectory() as temp_dir:
            root = Path(temp_dir)
            archive, checksum, metadata = create_package_release(root)
            options = dict(
                metadata_json=metadata,
                archive_path=archive,
                checksum_path=checksum,
                force_macos=True,
            )
            marker = root / "antex-home/packages/standalone/auto-update-version"
            latest, _ = run_installer_in(root, "latest", **options)
            self.assertEqual(latest.returncode, 0, latest.stderr)
            release_name = f"{VERSION}-aarch64-apple-darwin"
            self.assertEqual(marker.read_text(), release_name)

            pinned, _ = run_installer_in(root, VERSION, **options)
            self.assertEqual(pinned.returncode, 0, pinned.stderr)
            self.assertFalse(marker.exists())

            updater_record = (
                root / "antex-home/app-server-daemon/app-server-updater.pid"
            )
            updater_record.parent.mkdir(parents=True)
            updater_record.write_text(
                json.dumps(
                    {"pid": os.getpid(), "processStartTime": process_start_time()}
                )
            )
            old_updater, _ = run_installer_in(
                root, "latest", old_updater_parent_pid=os.getpid(), **options
            )
            self.assertEqual(old_updater.returncode, 0, old_updater.stderr)
            self.assertFalse(marker.exists())

            skipped, _ = run_installer_in(
                root, "latest", update_guard_from_release=release_name, **options
            )
            self.assertEqual(skipped.returncode, 0, skipped.stderr)
            self.assertFalse(marker.exists())

            updater_record.write_text(
                json.dumps({"pid": os.getpid(), "processStartTime": "stale"})
            )
            latest_again, _ = run_installer_in(
                root, "latest", old_updater_parent_pid=os.getpid(), **options
            )
            self.assertEqual(latest_again.returncode, 0, latest_again.stderr)
            self.assertEqual(marker.read_text(), release_name)

            managed = (
                root
                / f"antex-home/packages/standalone/releases/{release_name}/bin/antex"
            )
            managed.unlink()
            guarded, _ = run_installer_in(
                root,
                "latest",
                update_guard_from_release=release_name,
                **options,
            )
            self.assertEqual(guarded.returncode, 0, guarded.stderr)
            self.assertTrue(managed.exists())

    def test_uninspectable_legacy_updater_does_not_clear_pin(self) -> None:
        with tempfile.TemporaryDirectory() as temp_dir:
            root = Path(temp_dir)
            archive, checksum, metadata = create_package_release(root)
            options = dict(
                metadata_json=metadata,
                archive_path=archive,
                checksum_path=checksum,
                force_macos=True,
            )
            pinned, _ = run_installer_in(root, VERSION, **options)
            self.assertEqual(pinned.returncode, 0, pinned.stderr)
            updater_record = (
                root / "antex-home/app-server-daemon/app-server-updater.pid"
            )
            updater_record.parent.mkdir(parents=True)
            updater_record.write_text(
                json.dumps(
                    {"pid": os.getpid(), "processStartTime": process_start_time()}
                )
            )

            attempted, _ = run_installer_in(
                root,
                "latest",
                old_updater_parent_pid=os.getpid(),
                fail_ps=True,
                **options,
            )
            self.assertEqual(attempted.returncode, 0, attempted.stderr)
            self.assertFalse(
                (root / "antex-home/packages/standalone/auto-update-version").exists()
            )

    def test_releases_unusable_metadata_falls_back_to_github(self) -> None:
        unusable_metadata = {
            "html": "<html>proxy error</html>",
            "empty": "",
            "malformed_json": '{"tag_name":',
            "missing_tag": json.dumps({"assets": []}),
            "missing_assets": json.dumps({"tag_name": f"v{VERSION}", "assets": []}),
            "invalid_checksum_digest": json.dumps(
                {
                    "tag_name": f"v{VERSION}",
                    "assets": [
                        {
                            "name": "antex-package-aarch64-apple-darwin.tar.gz",
                            "digest": "sha256:" + "a" * 64,
                        },
                        {
                            "name": "antex-package_SHA256SUMS",
                            "digest": "sha256:" + "z" * 64,
                        },
                    ],
                }
            ),
            "invalid_version": json.dumps({"tag_name": "vinvalid"}),
        }

        for name, releases_metadata_json in unusable_metadata.items():
            with self.subTest(metadata=name):
                with tempfile.TemporaryDirectory() as temp_dir:
                    root = Path(temp_dir)
                    archive_path, checksum_path, metadata_json = create_package_release(
                        root
                    )

                    result, requests = run_installer_in(
                        root,
                        "latest",
                        metadata_json=metadata_json,
                        releases_metadata_json=releases_metadata_json,
                        archive_path=archive_path,
                        checksum_path=checksum_path,
                        force_macos=True,
                        use_mirror=True,
                    )

                    self.assertEqual(result.returncode, 0, result.stderr)
                    self.assertEqual(
                        requests,
                        [
                            "https://releases.example.com/antex/channels/latest",
                            "https://api.github.com/repos/alchemmist/antex/releases/latest",
                            "https://github.com/alchemmist/antex/releases/download/"
                            f"v{VERSION}/antex-package_SHA256SUMS",
                            "https://github.com/alchemmist/antex/releases/download/"
                            f"v{VERSION}/antex-package-aarch64-apple-darwin.tar.gz",
                        ],
                    )
                    self.assertIn("falling back to GitHub Releases", result.stderr)

    def test_releases_exact_metadata_version_mismatch_falls_back_to_github(
        self,
    ) -> None:
        with tempfile.TemporaryDirectory() as temp_dir:
            root = Path(temp_dir)
            archive_path, checksum_path, metadata_json = create_package_release(root)
            releases_metadata = json.loads(metadata_json)
            releases_metadata["tag_name"] = f"v{MISMATCH_VERSION}"

            result, requests = run_installer_in(
                root,
                VERSION,
                metadata_json=metadata_json,
                releases_metadata_json=json.dumps(releases_metadata),
                archive_path=archive_path,
                checksum_path=checksum_path,
                force_macos=True,
                use_mirror=True,
            )

            self.assertEqual(result.returncode, 0, result.stderr)
            self.assertEqual(
                requests,
                [
                    f"https://releases.example.com/antex/releases/{VERSION}/release.json",
                    "https://api.github.com/repos/alchemmist/antex/releases/tags/"
                    f"v{VERSION}",
                    "https://github.com/alchemmist/antex/releases/download/"
                    f"v{VERSION}/antex-package_SHA256SUMS",
                    "https://github.com/alchemmist/antex/releases/download/"
                    f"v{VERSION}/antex-package-aarch64-apple-darwin.tar.gz",
                ],
            )
            self.assertIn("falling back to GitHub Releases", result.stderr)

    def test_releases_asset_download_falls_back_to_github(self) -> None:
        with tempfile.TemporaryDirectory() as temp_dir:
            root = Path(temp_dir)
            archive_path, checksum_path, metadata_json = create_package_release(root)

            result, requests = run_installer_in(
                root,
                "latest",
                metadata_json=metadata_json,
                archive_path=archive_path,
                checksum_path=checksum_path,
                force_macos=True,
                use_mirror=True,
                releases_mode="asset_fallback",
            )

            self.assertEqual(result.returncode, 0, result.stderr)
            self.assertEqual(
                requests,
                [
                    "https://releases.example.com/antex/channels/latest",
                    f"https://releases.example.com/antex/releases/{VERSION}/antex-package_SHA256SUMS",
                    "https://github.com/alchemmist/antex/releases/download/"
                    f"v{VERSION}/antex-package_SHA256SUMS",
                    f"https://releases.example.com/antex/releases/{VERSION}/antex-package-aarch64-apple-darwin.tar.gz",
                    "https://github.com/alchemmist/antex/releases/download/"
                    f"v{VERSION}/antex-package-aarch64-apple-darwin.tar.gz",
                ],
            )
            self.assertIn("retrying from GitHub Releases", result.stderr)

    def test_releases_corrupt_assets_fall_back_to_github(self) -> None:
        with tempfile.TemporaryDirectory() as temp_dir:
            root = Path(temp_dir)
            archive_path, checksum_path, metadata_json = create_package_release(root)

            result, requests = run_installer_in(
                root,
                "latest",
                metadata_json=metadata_json,
                archive_path=archive_path,
                checksum_path=checksum_path,
                force_macos=True,
                use_mirror=True,
                releases_mode="corrupt_assets",
            )

            self.assertEqual(result.returncode, 0, result.stderr)
            self.assertEqual(
                requests,
                [
                    "https://releases.example.com/antex/channels/latest",
                    f"https://releases.example.com/antex/releases/{VERSION}/antex-package_SHA256SUMS",
                    "https://github.com/alchemmist/antex/releases/download/"
                    f"v{VERSION}/antex-package_SHA256SUMS",
                    f"https://releases.example.com/antex/releases/{VERSION}/antex-package-aarch64-apple-darwin.tar.gz",
                    "https://github.com/alchemmist/antex/releases/download/"
                    f"v{VERSION}/antex-package-aarch64-apple-darwin.tar.gz",
                ],
            )
            self.assertIn("checksum did not match expected digest", result.stderr)
            self.assertIn("retrying from GitHub Releases", result.stderr)

    def test_releases_wrong_checksum_digest_uses_github_metadata(self) -> None:
        with tempfile.TemporaryDirectory() as temp_dir:
            root = Path(temp_dir)
            archive_path, checksum_path, metadata_json = create_package_release(root)
            mirror_metadata = json.loads(metadata_json)
            for release_asset in mirror_metadata["assets"]:
                if release_asset["name"] == "antex-package_SHA256SUMS":
                    release_asset["digest"] = "sha256:" + "0" * 64

            result, requests = run_installer_in(
                root,
                "latest",
                metadata_json=metadata_json,
                releases_metadata_json=json.dumps(mirror_metadata),
                archive_path=archive_path,
                checksum_path=checksum_path,
                force_macos=True,
                use_mirror=True,
            )

            self.assertEqual(result.returncode, 0, result.stderr)
            self.assertEqual(
                requests,
                [
                    "https://releases.example.com/antex/channels/latest",
                    f"https://releases.example.com/antex/releases/{VERSION}/antex-package_SHA256SUMS",
                    "https://github.com/alchemmist/antex/releases/download/"
                    f"v{VERSION}/antex-package_SHA256SUMS",
                    "https://api.github.com/repos/alchemmist/antex/releases/tags/"
                    f"v{VERSION}",
                    f"https://releases.example.com/antex/releases/{VERSION}/antex-package-aarch64-apple-darwin.tar.gz",
                ],
            )
            self.assertIn("checksum did not match expected digest", result.stderr)

    def test_releases_incomplete_checksum_manifest_falls_back_to_github(self) -> None:
        with tempfile.TemporaryDirectory() as temp_dir:
            root = Path(temp_dir)
            archive_path, checksum_path, metadata_json = create_package_release(root)
            mirror_checksum_path = root / "mirror-SHA256SUMS"
            mirror_checksum_path.write_text(
                f"{'a' * 64}  antex-package-other-platform.tar.gz\n",
                encoding="utf-8",
            )
            mirror_metadata = json.loads(metadata_json)
            for release_asset in mirror_metadata["assets"]:
                if release_asset["name"] == "antex-package_SHA256SUMS":
                    release_asset["digest"] = (
                        "sha256:"
                        + hashlib.sha256(mirror_checksum_path.read_bytes()).hexdigest()
                    )

            result, requests = run_installer_in(
                root,
                "latest",
                metadata_json=metadata_json,
                releases_metadata_json=json.dumps(mirror_metadata),
                archive_path=archive_path,
                checksum_path=checksum_path,
                releases_checksum_path=mirror_checksum_path,
                force_macos=True,
                use_mirror=True,
            )

            self.assertEqual(result.returncode, 0, result.stderr)
            self.assertEqual(
                requests,
                [
                    "https://releases.example.com/antex/channels/latest",
                    f"https://releases.example.com/antex/releases/{VERSION}/antex-package_SHA256SUMS",
                    "https://github.com/alchemmist/antex/releases/download/"
                    f"v{VERSION}/antex-package_SHA256SUMS",
                    "https://api.github.com/repos/alchemmist/antex/releases/tags/"
                    f"v{VERSION}",
                    f"https://releases.example.com/antex/releases/{VERSION}/antex-package-aarch64-apple-darwin.tar.gz",
                ],
            )
            self.assertIn("retrying from GitHub Releases", result.stderr)

    def test_releases_corrupt_github_fallback_still_fails(self) -> None:
        with tempfile.TemporaryDirectory() as temp_dir:
            root = Path(temp_dir)
            archive_path, checksum_path, metadata_json = create_package_release(root)

            result, requests = run_installer_in(
                root,
                "latest",
                metadata_json=metadata_json,
                archive_path=archive_path,
                checksum_path=checksum_path,
                force_macos=True,
                use_mirror=True,
                releases_mode="corrupt_checksum_and_github",
            )

            self.assertNotEqual(result.returncode, 0)
            self.assertEqual(
                requests,
                [
                    "https://releases.example.com/antex/channels/latest",
                    f"https://releases.example.com/antex/releases/{VERSION}/antex-package_SHA256SUMS",
                    "https://github.com/alchemmist/antex/releases/download/"
                    f"v{VERSION}/antex-package_SHA256SUMS",
                    "https://api.github.com/repos/alchemmist/antex/releases/tags/"
                    f"v{VERSION}",
                ],
            )
            self.assertIn("checksum did not match expected digest", result.stderr)

    def test_releases_exact_rejects_wrong_binary_version(self) -> None:
        with tempfile.TemporaryDirectory() as temp_dir:
            root = Path(temp_dir)
            archive_path, checksum_path, metadata_json = create_package_release(
                root,
                metadata_version=MISMATCH_VERSION,
            )

            result, requests = run_installer_in(
                root,
                MISMATCH_VERSION,
                metadata_json=metadata_json,
                archive_path=archive_path,
                checksum_path=checksum_path,
                force_macos=True,
                use_mirror=True,
            )

            self.assertNotEqual(result.returncode, 0)
            self.assertEqual(
                requests,
                [
                    f"https://releases.example.com/antex/releases/{MISMATCH_VERSION}/release.json",
                    f"https://releases.example.com/antex/releases/{MISMATCH_VERSION}/antex-package_SHA256SUMS",
                    f"https://releases.example.com/antex/releases/{MISMATCH_VERSION}/antex-package-aarch64-apple-darwin.tar.gz",
                ],
            )
            self.assertIn(
                f"did not report expected version {MISMATCH_VERSION}",
                result.stderr,
            )
            self.assertNotIn("installed successfully", result.stdout)

    def test_releases_exact_legacy_fallback_reuses_offline_install(self) -> None:
        with tempfile.TemporaryDirectory() as temp_dir:
            root = Path(temp_dir)
            archive_path, metadata_json = create_legacy_release(root)

            first_result, first_requests = run_installer_in(
                root,
                VERSION,
                metadata_json=metadata_json,
                legacy_archive_path=archive_path,
                force_macos=True,
                use_mirror=True,
                releases_mode="channel_failure",
            )

            self.assertEqual(first_result.returncode, 0, first_result.stderr)
            self.assertEqual(
                first_requests,
                [
                    f"https://releases.example.com/antex/releases/{VERSION}/release.json",
                    "https://api.github.com/repos/alchemmist/antex/releases/tags/"
                    f"v{VERSION}",
                    "https://github.com/alchemmist/antex/releases/download/"
                    f"v{VERSION}/antex-npm-darwin-arm64-{VERSION}.tgz",
                ],
            )

            (root / "requests.log").unlink()
            second_result, second_requests = run_installer_in(
                root,
                VERSION,
                metadata_json=metadata_json,
                force_macos=True,
                use_mirror=True,
                releases_mode="channel_failure",
            )

            self.assertEqual(second_result.returncode, 0, second_result.stderr)
            self.assertEqual(
                second_requests,
                [
                    f"https://releases.example.com/antex/releases/{VERSION}/release.json",
                    "https://api.github.com/repos/alchemmist/antex/releases/tags/"
                    f"v{VERSION}",
                ],
            )
            self.assertNotIn("Downloading Antex CLI", second_result.stdout)


def run_installer(
    release: str,
    *,
    metadata_failure: bool = False,
    metadata_json: str | None = None,
    use_mirror: bool | None = False,
) -> tuple[subprocess.CompletedProcess[str], list[str]]:
    with tempfile.TemporaryDirectory() as temp_dir:
        return run_installer_in(
            Path(temp_dir),
            release,
            metadata_failure=metadata_failure,
            metadata_json=metadata_json,
            use_mirror=use_mirror,
        )


def run_installer_in(
    root: Path,
    release: str,
    *,
    metadata_failure: bool = False,
    metadata_json: str | None = None,
    releases_metadata_json: str | None = None,
    archive_path: Path | None = None,
    checksum_path: Path | None = None,
    releases_checksum_path: Path | None = None,
    legacy_archive_path: Path | None = None,
    force_macos: bool = False,
    use_mirror: bool | None = False,
    releases_mode: str = "",
    update_guard_from_release: str | None = None,
    old_updater_parent_pid: int | None = None,
    fail_ps: bool = False,
    daemon_only: bool = False,
    manual_update: bool = False,
    script_input: str | None = None,
    script_args: tuple[str, ...] = (),
    interactive: bool = False,
) -> tuple[subprocess.CompletedProcess[str], list[str]]:
    bin_dir = root / "bin"
    bin_dir.mkdir(exist_ok=True)
    request_log = root / "requests.log"
    fake_curl = bin_dir / "curl"
    fake_curl.write_text(
        textwrap.dedent(
            """\
            #!/bin/sh
            url=""
            output=""
            previous=""
            for arg in "$@"; do
              case "$arg" in
                https://*) url="$arg" ;;
              esac
              if [ "$previous" = "-o" ]; then
                output="$arg"
              fi
              previous="$arg"
            done
            printf '%s\n' "$url" >>"$ANTEX_TEST_REQUEST_LOG"

            case "$url" in
              https://api.github.com/*)
                if [ "$ANTEX_TEST_METADATA_FAILURE" = "1" ]; then
                  echo "curl: (22) The requested URL returned error: 403" >&2
                  exit 22
                fi
                printf '%s\n' "$ANTEX_TEST_METADATA_JSON"
                ;;
              https://releases.example.com/antex/channels/latest|https://releases.example.com/antex/releases/*/release.json)
                if [ "$ANTEX_TEST_RELEASES_MODE" = "channel_failure" ]; then
                  exit 22
                fi
                printf '%s\n' "$ANTEX_TEST_RELEASES_METADATA_JSON"
                ;;
              https://releases.example.com/antex/releases/*/antex-package_SHA256SUMS)
                if [ "$ANTEX_TEST_RELEASES_MODE" = "asset_fallback" ]; then
                  exit 22
                fi
                if [ "$ANTEX_TEST_RELEASES_MODE" = "corrupt_assets" ] ||
                  [ "$ANTEX_TEST_RELEASES_MODE" = "corrupt_checksum_and_github" ]; then
                  printf '<html>proxy error</html>\n' >"$output"
                  exit 0
                fi
                if [ -n "$ANTEX_TEST_RELEASES_CHECKSUM_PATH" ]; then
                  cp "$ANTEX_TEST_RELEASES_CHECKSUM_PATH" "$output"
                else
                  exit 22
                fi
                ;;
              https://releases.example.com/antex/releases/*/antex-package-*.tar.gz)
                if [ "$ANTEX_TEST_RELEASES_MODE" = "asset_fallback" ]; then
                  exit 22
                fi
                if [ "$ANTEX_TEST_RELEASES_MODE" = "corrupt_assets" ]; then
                  printf '<html>proxy error</html>\n' >"$output"
                  exit 0
                fi
                if [ -n "$ANTEX_TEST_ARCHIVE_PATH" ]; then
                  cp "$ANTEX_TEST_ARCHIVE_PATH" "$output"
                else
                  exit 22
                fi
                ;;
              https://github.com/alchemmist/antex/releases/download/*/antex-package_SHA256SUMS)
                if [ "$ANTEX_TEST_RELEASES_MODE" = "corrupt_checksum_and_github" ]; then
                  printf '<html>proxy error</html>\n' >"$output"
                  exit 0
                fi
                if [ -n "$ANTEX_TEST_CHECKSUM_PATH" ]; then
                  cp "$ANTEX_TEST_CHECKSUM_PATH" "$output"
                else
                  exit 22
                fi
                ;;
              https://github.com/alchemmist/antex/releases/download/*/antex-*.tar.gz)
                if [ -n "$ANTEX_TEST_ARCHIVE_PATH" ]; then
                  cp "$ANTEX_TEST_ARCHIVE_PATH" "$output"
                else
                  exit 22
                fi
                ;;
              https://github.com/alchemmist/antex/releases/download/*/antex-npm-*.tgz)
                if [ -n "$ANTEX_TEST_LEGACY_ARCHIVE_PATH" ]; then
                  cp "$ANTEX_TEST_LEGACY_ARCHIVE_PATH" "$output"
                else
                  exit 22
                fi
                ;;
              *)
                exit 22
                ;;
            esac
            """
        ),
        encoding="utf-8",
    )
    fake_curl.chmod(0o755)
    if force_macos:
        fake_uname = bin_dir / "uname"
        fake_uname.write_text(
            "#!/bin/sh\n"
            'case "$1" in\n'
            "  -s) printf 'Darwin\\n' ;;\n"
            "  -m) printf 'arm64\\n' ;;\n"
            "esac\n",
            encoding="utf-8",
        )
        fake_uname.chmod(0o755)
    if old_updater_parent_pid is not None:
        fake_ps = bin_dir / "ps"
        fake_ps.write_text(
            "#!/bin/sh\nexit 1\n"
            if fail_ps
            else "#!/bin/sh\n"
            'case "$*" in\n'
            '  *lstart*) printf "S %s\\n" "$CODEX_TEST_PARENT_START" ;;\n'
            '  *) printf "%s\\n" "$CODEX_TEST_PARENT_PID" ;;\n'
            "esac\n",
            encoding="utf-8",
        )
        fake_ps.chmod(0o755)

    home = root / "home"
    home.mkdir(exist_ok=True)
    env = os.environ.copy()
    env.update(
        {
            "ANTEX_HOME": str(root / "antex-home"),
            "ANTEX_INSTALL_DIR": str(root / "install-bin"),
            "ANTEX_NON_INTERACTIVE": "0" if interactive else "1",
            "ANTEX_RELEASE": release,
            "ANTEX_INSTALL_DAEMON_ONLY": "1" if daemon_only else "0",
            "ANTEX_TEST_ARCHIVE_PATH": str(archive_path or ""),
            "ANTEX_TEST_CHECKSUM_PATH": str(checksum_path or ""),
            "ANTEX_TEST_RELEASES_CHECKSUM_PATH": str(
                releases_checksum_path or checksum_path or ""
            ),
            "ANTEX_TEST_LEGACY_ARCHIVE_PATH": str(legacy_archive_path or ""),
            "ANTEX_TEST_METADATA_FAILURE": "1" if metadata_failure else "0",
            "ANTEX_TEST_METADATA_JSON": (
                metadata_json if metadata_json is not None else release_metadata()
            ),
            "ANTEX_TEST_RELEASES_METADATA_JSON": (
                releases_metadata_json
                if releases_metadata_json is not None
                else metadata_json
                if metadata_json is not None
                else release_metadata()
            ),
            "ANTEX_TEST_RELEASES_MODE": releases_mode,
            "ANTEX_TEST_REQUEST_LOG": str(request_log),
            "HOME": str(home),
            "PATH": f"{bin_dir}:/usr/bin:/bin",
            "SHELL": "/bin/sh",
        }
    )
    env["ANTEX_INSTALL_IF_CURRENT"] = "1" if manual_update else "0"
    if update_guard_from_release is None:
        env.pop("ANTEX_INSTALL_IF_LATEST", None)
        env.pop("ANTEX_UPDATE_FROM_RELEASE", None)
    else:
        env["ANTEX_INSTALL_IF_LATEST"] = "0" if manual_update else "1"
        env["ANTEX_UPDATE_FROM_RELEASE"] = update_guard_from_release
    if old_updater_parent_pid is not None:
        env["CODEX_TEST_PARENT_PID"] = str(old_updater_parent_pid)
        env["CODEX_TEST_PARENT_START"] = process_start_time()
    if use_mirror:
        env["ANTEX_RELEASES_BASE_URL"] = "https://releases.example.com/antex"
    else:
        env.pop("ANTEX_RELEASES_BASE_URL", None)
    command = (
        ["/bin/sh", str(INSTALL_SCRIPT)]
        if script_input is None
        else ["/bin/sh", "-s", "--"]
    )
    result = subprocess.run(
        [*command, *script_args],
        input=script_input,
        cwd=root,
        capture_output=True,
        check=False,
        env=env,
        text=True,
    )
    requests = (
        request_log.read_text(encoding="utf-8").splitlines()
        if request_log.exists()
        else []
    )
    return result, requests


def process_start_time() -> str:
    details = subprocess.check_output(
        ["ps", "-p", str(os.getpid()), "-o", "stat=", "-o", "lstart="], text=True
    ).strip()
    return details.split(maxsplit=1)[1]


def create_package_release(
    root: Path,
    *,
    metadata_version: str = VERSION,
    target: str = "aarch64-apple-darwin",
    extra_files: dict[str, str] | None = None,
) -> tuple[Path, Path, str]:
    package_dir = root / "package"
    (package_dir / "bin").mkdir(parents=True)
    (package_dir / "antex-path").mkdir()
    (package_dir / "antex-package.json").write_text("{}\n", encoding="utf-8")
    write_executable(
        package_dir / "bin" / "antex",
        f"#!/bin/sh\nprintf 'antex-cli {VERSION}\\n'\n",
    )
    write_executable(
        package_dir / "bin" / "antex-code-mode-host",
        "#!/bin/sh\nexit 0\n",
    )
    write_executable(package_dir / "antex-path" / "rg", "#!/bin/sh\nexit 0\n")
    if "linux" in target:
        (package_dir / "antex-resources").mkdir()
        write_executable(package_dir / "antex-resources/bwrap", "#!/bin/sh\nexit 0\n")

    for name, content in (extra_files or {}).items():
        path = package_dir / name
        path.parent.mkdir(parents=True, exist_ok=True)
        write_executable(path, content)

    asset = f"antex-package-{target}.tar.gz"
    archive_path = root / asset
    with tarfile.open(archive_path, "w:gz") as archive:
        for path in package_dir.iterdir():
            archive.add(path, arcname=path.name)

    archive_digest = hashlib.sha256(archive_path.read_bytes()).hexdigest()
    checksum_path = root / "antex-package_SHA256SUMS"
    checksum_path.write_text(f"{archive_digest}  {asset}\n", encoding="utf-8")
    checksum_digest = hashlib.sha256(checksum_path.read_bytes()).hexdigest()
    metadata_json = json.dumps(
        {
            "assets": [
                {"name": asset, "digest": f"sha256:{archive_digest}"},
                {
                    "name": "antex-package_SHA256SUMS",
                    "digest": f"sha256:{checksum_digest}",
                },
            ],
            "tag_name": f"v{metadata_version}",
        },
        indent=2,
    )
    return archive_path, checksum_path, metadata_json


def create_legacy_release(root: Path) -> tuple[Path, str]:
    package_dir = root / "legacy-package"
    vendor_dir = package_dir / "package" / "vendor" / "aarch64-apple-darwin"
    (vendor_dir / "antex").mkdir(parents=True)
    (vendor_dir / "path").mkdir()
    write_executable(
        vendor_dir / "antex" / "antex",
        f"#!/bin/sh\nprintf 'antex-cli {VERSION}\\n'\n",
    )
    write_executable(vendor_dir / "path" / "rg", "#!/bin/sh\nexit 0\n")

    asset = f"antex-npm-darwin-arm64-{VERSION}.tgz"
    archive_path = root / asset
    with tarfile.open(archive_path, "w:gz") as archive:
        archive.add(package_dir / "package", arcname="package")

    archive_digest = hashlib.sha256(archive_path.read_bytes()).hexdigest()
    metadata_json = json.dumps(
        {
            "assets": [{"name": asset, "digest": f"sha256:{archive_digest}"}],
            "tag_name": f"v{VERSION}",
        },
        indent=2,
    )
    return archive_path, metadata_json


def write_executable(path: Path, contents: str) -> None:
    path.write_text(contents, encoding="utf-8")
    path.chmod(0o755)


def release_metadata(*, compact: bool = False, reorder: bool = False) -> str:
    assets = [
        asset_metadata(
            f"antex-package-{target}.tar.gz",
            f"sha256:{'a' * 64}",
            reorder=reorder,
        )
        for target in (
            "aarch64-apple-darwin",
            "x86_64-apple-darwin",
            "aarch64-unknown-linux-musl",
            "x86_64-unknown-linux-musl",
        )
    ]
    assets.append(
        asset_metadata(
            "antex-package_SHA256SUMS",
            f"sha256:{'b' * 64}",
            reorder=reorder,
        )
    )
    separators = (",", ":") if compact else None
    return json.dumps(
        {"assets": assets, "body": "braces: { } [ ]", "tag_name": f"v{VERSION}"},
        indent=None if compact else 2,
        separators=separators,
    )


def asset_metadata(name: str, digest: str, *, reorder: bool) -> dict[str, str]:
    if reorder:
        return {"digest": digest, "name": name}
    return {"name": name, "digest": digest}


def legacy_release_metadata_with_decoys() -> str:
    fake_digest = f"sha256:{'0' * 64}"
    assets = [
        {
            "metadata": {
                "name": "antex-package-x86_64-unknown-linux-musl.tar.gz",
                "digest": fake_digest,
            },
            "digest": f"sha256:{'c' * 64}",
            "name": f"antex-npm-{target}-{VERSION}.tgz",
        }
        for target in ("darwin-arm64", "darwin-x64", "linux-arm64", "linux-x64")
    ]
    return json.dumps(
        {
            "body": (
                f'fake: {{"name":"antex-package_SHA256SUMS","digest":"{fake_digest}"}}'
            ),
            "assets": assets,
            "tag_name": f"v{VERSION}",
        },
        separators=(",", ":"),
    )


if __name__ == "__main__":
    unittest.main()
