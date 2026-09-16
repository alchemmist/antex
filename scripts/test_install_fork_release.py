import hashlib
import os
from pathlib import Path
import platform
import shutil
import subprocess
import tarfile
import tempfile
import unittest


class ForkReleaseInstallerTest(unittest.TestCase):
    def test_installs_antex_and_code_mode_host(self) -> None:
        system = platform.system()
        machine = platform.machine()
        if system == "Darwin" and machine == "arm64":
            make_target = "install-mac"
            target = "aarch64-apple-darwin"
        elif system == "Linux" and machine == "x86_64":
            make_target = "install-linux"
            target = "x86_64-unknown-linux-gnu"
        else:
            self.skipTest(f"unsupported test platform: {system} {machine}")

        with tempfile.TemporaryDirectory() as temporary:
            root = Path(temporary)
            release_dir = root / "release"
            staging_dir = root / "staging"
            install_dir = root / "install"
            release_dir.mkdir()
            staging_dir.mkdir()
            for binary in ("antex", "antex-code-mode-host"):
                destination = staging_dir / binary
                destination.write_bytes(Path(shutil.which("true")).read_bytes())
                destination.chmod(0o755)

            if system == "Linux":
                (staging_dir / "antex-resources").mkdir()
                (staging_dir / "antex-resources" / "bwrap").write_bytes(
                    Path(shutil.which("true")).read_bytes()
                )
            archive = release_dir / f"antex-{target}.tar.gz"
            with tarfile.open(archive, "w:gz") as tar:
                tar.add(staging_dir / "antex", arcname="antex")
                tar.add(
                    staging_dir / "antex-code-mode-host",
                    arcname="antex-code-mode-host",
                )
                if system == "Linux":
                    tar.add(staging_dir / "antex-resources", arcname="antex-resources")
            digest = hashlib.sha256(archive.read_bytes()).hexdigest()
            archive.with_name(f"{archive.name}.sha256").write_text(
                f"{digest}  {archive.name}\n"
            )

            env = os.environ | {
                "ANTEX_RELEASE_BASE_URL": release_dir.as_uri(),
                "ANTEX_INSTALL_DIR": str(install_dir),
            }
            subprocess.run(
                ["make", make_target],
                cwd=Path(__file__).resolve().parent.parent,
                env=env,
                check=True,
            )

            self.assertTrue((install_dir / "antex").is_file())
            self.assertTrue((install_dir / "antex-code-mode-host").is_file())
            if system == "Linux":
                self.assertTrue(
                    os.access(install_dir / "antex-resources" / "bwrap", os.X_OK)
                )


if __name__ == "__main__":
    unittest.main()
