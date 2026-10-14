---
name: sync-fork
description: "Синхронизация Antex с upstream OpenAI Codex с сохранением истории Git и поведения форка. Используй для sync-fork, обновления upstream и удаления или повторного выпиливания voice/realtime mode после merge; включает проверки и отдельно разрешаемые релиз и установку."
---

# Sync Antex; keep voice removed

Maintain this Git fork without undoing its security, authentication or packaging
work. Use the actual request as input: upstream ref, desired voice scope and
whether the user authorizes publication or installation. A voice-only request
does not authorize an upstream merge. A sync request does not authorize release,
credential replacement, daemon restart or production deployment.

## 1. Establish the checkpoint

Read repository `AGENTS.md` and relevant nested guidance. Find the root with
`git rev-parse --show-toplevel`; all repository paths below are relative to it.
Inspect status, worktrees, remotes, branch, HEAD and tags. Preserve unrelated edits
and the user's active branch; use a separate Git worktree for conflicting work.
Do not apply Arc commands to this Git repository.

Resolve the requested upstream ref to a full commit SHA after an authorized fetch.
Verify remote URLs rather than assuming their names. Record outside the source tree:

- fork HEAD, selected upstream SHA, merge base and worktree;
- preservation/removal matrix, changed crates and generated outputs;
- permitted actions, test scope, pending failures and evidence paths;
- next action and exact snapshot tested, including failures and skips.

Use the current source, not old transcripts, as the command/version authority.
`FORK_VERSION` is the fork release version; Cargo's workspace version is the
internal upstream version. Keep them distinct. Do not bump `FORK_VERSION` as an
incidental part of source synchronization.

## 2. Preserve these boundaries

| Surface | Preserve |
| --- | --- |
| Sandbox | Never alter `CODEX_SANDBOX_NETWORK_DISABLED_ENV_VAR` or `CODEX_SANDBOX_ENV_VAR` handling. Preserve Bash `--noprofile --norc -pc`, bubblewrap `--dev-no-fd`, descriptor denial and protected metadata masks; check current code and regression coverage. |
| Product | Daemon/service/socket identities, chat, code-mode, subagents, permissions, stored history, generic widgets/manual plans, shared audio-file helpers and VoiceOver accessibility. |
| Authentication | Direct ChatGPT OAuth/public Responses **and** legacy Codex/API-key modes. Do not turn an upstream Codex-only assumption into a fork-wide restriction. |
| Request ownership | Old in-flight payloads, uploads and turn/tool context cannot silently switch account, user, workspace or auth source. New operations may bind new credentials. Preserve refresh leases, document CAS, publication/cancellation/ABA guards and canonical token destinations. |
| Compatibility | Passive legacy config/DTO/history decoding can remain. Removed active operations must reject explicitly, not silently succeed. |

Authentication metadata is not request authority. Preserve fallible request-auth
acquisition and zero unsafe/anonymous dispatch on credential-command failure.
Do not weaken these boundaries, increase deadlines, skip tests or retry failed
assertions merely to obtain green.

## 3. Integrate upstream with real ancestry

Inspect the three-way changes before choosing conflict resolutions. Integrate the
selected upstream commit with Git ancestry intact; do not replace the fork with
an upstream archive or manufacture a synthetic merge as a shortcut. Afterward:

```bash
git merge-base --is-ancestor "$UPSTREAM_SHA" HEAD
git log -1 --format='%H%n%P'
git diff --check
```

These checks prove ancestry/whitespace, not functional correctness. If upstream
was already an ancestor or integration was a fast-forward, report that accurately.

Maintain a path/name map (`codex-rs` upstream versus `antex-rs` here). Review
conflicts by subsystem: runtime/core, auth/providers, protocol/schema/SDK, TUI,
sandbox and build/package tooling. Never globally replace `codex`: OAuth
originators, wire/service names, upstream archives, protobuf sources and generated
Rust filenames may intentionally differ. Inspect the referenced producer/output.
Remove orphan targets only after confirming they are unused; retain active
`antex-mcp` functionality.

When splitting work, assign disjoint file ownership. One coordinator owns the
index, refs, schema/lock reconciliation, remote synchronization, validation and
release. Do not sync a partly edited worker tree or transfer while a build is
using the destination. A task-wide checkpoint is more useful than optimistic ETAs.

## 4. Reapply the no-voice policy

For upstream updates **and** voice-only removal, read
[the voice-removal map](references/voice-removal.md). Upstream can reintroduce UI,
transports or native dependencies independently; absence of a microphone button
is not removal of the runtime.

Classify findings before deleting. Finish with both a source/dependency audit and
behavioral evidence, not a repository-wide ban on the words `voice` or `realtime`.

## 5. Validate the final source

Read [validation and release gates](references/validation-and-release.md) before
running checks or remote builds. Follow `AGENTS.md`: scoped tests first; ask before
a complete Rust suite; perform final scoped `just fix` and `just fmt` only after
required tests. **Do not rerun Rust tests after final fix/fmt.** Review the mechanical
delta. If a substantive defect appears at that boundary, do not pretend the old
results cover it or silently violate the rule; report and agree a new validation
cycle before proceeding.

Keep generated schemas, SDK outputs and Cargo/Bazel locks consistent. Review
snapshots individually; do not bulk accept historical `*.snap.new`. Classify
failures as implementation, fixture, intentional contract change, infrastructure
or unknown. A changed test expectation needs a reason and any required user
approval, not merely a new observed output.

## 6. Separate completion from publication

For local-only work, stop at a reviewed diff and verification report. For explicit
commit/push/release/install requests, follow the separately gated procedure in
[validation and release gates](references/validation-and-release.md). Never infer
fresh deployment permission from a release authorized in an earlier task.

Report independently: source implementation; hermetic validation; published refs
and CI/assets; full-package acceptance; actual installation; live model entitlement.
A pushed tag is not a published release, `--version` is not package acceptance,
and a catalog entry is not model access. State gaps and the next action plainly.

## Maintenance review

Use [review scenarios](evals/scenarios.md) when changing this skill. They are
acceptance criteria, not evidence that live builds/releases have been performed.
Do not run a production sync or deployment merely to evaluate these instructions.
