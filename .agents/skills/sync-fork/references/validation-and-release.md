# Validation, remote builds and separately authorized delivery

## Source gates

Use current recipes in `justfile`, `Makefile` and the release workflow, not copied
commands from a past release. Run Rust commands from `antex-rs` unless the recipe
specifies otherwise. Typical gates, selected for the actual diff:

1. Config changes: `just write-config-schema`. Protocol changes: inspect
   `just write-app-server-schema` and regenerate affected SDK outputs.
2. Rust dependency changes: `just bazel-lock-update` from the repository root;
   retain both lockfiles and check drift.
3. Scoped `just test -p <changed-crate> --locked --retries 0`. Never invoke
   `cargo test` directly. Shared/core/protocol changes require the complete suite
   after user approval: `just test --workspace --locked --retries 0`.
4. A materially changed TUI needs normal trace-disabled validation as well as any
   diagnostic run. Do not explain a timing failure with an unproven root cause.
5. Run actual argument-comment lint targets (`just argument-comment-lint`) when
   required by scope, plus affected SDK and packaging checks. Listing/counting
   targets is not a successful build. Local lint is not all-platform CI.
6. Set `ANTEX_REPO_ROOT` for package Python imports in **each required execution
   context**, including the CI job/test step, not only the assembly step:

   ```bash
   export ANTEX_REPO_ROOT="$(git rev-parse --show-toplevel)"
   python3 -m unittest discover -s "$ANTEX_REPO_ROOT/scripts" -p 'test_package_fork.py'
   ```

7. After required tests: final scoped `just fix -p <changed-crate>` (repeat `-p`
   for the affected crates), then `just fmt`. Do not run Rust tests afterward.
   Review the resulting diff; distinguish mechanical cleanup from semantic edits.

If the formatter enumerates tracked-but-deleted BUILD/Starlark files, inspect the
index. Stage only reviewed intended deletions and rerun formatting; do not restore
voice files or suppress formatter failures. No blanket staging of snapshots or
unrelated changes. Finish with `git diff --check` and a reviewed file inventory.

Record failed checks too. An external download HTTP 504 can justify a bounded
infrastructure retry on unchanged source; an assertion failure cannot be retried
until green. If the outcome of a remote write is unknown, read back before retrying.

## Remote synchronization and Linux-to-macOS builds

- Verify the SSH host identity and actual machine. Aliases can refer to one host;
  never assume they provide independent workers. No global SSH/DNS changes.
- Freeze a complete source snapshot before transfer. Prefer a clean worktree at
  the exact published commit for release builds. Do not sync during running jobs.
- For source transfers use `rsync --checksum --no-times`; avoid `-a`/`-t` silently
  restoring old mtimes, which can leave incremental compiler inputs falsely fresh.
  Transfer generated source and locks deliberately. Exclude build caches, `.git`
  from ordinary source copies, credentials and historical `*.snap.new`. Review
  deletion plans; never blindly remove excluded/nonempty cache directories.
- Keep test homes, logs and build outputs outside the checkout. Choose a disk-backed
  temporary directory when tmpfs capacity is insufficient; do not weaken sandbox
  tests to accommodate fixture placement.
- Docker cross-compilation is **not a macOS VM**. Confirm the intended method.
  Mount only task source/output and non-secret dependency caches; never mount the
  user's whole home, SSH agent/keys, auth files, Pi credentials or Docker socket.
  If Git metadata is needed for stamping, verify it contains no credential config;
  otherwise use a sanitized source checkout and verified provenance mechanism.
- Give the container its own Bazel output root/server, not the host validation
  server. Use absolute cache paths and a deliberate JVM `user.home`: container
  UID 1000 may resolve to `/home/ubuntu` despite a different `$HOME`. Merely mounting
  a cache does not prove Bazel uses it. Keep checksum verification enabled.
- Read `make build-macos-arm64` and its variables. Stamp the full selected SHA,
  check a clean source tree, and record image/toolchain, flags, logs and hashes.
  Use the successful build's actual output symlink to extract artifacts; a later
  differently configured `bazel info` failure is not a compilation failure.
- Verify transfer hashes and Mach-O ARM64 architecture. Preserve raw inputs; sign
  executable copies on macOS as necessary before invoking them. Never relabel an
  old binary as a new release after a workflow/version fix: build the final commit.

## Commit, push and release — only if requested

