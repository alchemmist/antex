import argparse
import hashlib
import json
import os
import subprocess
import tempfile
from pathlib import Path


def smoke(binary):
    source = binary.resolve().parent.parent
    expected = json.loads((source / "antex-package.json").read_text())
    with tempfile.TemporaryDirectory(prefix="antex-daemon-", dir="/tmp") as temporary:
        home = Path(temporary)
        state = home / "app-server-daemon"
        state.mkdir()
        (state / "settings.json").write_text(
            json.dumps(
                {
                    "shutdownGraceSeconds": 1,
                    "updater": {"autoUpdateEnabled": False},
                }
            )
        )
        environment = os.environ | {"ANTEX_HOME": str(home)}
        command = [str(binary), "app-server", "daemon"]
        try:
            subprocess.run(
                [*command, "start"],
                env=environment,
                cwd=home,
                check=True,
                timeout=90,
                capture_output=True,
            )
            if any(
                (state / name).exists()
                for name in ("app-server-updater.pid", "daemon-updater.pid")
            ):
                raise RuntimeError("daemon smoke test unexpectedly started an updater")
            result = subprocess.check_output(
                [*command, "version"], env=environment, cwd=home, text=True, timeout=15
            )
            versions = json.loads(result)
            if versions["appServerVersion"] != versions["cliVersion"]:
                raise RuntimeError("daemon and CLI protocol versions differ")
            selected = home / "packages/app-server-daemon/current"
            if json.loads((selected / "antex-package.json").read_text()) != expected:
                raise RuntimeError("daemon did not retain package metadata")
            for source_file in source.rglob("*"):
                if source_file.is_file():
                    relative = source_file.relative_to(source)
                    with source_file.open("rb") as original:
                        expected_digest = hashlib.file_digest(
                            original, "sha256"
                        ).digest()
                    with (selected / relative).open("rb") as installed:
                        actual_digest = hashlib.file_digest(
                            installed, "sha256"
                        ).digest()
                    if actual_digest != expected_digest:
                        raise RuntimeError(f"daemon installation changed {relative}")
        finally:
            subprocess.run(
                [*command, "stop"],
                env=environment,
                cwd=home,
                timeout=30,
                capture_output=True,
                check=True,
            )
    print(
        "Daemon package passed: local installation, startup, matching protocol, resources, shutdown"
    )


if __name__ == "__main__":
    parser = argparse.ArgumentParser()
    parser.add_argument("binary", type=Path)
    args = parser.parse_args()
    smoke(args.binary.absolute())
