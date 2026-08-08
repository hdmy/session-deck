# Context Vault

Context Vault is a local desktop index and controlled continuation launcher for AI coding history. The P0 release scans Claude Code sessions, groups them by project, reconstructs conversation turns, provides a two-column context reader, searches historical content, and can resume an explicitly selected session through an embedded Claude CLI terminal.

## Safety model

- `~/.claude/projects` is opened read-only and remains the source of truth.
- Context Vault never edits Claude JSONL. After an explicit continuation action, the installed Claude CLI may append to its own session; Context Vault re-indexes only after that process exits.
- SQLite/FTS5 is a derived local index, not a permanent archive.
- A session disappears from the index only after its source file is gone and a complete scan succeeds. This follows Claude Code's configured 60-day retention policy.
- Failed or incomplete scans never reconcile deletions.
- No transcript, path, or query is uploaded. The app has no telemetry or analytics.

## Architecture

```text
provider source (read-only)
  -> provider discovery + loss-tolerant parser
  -> provider graph resolver + normalized conversation turns
  -> provider-scoped index reconciliation
  -> SQLite + FTS5
  -> typed Tauri commands
  -> Vue project tree + context reader + controlled PTY
```

The module boundaries are:

- `src-tauri/src/providers`: provider-owned discovery and transcript normalization. Claude is the first adapter.
- `src-tauri/src/scanner`: provider-neutral scan orchestration and safety status.
- `src-tauri/src/indexer`: provider-scoped reconciliation entry point.
- `src-tauri/src/storage`: SQLite schema, migrations, normalized reads, and FTS5 search.
- `src-tauri/src/runtime`: validated provider resume specs and bounded PTY lifecycle; it never accepts shell command strings.
- `src/features/browser`: search and project/session navigation.
- `src/features/reader`: Focus/Full conversation rendering and sanitized final-response Markdown.
- `src/features/terminal`: embedded Claude continuation terminal and temporary PTY state.
- `src/features/settings`: local executable and default-off permission-bypass controls.

Normalized sessions include a `provider_id`, use provider-namespaced IDs, and are reconciled only within that provider. A future Codex, Gemini CLI, or OpenCode adapter can therefore reuse storage, search, commands, and the viewer.

Claude continuation uses the historical `cwd` and native UUID with `claude --resume <session-id>`. The optional `--dangerously-skip-permissions` setting is off by default and requires explicit risk acknowledgement. Login, cloud sync, remote control, free-form launch arguments, and live transcript tailing are outside P0.

## Development

Prerequisites: current Node.js, pnpm, Rust, and the platform dependencies required by Tauri 2.

```bash
pnpm install
pnpm tauri dev
```

Validation:

```bash
pnpm typecheck
pnpm test
pnpm build
cd src-tauri && cargo test --all-targets
cd src-tauri && cargo clippy --all-targets -- -D warnings
```

The environment-dependent real-data smoke test is ignored by default and prints aggregate counts only:

```bash
cd src-tauri && cargo test --test real_data -- --ignored --nocapture
```

See [ADR 0001](docs/adr/0001-local-derived-index.md) for source-of-truth and reconciliation, [ADR 0002](docs/adr/0002-provider-owned-continuation.md) for controlled continuation, and [ADR 0003](docs/adr/0003-conversation-read-model.md) for graph/turn reconstruction.
