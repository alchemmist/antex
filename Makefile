SHELL := /bin/zsh
.DEFAULT_GOAL := install-local

CARGO ?= cargo
BAZEL ?= bazel
ANTEX_BAZEL_BUILD_FLAGS ?=
INSTALL ?= install
ANTEX_RS_DIR := $(CURDIR)/antex-rs
ANTEX_TARGET_DIR := $(ANTEX_RS_DIR)/target
ANTEX_BINARY := $(ANTEX_TARGET_DIR)/release/antex
ANTEX_CODE_MODE_HOST_BINARY := $(ANTEX_TARGET_DIR)/release/antex-code-mode-host
ANTEX_INSTALL_DIR ?= $(HOME)/.local/bin
ANTEX_RELEASE_REPOSITORY ?= alchemmist/antex
ANTEX_GIT_COMMIT := $(shell git rev-parse HEAD 2>/dev/null || printf unknown)
ANTEX_GIT_DIRTY := $(shell test -z "$$(git status --porcelain --untracked-files=normal -- . ':(exclude)antex-conversation-*.html' 2>/dev/null)" || printf +dirty)
ANTEX_BUILD_COMMIT := $(ANTEX_GIT_COMMIT)$(ANTEX_GIT_DIRTY)
ANTEX_FORK_VERSION := $(shell tr -d '[:space:]' < "$(CURDIR)/FORK_VERSION")
ANTEX_PACKAGE_FLAGS ?=
ANTEX_PACKAGE_DIR ?= $(CURDIR)/dist/package
PYTHON ?= python3

.PHONY: build build-linux build-macos-arm64 install-local install-mac install-linux release-patch release-minor release-major

build:
	ANTEX_REPO_ROOT="$(CURDIR)" CARGO_TARGET_DIR="$(ANTEX_TARGET_DIR)" STABLE_GIT_COMMIT="$(ANTEX_BUILD_COMMIT)" ALCHEMMIST_FORK_VERSION="$(ANTEX_FORK_VERSION)" python3 scripts/build-fork-local.py "$(CARGO)"

build-macos-arm64:
	ANTEX_BUILD_COMMIT="$(ANTEX_BUILD_COMMIT)" $(BAZEL) build $(ANTEX_BAZEL_BUILD_FLAGS) --stamp --workspace_status_command="$(PYTHON) scripts/workspace-status.py" -c opt --config=macos-arm64 //antex-rs/cli:antex //antex-rs/code-mode-host:antex-code-mode-host

build-linux:
	ANTEX_BUILD_COMMIT="$(ANTEX_BUILD_COMMIT)" $(BAZEL) build $(ANTEX_BAZEL_BUILD_FLAGS) --stamp --workspace_status_command="$(PYTHON) scripts/workspace-status.py" -c opt --platforms=//:local_linux //antex-rs/cli:antex //antex-rs/code-mode-host:antex-code-mode-host //antex-rs/voice-host:antex-voice-host //third_party/voice:native_runtime //antex-rs/bwrap:bwrap

.PHONY: package-macos-arm64 package-linux
package-macos-arm64: build-macos-arm64
	ANTEX_BUILD_COMMIT="$(ANTEX_BUILD_COMMIT)" $(BAZEL) build $(ANTEX_BAZEL_BUILD_FLAGS) --stamp --workspace_status_command="$(PYTHON) scripts/workspace-status.py" -c opt --config=macos-arm64 //antex-rs/voice-host:antex-voice-host //third_party/voice:native_runtime
	ANTEX_REPO_ROOT="$(CURDIR)" $(PYTHON) scripts/package-fork.py --target aarch64-apple-darwin --commit "$(ANTEX_BUILD_COMMIT)" --output "$(ANTEX_PACKAGE_DIR)" $(ANTEX_PACKAGE_FLAGS)

package-linux: build-linux
	ANTEX_REPO_ROOT="$(CURDIR)" $(PYTHON) scripts/package-fork.py --target x86_64-unknown-linux-gnu --commit "$(ANTEX_BUILD_COMMIT)" --output "$(ANTEX_PACKAGE_DIR)" $(ANTEX_PACKAGE_FLAGS)

install-local:
	@set -eu; \
	case "$$(uname -s)-$$(uname -m)" in \
	  Darwin-arm64) package_target=package-macos-arm64; target=aarch64-apple-darwin ;; \
	  Linux-x86_64) package_target=package-linux; target=x86_64-unknown-linux-gnu ;; \
	  *) echo "Unsupported local package platform" >&2; exit 1 ;; \
	esac; \
	staging="$$(mktemp -d)"; trap 'rm -rf -- "$$staging"' EXIT; \
	$(MAKE) "$$package_target" ANTEX_PACKAGE_DIR="$$staging/package"; \
	$(PYTHON) scripts/smoke-code-mode-host.py "$$staging/package/bin/antex-code-mode-host"; \
	$(PYTHON) scripts/smoke-voice-package.py "$$staging/package/bin/antex" "$(ANTEX_BUILD_COMMIT)"; \
	$(PYTHON) scripts/install-fork-package.py "$$staging/package" "$(ANTEX_INSTALL_DIR)" "$$target"

install-mac:
	ANTEX_INSTALL_DIR="$(ANTEX_INSTALL_DIR)" ANTEX_RELEASE_REPOSITORY="$(ANTEX_RELEASE_REPOSITORY)" ./scripts/install-fork-release.sh mac

install-linux:
	ANTEX_INSTALL_DIR="$(ANTEX_INSTALL_DIR)" ANTEX_RELEASE_REPOSITORY="$(ANTEX_RELEASE_REPOSITORY)" ./scripts/install-fork-release.sh linux

release-patch:
	./scripts/release-fork.sh patch

release-minor:
	./scripts/release-fork.sh minor

release-major:
	./scripts/release-fork.sh major
