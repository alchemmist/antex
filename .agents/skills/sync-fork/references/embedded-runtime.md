# Embedded foreground runtime

Antex interactive sessions must use the engine embedded in the launched CLI. New,
resume, fork and worktree launches must never probe or attach to a previously
installed shared app-server. Subagents still run through the core agent controller;
within-conversation agent navigation and interactive questions must remain usable.

## What must stay removed

- The app-server-daemon workspace crate and CLI/TUI dependencies on it.
- Managed daemon lifecycle commands, updater loops and server-owned installations.
- Implicit control-socket discovery, auto-start, daemon compatibility/recovery menus
  and server-version notices in the interactive UI.
- Public daemon-wide agents/remote-control launch modes and network-listener flags
  on the distributed CLI's app-server helper.
- Daemon-only/deferred installer flows. Legacy invocations of those flows must fail
  before mutation, not be silently reinterpreted as foreground CLI installations.

`--no-daemon` is a hidden compatibility no-op for existing launchers, including
user tmux fork bindings. It must remain accepted while local launch is always
embedded. `features.daemon_auto_start` is retained only as a removed compatibility
key; it must not regain runtime consumers or a true default.

## What is not a daemon

Do not delete the in-process app-server library/protocol, the on-demand foreground
stdio helper used by SDK clients, code-mode host subprocesses, or exec-server and
sandbox isolation. These do not justify reinstalling a shared GUI backend or
selecting a second independently updated CLI package. Test-only transport harnesses
are not installed production daemons.

Keep authentication acquisition/ownership, CLI override precedence, saved-session
restoration, tool approval/questions, sandbox limits and durable rollouts intact.
A CLI disconnect or frontend exit must not accidentally leave its own engine and
subagents running as a replacement background service.

## Validation and migration

Verify ordinary startup, resume, fork and worktree launch; exercise a stale legacy
control socket without connecting to it. Exercise a real subagent's message flow
and code-mode tools through the embedded engine, plus installed-package TUI startup
and process cleanup. Test foreground/stdio SDK compatibility separately.

Do not automatically stop/delete an old running daemon, its package, auth or
history during source changes or installation. Existing old clients may still use
it. A one-time retirement requires an explicitly approved quiescent window; new
Antex processes must not depend on that cleanup to run correctly.