Review status/index/diff and ensure only intended files are staged. Commit and push
only within the authorized scope. Fast-forward a clean `main` when appropriate;
never reset the user's worktree or force-push over concurrent changes.

Read `scripts/release-fork.sh` before invoking `make release-patch`. It checks clean
`main` against `origin/main`, bumps `FORK_VERSION`, creates a commit/annotated tag
and atomically pushes them. Do not bypass these safeguards or manually retag an
already published ref. A fix after a pushed tag may require another patch version.

Wait for the actual release workflow. Verify the exact source SHA, both platform
jobs, publication job and expected nonempty assets. Compare published checksum
manifests with downloaded bytes or available server asset digests. Distinguish a
pushed tag, a queued workflow, an uploaded artifact and a published GitHub Release.
Retry only the diagnosed infrastructure failure, preserving the original evidence;
wait until GitHub allows the retry. Do not cancel a healthy build to accelerate it.

## Full-package acceptance — never just a raw executable

Read `scripts/package-fork.py` and current CLI help before assembly. Provide the
exact full build commit, target and explicit input binaries; use the final source's
`FORK_VERSION`. Keep fork/package version separate from internal Cargo version.
Use a fresh output directory; preserve original inputs and previous installations.

Validate layout, helpers, resources, architecture, signatures and archive hashes.
For the current macOS layout this includes CLI, code-mode-host, ripgrep and the
resource directory/manifest; Linux also needs bubblewrap. Do not restore obsolete
voice/zsh requirements just because an old installer expected them.

Run in disposable homes using the existing scripts:

- `scripts/smoke-tui-startup.py <packaged-cli>` — ordinary TUI reaches the composer;
- `scripts/smoke-code-mode-host.py <packaged-host>` — JS, nested tool, result, shutdown;
- embedded engine and foreground stdio SDK checks — startup, subagent messaging,
  questions, resume/fork and shutdown without probing a legacy daemon socket;
- a harmless real platform sandbox command, plus installer/package tests.

Inspect that smoke isolation really addresses disposable state. Keep production
auth/config/history out of fixtures. `--version` and `--help` are supplementary,
not substitutes for these gates.

## Installation — explicit permission and fail-closed sequencing

Back up the existing complete package/link selection and relevant settings. Keep
credential backups local and access-restricted; never put them in the repository,
ordinary build artifacts or a remote build host. Preserve user history; do not
blindly restore an old auth backup over a newer login during rollback.

The new CLI owns its embedded engine; there is no second server package to update.
Do not terminate existing foreground clients during installation. If an older
release still has a live daemon, leave it untouched unless retirement was separately
authorized. Before that one-time retirement, use the old version's actual interface
to inspect live work without dumping conversation contents. Active/waiting/error/
unknown states or an unreadable inventory block automatic termination. A stale idle
inventory is not a lock; agree a quiescent window with the user.

Put the **entire mutating sequence** in one strict shell/script. This is the required
control-flow shape, not executable commands to paste with guessed helper names:

```text
set -euo pipefail
confirm installation consent and backup complete current selection
install complete accepted package → verify installed hashes/manifest/version
verify a new foreground launch uses the new embedded engine
leave existing foreground clients and legacy daemon processes untouched
```

A guarded `check && install` followed by an unguarded newline `restart` is unsafe:
failed checks must prevent **every** later mutation. If a step fails, stop and inspect
actual state; do not assume the preceding installation happened.

Do not run obsolete daemon lifecycle commands from the new CLI. Recheck installed
startup, code-mode and version/resource agreement, retaining rollback. Old running
frontend processes need a new invocation to load the new binary. Retiring old
service registrations/packages is a separate, explicitly approved migration.

## Live authentication/model verification — separate permission

Use the user's own `antex login --chatgpt-direct` browser flow when authorized;
never borrow Pi credentials, export tokens to Linux or override production endpoints
with test authorities. Preserve legacy login/API-key alternatives. Verify account
and catalog state in the new foreground process; never use an old shared daemon's
cached catalog as evidence for the embedded runtime.

Prove entitlement with a minimal actual request, not a catalog list. Use ephemeral
sessions and a temporary working directory, request a short exact answer with no
tools, and limit the number of model probes. Isolate API-key overrides for a direct
subscription probe without printing them. Distinguish an isolated-config transport
probe from a separate authorized check using the user's normal config/default model.
Report only models actually verified; never claim every catalog entry is available.
