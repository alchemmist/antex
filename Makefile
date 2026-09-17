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

.PHONY: build build-linux build-macos-arm64 install-local install-mac install-linux release-patch release-minor release-major

build:
	ANTEX_REPO_ROOT="$(CURDIR)" CARGO_TARGET_DIR="$(ANTEX_TARGET_DIR)" STABLE_GIT_COMMIT="$(ANTEX_BUILD_COMMIT)" ALCHEMMIST_FORK_VERSION="$(ANTEX_FORK_VERSION)" python3 scripts/build-fork-local.py "$(CARGO)"

build-macos-arm64:
	ANTEX_BUILD_COMMIT="$(ANTEX_BUILD_COMMIT)" $(BAZEL) build $(ANTEX_BAZEL_BUILD_FLAGS) --workspace_status_command="python3 scripts/workspace-status.py" -c opt --config=macos-arm64 //antex-rs/cli:antex //antex-rs/code-mode-host:antex-code-mode-host

build-linux:
	ANTEX_BUILD_COMMIT="$(ANTEX_BUILD_COMMIT)" $(BAZEL) build $(ANTEX_BAZEL_BUILD_FLAGS) --workspace_status_command="python3 scripts/workspace-status.py" -c opt --platforms=@rules_rs//rs/platforms:x86_64-unknown-linux-gnu //antex-rs/cli:antex //antex-rs/code-mode-host:antex-code-mode-host

install-local: build
	python3 scripts/smoke-code-mode-host.py "$(ANTEX_CODE_MODE_HOST_BINARY)"
	$(INSTALL) -d "$(ANTEX_INSTALL_DIR)"
	$(INSTALL) -m 755 "$(ANTEX_BINARY)" "$(ANTEX_INSTALL_DIR)/antex"
	$(INSTALL) -m 755 "$(ANTEX_CODE_MODE_HOST_BINARY)" "$(ANTEX_INSTALL_DIR)/antex-code-mode-host"
	@/bin/zsh -fc 'rehash'
	@echo "Installed $(ANTEX_INSTALL_DIR)/antex"

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
