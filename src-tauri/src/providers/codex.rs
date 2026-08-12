//! Read-only Codex rollout adapter.
//!
//! Codex writes one JSON object per line.  The parser deliberately keeps the
//! provider-specific envelope here and emits only the normalized domain model.

use crate::{
    domain::{
        AppError, BranchSummary, ConversationTurn, Diagnostic, ParsedBranch, ParsedSession, Result,
        SessionSummary, TimelineEvent, TurnActivity,
    },
    providers::{ProviderCapabilities, SessionProvider, SourceDiscovery},
};
use serde_json::Value;
use sha2::{Digest, Sha256};
use std::{
    fs::{self, File},
    io::{BufRead, BufReader},
    path::{Path, PathBuf},
    time::UNIX_EPOCH,
};

pub const PROVIDER_ID: &str = "codex";

#[derive(Debug, Clone, Copy, Default)]
pub struct CodexProvider;

impl SessionProvider for CodexProvider {
    fn id(&self) -> &'static str {
        PROVIDER_ID
    }

    fn default_root(&self) -> PathBuf {
        directories::BaseDirs::new()
            .map(|dirs| dirs.home_dir().join(".codex"))
            .unwrap_or_else(|| PathBuf::from(".codex"))
    }

    fn discover(&self, root: &Path) -> Result<SourceDiscovery> {
        discover_sources(root)
    }

    fn parse(&self, path: &Path) -> Result<ParsedSession> {
        parse_session(path).map_err(AppError::from)
    }

    fn capabilities(&self) -> ProviderCapabilities {
        ProviderCapabilities {
            supports_reader: true,
            supports_search: true,
            supports_resume: false,
            ..Default::default()
        }
    }
}

fn discover_sources(root: &Path) -> Result<SourceDiscovery> {
    if !root.exists() || !root.is_dir() {
        return Err(AppError::InvalidRoot(root.display().to_string()));
    }
    let sessions = root.join("sessions");
    if !sessions.exists() || !sessions.is_dir() {
        return Err(AppError::InvalidRoot(sessions.display().to_string()));
    }
    let mut out = SourceDiscovery {
        complete: true,
        ..Default::default()
    };
    visit_jsonl(&sessions, &mut out)?;
    out.paths.sort();
    Ok(out)
}

fn visit_jsonl(dir: &Path, out: &mut SourceDiscovery) -> std::io::Result<()> {
    for entry in fs::read_dir(dir)? {
        let entry = match entry {
            Ok(value) => value,
            Err(_) => {
                out.complete = false;
                out.diagnostics.push("entry_unreadable".into());
                continue;
            }
        };
        let kind = match entry.file_type() {
            Ok(value) => value,
            Err(_) => {
                out.complete = false;
                out.diagnostics.push("entry_type_unreadable".into());
                continue;
            }
        };
        if kind.is_symlink() {
            out.complete = false;
            out.diagnostics.push("entry_unsafe".into());
            continue;
        }
        if kind.is_dir() {
            visit_jsonl(&entry.path(), out)?;
            continue;
        }
        let path = entry.path();
        if kind.is_file()
            && path.extension().and_then(|v| v.to_str()) == Some("jsonl")
            && !is_subagent_rollout(&path)?
        {
            out.paths.push(path);
        }
    }
    Ok(())
}

fn is_subagent_rollout(path: &Path) -> std::io::Result<bool> {
    let file = File::open(path)?;
    for raw in BufReader::new(file).lines() {
        let raw = match raw {
            Ok(value) => value,
            Err(_) => continue,
        };
        let value: Value = match serde_json::from_str(&raw) {
            Ok(value) => value,
            Err(_) => continue,
        };
        if value.get("type").and_then(Value::as_str) != Some("session_meta") {
            continue;
        }
        let payload = value.get("payload").unwrap_or(&value);
        return Ok(is_subagent_metadata(payload));
    }
    Ok(false)
}

fn is_subagent_metadata(payload: &Value) -> bool {
    payload
        .get("agent_role")
        .and_then(Value::as_str)
        .is_some_and(|role| role != "main")
        || payload
            .get("forked_from_id")
            .and_then(Value::as_str)
            .is_some()
        || payload
            .get("source")
            .and_then(|source| source.get("subagent"))
            .is_some()
}

