"""Assemble the fork's canonical CLI package and release archives."""

import argparse
import hashlib
import json
import os
import re
import shutil
import subprocess
import sys
import tempfile
from pathlib import Path

sys.path.insert(0, str(Path(__file__).resolve().parent))

from antex_package.archive import write_archive
from antex_package.layout import validate_package_dir
from antex_package.targets import PACKAGE_VARIANTS, TARGET_SPECS
from antex_package.version import read_workspace_version

ROOT = Path(__file__).resolve().parent.parent


def main():
    parser = argparse.ArgumentParser(description=__doc__)
    parser.add_argument(
        "--target",
        required=True,
        choices=("aarch64-apple-darwin", "x86_64-unknown-linux-gnu"),
    )
    parser.add_argument("--commit", required=True)
    parser.add_argument("--output", type=Path, required=True)
    parser.add_argument("--release", action="store_true")
    parser.add_argument("--entrypoint-bin", type=Path)
    parser.add_argument("--code-mode-host-bin", type=Path)
    parser.add_argument("--bwrap-bin", type=Path)
    parser.add_argument("--rg-bin", type=Path)
    args = parser.parse_args()
    output = args.output.absolute()
    if output.exists() or output.is_symlink():
        parser.error("output must be a fresh directory")
    if not re.fullmatch(r"[0-9a-f]{40}", args.commit):
        parser.error(
            "a full build commit is required; dev builds are not distributable"
        )
    version = (ROOT / "FORK_VERSION").read_text().strip()
    if not re.fullmatch(
        r"[0-9]+\.[0-9]+\.[0-9]+(?:-alpha(?:\.[0-9]+){0,2}|-beta(?:\.[0-9]+)?)?",
        version,
    ):
        parser.error("invalid fork release version")
    if args.release and os.environ.get("GITHUB_REF", "").startswith("refs/tags/"):
        if os.environ["GITHUB_REF"] != f"refs/tags/v{version}":
            parser.error("release tag does not match FORK_VERSION")
    if not args.release:
        version += "+" + args.commit
    output.parent.mkdir(parents=True, exist_ok=True)
    binaries = ROOT / "bazel-bin/antex-rs"
    with tempfile.TemporaryDirectory(prefix="antex-package-") as temporary:
        base = Path(temporary) / "package"
        command = [
            sys.executable,
            str(ROOT / "scripts/build_antex_package.py"),
            "--target",
            args.target,
            "--package-version",
            version,
            "--package-dir",
            str(base),
            "--entrypoint-bin",
            str(args.entrypoint_bin or binaries / "cli/antex"),
            "--code-mode-host-bin",
            str(
                args.code_mode_host_bin
                or binaries / "code-mode-host/antex-code-mode-host"
            ),
        ]
        if args.target.endswith("linux-gnu"):
            command += ["--bwrap-bin", str(args.bwrap_bin or binaries / "bwrap/bwrap")]
        if args.rg_bin:
            command += ["--rg-bin", str(args.rg_bin)]
        subprocess.run(
            command, env=os.environ | {"ANTEX_REPO_ROOT": str(ROOT)}, check=True
        )
        manifest_path = base / "antex-package.json"
        metadata = json.loads(manifest_path.read_text())
        metadata["upstreamVersion"] = read_workspace_version()
        manifest_path.write_text(json.dumps(metadata, indent=2) + "\n")
        if args.release:
            reported = subprocess.check_output(
                [str(base / "bin/antex"), "--version"], text=True, timeout=20
            ).split()[-1]
            if reported != read_workspace_version():
                raise ValueError(
                    "release entrypoint must carry the Cargo workspace version"
                )
            subprocess.run(
                [
                    "strip",
                    str(base / "bin/antex"),
                    str(base / "bin/antex-code-mode-host"),
                ],
                check=True,
            )
            if args.target.endswith("darwin"):
                for file in base.rglob("*"):
                    if file.is_file() and os.access(file, os.X_OK):
                        file.chmod(file.stat().st_mode | 0o200)
                        subprocess.run(
                            ["codesign", "--force", "--sign", "-", str(file)],
                            check=True,
                        )
        validate_package_dir(base, PACKAGE_VARIANTS["antex"], TARGET_SPECS[args.target])
        output.mkdir()  # Never overwrite an existing package, even after a race.
        try:
            shutil.copytree(base, output, dirs_exist_ok=True)
        except BaseException:
            shutil.rmtree(output)
            raise
    archive = output.parent / f"antex-{args.target}.tar.gz"
    write_archive(output, archive, force=False)
    alias = output.parent / f"antex-package-{args.target}.tar.gz"
    alias.hardlink_to(archive)
    digest = hashlib.sha256(archive.read_bytes()).hexdigest()
    for path in (archive, alias):
        path.with_suffix(path.suffix + ".sha256").write_text(f"{digest}  {path.name}\n")
    print(output)


if __name__ == "__main__":
    main()
