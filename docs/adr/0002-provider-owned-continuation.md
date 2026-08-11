# ADR 0002: Provider-owned local continuation

Status: accepted for P0.

## Context

Session Deck is primarily a historical browser, but users also need to return
to a session and continue it with the original coding agent. Session Deck must
not become a second Claude implementation or write Claude transcripts itself.

## Decision

- Historical indexing remains read-only. Session Deck never edits a provider transcript.
- Continuation is an explicit runtime capability implemented by each provider adapter.
- Claude continuation launches the installed Claude CLI in the historical working directory with `--resume <native-session-id>`.
- Commands are spawned as a validated executable plus an argument array. Shell command strings and arbitrary executable names are not accepted from the webview.
- The normalized Session Deck ID and the provider-native session ID are stored separately.
- A scan and continuation process are mutually exclusive for P0. When the PTY exits, Session Deck refreshes the same session and returns the reader to its newest turn.
- The terminal is embedded in the reader column; the two-column application layout remains unchanged.
- A global `dangerously_skip_permissions` setting is default-off. Enabling it requires explicit risk confirmation and adds exactly `--dangerously-skip-permissions`; P0 does not expose a free-form argument field.
- Session Deck validates the executable, supported CLI flag, UUID-shaped native ID, and working directory before launch.
- Session Deck does not implement authentication, remote control, teleport, cloud sessions, or cloud synchronization. Authentication remains the installed CLI's concern.

## Consequences

Claude may append to its own JSONL while a continuation is running. Session Deck
shows the previously indexed context beside the terminal and synchronizes
new history after process exit. Live JSONL tailing may be evaluated after the
first version, but is not part of P0.
