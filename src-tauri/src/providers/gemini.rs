//! Read-only Gemini CLI session adapter.

use crate::{
    domain::{
        BranchSummary, ConversationTurn, Diagnostic, ParsedBranch, ParsedSession, Result,
        SessionSummary, TimelineEvent, TurnActivity,
    },
    providers::{ProviderCapabilities, SessionProvider, SourceDiscovery},
};
use serde_json::Value;
use sha2::{Digest, Sha256};
use std::{
    fs::{self, File},
    io::{BufRead, BufReader, Read},
    path::{Path, PathBuf},
    time::UNIX_EPOCH,
};

pub const PROVIDER_ID: &str = "gemini";
const MAX_PROJECT_ROOT_BYTES: u64 = 4096;
const MAX_TITLE_CHARS: usize = 96;

#[derive(Debug, Clone, Copy, Default)]
pub struct GeminiProvider;

impl SessionProvider for GeminiProvider {
    fn id(&self) -> &'static str {
        PROVIDER_ID
    }

    fn default_root(&self) -> PathBuf {
        directories::BaseDirs::new()
            .map(|dirs| dirs.home_dir().join(".gemini"))
            .unwrap_or_else(|| PathBuf::from(".gemini"))
    }

    fn discover(&self, root: &Path) -> Result<SourceDiscovery> {
        discover_sources(root)
    }

    fn parse(&self, path: &Path) -> Result<ParsedSession> {
        parse_session(path).map_err(crate::domain::AppError::from)
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
        return Err(crate::domain::AppError::InvalidRoot(
            root.display().to_string(),
        ));
    }
    let chats = root.join("tmp");
    if !chats.exists() || !chats.is_dir() {
        return Err(crate::domain::AppError::InvalidRoot(
            chats.display().to_string(),
        ));
    }
    let mut out = SourceDiscovery {
        complete: true,
        ..Default::default()
    };
    visit(&chats, &mut out)?;
    out.paths.sort();
    Ok(out)
}