fn uses_canonical_event_messages(path: &Path) -> std::io::Result<bool> {
    for raw in BufReader::new(File::open(path)?).lines() {
        let Ok(raw) = raw else { continue };
        let Ok(value) = serde_json::from_str::<Value>(&raw) else {
            continue;
        };
        if value.get("type").and_then(Value::as_str) == Some("event_msg")
            && matches!(
                value
                    .get("payload")
                    .and_then(|payload| payload.get("type"))
                    .and_then(Value::as_str),
                Some("user_message" | "agent_message" | "agent_reasoning")
            )
        {
            return Ok(true);
        }
    }
    Ok(false)
}

pub fn parse_session(path: &Path) -> std::io::Result<ParsedSession> {
    let canonical_event_messages = uses_canonical_event_messages(path)?;
    let initial = fs::metadata(path)?;
    let file = File::open(path)?;
    let mut reader = BufReader::new(file);
    let mut buffer = Vec::new();
    let mut hasher = Sha256::new();
    let mut diagnostics = Vec::new();
    let mut events = Vec::new();
    let mut turns = Vec::<ConversationTurn>::new();
    let mut session_id = path
        .file_stem()
        .and_then(|v| v.to_str())
        .unwrap_or("unknown")
        .to_owned();
    let mut cwd = None;
    let mut branch = None;
    let mut models = Vec::new();
    let mut first_prompt = None;
    let mut last_prompt = None;
    let mut started_at = None;
    let mut ended_at = None;
    let mut tool_count = 0_i64;
    let mut sequence = 0_i64;
    let mut line_number = 0_usize;
    let mut source_size = 0_i64;
    let mut is_subagent = false;

    while {
        buffer.clear();
        let read = reader.read_until(b'\n', &mut buffer)?;
        if read == 0 {
            false
        } else {
            source_size += read as i64;
            hasher.update(&buffer);
            line_number += 1;
            true
        }
    } {
        let terminated = buffer.last() == Some(&b'\n');
        if terminated {
            buffer.pop();
            if buffer.last() == Some(&b'\r') {
                buffer.pop();
            }
        }
        if buffer.iter().all(u8::is_ascii_whitespace) {
            continue;
        }
        let value: Value = match serde_json::from_slice(&buffer) {
            Ok(value) => value,
            Err(_) => {
                diagnostics.push(Diagnostic {
                    line: line_number,
                    code: if terminated {
                        "malformed_json"
                    } else {
                        "partial_json"
                    }
                    .into(),
                });
                continue;
            }
        };
        let timestamp = event_timestamp(&value);
        if let Some(ts) = timestamp {
            started_at = Some(started_at.map_or(ts, |v: i64| v.min(ts)));
            ended_at = Some(ended_at.map_or(ts, |v: i64| v.max(ts)));
        }
        let kind = value.get("type").and_then(Value::as_str).unwrap_or("");
        let payload = value.get("payload").unwrap_or(&value);
        if kind == "session_meta" {
            if let Some(id) = payload
                .get("session_id")
                .or_else(|| payload.get("id"))
                .and_then(Value::as_str)
            {
                session_id = id.to_owned();
            }
            cwd = payload
                .get("cwd")
                .and_then(Value::as_str)
                .map(str::to_owned);
            branch = payload
                .get("git")
                .and_then(|v| v.get("branch"))
                .and_then(Value::as_str)
                .or_else(|| payload.get("branch").and_then(Value::as_str))
                .map(str::to_owned);
            is_subagent = is_subagent_metadata(payload);
            if let Some(model) = payload.get("model").and_then(Value::as_str) {
                if !models.iter().any(|value| value == model) {
                    models.push(model.to_owned());
                }
            }
            sequence += 1;
            continue;
        }
        if is_subagent {
            sequence += 1;
            continue;
        }
        if kind == "turn_context" {
            if cwd.is_none() {
                cwd = payload
                    .get("cwd")
                    .and_then(Value::as_str)
                    .map(str::to_owned);
            }
            if let Some(model) = payload.get("model").and_then(Value::as_str) {
                if !models.iter().any(|value| value == model) {
                    models.push(model.to_owned());
                }
            }
            sequence += 1;
            continue;
        }
        if kind == "event_msg" {
            let event_kind = payload.get("type").and_then(Value::as_str);
            let known = match event_kind {
                Some("user_message" | "agent_message" | "agent_reasoning") => {
                    if canonical_event_messages {
                        let before = events.len();
                        parse_event_message(
                            payload,
                            ParsePayloadContext {
                                session_id: &session_id,
                                timestamp,
                                events: &mut events,
                                turns: &mut turns,
                                tool_count: &mut tool_count,
                                first: &mut first_prompt,
                                last: &mut last_prompt,
                            },
                        );
                        for event in &mut events[before..] {
                            event.sequence = sequence;
                        }
                    }
                    true
                }
                Some(
                    "token_count"
                    | "patch_apply_end"
                    | "sub_agent_activity"
                    | "task_started"
                    | "task_complete"
                    | "mcp_tool_call_end"
                    | "thread_settings_applied"
                    | "context_compacted"
                    | "web_search_end"
                    | "turn_aborted"
                    | "thread_rolled_back",
                ) => true,
                _ => false,
            };
            if !known {
                diagnostics.push(Diagnostic {
                    line: line_number,
                    code: "unsupported_event".into(),
                });
            }
            sequence += 1;
            continue;
        }
        if matches!(
            kind,
            "world_state" | "compacted" | "inter_agent_communication_metadata"
        ) {
            sequence += 1;
            continue;
        }
        if kind != "response_item" {
            diagnostics.push(Diagnostic {
                line: line_number,
                code: "unsupported_event".into(),
            });
            sequence += 1;
            continue;
        }
        let item_kind = payload.get("type").and_then(Value::as_str);
        let supported = matches!(
            item_kind,
            Some(
                "message"
                    | "reasoning"
                    | "function_call"
                    | "function_call_output"
                    | "custom_tool_call"
                    | "custom_tool_call_output"
                    | "agent_message"
            )
        ) || (item_kind.is_none()
            && (payload.get("message").is_some() || payload.get("role").is_some()));
        if !supported {
            diagnostics.push(Diagnostic {
                line: line_number,
                code: "unsupported_event".into(),
            });
            sequence += 1;
            continue;
        }
        if item_kind == Some("agent_message")
            || (canonical_event_messages && matches!(item_kind, Some("message" | "reasoning")))
        {
            sequence += 1;
            continue;
        }
        if let Some(model) = payload.get("model").and_then(Value::as_str) {
            if !models.iter().any(|m| m == model) {
                models.push(model.to_owned());
            }
        }
        let before = events.len();
        parse_payload(
            payload,
            ParsePayloadContext {
                session_id: &session_id,
                timestamp,
                events: &mut events,
                turns: &mut turns,
                tool_count: &mut tool_count,
                first: &mut first_prompt,
                last: &mut last_prompt,
            },
        );
        for event in &mut events[before..] {
            event.sequence = sequence;
        }
        sequence += 1;
    }
    let final_meta = fs::metadata(path)?;
    if final_meta.len() != initial.len() || final_meta.modified().ok() != initial.modified().ok() {
        diagnostics.push(Diagnostic {
            line: line_number,
            code: "source_changed_during_scan".into(),
        });
    }
    let index_title = read_index_title(path, &session_id).ok().flatten();
    let title = index_title
        .or_else(|| first_prompt.clone())
        .unwrap_or_else(|| {
            format!(
                "Untitled session · {}",
                session_id.chars().take(8).collect::<String>()
            )
        });
    let normalized = format!("{PROVIDER_ID}:{session_id}");
    for (index, event) in events.iter_mut().enumerate() {
        event.id = index as i64 + 1;
        event.session_id = normalized.clone();
    }
    for turn in &mut turns {
        turn.session_id = normalized.clone();
        for activity in &mut turn.activities {
            activity.event_id = activity.event_id.max(1);
        }
    }
    let branch_summary = BranchSummary {
        id: "main".into(),
        session_id: normalized.clone(),
        label: "main".into(),
        kind: "main".into(),
        root_uuid: None,
        leaf_uuid: None,
        fork_point_uuid: None,
        is_active: true,
        event_count: events.len(),
        turn_count: turns.len(),
        started_at,
        ended_at,
        compacted: false,
    };
    let summary = SessionSummary {
        id: normalized.clone(),
        native_session_id: Some(session_id),
        provider_id: PROVIDER_ID.into(),
        project_id: cwd
            .clone()
            .unwrap_or_else(|| format!("{PROVIDER_ID}:unknown")),
        workspace_id: cwd
            .clone()
            .unwrap_or_else(|| format!("{PROVIDER_ID}:unknown")),
        project_path: cwd.clone(),
        worktree_path: None,
        source_title: title.clone(),
        title,
        hidden: false,
        pinned: false,
        last_used_at: None,
        started_at,
        ended_at,
        branch,
        first_prompt,
        last_prompt,
        cwd,
        models,
        tool_count,
        source_mtime: final_meta
            .modified()
            .ok()
            .and_then(|v| v.duration_since(UNIX_EPOCH).ok())
            .map(|v| v.as_millis() as i64)
            .unwrap_or_default(),
        partial: !diagnostics.is_empty(),
    };
    Ok(ParsedSession {
        summary,
        events: events.clone(),
        turns: turns.clone(),
        branches: vec![ParsedBranch {
            summary: branch_summary,
            events,
            turns,
            tool_stats: Vec::new(),
            turn_insights: Vec::new(),
        }],
        diagnostics,
        source_path: path.to_path_buf(),
        source_size,
        source_hash: format!("{:x}", hasher.finalize()),
        cwd_history: Vec::new(),
    })
}

