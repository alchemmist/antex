#!/usr/bin/env python3

import os
from pathlib import Path
import subprocess
import sys

SCRIPT_DIR = Path(__file__).resolve().parent
sys.path.insert(0, str(SCRIPT_DIR))

from antex_package.targets import TARGET_SPECS
from antex_package.targets import default_target
from antex_package.v8 import resolve_antex_v8_cargo_env


def main() -> int:
    cargo = sys.argv[1] if len(sys.argv) > 1 else "cargo"
    spec = TARGET_SPECS[default_target()]
    cargo_env = {**os.environ, **resolve_antex_v8_cargo_env(spec)}
    subprocess.run(
        [
            cargo,
            "build",
            "--release",
            "--bin",
            "antex",
            "--bin",
            "antex-code-mode-host",
        ],
        cwd=Path(os.environ["ANTEX_REPO_ROOT"]) / "antex-rs",
        env=cargo_env,
        check=True,
    )
    return 0


if __name__ == "__main__":
    raise SystemExit(main())
