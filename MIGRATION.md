# Migrating from Codex to Antex

Antex keeps the fork's terminal interface and workflows. The executable is now
`antex`; the default data directory is `~/.antex`.

Install the matching `antex` and `antex-code-mode-host` binaries from one release.
Linux releases also include `antex-resources/bwrap`, which must remain beside the
binaries. Do not mix helpers from different releases.

Inspect the migration before applying it:

```sh
antex migrate
```

Close Codex processes, then copy the data:

```sh
antex migrate --apply
```

The migration preserves the source directory. It copies sessions, updates stored
paths, and transfers supported credential stores without deleting the original
credentials. Existing destination directories are never overwritten. Use
`--source` and `--destination` to select explicit directories.

Resume a migrated conversation with its existing identifier:

```sh
antex resume THREAD_ID
```

Project settings live in `.antex`. Copy each project's `.codex` directory to
`.antex`, preserving the original, and update references to project hook files in
the migrated configuration. Do not overwrite existing project settings.

Native environment variables use the `ANTEX_` prefix. External OpenAI protocol
names, model identifiers, and sandbox contracts retain their original spelling.
External plugins using `.codex-plugin/plugin.json` remain supported. Child tools
receive legacy `CODEX_HOME`, `CODEX_THREAD_ID`, `CODEX_SESSION_ID`, and
`CODEX_VERSION` aliases for integrations that still require them.

To return to the previous installation, close Antex and launch the saved Codex
binary with the original data directory. New Antex conversations are stored in
the new directory; rollback does not merge them into the original history.
