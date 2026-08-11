# Session Deck Engineering Rules

## Product Scope

Session Deck is a local-first desktop reader, search index, and controlled
continuation launcher for AI coding sessions. The first provider is Claude
Code. Provider session files are the source of truth for historical context.

## Safety Invariants

- Treat configured provider roots as read-only from Session Deck code. Never rename, lock, delete, truncate, or rewrite provider files.
- A provider-owned CLI may update its own transcript only after an explicit user continuation action. Session Deck must launch it with validated executable/argument arrays, never a shell command string.
- Do not start continuation while a scan is active, and do not scan a session while its continuation process is active. Re-index it after the process exits.
- Do not add telemetry, analytics, remote assets, cloud APIs, or network calls.
- Do not implement provider login, remote-control, teleport, or cloud-session flows.
- Transcript Markdown must be sanitized and must not automatically load remote images, media, frames, or other network resources.
- SQLite is a rebuildable derived index. When a successful root scan confirms a source session was removed, remove its derived rows in the same reconciliation transaction.
- A failed or unauthorized root scan must never be interpreted as source deletion.
- Diagnostics must not include prompt, response, tool output, credentials, or raw transcript excerpts.
- Test fixtures must be synthetic or irreversibly sanitized. Never commit real transcripts.

## Architecture

- `src/`: Vue 3 UI. It owns presentation and temporary selection/filter state only.
- `src-tauri/src/domain.rs`: provider-neutral models and application errors.
- `src-tauri/src/providers/`: provider discovery and parsing adapters.
- `src-tauri/src/scanner/`: read-only enumeration and reconciliation inputs.
- `src-tauri/src/indexer/`: normalization and transactional indexing.
- `src-tauri/src/storage/`: SQLite migrations and repositories.
- `src-tauri/src/search/`: FTS query construction and result grouping.
- `src-tauri/src/runtime/`: validated provider continuation specs, PTY lifecycle, and process events. It must not parse transcripts or accept arbitrary shell strings.
- `src-tauri/src/commands.rs`: narrow typed Tauri commands. UI code must not access arbitrary filesystem APIs.
- `src-tauri/tests/`: synthetic parser/storage tests and opt-in aggregate-only real-data smoke tests.
- `docs/adr/`: architectural decisions and their consequences.
- `src/features/reader/`: conversation-turn, Focus/Full, Markdown, and historical-change presentation.
- `src/features/terminal/`: embedded terminal presentation and temporary PTY UI state.
- `src/features/settings/`: typed local settings controls. Provider settings are local only and never written into provider config files.

Keep these as modules in one Rust crate for the MVP. Do not split crates until a second provider creates a demonstrated need.

## Rust Conventions

- Prefer explicit types and typed errors at module boundaries.
- Parse JSONL as a stream; one malformed line must not discard other valid lines.
- Preserve unknown provider events as diagnostics or provider payloads instead of panicking.
- Use parameterized SQL only.
- Store timestamps as UTC epoch milliseconds and render them in the UI locale.
- Historical branch data must come from the transcript, never from the repository's current branch.
- Provider-specific fields must not leak into provider-neutral table columns unless they represent a shared domain concept.

## Vue Conventions

- Use Vue 3 Composition API with `<script setup lang="ts">`.
- Keep route/root components as composition surfaces.
- Use typed props and emits; props flow down and events flow up.
- Keep source state minimal and derive filtered/sorted views with `computed`.
- Use `shallowRef` for primitive or opaque replacement-based state.
- Never render transcript content with unsanitized `v-html`.
- Place feature components under `src/features/<feature>/` and shared primitives under `src/components/`.
- The main UI is two columns: combined global search/project/session navigation on the left and the context reader on the right.

## UX Requirements

- Search is global across projects and sessions.
- Project hover/focus details include name, path, and latest activity.
- Session hover/focus details include timestamp, branch, and last meaningful user input.
- Tool calls and thinking are collapsed by default; user and assistant text remain primary.
- Completed turns render only their final assistant response as sanitized Markdown; raw Markdown remains available.
- Session actions use the Chinese label `隐藏`; hiding, renaming, and pinning affect only the local index.
- Claude continuation uses the native session ID and historical cwd. A global, default-off setting may add `--dangerously-skip-permissions` after explicit risk confirmation.
- The first continuation version refreshes the same session after the PTY exits; live transcript tailing is deferred.
- Every asynchronous surface must handle loading, error, empty, and partial-parse states.
- Keyboard focus must expose the same information as pointer hover.

## Validation

- Run the smallest relevant Rust or Vue test first, then broader checks.
- Include parser tests for human messages versus tool results, partial final lines, malformed lines, title precedence, and subagent discovery.
- Include storage tests proving that successful reconciliation removes missing sources and failed scans do not.
- Verify source file size, modification time, and content hash remain unchanged during integration tests.
- Before completion, run formatting, type checking, unit tests, and the production build.

## Cleanup

- Do not commit build output, local databases, logs, copied transcripts, or user paths.
- Remove temporary debugging output after diagnosis.
- Keep dependencies minimal and document why each non-standard dependency is needed.
