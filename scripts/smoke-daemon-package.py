import argparse
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
                    "updater": {"auto_update_enabled": False},
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
            result = subprocess.check_output(
                [*command, "version"], env=environment, cwd=home, text=True, timeout=15
            )
            versions = json.loads(result)
            if versions["appServerVersion"] != versions["cliVersion"]:
                raise RuntimeError("daemon and CLI protocol versions differ")
            selected = home / "packages/app-server-daemon/current"
            if json.loads((selected / "antex-package.json").read_text()) != expected:
                raise RuntimeError("daemon did not retain package metadata")
            if not (selected / "antex-resources/voice/bin/antex-voice-host").is_file():
                raise RuntimeError("daemon installation lost voice resources")
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
