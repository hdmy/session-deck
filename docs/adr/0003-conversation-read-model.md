# ADR 0003: Conversation graph and turn read model

Status: accepted for P0.

## Context

Claude transcripts are append-oriented JSONL files, but their user-visible
conversation is a graph linked by UUIDs. Rewind, compaction, assistant message
fragments, tool results, and sidechains mean file order alone is not a reliable
reader model.

## Decision

- The Claude provider first parses loss-tolerant transcript entries with their graph and correlation metadata.
- A provider-specific graph resolver selects the current main branch, repairs supported compact-boundary relationships, and excludes meta/team/sidechain nodes from the top-level conversation.
- A provider-neutral turn assembler groups a meaningful user prompt, assistant content, thinking, tool calls, tool results, and nested subagent activity into one turn.
- Tool calls are correlated by `tool_use_id`; nested activity is correlated by `parent_tool_use_id`.
- The final non-empty assistant text in each completed turn is marked as its final response.
- Focus view projects user prompts, final responses, and one folded activity summary per turn. Full view exposes the underlying activity.
- Only final responses are rendered as sanitized Markdown. Search indexes readable source text, not rendered HTML.

## Consequences

One source session remains one sidebar session even if its transcript contains
alternate branches. The storage model retains enough provider-neutral lineage
and turn information for accurate reading without exposing Claude-specific JSON
to the Vue UI.
