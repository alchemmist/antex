import argparse
import hashlib
import json
import os
import shutil
import tempfile
import uuid
from pathlib import Path


def install_package(source, destination, target):
    metadata = json.loads((source / "antex-package.json").read_text())
    expected = {
        "layoutVersion": 1,
        "variant": "antex",
        "entrypoint": "bin/antex",
        "target": target,
        "resourcesDir": "antex-resources",
        "pathDir": "antex-path",
    }
    if any(metadata.get(key) != value for key, value in expected.items()):
        raise ValueError("release has an incompatible package layout")
    required = [
        "bin/antex",
        "bin/antex-code-mode-host",
        "antex-path/rg",
        "antex-resources/zsh/bin/zsh",
        "antex-resources/voice/bin/antex-voice-host",
    ]
    if target.endswith("linux-gnu"):
        required.append("antex-resources/bwrap")
    for name in required:
        if not (source / name).is_file() or not os.access(source / name, os.X_OK):
            raise ValueError(f"release is missing executable {name}")
    manifest = json.loads((source / "antex-resources/voice/manifest.json").read_text())
    for name, expected_digest in manifest["sha256"].items():
        file = source / name
        if not file.resolve().is_relative_to(source.resolve()):
            raise ValueError("voice manifest contains an external path")
        if hashlib.sha256(file.read_bytes()).hexdigest() != expected_digest:
            raise ValueError(f"voice package checksum mismatch: {name}")
    destination.mkdir(parents=True, exist_ok=True)
    packages = destination / ".antex-packages"
    packages.mkdir(exist_ok=True)
    package = packages / uuid.uuid4().hex
    shutil.copytree(source, package)
    backup = packages / ("previous-" + uuid.uuid4().hex)
    backup.mkdir()
    names = ("antex", "antex-code-mode-host")
    for name in names:
        original = destination / name
        if original.is_symlink():
            (backup / name).symlink_to(original.resolve())
        elif original.exists():
            shutil.copy2(original, backup / name)
    with tempfile.TemporaryDirectory(
        prefix=".antex-links-", dir=destination
    ) as temporary:
        links = Path(temporary)
        current = links / "current"
        current.symlink_to(package)
        os.replace(current, destination / ".antex-current")
        for name in names:
            link = links / name
            link.symlink_to(f".antex-current/bin/{name}")
            os.replace(link, destination / name)
    print(f"Installed complete Antex package {metadata['version']} in {package}")


if __name__ == "__main__":
    parser = argparse.ArgumentParser()
    parser.add_argument("source", type=Path)
    parser.add_argument("destination", type=Path)
    parser.add_argument("target")
    args = parser.parse_args()
    install_package(args.source, args.destination.absolute(), args.target)
