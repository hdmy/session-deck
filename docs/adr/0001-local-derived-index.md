# ADR 0001: Local, derived session index

Status: accepted for MVP.

## Context

Claude Code owns transcripts under `~/.claude/projects/<project-slug>/<session-id>.jsonl` and may remove them according to its configured retention policy. Session Deck must make those sessions easy to browse without becoming a second archive or changing provider-owned data.

## Decision

- Claude Code JSONL files remain the only source of truth.
- Session Deck opens provider roots read-only. It never renames, edits, truncates, or deletes a transcript.
- SQLite and FTS5 contain a rebuildable projection for navigation and search, not an archive.
- The MVP indexes one active Claude source root. Normalized sessions carry a `provider_id`, their IDs are provider-namespaced, and reconciliation deletes rows only inside the provider being scanned. A provider/source registry will replace this single-root state when additional agents are added.
- A complete scan reconciles the index to the files currently present. When Claude removes a transcript after its 60-day retention window, the next complete scan removes that session from the index.
- A missing root, permission failure, I/O error, or incomplete directory enumeration cannot trigger reconciliation. Existing index rows remain untouched until a complete scan succeeds.
- Parser errors are isolated to individual JSONL lines. Readable events remain available and the session is marked partial.
- No transcript content, path, or search query is sent over the network. The MVP contains no analytics or telemetry.

## Consequences

The index can always be deleted and rebuilt. Session Deck intentionally does not preserve a transcript after Claude Code deletes the source file. Adding Codex, Gemini CLI, or OpenCode requires a provider adapter that emits the same normalized project, session, and timeline records while retaining the same read-only and reconciliation guarantees.