struct ParsePayloadContext<'a> {
    session_id: &'a str,
    timestamp: Option<i64>,
    events: &'a mut Vec<TimelineEvent>,
    turns: &'a mut Vec<ConversationTurn>,
    tool_count: &'a mut i64,
    first: &'a mut Option<String>,
    last: &'a mut Option<String>,
}

fn parse_event_message(payload: &Value, context: ParsePayloadContext<'_>) {
    let normalized = match payload.get("type").and_then(Value::as_str) {
        Some("user_message") => serde_json::json!({
            "role": "user",
            "content": [{ "type": "input_text", "text": payload.get("message") }],
        }),
        Some("agent_message") => serde_json::json!({
            "role": "assistant",
            "phase": payload.get("phase"),
            "content": [{ "type": "output_text", "text": payload.get("message") }],
        }),
        Some("agent_reasoning") => serde_json::json!({
            "type": "reasoning",
            "text": payload.get("text"),
        }),
        _ => return,
    };
    parse_payload(&normalized, context);
}

fn parse_payload(payload: &Value, context: ParsePayloadContext<'_>) {
    let ParsePayloadContext {
        session_id,
        timestamp,
        events,
        turns,
        tool_count,
        first,
        last,
    } = context;
    let message = payload.get("message").unwrap_or(payload);
    let role = message
        .get("role")
        .and_then(Value::as_str)
        .or_else(|| payload.get("role").and_then(Value::as_str));
    let assistant_final = role == Some("assistant")
        && payload
            .get("phase")
            .and_then(Value::as_str)
            .is_none_or(|phase| phase == "final_answer");
    let null_content = Value::Null;
    let content = message
        .get("content")
        .or_else(|| message.get("parts"))
        .unwrap_or(&null_content);
    let blocks = if content.is_null()
        && matches!(
            payload.get("type").and_then(Value::as_str),
            Some(
                "function_call"
                    | "function_call_output"
                    | "tool_call"
                    | "custom_tool_call"
                    | "custom_tool_call_output"
                    | "reasoning"
                    | "thought"
            )
        ) {
        vec![payload.clone()]
    } else {
        content
            .as_array()
            .cloned()
            .unwrap_or_else(|| vec![content.clone()])
    };
    for block in blocks {
        let (kind, text, tool_name, tool_id, final_response) = match block
            .get("type")
            .and_then(Value::as_str)
        {
            Some("input_text") | Some("output_text")
                if matches!(role, Some("user" | "assistant")) =>
            {
                (
                    if role == Some("user") {
                        "user"
                    } else {
                        "assistant"
                    },
                    block
                        .get("text")
                        .and_then(Value::as_str)
                        .unwrap_or_default()
                        .to_owned(),
                    None,
                    None,
                    assistant_final,
                )
            }
            Some("input_text") | Some("output_text") => continue,
            Some("function_call") | Some("tool_call") | Some("custom_tool_call") => (
                "tool_use",
                block
                    .get("arguments")
                    .or_else(|| block.get("input"))
                    .map(value_text)
                    .unwrap_or_default(),
                block.get("name").and_then(Value::as_str).map(str::to_owned),
                block
                    .get("call_id")
                    .or_else(|| block.get("id"))
                    .and_then(Value::as_str)
                    .map(str::to_owned),
                false,
            ),
            Some(
                "function_output"
                | "function_call_output"
                | "tool_result"
                | "custom_tool_call_output",
            ) => (
                "tool_result",
                block
                    .get("output")
                    .or_else(|| block.get("content"))
                    .map(value_text)
                    .unwrap_or_default(),
                block.get("name").and_then(Value::as_str).map(str::to_owned),
                block
                    .get("call_id")
                    .or_else(|| block.get("id"))
                    .and_then(Value::as_str)
                    .map(str::to_owned),
                false,
            ),
            Some("reasoning") | Some("thought") => (
                "thinking",
                block
                    .get("text")
                    .or_else(|| block.get("summary"))
                    .map(value_text)
                    .unwrap_or_default(),
                None,
                None,
                false,
            ),
            _ => {
                if role == Some("user") || role == Some("assistant") {
                    let text = block
                        .as_str()
                        .map(str::to_owned)
                        .or_else(|| block.get("text").and_then(Value::as_str).map(str::to_owned))
                        .unwrap_or_default();
                    (
                        if role == Some("user") {
                            "user"
                        } else {
                            "assistant"
                        },
                        text,
                        None,
                        None,
                        assistant_final,
                    )
                } else {
                    continue;
                }
            }
        };
        if text.trim().is_empty() && kind != "tool_use" {
            continue;
        }
        let collapsed = matches!(kind, "tool_use" | "tool_result" | "thinking");
        let event_id = events.len() as i64 + 1;
        let turn_id = if role == Some("user") {
            Some(turns.len() as i64 + 1)
        } else {
            turns.last().map(|turn| turn.id)
        };
        if final_response {
            if let Some(previous) = events.iter_mut().rev().find(|event| {
                event.kind == "assistant"
                    && event.role.as_deref() == Some("assistant")
                    && event.final_response
                    && event.turn_id == turn_id
            }) {
                previous.final_response = false;
            }
        }
        events.push(TimelineEvent {
            id: event_id,
            session_id: session_id.to_owned(),
            kind: kind.into(),
            role: role.map(str::to_owned),
            content: text.clone(),
            timestamp,
            tool_name: tool_name.clone(),
            collapsed,
            uuid: None,
            parent_uuid: None,
            logical_parent_uuid: None,
            message_id: None,
            parent_tool_use_id: None,
            tool_use_id: tool_id.clone(),
            sequence: 0,
            is_sidechain: false,
            is_meta: false,
            turn_id,
            final_response,
            compact_boundary: false,
            compact_preserved_ids: Vec::new(),
        });
        let needs_turn = role == Some("user");
        if needs_turn {
            turns.push(ConversationTurn {
                id: turns.len() as i64 + 1,
                session_id: session_id.to_owned(),
                user_prompt: Some(text.clone()),
                activities: Vec::new(),
                final_response: None,
                timestamp,
                completed: false,
            });
            if first.is_none() {
                *first = Some(text.clone());
            }
            *last = Some(text.clone());
        }
        let turn = turns.last_mut();
        if let Some(turn) = turn {
            if final_response {
                if let Some(previous) = turn
                    .activities
                    .iter_mut()
                    .rev()
                    .find(|activity| activity.kind == "assistant" && activity.final_response)
                {
                    previous.final_response = false;
                }
                turn.final_response = Some(text.clone());
                turn.completed = true;
            }
            if role != Some("user") {
                turn.activities.push(TurnActivity {
                    event_id,
                    kind: kind.into(),
                    role: role.map(str::to_owned),
                    content: text,
                    timestamp,
                    tool_name,
                    tool_use_id: tool_id,
                    parent_tool_use_id: None,
                    collapsed,
                    final_response,
                });
            }
        }
        if kind == "tool_use" {
            *tool_count += 1;
        }
    }
}

