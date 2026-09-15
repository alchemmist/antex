#!/usr/bin/env python3

import argparse
import re
import sys
from pathlib import Path
from typing import Sequence

PYTHON_RUNTIME_VERSION_PATTERN = re.compile(r"[0-9]+\.[0-9]+\.[0-9]+(?:a[0-9]+(?:\.post[0-9]+)?)?")
NORMALIZED_ANTEX_VERSION_PATTERN = re.compile(
    r"[0-9]+(?:\.[0-9]+)*(?:(?:a|b|rc)[0-9]+)?(?:\.post[0-9]+)?(?:\.dev[0-9]+)?"
)


def main(argv: Sequence[str] | None = None) -> int:
    parser = argparse.ArgumentParser(
        description="Resolve a Python runtime package version to its Antex release tag."
    )
    parser.add_argument("python_version")
    parser.add_argument("--github-output", type=Path, required=True)
    args = parser.parse_args(argv)

    try:
        python_version, release_tag = resolve_python_runtime_release(args.python_version)
    except RuntimeError as exc:
        print(exc, file=sys.stderr)
        return 1

    with args.github_output.open("a", encoding="utf-8") as output:
        print(f"python_version={python_version}", file=output)
        print(f"release_tag={release_tag}", file=output)
    return 0


def resolve_python_runtime_release(python_version: str) -> tuple[str, str]:
    if PYTHON_RUNTIME_VERSION_PATTERN.fullmatch(python_version) is None:
        raise RuntimeError(
            "Python runtime version must be stable, a numbered alpha, or an "
            "alpha post-release, for example 0.136.0, 0.136.0a2, or "
            f"0.136.0a2.post1; found {python_version}"
        )
    return python_version, antex_release_tag(python_version)


def antex_release_tag(version: str) -> str:
    return f"v{antex_release_version(version)}"


def antex_release_version(version: str) -> str:
    normalized = normalize_antex_version(version)
    alpha_hotfix = re.fullmatch(
        r"([0-9]+(?:\.[0-9]+)*)a([0-9]+)\.post([0-9]+)",
        normalized,
    )
    if alpha_hotfix is not None:
        base, alpha, hotfix = alpha_hotfix.groups()
        return f"{base}-alpha.{alpha}.{hotfix}"

    prerelease = re.fullmatch(r"([0-9]+(?:\.[0-9]+)*)(a|b|rc)([0-9]+)", normalized)
    if prerelease is None:
        return normalized

    base, prerelease_kind, number = prerelease.groups()
    prerelease_name = {"a": "alpha", "b": "beta", "rc": "rc"}[prerelease_kind]
    return f"{base}-{prerelease_name}.{number}"


def normalize_antex_version(version: str) -> str:
    normalized = version.strip()
    normalized = re.sub(
        r"-dev(?:\.([0-9]+))?$", lambda match: ".dev" + (match.group(1) or "0"), normalized
    )
    if normalized.startswith("rust-v"):
        normalized = normalized.removeprefix("rust-v")
    elif normalized.startswith("v"):
        normalized = normalized.removeprefix("v")

    normalized = re.sub(r"-alpha\.?([0-9]+)\.([0-9]+)$", r"a\1.post\2", normalized)
    normalized = re.sub(r"-alpha\.?([0-9]+)$", r"a\1", normalized)
    normalized = re.sub(r"-beta\.?([0-9]+)$", r"b\1", normalized)
    normalized = re.sub(r"-rc\.?([0-9]+)$", r"rc\1", normalized)

    if NORMALIZED_ANTEX_VERSION_PATTERN.fullmatch(normalized) is None:
        raise RuntimeError(f"Could not normalize Antex version {version!r} to a PEP 440 version")
    return normalized


if __name__ == "__main__":
    raise SystemExit(main())
