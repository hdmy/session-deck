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
    io::Read,
    path::{Path, PathBuf},
    time::UNIX_EPOCH,
};

pub const PROVIDER_ID: &str = "gemini";

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
            && path
                .file_name()
                .and_then(|v| v.to_str())
                .is_some_and(|v| v.starts_with("session-") && v.ends_with(".json"))
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
    File::open(path)?.read_to_end(&mut bytes)?;
    let value: Value = match serde_json::from_slice(&bytes) {
        Ok(value) => value,
        Err(_) => return Ok(false),
    };
    Ok(value
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
            .is_some())
}

pub fn parse_session(path: &Path) -> std::io::Result<ParsedSession> {
    let initial = fs::metadata(path)?;
    let mut bytes = Vec::new();
    File::open(path)?.read_to_end(&mut bytes)?;
    let source_size = bytes.len() as i64;
    let mut hasher = Sha256::new();
    hasher.update(&bytes);
    let mut diagnostics = Vec::new();
    let value: Value = match serde_json::from_slice(&bytes) {
        Ok(v) => v,
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
    let summary_title = value
        .get("summary")
        .and_then(Value::as_str)
        .filter(|v| !v.trim().is_empty())
        .map(|v| v.trim().to_owned());
    let messages = value
        .get("messages")
        .and_then(Value::as_array)
        .cloned()
        .unwrap_or_default();
    let mut events = Vec::new();
    let mut turns = Vec::new();
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
            let prompt = text(content);
            if prompt.trim().is_empty() {
                continue;
            }
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
    let title = summary_title.or(first_prompt.clone()).unwrap_or_else(|| {
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
        project_id: format!("{PROVIDER_ID}:{project_hash}"),
        workspace_id: format!("{PROVIDER_ID}:{project_hash}"),
        project_path: None,
        worktree_path: None,
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
        cwd: None,
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
fn text_opt(value: &Value) -> Option<String> {
    let raw = text(value);
    (!raw.trim().is_empty()).then_some(raw)
}
fn text(value: &Value) -> String {
    value
        .as_str()
        .map(str::to_owned)
        .or_else(|| value.get("text").and_then(Value::as_str).map(str::to_owned))
        .or_else(|| {
            value
                .get("output")
                .and_then(Value::as_str)
                .map(str::to_owned)
        })
        .unwrap_or_else(|| value.to_string())
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
    use super::{discover_sources, parse_session};
    use std::fs;
    use std::io::Write;
    use tempfile::NamedTempFile;

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
            chats.join("session-main.json"),
            r#"{"sessionId":"main","messages":[]}"#,
        )
        .unwrap();
        fs::write(
            chats.join("session-child.json"),
            r#"{"sessionId":"child","isSubagent":true,"messages":[]}"#,
        )
        .unwrap();
        let found = discover_sources(root.path()).unwrap();
        assert_eq!(found.paths.len(), 1);
        assert!(found.paths[0].ends_with("session-main.json"));
    }
}
