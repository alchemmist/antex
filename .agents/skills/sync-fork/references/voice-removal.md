# Voice removal map

Read for a voice-only task and after every upstream merge. Paths are search
starting points, not a frozen deletion list; upstream can rename/move components.

## Inventory before changing files

Start with tracked files and focused searches:

```bash
git ls-files '*voice*' '*realtime*' '*webrtc*'
rg -n -i 'voice[-_]host|realtime[-_]webrtc|webrtc[-_]sys|third_party/voice' \
  antex-rs .github scripts bazel MODULE.bazel defs.bzl Makefile justfile
```

`rg` exit 1 means no matches, not a scan failure; exit 2 is an error. Account for
this distinction in any strict-shell audit. Categorize each hit as active runtime,
shared functionality, passive compatibility, documentation or uncertain. Trace
callers/build edges for uncertain hits; do not delete by filename or substring.

## Remove the active dependency chain

| Layer | Inspect and remove when voice-specific |
| --- | --- |
| UI | Composer microphone controls, slash commands, keybindings, footers, strips, device/voice settings, startup affordances, delivery handlers and voice-only snapshots. Keep generic layout/motion/widgets and accessibility. |
| Session/core | Realtime attachment/conversation state, event dispatch, prompt fragments, transcript/handoff machinery, sideband/BEM transports and runtime initialization. Shared session channels may have non-voice callers. |
| API/transports | Voice-only realtime HTTP/WebSocket/WebRTC endpoints and dependency edges. Keep ordinary Responses SSE/WebSocket support and non-voice realtime features. |
| Native | Reintroduced `voice-host`, `realtime-webrtc`, `third_party/voice`, native archives, patches, licenses tied solely to removed distributions, helper processes, device libraries and build toolchains. |
| Build | Workspace members/dependencies, `Cargo.lock`, Bazel targets/compile data, module extensions/locks, platform setup scripts, signing entitlements and CI matrices. Check all supported platforms, not just the host's cfg. |
| Packaging | Package manifests/required-file checks, helper aliases, resource copies, smoke scripts, installer tests, release workflow and artifact names. Remove voice-only requirements, not code-mode, ripgrep or Linux bubblewrap. |

The app-server protocol/config/history boundary is different from active runtime.
Retain passive types/defaults needed to read existing rollouts/configuration and
reject obsolete operations explicitly. Check resume/replay behavior. Do not erase
user history, rewrite recorded messages or remove shared audio-file helpers or
VoiceOver because their names resemble voice features.

When removing a voice channel or task, inspect its sender/receiver ownership and
shutdown paths: generic retained request channels may still be necessary. Preserve
manual plans and unrelated UI controls that happened to share a voice module.

## Tests and generation

- Delete tests exclusively asserting the intentionally removed implementation.
  Do not add negative tests for deleted internals. Retain/add relevant public
  compatibility tests for explicit unsupported operations and legacy decoding.
- Review each UI snapshot change against the intended UI. Keep coverage for
  surviving generic controls, layout and startup. Never accept all `.snap.new`
  merely because a large deletion changed snapshots.
- Replace a voice-only package smoke with **ordinary startup** coverage: real
  packaged TUI reaches the composer in a disposable home, daemon and code-mode
  work, and owned processes shut down. Do not replace it with `--version` alone.
- Regenerate config/app-server schemas and affected SDK bindings with current
  repository recipes; reconcile Cargo and Bazel locks and resource declarations.
- Check argument-comment annotations against actual parameter names after renames.
  A target-list generator failure must propagate, not become an empty green build.

## Final audit and evidence

Repeat the inventory across source, manifests, workflow/setup scripts and package
contents. Explain intentional residual matches. Verify that removed libraries are
not linked, packaged or fetched by the supported build graphs; text search alone
cannot prove that. Inspect generated manifests and executable/resource inventories.

Required preservation evidence includes ordinary chat/turns, daemon startup,
code-mode, subagent/permission behavior affected by the diff, and old history/config
compatibility. If authentication was touched, also cover account/source switching,
refresh/cancellation and upload ownership. Use focused integration tests and widen
scope under repository policy, not an indiscriminate rewrite of expectations.

Do not remove old installed packages kept for rollback merely to make a filesystem
search empty. The goal is no active voice runtime or new package dependency, not
destruction of recovery artifacts.