fn visit(dir: &Path, out: &mut SourceDiscovery) -> std::io::Result<()> {
    for entry in fs::read_dir(dir)? {
        let entry = match entry {
            Ok(v) => v,
            Err(_) => {
                out.complete = false;
                out.diagnostics.push("entry_unreadable".into());
                continue;
            }
        };
        let kind = match entry.file_type() {
            Ok(v) => v,
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
        let path = entry.path();
        if kind.is_dir() {
            visit(&path, out)?;
        } else if kind.is_file()
            && path.file_name().and_then(|v| v.to_str()).is_some_and(|v| {
                v.starts_with("session-") && (v.ends_with(".json") || v.ends_with(".jsonl"))
            })
            && path
                .parent()
                .and_then(Path::file_name)
                .and_then(|v| v.to_str())
                == Some("chats")
            && !is_subagent_session(&path)?
        {
            out.paths.push(path);
        }
    }
    Ok(())
}

fn is_subagent_session(path: &Path) -> std::io::Result<bool> {
    let mut bytes = Vec::new();
    if path.extension().and_then(|value| value.to_str()) == Some("jsonl") {
        BufReader::new(File::open(path)?).read_until(b'\n', &mut bytes)?;
    } else {
        File::open(path)?.read_to_end(&mut bytes)?;
    }
    let value: Value = match serde_json::from_slice(&bytes) {
        Ok(value) => value,
        Err(_) => return Ok(false),
    };
    Ok(is_subagent_value(&value))
}

fn is_subagent_value(value: &Value) -> bool {
    value.get("kind").and_then(Value::as_str) == Some("subagent")
        || value
            .get("isSubagent")
            .or_else(|| value.get("is_subagent"))
            .and_then(Value::as_bool)
            .unwrap_or(false)
        || value
            .get("agentRole")
            .or_else(|| value.get("agent_role"))
            .and_then(Value::as_str)
            .is_some_and(|role| role != "main")
        || value
            .get("forkedFromId")
            .or_else(|| value.get("forked_from_id"))
            .and_then(Value::as_str)
            .is_some()
}

fn project_path_hash(path: &Path) -> Option<String> {
    path.to_str()
        .map(|value| format!("{:x}", Sha256::digest(value.as_bytes())))
}

fn source_project_root(path: &Path, expected_hash: &str) -> Option<PathBuf> {
    let marker = path.parent()?.parent()?.join(".project_root");
    let metadata = fs::symlink_metadata(&marker).ok()?;
    if metadata.file_type().is_symlink()
        || !metadata.is_file()
        || metadata.len() > MAX_PROJECT_ROOT_BYTES
    {
        return None;
    }
    let root = PathBuf::from(fs::read_to_string(marker).ok()?.trim());
    (root.is_absolute() && project_path_hash(&root).as_deref() == Some(expected_hash))
        .then_some(root)
}

pub(crate) fn resolve_legacy_project_paths(
    sessions: &mut [ParsedSession],
    known_paths: &[PathBuf],
) {
    for session in sessions.iter_mut().filter(|session| {
        session.summary.provider_id == PROVIDER_ID && session.summary.project_path.is_none()
    }) {
        let Some(hash) = session.summary.project_id.strip_prefix("gemini:") else {
            continue;
        };
        let Some(path) = known_paths
            .iter()
            .find(|path| project_path_hash(path).as_deref() == Some(hash))
        else {
            continue;
        };
        let path = path.display().to_string();
        let fallback_workspace = session.summary.workspace_id == session.summary.project_id;
        session.summary.project_id = path.clone();
        if fallback_workspace {
            session.summary.workspace_id = path.clone();
        }
        session.summary.project_path = Some(path.clone());
        session.summary.cwd = Some(path);
    }
}

struct LoadedSession {
    metadata: Value,
    messages: Vec<Value>,
    diagnostics: Vec<Diagnostic>,
}

fn load_session(path: &Path, bytes: &[u8]) -> LoadedSession {
    if path.extension().and_then(|value| value.to_str()) == Some("jsonl") {
        return load_jsonl_session(bytes);
    }

    let mut diagnostics = Vec::new();
    let metadata = match serde_json::from_slice(bytes) {
        Ok(value) => value,
        Err(error) => {
            diagnostics.push(Diagnostic {
                line: 0,
                code: if error.is_eof() {
                    "partial_json"
                } else {
                    "malformed_json"
                }
                .into(),
            });
            Value::Object(Default::default())
        }
    };
    let messages = metadata
        .get("messages")
        .and_then(Value::as_array)
        .cloned()
        .unwrap_or_default();
    LoadedSession {
        metadata,
        messages,
        diagnostics,
    }
}

fn load_jsonl_session(bytes: &[u8]) -> LoadedSession {
    let mut metadata = Value::Object(Default::default());
    let mut messages = Vec::new();
    let mut diagnostics = Vec::new();

    for (line, raw) in bytes.split(|byte| *byte == b'\n').enumerate() {
        if raw.iter().all(|byte| byte.is_ascii_whitespace()) {
            continue;
        }
        let value: Value = match serde_json::from_slice(raw) {
            Ok(value) => value,
            Err(error) => {
                diagnostics.push(Diagnostic {
                    line: line + 1,
                    code: if error.is_eof() {
                        "partial_json"
                    } else {
                        "malformed_json"
                    }
                    .into(),
                });
                continue;
            }
        };
        if let Some(id) = value.get("$rewindTo").and_then(Value::as_str) {
            rewind_messages(&mut messages, id);
        } else if let Some(update) = value.get("$set") {
            apply_metadata_update(&mut metadata, &mut messages, update);
        } else if is_message_record(&value) {
            upsert_message(&mut messages, value);
        } else if is_partial_metadata(&value) {
            if let Some(seed) = value.get("messages").and_then(Value::as_array) {
                for message in seed {
                    upsert_message(&mut messages, message.clone());
                }
            }
            merge_metadata(&mut metadata, &value);
        }
    }

    LoadedSession {
        metadata,
        messages,
        diagnostics,
    }
}

fn is_message_record(value: &Value) -> bool {
    value.get("id").and_then(Value::as_str).is_some()
}

fn is_partial_metadata(value: &Value) -> bool {
    value.get("sessionId").and_then(Value::as_str).is_some()
        && value.get("projectHash").and_then(Value::as_str).is_some()
}

fn merge_metadata(target: &mut Value, source: &Value) {
    let Some(source) = source.as_object() else {
        return;
    };
    let Some(target) = target.as_object_mut() else {
        return;
    };
    for (key, value) in source {
        if key != "messages" {
            target.insert(key.clone(), value.clone());
        }
    }
}

fn apply_metadata_update(target: &mut Value, messages: &mut Vec<Value>, update: &Value) {
    if let Some(seed) = update.get("messages").and_then(Value::as_array) {
        messages.clear();
        for message in seed {
            upsert_message(messages, message.clone());
        }
    }
    merge_metadata(target, update);
}

// ponytail: O(n²) message dedupe keeps this adapter dependency-free; use an
// indexed map if unusually large Gemini sessions make parsing measurable.
fn upsert_message(messages: &mut Vec<Value>, message: Value) {
    let Some(id) = message.get("id").and_then(Value::as_str) else {
        messages.push(message);
        return;
    };
    if let Some(existing) = messages
        .iter_mut()
        .find(|existing| existing.get("id").and_then(Value::as_str) == Some(id))
    {
        *existing = message;
    } else {
        messages.push(message);
    }
}

fn rewind_messages(messages: &mut Vec<Value>, id: &str) {
    if let Some(index) = messages
        .iter()
        .position(|message| message.get("id").and_then(Value::as_str) == Some(id))
    {
        messages.truncate(index);
    } else {
        messages.clear();
    }
}

pub fn parse_session(path: &Path) -> std::io::Result<ParsedSession> {
    let initial = fs::metadata(path)?;
    let mut bytes = Vec::new();
    File::open(path)?.read_to_end(&mut bytes)?;
    let source_size = bytes.len() as i64;
    let mut hasher = Sha256::new();
    hasher.update(&bytes);
    let LoadedSession {
        metadata: value,
        messages,
        mut diagnostics,
    } = load_session(path, &bytes);
    let native_id = value
        .get("sessionId")
        .or_else(|| value.get("session_id"))
        .and_then(Value::as_str)
        .map(str::to_owned)
        .unwrap_or_else(|| {
            path.file_stem()
                .and_then(|v| v.to_str())
                .unwrap_or("unknown")
                .trim_start_matches("session-")
                .to_owned()
        });
    let project_hash = value
        .get("projectHash")
        .and_then(Value::as_str)
        .or_else(|| {
            path.parent()
                .and_then(Path::parent)
                .and_then(Path::file_name)
                .and_then(|v| v.to_str())
        })
        .unwrap_or("unknown")
        .to_owned();
    let project_root = source_project_root(path, &project_hash);
    let identity = project_root
        .as_deref()
        .and_then(|root| crate::project_identity::identify(root).ok());
    let cwd = project_root
        .as_deref()
        .and_then(Path::to_str)
        .map(str::to_owned);
    let project_id = cwd
        .clone()
        .unwrap_or_else(|| format!("{PROVIDER_ID}:{project_hash}"));
    let workspace_id = identity
        .as_ref()
        .map(|value| value.workspace_id.clone())
        .unwrap_or_else(|| project_id.clone());
    let project_path = identity
        .as_ref()
        .map(|value| value.project_path.display().to_string())
        .or_else(|| cwd.clone());
    let worktree_path = identity
        .as_ref()
        .map(|value| value.worktree_path.display().to_string());
    let summary_title = value
        .get("summary")
        .and_then(Value::as_str)
        .and_then(strip_session_context)
        .map(|value| compact_title(&value));
    let mut events = Vec::new();
    let mut turns: Vec<ConversationTurn> = Vec::new();
    let mut first_prompt = None;
    let mut last_prompt = None;
    let mut models = Vec::new();
    let mut tool_count = 0_i64;
    let mut started_at = value.get("startTime").and_then(parse_time);
    let mut ended_at = value.get("lastUpdated").and_then(parse_time);
    for message in messages {
        let timestamp = message
            .get("timestamp")
            .or_else(|| message.get("time"))
            .and_then(parse_time);
        if let Some(ts) = timestamp {
            started_at = Some(started_at.map_or(ts, |v| v.min(ts)));
            ended_at = Some(ended_at.map_or(ts, |v| v.max(ts)));
        }
        let kind = message.get("type").and_then(Value::as_str).unwrap_or("");
        let content = message.get("content").unwrap_or(&Value::Null);
        if kind == "user" {
            if let Some(responses) = function_responses(content) {
                for response in responses {
                    let raw = response
                        .get("response")
                        .map(text)
                        .unwrap_or_else(|| text(&response));
                    let name = response
                        .get("name")
                        .and_then(Value::as_str)
                        .map(str::to_owned);
                    let call_id = response
                        .get("id")
                        .and_then(Value::as_str)
                        .map(str::to_owned);
                    let id = events.len() as i64 + 1;
                    events.push(event(
                        &native_id,
                        id,
                        "tool_result",
                        raw.clone(),
                        EventMetadata {
                            role: Some("tool"),
                            timestamp,
                            tool_name: name.clone(),
                            tool_id: call_id.clone(),
                            final_response: false,
                        },
                    ));
                    if let Some(turn) = turns.last_mut() {
                        turn.activities.push(activity(
                            id,
                            "tool_result",
                            raw,
                            timestamp,
                            name,
                            call_id,
                            true,
                        ));
                    }
                }
                continue;
            }
            let Some(prompt) = strip_session_context(&text(content)) else {
                continue;
            };
            let event_id = events.len() as i64 + 1;
            events.push(event(
                &native_id,
                event_id,
                "user",
                prompt.clone(),
                EventMetadata {
                    role: Some("user"),
                    timestamp,
                    tool_name: None,
                    tool_id: None,
                    final_response: false,
                },
            ));
            if first_prompt.is_none() {
                first_prompt = Some(prompt.clone());
            }
            last_prompt = Some(prompt.clone());
            turns.push(ConversationTurn {
                id: turns.len() as i64 + 1,
                session_id: format!("{PROVIDER_ID}:{native_id}"),
                user_prompt: Some(prompt),
                activities: Vec::new(),
                final_response: None,
                timestamp,
                completed: false,
            });
            continue;
        }
        if matches!(kind, "info" | "error") {
            continue;
        }
        if kind != "gemini" {
            diagnostics.push(Diagnostic {
                line: 0,
                code: "unsupported_event".into(),
            });
            continue;
        }
        let turn = turns.last_mut();
        let mut final_text = None;
        if let Some(raw) = text_opt(content) {
            final_text = Some(raw);
        }
        if let Some(raw) = final_text {
            let id = events.len() as i64 + 1;
            events.push(event(
                &native_id,
                id,
                "assistant",
                raw.clone(),
                EventMetadata {
                    role: Some("assistant"),
                    timestamp,
                    tool_name: None,
                    tool_id: None,
                    final_response: true,
                },
            ));
            if let Some(turn) = turn {
                turn.final_response = Some(raw);
                turn.completed = true;
            }
        }
        for key in ["thoughts", "thinking"] {
            if let Some(thoughts) = message.get(key) {
                for item in values(thoughts) {
                    let raw = text(&item);
                    if raw.trim().is_empty() {
                        continue;
                    }
                    let id = events.len() as i64 + 1;
                    events.push(event(
                        &native_id,
                        id,
                        "thinking",
                        raw.clone(),
                        EventMetadata {
                            role: Some("assistant"),
                            timestamp,
                            tool_name: None,
                            tool_id: None,
                            final_response: false,
                        },
                    ));
                    if let Some(turn) = turns.last_mut() {
                        turn.activities
                            .push(activity(id, "thinking", raw, timestamp, None, None, true));
                    }
                }
            }
        }
        for key in ["toolCalls", "tool_calls", "toolResults", "tool_results"] {
            if let Some(calls) = message.get(key) {
                for item in values(calls) {
                    let is_result = key.to_ascii_lowercase().contains("result");
                    let raw = text(
                        item.get("output")
                            .or_else(|| item.get("result"))
                            .unwrap_or(&item),
                    );
                    let name = item
                        .get("name")
                        .or_else(|| item.get("tool"))
                        .and_then(Value::as_str)
                        .map(str::to_owned);
                    let call_id = item
                        .get("id")
                        .or_else(|| item.get("callId"))
                        .and_then(Value::as_str)
                        .map(str::to_owned);
                    let kind_name = if is_result {
                        "tool_result"
                    } else {
                        tool_count += 1;
                        "tool_use"
                    };
                    let id = events.len() as i64 + 1;
                    events.push(event(
                        &native_id,
                        id,
                        kind_name,
                        raw.clone(),
                        EventMetadata {
                            role: Some("tool"),
                            timestamp,
                            tool_name: name.clone(),
                            tool_id: call_id.clone(),
                            final_response: false,
                        },
                    ));
                    if let Some(turn) = turns.last_mut() {
                        turn.activities
                            .push(activity(id, kind_name, raw, timestamp, name, call_id, true));
                    }
                }
            }
        }
        if let Some(model) = message.get("model").and_then(Value::as_str) {
            if !models.iter().any(|v| v == model) {
                models.push(model.to_owned());
            }
        }
    }
    let final_meta = fs::metadata(path)?;
    if final_meta.len() != initial.len() || final_meta.modified().ok() != initial.modified().ok() {
        diagnostics.push(Diagnostic {
            line: 0,
            code: "source_changed_during_scan".into(),
        });
    }
    let title = summary_title
        .or_else(|| first_prompt.as_deref().map(compact_title))
        .unwrap_or_else(|| {
            format!(
                "Untitled session · {}",
                native_id.chars().take(8).collect::<String>()
            )
        });
    let session_id = format!("{PROVIDER_ID}:{native_id}");
    for (index, event) in events.iter_mut().enumerate() {
        event.id = index as i64 + 1;
        event.session_id = session_id.clone();
    }
    let branch = BranchSummary {
        id: "main".into(),
        session_id: session_id.clone(),
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
        id: session_id.clone(),
        native_session_id: Some(native_id),
        provider_id: PROVIDER_ID.into(),
        project_id,
        workspace_id,
        project_path,
        worktree_path,
        source_title: title.clone(),
        title,
        hidden: false,
        pinned: false,
        last_used_at: None,
        started_at,
        ended_at,
        branch: None,
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
            summary: branch,
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

struct EventMetadata {
    role: Option<&'static str>,
    timestamp: Option<i64>,
    tool_name: Option<String>,
    tool_id: Option<String>,
    final_response: bool,
}

fn event(
    session: &str,
    id: i64,
    kind: &str,
    content: String,
    metadata: EventMetadata,
) -> TimelineEvent {
    TimelineEvent {
        id,
        session_id: format!("{PROVIDER_ID}:{session}"),
        kind: kind.into(),
        role: metadata.role.map(str::to_owned),
        content,
        timestamp: metadata.timestamp,
        tool_name: metadata.tool_name,
        collapsed: matches!(kind, "tool_use" | "tool_result" | "thinking"),
        uuid: None,
        parent_uuid: None,
        logical_parent_uuid: None,
        message_id: None,
        parent_tool_use_id: None,
        tool_use_id: metadata.tool_id,
        sequence: id,
        is_sidechain: false,
        is_meta: false,
        turn_id: None,
        final_response: metadata.final_response,
        compact_boundary: false,
        compact_preserved_ids: Vec::new(),
    }
}
fn activity(
    event_id: i64,
    kind: &str,
    content: String,
    timestamp: Option<i64>,
    tool_name: Option<String>,
    tool_id: Option<String>,
    collapsed: bool,
) -> TurnActivity {
    TurnActivity {
        event_id,
        kind: kind.into(),
        role: Some(if kind.starts_with("tool") {
            "tool".into()
        } else {
            "assistant".into()
        }),
        content,
        timestamp,
        tool_name,
        tool_use_id: tool_id,
        parent_tool_use_id: None,
        collapsed,
        final_response: false,
    }
}
fn values(value: &Value) -> Vec<Value> {
    value
        .as_array()
        .cloned()
        .unwrap_or_else(|| vec![value.clone()])
}

fn function_responses(value: &Value) -> Option<Vec<Value>> {
    let blocks = value.as_array()?;
    let responses = blocks
        .iter()
        .filter_map(|block| block.get("functionResponse").cloned())
        .collect::<Vec<_>>();
    (responses.len() == blocks.len() && !responses.is_empty()).then_some(responses)
}

fn text_opt(value: &Value) -> Option<String> {
    let raw = text(value);
    (!raw.trim().is_empty()).then_some(raw)
}

fn strip_session_context(value: &str) -> Option<String> {
    let value = value.trim();
    let value = if let Some(context) = value.strip_prefix("<session_context>") {
        context
            .split_once("</session_context>")
            .map(|(_, trailing)| trailing.trim())
            .unwrap_or_default()
    } else {
        value
    };
    (!value.is_empty()).then(|| value.to_owned())
}

fn compact_title(value: &str) -> String {
    value
        .split_whitespace()
        .collect::<Vec<_>>()
        .join(" ")
        .chars()
        .take(MAX_TITLE_CHARS)
        .collect()
}

fn text(value: &Value) -> String {
    match value {
        Value::String(value) => value.clone(),
        Value::Array(values) => values
            .iter()
            .map(text)
            .filter(|value| !value.trim().is_empty())
            .collect::<Vec<_>>()
            .join("\n"),
        Value::Object(object) => ["text", "description", "output", "result"]
            .into_iter()
            .find_map(|key| {
                object
                    .get(key)
                    .map(text)
                    .filter(|value| !value.trim().is_empty())
            })
            .unwrap_or_else(|| value.to_string()),
        _ => value.to_string(),
    }
}
fn parse_time(value: &Value) -> Option<i64> {
    value
        .as_i64()
        .or_else(|| value.as_f64().map(|v| v as i64))
        .or_else(|| {
            value
                .as_str()
                .and_then(|v| chrono::DateTime::parse_from_rfc3339(v).ok())
                .map(|v| v.timestamp_millis())
        })
}

#[cfg(test)]
mod tests {
    use super::{discover_sources, parse_session, project_path_hash, resolve_legacy_project_paths};
    use std::fs;
    use std::io::Write;
    use std::path::PathBuf;
    use tempfile::{Builder, NamedTempFile};

    #[test]
    fn summary_and_first_prompt_are_normalized_without_inventing_cwd() {
        let mut file = NamedTempFile::new().unwrap();
        write!(file, r#"{{"sessionId":"g1","projectHash":"hash","summary":"A title","messages":[{{"type":"user","content":"hello"}},{{"type":"gemini","content":"answer","thoughts":["hidden reasoning"],"toolCalls":[{{"name":"search","id":"t1","input":"x"}}]}}]}}"#).unwrap();
        let parsed = parse_session(file.path()).unwrap();
        assert_eq!(parsed.summary.id, "gemini:g1");
        assert_eq!(parsed.summary.title, "A title");
        assert_eq!(parsed.summary.first_prompt.as_deref(), Some("hello"));
        assert!(parsed.summary.cwd.is_none());
        assert!(parsed
            .events
            .iter()
            .any(|event| event.kind == "thinking" && event.collapsed));
        assert!(parsed
            .events
            .iter()
            .any(|event| event.kind == "tool_use" && event.collapsed));
    }

    #[test]
    fn truncated_json_is_partial_not_fatal() {
        let mut file = NamedTempFile::new().unwrap();
        write!(
            file,
            r#"{{"sessionId":"g2","messages":[{{"type":"user","content":"hello"}}]"#
        )
        .unwrap();
        let parsed = parse_session(file.path()).unwrap();
        assert!(parsed
            .diagnostics
            .iter()
            .any(|item| item.code == "partial_json"));
    }

    #[test]
    fn discovery_excludes_explicit_subagent_sessions() {
        let root = tempfile::tempdir().unwrap();
        let chats = root.path().join("tmp/hash/chats");
        fs::create_dir_all(&chats).unwrap();
        fs::write(
            chats.join("session-main.jsonl"),
            r#"{"sessionId":"main","projectHash":"hash","kind":"main"}"#,
        )
        .unwrap();
        fs::write(
            chats.join("session-child.jsonl"),
            r#"{"sessionId":"child","projectHash":"hash","kind":"subagent"}"#,
        )
        .unwrap();
        let found = discover_sources(root.path()).unwrap();
        assert_eq!(found.paths.len(), 1);
        assert!(found.paths[0].ends_with("session-main.jsonl"));
    }

    #[test]
    fn parses_current_jsonl_records_and_function_responses() {
        let mut file = Builder::new().suffix(".jsonl").tempfile().unwrap();
        write!(
            file,
            "{}",
            concat!(
                r#"{"sessionId":"g-jsonl","projectHash":"hash","startTime":"2026-01-01T00:00:00Z","lastUpdated":"2026-01-01T00:04:00Z","kind":"main"}"#,
                "\n",
                r#"{"$set":{"messages":[{"id":"context","type":"user","content":[{"text":"<session_context>synthetic provider context</session_context>"}]},{"id":"u1","type":"user","content":[{"text":"hello"}],"timestamp":"2026-01-01T00:01:00Z"}],"lastUpdated":"2026-01-01T00:01:00Z"}}"#,
                "\n",
                r#"{"id":"g1","type":"gemini","content":"","thoughts":[{"subject":"Inspect","description":"Inspect the project"}],"toolCalls":[{"id":"tool-1","name":"shell","result":{"output":"ok"}}],"timestamp":"2026-01-01T00:02:00Z","model":"gemini-test"}"#,
                "\n",
                r#"{"id":"u2","type":"user","content":[{"functionResponse":{"id":"tool-1","name":"shell","response":{"output":"ok"}}}],"timestamp":"2026-01-01T00:03:00Z"}"#,
                "\n",
                r#"{"id":"g2","type":"gemini","content":"done","timestamp":"2026-01-01T00:04:00Z","model":"gemini-test"}"#,
                "\n",
                r#"{"id":"info-1","type":"info","content":"status","timestamp":"2026-01-01T00:04:00Z"}"#,
                "\n",
                r#"{"id":"error-1","type":"error","content":"provider error","timestamp":"2026-01-01T00:04:00Z"}"#,
                "\n"
            )
        )
        .unwrap();

        let parsed = parse_session(file.path()).unwrap();
        assert_eq!(parsed.summary.id, "gemini:g-jsonl");
        assert_eq!(parsed.summary.first_prompt.as_deref(), Some("hello"));
        assert_eq!(parsed.summary.last_prompt.as_deref(), Some("hello"));
        assert_eq!(parsed.summary.title, "hello");
        assert_eq!(parsed.summary.models, ["gemini-test"]);
        assert_eq!(parsed.summary.tool_count, 1);
        assert!(!parsed.summary.partial);
        assert!(parsed
            .events
            .iter()
            .any(|event| { event.kind == "thinking" && event.content == "Inspect the project" }));
        assert!(parsed
            .events
            .iter()
            .any(|event| { event.kind == "tool_result" && event.content == "ok" }));
        assert!(parsed
            .events
            .iter()
            .any(|event| event.kind == "assistant" && event.content == "done"));
        assert!(parsed
            .events
            .iter()
            .all(|event| !event.content.contains("<session_context>")));
    }

    #[test]
    fn project_root_marker_sets_provider_neutral_identity() {
        let provider = tempfile::tempdir().unwrap();
        let project = tempfile::tempdir().unwrap();
        fs::create_dir(project.path().join(".git")).unwrap();
        let project_store = provider.path().join("tmp/session-deck");
        let chats = project_store.join("chats");
        fs::create_dir_all(&chats).unwrap();
        fs::write(
            project_store.join(".project_root"),
            project.path().display().to_string(),
        )
        .unwrap();
        let hash = project_path_hash(project.path()).unwrap();
        let source = chats.join("session-current.jsonl");
        fs::write(
            &source,
            format!(
                "{{\"sessionId\":\"current\",\"projectHash\":\"{hash}\",\"kind\":\"main\"}}\n{{\"id\":\"u1\",\"type\":\"user\",\"content\":\"hello\"}}\n"
            ),
        )
        .unwrap();

        let parsed = parse_session(&source).unwrap();
        let project = project.path().display().to_string();
        assert_eq!(parsed.summary.project_id, project);
        assert_eq!(
            parsed.summary.project_path.as_deref(),
            Some(project.as_str())
        );
        assert_eq!(parsed.summary.cwd.as_deref(), Some(project.as_str()));
    }

    #[test]
    fn legacy_hash_resolves_against_known_project_paths() {
        let project = PathBuf::from("/workspace/context-vault");
        let hash = project_path_hash(&project).unwrap();
        let mut file = NamedTempFile::new().unwrap();
        write!(
            file,
            "{{\"sessionId\":\"legacy\",\"projectHash\":\"{hash}\",\"messages\":[{{\"type\":\"user\",\"content\":\"hello\"}}]}}"
        )
        .unwrap();
        let mut sessions = vec![parse_session(file.path()).unwrap()];

        resolve_legacy_project_paths(&mut sessions, std::slice::from_ref(&project));

        let project = project.display().to_string();
        assert_eq!(sessions[0].summary.project_id, project);
        assert_eq!(
            sessions[0].summary.project_path.as_deref(),
            Some(project.as_str())
        );
        assert_eq!(sessions[0].summary.cwd.as_deref(), Some(project.as_str()));
    }

    #[test]
    fn malformed_jsonl_tail_keeps_valid_records() {
        let mut file = Builder::new().suffix(".jsonl").tempfile().unwrap();
        write!(
            file,
            "{}",
            concat!(
                r#"{"sessionId":"g-partial","projectHash":"hash","kind":"main"}"#,
                "\n",
                r#"{"id":"u1","type":"user","content":[{"text":"hello"}]}"#,
                "\n",
                r#"{"id":"g1","type":"gemini","content":"answer"}"#,
                "\n",
                "{\"id\":\"g2\",\"type\":\"gemini\",\"content\":\""
            )
        )
        .unwrap();

        let parsed = parse_session(file.path()).unwrap();
        assert_eq!(parsed.summary.first_prompt.as_deref(), Some("hello"));
        assert!(parsed.summary.partial);
        assert!(parsed
            .diagnostics
            .iter()
            .any(|diagnostic| diagnostic.code == "partial_json"));
    }
}
