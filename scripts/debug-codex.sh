#!/bin/bash

# Set "chatgpt.cliExecutable": "/Users/<USERNAME>/code/codex/scripts/debug-codex.sh" in VSCode settings to always get the 
# latest antex-rs binary when debugging Codex Extension.


set -euo pipefail

ANTEX_RS_DIR=$(realpath "$(dirname "$0")/../antex-rs")
(cd "$ANTEX_RS_DIR" && cargo run --quiet --bin codex -- "$@")