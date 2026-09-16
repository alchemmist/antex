# Antex

A terminal-native coding agent built on [OpenAI Codex](https://github.com/openai/codex) and kept in sync with upstream.

## What's included

- The TUI uses the terminal palette and updates the composer, conversation history, plans, and diffs immediately when the terminal theme changes.
- The configurable startup cockpit identifies alchemmist antex, shows its exact build commit, rotates fork-specific feature tips, and includes two animated ant mascot skins.
- `Ctrl+S` stashes the current prompt draft, persists it across restarts, and restores it on the next press.
- `/subagents <prompt>` explicitly enables subagents for one request; `/subagents` arms them for the next prompt.
- `/statusline` can show the number of active subagents, while `/agents` opens an overview of their work.
- Fast mode is process-local, resets to standard on every start or resume, and shows `⚡` in the status line while active.
- `/cd <path>` changes the current session's working directory without restarting Antex.
- `/todo` shows the complete plan, while a compact adaptive list of active items remains visible at the bottom of the TUI.
- `/workflow` runs configurable Python workflows with persistent state, per-agent models and reasoning, and parallel-agent support. The PR babysitter keeps CI and review feedback moving with fresh repair agents and forbids Quality Graph ignores.
- `/tmux-command-log` creates a separate tmux window containing Antex commands and their output.
- `/context` summarizes the model-visible context, while `/system-prompt` opens the complete latest logical model request in Neovim inside a new tmux window.
- `/dump` exports the full conversation to a responsive HTML file styled like [alchemmist.xyz](https://alchemmist.xyz), with tool activity collapsed between messages.
- The composer and submitted user messages share a cyan vertical rail, making prompts easy to find throughout the conversation.
- Vim editing supports Insert, Normal, character/line/block Visual modes, Russian keyboard aliases, system-clipboard yanks, and terminal-native selection colors.
- Prompts interrupted before work begins return to the editor; later interruptions are shown without a noisy error message.
- Force pushes always require an explicit Yes or No selection in the TUI.
- Fixes include tmux pane resize redraws, focus-related flickering, and a stable `Working` animation.
- The root `Makefile` installs a local build or downloads ready-made macOS and Linux releases.

## Build and install

```shell
make install-local
make install-mac
make install-linux
```

Upstream documentation: [developers.openai.com/codex](https://developers.openai.com/codex).

## Existing installations

Launch the application with `antex`. To bring over an existing installation, run `antex migrate` to inspect the source and destination. After stopping Codex clients, repeat the command with `--apply`, then use `antex resume`. The migration keeps the original data and writes the new home to `~/.antex`; `--source` and `--destination` select other locations.

`ANTEX_HOME` selects the Antex home. OpenAI service identifiers, model names, and existing session/protocol field names retain their upstream spelling for compatibility.

See [MIGRATION.md](MIGRATION.md) for project settings, matching helper binaries, and rollback instructions.