fn value_text(value: &Value) -> String {
    match value {
        Value::Null => String::new(),
        Value::String(text) => text.clone(),
        Value::Array(values) => values
            .iter()
            .map(value_text)
            .filter(|text| !text.trim().is_empty())
            .collect::<Vec<_>>()
            .join("\n"),
        Value::Object(_) => {
            for key in ["text", "output", "content", "summary"] {
                let text = value.get(key).map(value_text).unwrap_or_default();
                if !text.trim().is_empty() {
                    return text;
                }
            }
            value.to_string()
        }
        _ => value.to_string(),
    }
}
fn event_timestamp(value: &Value) -> Option<i64> {
    value.get("timestamp").and_then(Value::as_i64).or_else(|| {
        value
            .get("timestamp")
            .and_then(Value::as_str)
            .and_then(|v| chrono::DateTime::parse_from_rfc3339(v).ok())
            .map(|v| v.timestamp_millis())
    })
}

fn read_index_title(path: &Path, native_id: &str) -> std::io::Result<Option<String>> {
    let Some(codex_root) = path.ancestors().find(|p| {
        let index = p.join("session_index.jsonl");
        std::fs::symlink_metadata(index)
            .map(|metadata| metadata.is_file() && !metadata.file_type().is_symlink())
            .unwrap_or(false)
    }) else {
        return Ok(None);
    };
    let file = File::open(codex_root.join("session_index.jsonl"))?;
    for raw in BufReader::new(file)
        .lines()
        .map_while(std::result::Result::ok)
    {
        let value: Value = match serde_json::from_str(&raw) {
            Ok(v) => v,
            Err(_) => continue,
        };
        let id = value
            .get("session_id")
            .or_else(|| value.get("id"))
            .and_then(Value::as_str);
        if id == Some(native_id) {
            if let Some(title) = value
                .get("thread_name")
                .or_else(|| value.get("title"))
                .and_then(Value::as_str)
                .filter(|v| !v.trim().is_empty())
            {
                return Ok(Some(title.trim().to_owned()));
            }
        }
    }
    Ok(None)
}

