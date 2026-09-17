import argparse
import json
import os
import shutil
import subprocess
import sys
import tempfile
from pathlib import Path

from antex_package.archive import write_archive
from antex_package.version import read_workspace_version

ROOT = Path(__file__).resolve().parent.parent
sys.path.insert(0, str(ROOT / "third_party/voice"))

from assemble_package import assemble
from release_runtime import seal, stage


def main():
    parser = argparse.ArgumentParser()
    parser.add_argument(
        "--target",
        required=True,
        choices=(
            "aarch64-apple-darwin",
            "x86_64-unknown-linux-gnu",
        ),
    )
    parser.add_argument("--commit", required=True)
    parser.add_argument("--output", type=Path, required=True)
    parser.add_argument("--release", action="store_true")
    parser.add_argument("--entrypoint-bin", type=Path)
    parser.add_argument("--code-mode-host-bin", type=Path)
    parser.add_argument("--bwrap-bin", type=Path)
    args = parser.parse_args()
    output = args.output.absolute()
    if output.exists():
        parser.error("output must be a fresh directory")
    output.parent.mkdir(parents=True, exist_ok=True)
    version = (ROOT / "FORK_VERSION").read_text().strip()
    if not args.release:
        version += "+" + args.commit
    prefix = "macos_aarch64" if args.target.endswith("darwin") else "linux_x86_64"
    binaries = ROOT / "bazel-bin/antex-rs"
    with tempfile.TemporaryDirectory(prefix="antex-package-") as temporary:
        work = Path(temporary)
        base = work / "base"
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
        runtime = work / "runtime"
        stage(
            ROOT / f"bazel-bin/third_party/voice/native_runtime_{prefix}",
            runtime,
            args.target,
        )
        helper = work / "antex-voice-host"
        shutil.copy2(binaries / "voice-host/antex-voice-host", helper)
        helper.chmod(0o755)
        if args.release:
            subprocess.run(
                [
                    "strip",
                    str(helper),
                    str(base / "bin/antex"),
                    str(base / "bin/antex-code-mode-host"),
                ],
                check=True,
            )
        if args.target.endswith("darwin") and args.release:
            for file in [helper, *base.rglob("*"), *runtime.rglob("*.dylib")]:
                if file.is_file() and (
                    file.suffix == ".dylib" or os.access(file, os.X_OK)
                ):
                    file.chmod(file.stat().st_mode | 0o200)
                    command = ["codesign", "--force", "--sign", "-"]
                    if file == helper:
                        command += [
                            "--entitlements",
                            str(
                                ROOT
                                / ".github/scripts/macos-signing/codex-voice-host.entitlements.plist"
                            ),
                        ]
                    subprocess.run([*command, str(file)], check=True)
        if args.release:
            seal(runtime, args.target)
        assemble(
            base,
            helper,
            args.target,
            args.commit,
            output,
            runtime=runtime,
            release_version=version if args.release else None,
        )
    write_archive(output, output.parent / f"antex-{args.target}.tar.gz", force=False)
    (output.parent / f"antex-package-{args.target}.tar.gz").hardlink_to(
        output.parent / f"antex-{args.target}.tar.gz"
    )
    print(output)


if __name__ == "__main__":
    main()
