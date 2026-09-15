#!/bin/zsh

set -euo pipefail

release_level="${1:-}"
repo_root="$(git rev-parse --show-toplevel)"

case "$release_level" in
  patch|minor|major) ;;
  *)
    echo "Usage: $0 patch|minor|major" >&2
    exit 2
    ;;
esac

cd "$repo_root"

if [[ "$(git branch --show-current)" != "main" ]]; then
  echo "Releases must be created from main." >&2
  exit 1
fi

if [[ -n "$(git status --porcelain --untracked-files=normal)" ]]; then
  echo "The working tree must be clean before a release." >&2
  exit 1
fi

git fetch origin main --tags

if [[ "$(git rev-parse HEAD)" != "$(git rev-parse origin/main)" ]]; then
  echo "Local main must exactly match origin/main before a release." >&2
  exit 1
fi

release_commit_created=0

cleanup_uncommitted_release() {
  if (( ! release_commit_created )); then
    git restore -- FORK_VERSION
  fi
}

trap cleanup_uncommitted_release EXIT

next_version="$(python3 - "$release_level" <<'PY'
import pathlib
import re
import sys

level = sys.argv[1]
version_file = pathlib.Path("FORK_VERSION")
text = version_file.read_text()
match = re.fullmatch(r'(\d+)\.(\d+)\.(\d+)\n?', text)
if match is None:
    raise SystemExit("FORK_VERSION is not a plain semantic version")

major, minor, patch = map(int, match.groups())
if level == "major":
    major, minor, patch = major + 1, 0, 0
elif level == "minor":
    minor, patch = minor + 1, 0
else:
    patch += 1

version = f"{major}.{minor}.{patch}"
version_file.write_text(f"{version}\n")
print(version)
PY
)"

tag="v${next_version}"

if git rev-parse --verify --quiet "refs/tags/${tag}" >/dev/null; then
  echo "Tag ${tag} already exists." >&2
  exit 1
fi

git add FORK_VERSION
git commit -m "release ${next_version}"
release_commit_created=1
git tag -a "$tag" -m "Release ${next_version}"
git push --atomic origin main "refs/tags/${tag}"

echo "Released ${tag}. GitHub Actions is building the macOS and Linux artifacts."