#[cfg(test)]
mod tests {
    use super::{discover_sources, parse_session, read_index_title};
    use std::fs;
    use std::io::Write;
    use tempfile::NamedTempFile;

    #[test]
    fn malformed_and_partial_lines_keep_valid_messages_and_mark_final_text() {
        let mut file = NamedTempFile::new().unwrap();
        writeln!(file, r#"{{"timestamp":1,"type":"session_meta","payload":{{"session_id":"native","cwd":"/tmp/project","agent_role":"main"}}}}"#).unwrap();
        writeln!(file, r#"{{"timestamp":2,"type":"response_item","payload":{{"message":{{"role":"user","content":[{{"type":"input_text","text":"hello"}}]}}}}}}"#).unwrap();
        writeln!(file, r#"{{"timestamp":3,"type":"response_item","payload":{{"message":{{"role":"assistant","content":[{{"type":"output_text","text":"done"}},{{"type":"function_call","name":"ls","call_id":"c1","arguments":{{}}}}]}}}}}}"#).unwrap();
        writeln!(file, "not-json").unwrap();
        write!(file, "{{\"type\":").unwrap();
        let parsed = parse_session(file.path()).unwrap();
        assert_eq!(parsed.summary.id, "codex:native");
        assert_eq!(parsed.summary.first_prompt.as_deref(), Some("hello"));
        assert!(parsed
            .events
            .iter()
            .any(|event| event.kind == "tool_use" && event.collapsed));
        assert!(parsed.events.iter().any(|event| event.final_response));
        assert!(parsed
            .diagnostics
            .iter()
            .any(|item| item.code == "malformed_json"));
        assert!(parsed
            .diagnostics
            .iter()
            .any(|item| item.code == "partial_json"));
    }

    #[test]
    fn current_event_stream_keeps_visible_messages_reasoning_and_tools() {
        let mut file = NamedTempFile::new().unwrap();
        file.write_all(
            br#"{"timestamp":1,"type":"session_meta","payload":{"session_id":"native","cwd":"/tmp/project","source":"vscode"}}
{"timestamp":2,"type":"event_msg","payload":{"type":"task_started"}}
{"timestamp":3,"type":"response_item","payload":{"type":"message","role":"developer","content":[{"type":"input_text","text":"hidden instructions"}]}}
{"timestamp":4,"type":"response_item","payload":{"type":"message","role":"user","content":[{"type":"input_text","text":"hidden context"},{"type":"input_text","text":"visible prompt"}]}}
{"timestamp":5,"type":"turn_context","payload":{"cwd":"/tmp/project","model":"gpt-test"}}
{"timestamp":6,"type":"event_msg","payload":{"type":"user_message","message":"visible prompt"}}
{"timestamp":7,"type":"response_item","payload":{"type":"message","role":"user","content":[{"type":"input_text","text":"visible prompt"}]}}
{"timestamp":8,"type":"event_msg","payload":{"type":"agent_reasoning","text":"checking"}}
{"timestamp":9,"type":"response_item","payload":{"type":"reasoning","summary":[{"type":"summary_text","text":"checking"}]}}
{"timestamp":10,"type":"event_msg","payload":{"type":"agent_message","phase":"commentary","message":"working"}}
{"timestamp":11,"type":"response_item","payload":{"type":"message","role":"assistant","phase":"commentary","content":[{"type":"output_text","text":"working"}]}}
{"timestamp":12,"type":"response_item","payload":{"type":"custom_tool_call","name":"apply_patch","call_id":"call-1","input":"patch"}}
{"timestamp":13,"type":"response_item","payload":{"type":"custom_tool_call_output","call_id":"call-1","output":"ok"}}
{"timestamp":14,"type":"event_msg","payload":{"type":"agent_message","phase":"final_answer","message":"done"}}
{"timestamp":15,"type":"response_item","payload":{"type":"message","role":"assistant","phase":"final_answer","content":[{"type":"output_text","text":"done"}]}}
{"timestamp":16,"type":"world_state","payload":{"full":true}}
{"timestamp":17,"type":"compacted","payload":{"replacement_history":[]}}
{"timestamp":18,"type":"inter_agent_communication_metadata","payload":{"trigger_turn":{}}}
{"timestamp":19,"type":"event_msg","payload":{"type":"task_complete"}}
"#,
        )
        .unwrap();

        let parsed = parse_session(file.path()).unwrap();

        assert!(parsed.diagnostics.is_empty());
        assert!(!parsed.summary.partial);
        assert_eq!(
            parsed.summary.first_prompt.as_deref(),
            Some("visible prompt")
        );
        assert_eq!(parsed.summary.models, vec!["gpt-test"]);
        assert_eq!(parsed.summary.tool_count, 1);
        assert_eq!(
            parsed
                .events
                .iter()
                .map(|event| (event.kind.as_str(), event.content.as_str()))
                .collect::<Vec<_>>(),
            vec![
                ("user", "visible prompt"),
                ("thinking", "checking"),
                ("assistant", "working"),
                ("tool_use", "patch"),
                ("tool_result", "ok"),
                ("assistant", "done"),
            ]
        );
        assert_eq!(parsed.turns.len(), 1);
        assert_eq!(parsed.turns[0].final_response.as_deref(), Some("done"));
        assert!(parsed.turns[0].completed);
        assert_eq!(
            parsed.turns[0]
                .activities
                .iter()
                .filter(|activity| activity.final_response)
                .count(),
            1
        );
    }

    #[test]
    fn discovery_excludes_non_main_rollouts() {
        let root = tempfile::tempdir().unwrap();
        let day = root.path().join("sessions/2026/01/01");
        fs::create_dir_all(&day).unwrap();
        fs::write(
            day.join("rollout-main.jsonl"),
            r#"{"type":"session_meta","payload":{"session_id":"main","agent_role":"main"}}"#,
        )
        .unwrap();
        fs::write(
            day.join("rollout-child.jsonl"),
            r#"{"type":"session_meta","payload":{"session_id":"child","agent_role":"subagent"}}"#,
        )
        .unwrap();
        fs::write(
            day.join("rollout-source-child.jsonl"),
            r#"{"type":"session_meta","payload":{"session_id":"source-child","source":{"subagent":{"thread_spawn":{"parent_thread_id":"main"}}}}}"#,
        )
        .unwrap();
        let found = discover_sources(root.path()).unwrap();
        assert_eq!(found.paths.len(), 1);
        assert!(found.paths[0].ends_with("rollout-main.jsonl"));
    }

    #[cfg(unix)]
    #[test]
    fn session_index_symlink_is_ignored_for_titles() {
        let root = tempfile::tempdir().unwrap();
        let day = root.path().join("sessions/2026/01/01");
        fs::create_dir_all(&day).unwrap();
        let source = day.join("rollout.jsonl");
        fs::write(&source, b"{}\n").unwrap();
        let external = root.path().join("external-index.jsonl");
        fs::write(
            &external,
            "{\"session_id\":\"native\",\"title\":\"secret\"}\n",
        )
        .unwrap();
        std::os::unix::fs::symlink(&external, root.path().join("session_index.jsonl")).unwrap();
        assert_eq!(read_index_title(&source, "native").unwrap(), None);
    }
}
