use super::claude_graph::{
    assemble_turns, compact_preserved_ids, resolve_branches_with_anomalies, LineageNode,
    ResolvedBranch,
};
use crate::{
    domain::{
        AppError, BranchSummary, Diagnostic, ParsedBranch, ParsedSession, Result, SessionSummary,
        TimelineEvent,
    },
    providers::insights::ToolResultStatus,
    providers::{ProviderCapabilities, SessionProvider, SourceDiscovery},
};
use serde_json::Value;
use sha2::{Digest, Sha256};
use std::{
    collections::{HashMap, HashSet},
    fs::{self, File},
    io::{BufRead, BufReader},
    path::{Path, PathBuf},
    time::UNIX_EPOCH,
};

pub const PROVIDER_ID: &str = "claude";

#[derive(Debug, Clone)]
struct ToolResultStatusRecord {
    source_uuid: Option<String>,
    sequence: i64,
    tool_use_id: String,
    status: ToolResultStatus,
}

#[derive(Debug, Clone, Copy, Default)]
pub struct ClaudeProvider;

impl SessionProvider for ClaudeProvider {
    fn id(&self) -> &'static str {
        PROVIDER_ID
    }

    fn default_root(&self) -> PathBuf {
        directories::BaseDirs::new()
            .map(|dirs| dirs.home_dir().join(".claude/projects"))
            .unwrap_or_else(|| PathBuf::from(".claude/projects"))
    }

    fn discover(&self, root: &Path) -> Result<SourceDiscovery> {
        discover_sources(root)
    }

    fn parse(&self, path: &Path) -> Result<ParsedSession> {
        parse_session(path).map_err(AppError::from)
    }

    fn capabilities(&self) -> ProviderCapabilities {
        ProviderCapabilities {
            supports_resume: true,
            supports_changes: true,
            supports_worktree: true,
            supports_branching: true,
        }
    }
}

/// Enumerates only `<root>/<project>/<session>.jsonl`. Claude subagent
/// transcripts are nested below the session and intentionally excluded.
fn discover_sources(root: &Path) -> Result<SourceDiscovery> {
    if !root.exists() {
        return Err(AppError::InvalidRoot(root.display().to_string()));
    }

    let canonical_root = fs::canonicalize(root)?;
    if !canonical_root.is_dir() {
        return Err(AppError::InvalidRoot(root.display().to_string()));
    }

    let mut discovery = SourceDiscovery {
        complete: true,
        ..Default::default()
    };

    for project_entry in fs::read_dir(&canonical_root)? {
        let project_entry = match project_entry {
            Ok(entry) => entry,
            Err(_) => {
                discovery.complete = false;
                discovery
                    .diagnostics
                    .push("project_entry_unreadable".to_owned());
                continue;
            }
        };
        let project_type = match project_entry.file_type() {
            Ok(file_type) => file_type,
            Err(_) => {
                discovery.complete = false;
                discovery
                    .diagnostics
                    .push("project_type_unreadable".to_owned());
                continue;
            }
        };
        if project_type.is_symlink() {
            discovery.complete = false;
            discovery
                .diagnostics
                .push("project_entry_unsafe".to_owned());
            continue;
        }
        if !project_type.is_dir() {
            continue;
        }

        let session_entries = match fs::read_dir(project_entry.path()) {
            Ok(entries) => entries,
            Err(_) => {
                discovery.complete = false;
                discovery
                    .diagnostics
                    .push("project_directory_unreadable".to_owned());
                continue;
            }
        };

        for session_entry in session_entries {
            let session_entry = match session_entry {
                Ok(entry) => entry,
                Err(_) => {
                    discovery.complete = false;
                    discovery
                        .diagnostics
                        .push("session_entry_unreadable".to_owned());
                    continue;
                }
            };
            let session_type = match session_entry.file_type() {
                Ok(file_type) => file_type,
                Err(_) => {
                    discovery.complete = false;
                    discovery
                        .diagnostics
                        .push("session_type_unreadable".to_owned());
                    continue;
                }
            };
            let path = session_entry.path();
            if path.extension().and_then(|value| value.to_str()) == Some("jsonl") {
                if session_type.is_symlink() || !session_type.is_file() {
                    discovery.complete = false;
                    discovery
                        .diagnostics
                        .push("session_entry_unsafe".to_owned());
                } else {
                    discovery.paths.push(path);
                }
            }
        }
    }

    discovery.paths.sort();
    Ok(discovery)
}

pub(crate) fn timestamp(value: &Value) -> Option<i64> {
    value.get("timestamp").and_then(Value::as_i64).or_else(|| {
        value
            .get("timestamp")
            .and_then(Value::as_str)
            .and_then(|raw| chrono::DateTime::parse_from_rfc3339(raw).ok())
            .map(|parsed| parsed.timestamp_millis())
    })
}

pub(crate) fn content_text(value: &Value) -> Option<String> {
    match value {
        Value::String(text) => non_empty(text.clone()),
        Value::Array(blocks) => {
            let joined = blocks
                .iter()
                .filter_map(|block| {
                    block
                        .get("text")
                        .or_else(|| block.get("thinking"))
                        .and_then(Value::as_str)
                })
                .collect::<Vec<_>>()
                .join("\n");
            non_empty(joined)
        }
        Value::Object(object) => object
            .get("text")
            .or_else(|| object.get("thinking"))
            .and_then(Value::as_str)
            .map(str::to_owned)
            .and_then(non_empty),
        _ => None,
    }
}

fn non_empty(value: String) -> Option<String> {
    (!value.trim().is_empty()).then_some(value)
}

fn compact_title(value: &str) -> String {
    visible_text(value)
        .split_whitespace()
        .collect::<Vec<_>>()
        .join(" ")
        .chars()
        .take(96)
        .collect()
}

/// Normalizes Claude's title metadata to the text a person would see rather
/// than exposing control markup such as `<command-message>` in navigation.
fn visible_text(value: &str) -> String {
    let mut text = String::new();
    let mut index = 0;

    while index < value.len() {
        let remainder = &value[index..];
        if let Some(after_open) = remainder.strip_prefix('<') {
            if let Some(tag_end) = after_open.find('>') {
                let tag = &after_open[..tag_end];
                if is_markup_tag(tag) {
                    if tag_introduces_line_break(tag) {
                        text.push(' ');
                    }
                    index += tag_end + 2;
                    continue;
                }
            }
        }
        if let Some(after_ampersand) = remainder.strip_prefix('&') {
            if let Some(entity_end) = after_ampersand.find(';') {
                if let Some(character) = decode_html_entity(&after_ampersand[..entity_end]) {
                    text.push(character);
                    index += entity_end + 2;
                    continue;
                }
            }
        }
        let character = remainder
            .chars()
            .next()
            .expect("remainder is non-empty while scanning title");
        text.push(character);
        index += character.len_utf8();
    }

    text
}

fn is_markup_tag(tag: &str) -> bool {
    let Some(first) = tag.trim_start().chars().next() else {
        return false;
    };
    first.is_ascii_alphabetic() || matches!(first, '/' | '!' | '?')
}

fn tag_introduces_line_break(tag: &str) -> bool {
    let name = tag
        .trim_start()
        .trim_start_matches('/')
        .chars()
        .take_while(|character| character.is_ascii_alphanumeric() || *character == '-')
        .collect::<String>()
        .to_ascii_lowercase();
    matches!(
        name.as_str(),
        "br" | "div"
            | "p"
            | "li"
            | "pre"
            | "blockquote"
            | "hr"
            | "h1"
            | "h2"
            | "h3"
            | "h4"
            | "h5"
            | "h6"
    )
}

fn decode_html_entity(entity: &str) -> Option<char> {
    match entity {
        "amp" => Some('&'),
        "apos" | "#39" => Some('\''),
        "quot" => Some('"'),
        "lt" => Some('<'),
        "gt" => Some('>'),
        "nbsp" => Some(' '),
        _ => {
            let value = if let Some(hex) = entity
                .strip_prefix("#x")
                .or_else(|| entity.strip_prefix("#X"))
            {
                u32::from_str_radix(hex, 16).ok()
            } else if let Some(decimal) = entity.strip_prefix('#') {
                decimal.parse::<u32>().ok()
            } else {
                None
            };
            value.and_then(char::from_u32)
        }
    }
}

fn push_event(
    events: &mut Vec<TimelineEvent>,
    session_id: &str,
    kind: &str,
    role: Option<&str>,
    content: String,
    timestamp: Option<i64>,
    tool_name: Option<String>,
) {
    events.push(TimelineEvent {
        id: events.len() as i64 + 1,
        session_id: session_id.to_owned(),
        kind: kind.to_owned(),
        role: role.map(str::to_owned),
        content,
        timestamp,
        tool_name,
        collapsed: matches!(kind, "thinking" | "tool_use" | "tool_result"),
        uuid: None,
        parent_uuid: None,
        logical_parent_uuid: None,
        message_id: None,
        parent_tool_use_id: None,
        tool_use_id: None,
        sequence: events.len() as i64,
        is_sidechain: false,
        is_meta: false,
        turn_id: None,
        final_response: false,
        compact_boundary: false,
        compact_preserved_ids: Vec::new(),
    });
}

pub(crate) fn parse_user_content(
    message_content: &Value,
    exclude_human_prompt: bool,
    events: &mut Vec<TimelineEvent>,
    session_id: &str,
    event_timestamp: Option<i64>,
    tool_names: &HashMap<String, String>,
) -> Option<String> {
    match message_content {
        Value::String(text) if !exclude_human_prompt => {
            let prompt = non_empty(text.clone())?;
            push_event(
                events,
                session_id,
                "user",
                Some("user"),
                prompt.clone(),
                event_timestamp,
                None,
            );
            Some(prompt)
        }
        Value::Array(blocks) => {
            let contains_tool_result = blocks
                .iter()
                .any(|block| block.get("type").and_then(Value::as_str) == Some("tool_result"));

            for block in blocks {
                if block.get("type").and_then(Value::as_str) == Some("tool_result") {
                    let result = block
                        .get("content")
                        .and_then(content_text)
                        .unwrap_or_default();
                    push_event(
                        events,
                        session_id,
                        "tool_result",
                        Some("tool"),
                        result,
                        event_timestamp,
                        block
                            .get("name")
                            .and_then(Value::as_str)
                            .map(str::to_owned)
                            .or_else(|| {
                                block
                                    .get("tool_use_id")
                                    .and_then(Value::as_str)
                                    .and_then(|id| tool_names.get(id))
                                    .cloned()
                            }),
                    );
                }
            }

            if exclude_human_prompt || contains_tool_result {
                return None;
            }
            let prompt = content_text(message_content)?;
            push_event(
                events,
                session_id,
                "user",
                Some("user"),
                prompt.clone(),
                event_timestamp,
                None,
            );
            Some(prompt)
        }
        _ => None,
    }
}

pub(crate) fn parse_assistant_content(
    message_content: &Value,
    events: &mut Vec<TimelineEvent>,
    session_id: &str,
    event_timestamp: Option<i64>,
    tool_names: &mut HashMap<String, String>,
) -> i64 {
    match message_content {
        Value::String(text) => {
            if let Some(content) = non_empty(text.clone()) {
                push_event(
                    events,
                    session_id,
                    "assistant",
                    Some("assistant"),
                    content,
                    event_timestamp,
                    None,
                );
            }
            0
        }
        Value::Array(blocks) => {
            let mut tools = 0;
            for block in blocks {
                match block.get("type").and_then(Value::as_str) {
                    Some("text") => {
                        if let Some(content) = block.get("text").and_then(content_text) {
                            push_event(
                                events,
                                session_id,
                                "assistant",
                                Some("assistant"),
                                content,
                                event_timestamp,
                                None,
                            );
                        }
                    }
                    Some("thinking") | Some("redacted_thinking") => {
                        let content = block
                            .get("thinking")
                            .or_else(|| block.get("text"))
                            .and_then(content_text)
                            .unwrap_or_else(|| "Reasoning content unavailable".to_owned());
                        push_event(
                            events,
                            session_id,
                            "thinking",
                            Some("assistant"),
                            content,
                            event_timestamp,
                            None,
                        );
                    }
                    Some("tool_use") => {
                        tools += 1;
                        let tool_name =
                            block.get("name").and_then(Value::as_str).map(str::to_owned);
                        if let (Some(tool_id), Some(name)) =
                            (block.get("id").and_then(Value::as_str), tool_name.as_ref())
                        {
                            tool_names.insert(tool_id.to_owned(), name.clone());
                        }
                        let input = block
                            .get("input")
                            .and_then(|value| serde_json::to_string_pretty(value).ok())
                            .unwrap_or_default();
                        push_event(
                            events,
                            session_id,
                            "tool_use",
                            Some("assistant"),
                            input,
                            event_timestamp,
                            tool_name,
                        );
                    }
                    _ => {}
                }
            }
            tools
        }
        _ => 0,
    }
}

fn annotate_event_metadata(value: &Value, events: &mut [TimelineEvent], sequence: i64) {
    let message = value.get("message").unwrap_or(value);
    let uuid = event_uuid(value);
    let parent_uuid = event_parent_uuid(value);
    let logical_parent_uuid = event_logical_parent_uuid(value);
    let message_id = message.get("id").and_then(Value::as_str).map(str::to_owned);
    let parent_tool_use_id = value
        .get("parent_tool_use_id")
        .or_else(|| value.get("parentToolUseId"))
        .or_else(|| message.get("parent_tool_use_id"))
        .and_then(Value::as_str)
        .map(str::to_owned);
    let is_sidechain = value
        .get("isSidechain")
        .or_else(|| value.get("is_sidechain"))
        .and_then(Value::as_bool)
        .unwrap_or(false)
        || value.get("agentId").and_then(Value::as_str).is_some()
        || value.get("agent_id").and_then(Value::as_str).is_some()
        || value.get("teamName").and_then(Value::as_str).is_some()
        || value.get("team_name").and_then(Value::as_str).is_some();
    let is_meta = value
        .get("isMeta")
        .or_else(|| value.get("is_meta"))
        .and_then(Value::as_bool)
        .unwrap_or(false);
    let compact_boundary = value.get("subtype").and_then(Value::as_str) == Some("compact_boundary")
        || value
            .get("compactMetadata")
            .is_some_and(|entry| !entry.is_null())
        || value
            .get("compact_metadata")
            .is_some_and(|entry| !entry.is_null());
    let mut tool_result_blocks = message
        .get("content")
        .and_then(Value::as_array)
        .map(|blocks| {
            blocks
                .iter()
                .filter(|block| block.get("type").and_then(Value::as_str) == Some("tool_result"))
                .filter_map(|block| {
                    block
                        .get("tool_use_id")
                        .and_then(Value::as_str)
                        .map(str::to_owned)
                })
                .collect::<Vec<_>>()
        })
        .unwrap_or_default();
    let mut tool_use_blocks = message
        .get("content")
        .and_then(Value::as_array)
        .map(|blocks| {
            blocks
                .iter()
                .filter(|block| block.get("type").and_then(Value::as_str) == Some("tool_use"))
                .filter_map(|block| block.get("id").and_then(Value::as_str).map(str::to_owned))
                .collect::<Vec<_>>()
        })
        .unwrap_or_default();
    for event in events {
        event.uuid = uuid.clone();
        event.parent_uuid = parent_uuid.clone();
        event.logical_parent_uuid = logical_parent_uuid.clone();
        event.message_id = message_id.clone();
        event.parent_tool_use_id = parent_tool_use_id.clone();
        event.sequence = sequence;
        event.is_sidechain = is_sidechain;
        event.is_meta = is_meta;
        event.compact_boundary = compact_boundary;
        event.compact_preserved_ids = compact_preserved_ids(value);
        if event.kind == "tool_use" {
            event.tool_use_id = tool_use_blocks.first().cloned();
            if !tool_use_blocks.is_empty() {
                tool_use_blocks.remove(0);
            }
        } else if event.kind == "tool_result" {
            event.tool_use_id = tool_result_blocks.first().cloned().or_else(|| {
                value
                    .get("tool_use_id")
                    .or_else(|| message.get("tool_use_id"))
                    .and_then(Value::as_str)
                    .map(str::to_owned)
            });
            if !tool_result_blocks.is_empty() {
                tool_result_blocks.remove(0);
            }
        }
    }
}

fn event_uuid(value: &Value) -> Option<String> {
    value
        .get("uuid")
        .or_else(|| value.get("message").and_then(|message| message.get("uuid")))
        .and_then(Value::as_str)
        .map(str::to_owned)
}

fn event_parent_uuid(value: &Value) -> Option<String> {
    value
        .get("parentUuid")
        .or_else(|| value.get("parent_uuid"))
        .or_else(|| {
            value
                .get("message")
                .and_then(|message| message.get("parentUuid"))
        })
        .and_then(Value::as_str)
        .map(str::to_owned)
}

fn event_logical_parent_uuid(value: &Value) -> Option<String> {
    value
        .get("logicalParentUuid")
        .or_else(|| value.get("logical_parent_uuid"))
        .and_then(Value::as_str)
        .map(str::to_owned)
}

/// Events that belong to Claude's internal coordination/meta streams are not
/// user-facing transcript content. Keep this predicate shared by the full
/// parser and the live tail so both views apply the same filtering semantics.
pub(crate) fn is_filtered_event(value: &Value) -> bool {
    value
        .get("isMeta")
        .or_else(|| value.get("is_meta"))
        .and_then(Value::as_bool)
        .unwrap_or(false)
        || value
            .get("isSidechain")
            .or_else(|| value.get("is_sidechain"))
            .and_then(Value::as_bool)
            .unwrap_or(false)
        || value.get("agentId").and_then(Value::as_str).is_some()
        || value.get("agent_id").and_then(Value::as_str).is_some()
        || value.get("teamName").and_then(Value::as_str).is_some()
        || value.get("team_name").and_then(Value::as_str).is_some()
}

pub fn parse_session(path: &Path) -> std::io::Result<ParsedSession> {
    let file = File::open(path)?;
    let initial_metadata = file.metadata()?;
    let mut reader = BufReader::new(file);
    let mut line_buffer = Vec::new();
    let mut hasher = Sha256::new();

    let mut events = Vec::new();
    let mut diagnostics = Vec::new();
    let mut first_prompt = None;
    let mut last_prompt = None;
    let mut last_prompt_metadata = None;
    let mut started_at = None;
    let mut ended_at = None;
    let mut branch = None;
    let mut cwd = None;
    let mut observed_cwds: Vec<crate::domain::ObservedCwd> = Vec::new();
    let mut custom_title = None;
    let mut ai_title = None;
    let mut models = Vec::new();
    let mut tool_count = 0_i64;
    // Kept only for the parser helper contract used by the live tail. Canonical
    // branch insights never consult this map; branch-local names are rebuilt
    // after lineage selection.
    let mut tool_names = HashMap::new();
    let canonical_result_tool_names = HashMap::new();
    let mut tool_result_statuses = Vec::new();
    let filename_session_id = path
        .file_stem()
        .and_then(|value| value.to_str())
        .unwrap_or("unknown")
        .to_owned();
    let filename_is_uuid = uuid::Uuid::parse_str(&filename_session_id).is_ok();
    let mut session_id = filename_session_id.clone();
    let mut session_id_mismatch = false;
    let mut line_number = 0_usize;
    let mut source_size = 0_i64;
    let mut sequence = 0_i64;
    let mut compact_preserved = HashMap::new();
    let mut lineage = HashMap::new();

    loop {
        line_buffer.clear();
        let read = reader.read_until(b'\n', &mut line_buffer)?;
        if read == 0 {
            break;
        }
        line_number += 1;
        source_size += read as i64;
        hasher.update(&line_buffer);

        let terminated = line_buffer.last() == Some(&b'\n');
        if terminated {
            line_buffer.pop();
            if line_buffer.last() == Some(&b'\r') {
                line_buffer.pop();
            }
        }
        if line_buffer.iter().all(u8::is_ascii_whitespace) {
            continue;
        }

        let value: Value = match serde_json::from_slice(&line_buffer) {
            Ok(value) => value,
            Err(_) => {
                diagnostics.push(Diagnostic {
                    line: line_number,
                    code: if terminated {
                        "malformed_json"
                    } else {
                        "partial_json"
                    }
                    .to_owned(),
                });
                continue;
            }
        };

        if let Some(uuid) = event_uuid(&value) {
            lineage.insert(
                uuid,
                LineageNode {
                    parent_uuid: event_parent_uuid(&value),
                    logical_parent_uuid: event_logical_parent_uuid(&value),
                },
            );
        }

        if let Some(value) = value.get("sessionId").and_then(Value::as_str) {
            if filename_is_uuid && value != filename_session_id {
                session_id_mismatch = true;
            }
            session_id = value.to_owned();
        }
        if let Some(observed) = value.get("cwd").and_then(Value::as_str) {
            cwd = Some(observed.to_owned());
            let timestamp = timestamp(&value);
            if let Some(existing) = observed_cwds.iter_mut().find(|entry| entry.cwd == observed) {
                existing.last_sequence = sequence;
                existing.last_timestamp = timestamp;
            } else {
                observed_cwds.push(crate::domain::ObservedCwd {
                    cwd: observed.to_owned(),
                    first_sequence: sequence,
                    last_sequence: sequence,
                    first_timestamp: timestamp,
                    last_timestamp: timestamp,
                    resume: !observed_cwds.is_empty(),
                });
            }
        }
        if let Some(value) = value.get("gitBranch").and_then(Value::as_str) {
            if !value.is_empty() {
                branch = Some(value.to_owned());
            }
        }
        if let Some(value) = value.get("customTitle").and_then(Value::as_str) {
            if let Some(title) = non_empty(compact_title(value)) {
                custom_title = Some(title);
            }
        }
        if let Some(value) = value.get("aiTitle").and_then(Value::as_str) {
            if let Some(title) = non_empty(compact_title(value)) {
                ai_title = Some(title);
            }
        }
        if let Some(value) = value
            .get("lastPrompt")
            .and_then(Value::as_str)
            .and_then(|value| non_empty(value.to_owned()))
        {
            last_prompt_metadata = Some(value);
        }

        let event_timestamp = timestamp(&value);
        if let Some(timestamp) = event_timestamp {
            started_at = Some(started_at.map_or(timestamp, |current: i64| current.min(timestamp)));
            ended_at = Some(ended_at.map_or(timestamp, |current: i64| current.max(timestamp)));
        }

        let event_type = value.get("type").and_then(Value::as_str).unwrap_or("");
        let event_start = events.len();
        let message = value.get("message").unwrap_or(&value);
        let role = message.get("role").and_then(Value::as_str);

        if let Some(model) = message.get("model").and_then(Value::as_str) {
            if !models.iter().any(|known| known == model) {
                models.push(model.to_owned());
            }
        }

        // Keep canonical and live views on the same visibility contract.  In
        // particular, metadata/team records without UUIDs must not leak into
        // the canonical timeline just because branch selection cannot see
        // their lineage.
        if !is_filtered_event(&value) {
            match (event_type, role) {
                ("user", Some("user")) => {
                    if let Some(content) = message.get("content") {
                        collect_tool_result_statuses(
                            content,
                            &mut tool_result_statuses,
                            event_uuid(&value),
                            sequence,
                        );
                        let exclude_human_prompt =
                            ["isMeta", "isCompactSummary", "isVisibleInTranscriptOnly"]
                                .iter()
                                .any(|key| {
                                    value.get(key).and_then(Value::as_bool).unwrap_or(false)
                                });
                        if let Some(prompt) = parse_user_content(
                            content,
                            exclude_human_prompt,
                            &mut events,
                            &session_id,
                            event_timestamp,
                            &canonical_result_tool_names,
                        ) {
                            if first_prompt.is_none() {
                                first_prompt = Some(prompt.clone());
                            }
                            last_prompt = Some(prompt);
                        }
                    }
                }
                ("assistant", Some("assistant")) => {
                    if let Some(content) = message.get("content") {
                        tool_count += parse_assistant_content(
                            content,
                            &mut events,
                            &session_id,
                            event_timestamp,
                            &mut tool_names,
                        );
                    }
                }
                ("tool_result", _) => {
                    collect_tool_result_status(
                        &value,
                        &mut tool_result_statuses,
                        event_uuid(&value),
                        sequence,
                    );
                    let content = value
                        .get("content")
                        .or_else(|| message.get("content"))
                        .and_then(content_text)
                        .unwrap_or_default();
                    let tool_name = value.get("name").and_then(Value::as_str).map(str::to_owned);
                    push_event(
                        &mut events,
                        &session_id,
                        "tool_result",
                        Some("tool"),
                        content,
                        event_timestamp,
                        tool_name,
                    );
                }
                ("system", _) => {
                    if value.get("subtype").and_then(Value::as_str) == Some("compact_boundary") {
                        push_event(
                            &mut events,
                            &session_id,
                            "compact_boundary",
                            Some("system"),
                            String::new(),
                            event_timestamp,
                            None,
                        );
                    } else if let Some(content) = value.get("content").and_then(content_text) {
                        push_event(
                            &mut events,
                            &session_id,
                            "system",
                            Some("system"),
                            content,
                            event_timestamp,
                            None,
                        );
                    }
                }
                (
                    "agent-name"
                    | "ai-title"
                    | "attachment"
                    | "custom-title"
                    | "file-history-delta"
                    | "file-history-snapshot"
                    | "last-prompt"
                    | "mode"
                    | "permission-mode"
                    | "pr-link"
                    | "queue-operation",
                    _,
                ) => {}
                _ => diagnostics.push(Diagnostic {
                    line: line_number,
                    code: "unsupported_event".to_owned(),
                }),
            }
        }
        let preserved_ids = compact_preserved_ids(&value);
        if !preserved_ids.is_empty() {
            if let Some(uuid) = value
                .get("uuid")
                .or_else(|| value.get("message").and_then(|message| message.get("uuid")))
                .and_then(Value::as_str)
            {
                compact_preserved.insert(uuid.to_owned(), preserved_ids);
            }
        }
        annotate_event_metadata(&value, &mut events[event_start..], sequence);
        sequence += 1;
    }

    let final_metadata = std::fs::metadata(path)?;
    if session_id_mismatch {
        diagnostics.push(Diagnostic {
            line: 0,
            code: "source_session_id_mismatch".to_owned(),
        });
    }
    if final_metadata.len() != initial_metadata.len()
        || final_metadata.modified().ok() != initial_metadata.modified().ok()
    {
        diagnostics.push(Diagnostic {
            line: line_number,
            code: "source_changed_during_scan".to_owned(),
        });
    }

    let project_slug = path
        .parent()
        .and_then(Path::file_name)
        .and_then(|value| value.to_str())
        .unwrap_or("unknown");
    let project_id = cwd
        .clone()
        .unwrap_or_else(|| format!("{PROVIDER_ID}:{project_slug}"));
    let identity = cwd
        .as_deref()
        .and_then(|path| crate::project_identity::identify(Path::new(path)).ok());
    let normalized_session_id = format!("{PROVIDER_ID}:{session_id}");
    let (resolved_branches, graph_anomalies) =
        resolve_branches_with_anomalies(&events, &compact_preserved, &lineage);
    for anomaly in graph_anomalies {
        diagnostics.push(Diagnostic {
            // Resolver metadata intentionally carries no raw transcript text
            // or guessed source line; zero denotes a file-level anomaly.
            line: 0,
            code: anomaly.code.to_owned(),
        });
    }
    let active_path = resolved_branches
        .iter()
        .find(|branch| branch.is_active)
        .map(|branch| branch.path_uuids.clone())
        .unwrap_or_default();
    let branches = resolved_branches
        .into_iter()
        .enumerate()
        .map(|(index, branch)| {
            parse_branch(
                &normalized_session_id,
                index,
                branch,
                &active_path,
                cwd.as_deref(),
                &tool_result_statuses,
            )
        })
        .collect::<Vec<_>>();
    let active_index = branches
        .iter()
        .position(|branch| branch.summary.is_active)
        .unwrap_or(0);
    let events = branches[active_index].events.clone();
    let turns = branches[active_index].turns.clone();
    let title = custom_title
        .or(ai_title)
        .or_else(|| first_prompt.as_deref().map(compact_title))
        .unwrap_or_else(|| {
            format!(
                "Untitled session · {}",
                session_id.chars().take(8).collect::<String>()
            )
        });
    let source_mtime = final_metadata
        .modified()
        .ok()
        .and_then(|modified| modified.duration_since(UNIX_EPOCH).ok())
        .map(|duration| duration.as_millis() as i64)
        .unwrap_or_default();

    Ok(ParsedSession {
        summary: SessionSummary {
            id: normalized_session_id,
            native_session_id: Some(session_id),
            provider_id: PROVIDER_ID.to_owned(),
            project_id: project_id.clone(),
            workspace_id: identity
                .as_ref()
                .map(|value| value.workspace_id.clone())
                .unwrap_or_else(|| project_id.clone()),
            project_path: identity
                .as_ref()
                .map(|value| value.project_path.display().to_string()),
            worktree_path: identity
                .as_ref()
                .map(|value| value.worktree_path.display().to_string()),
            source_title: title.clone(),
            title,
            hidden: false,
            pinned: false,
            last_used_at: None,
            started_at,
            ended_at,
            branch,
            first_prompt,
            last_prompt: last_prompt_metadata.or(last_prompt),
            cwd,
            models,
            tool_count,
            source_mtime,
            partial: !diagnostics.is_empty(),
        },
        events,
        turns,
        branches,
        diagnostics,
        source_path: path.to_path_buf(),
        source_size,
        source_hash: format!("{:x}", hasher.finalize()),
        cwd_history: observed_cwds,
    })
}

fn parse_branch(
    session_id: &str,
    index: usize,
    branch: ResolvedBranch,
    active_path: &[String],
    cwd: Option<&str>,
    tool_result_statuses: &[ToolResultStatusRecord],
) -> ParsedBranch {
    let mut events = branch.events;
    let branch_uuids: HashSet<&str> = branch.path_uuids.iter().map(String::as_str).collect();
    // `message.id` is not a lineage key and may collide across branches. Keep
    // a result only when its own source UUID or its parent UUID proves branch
    // membership; UUID-less transcripts retain their legacy single branch.
    if !branch_uuids.is_empty() {
        events.retain(|event| {
            event.kind != "tool_result"
                || event
                    .uuid
                    .as_deref()
                    .is_some_and(|uuid| branch_uuids.contains(uuid))
                || event
                    .parent_uuid
                    .as_deref()
                    .is_some_and(|parent| branch_uuids.contains(parent))
        });
    }
    for (event_index, event) in events.iter_mut().enumerate() {
        event.id = event_index as i64 + 1;
        event.session_id = session_id.to_owned();
    }
    annotate_branch_tool_result_names(&mut events);
    let tool_result_status =
        branch_tool_result_statuses(&events, tool_result_statuses, !branch.path_uuids.is_empty());
    let turns = assemble_turns(session_id, &mut events);
    let (tool_stats, turn_insights) =
        super::insights::derive_insights(&turns, cwd, &tool_result_status);
    let id = branch
        .leaf_uuid
        .as_deref()
        .map(|uuid| format!("leaf:{uuid}"))
        .unwrap_or_else(|| "main".to_owned());
    let label = if branch.is_active {
        "main".to_owned()
    } else {
        // Branch order is deterministic (active first, then leaf UUID), so
        // alternate-N labels remain stable across rescans.
        format!("alternate-{index}")
    };
    let fork_point_uuid = if branch.is_active {
        None
    } else {
        active_path
            .iter()
            .zip(branch.path_uuids.iter())
            .take_while(|(active, alternate)| active == alternate)
            .last()
            .map(|(uuid, _)| uuid.clone())
    };
    let started_at = events.iter().filter_map(|event| event.timestamp).min();
    let ended_at = events.iter().filter_map(|event| event.timestamp).max();
    let compacted = events.iter().any(|event| event.compact_boundary);
    let summary = BranchSummary {
        id,
        session_id: session_id.to_owned(),
        label,
        kind: if branch.is_active {
            "main".to_owned()
        } else {
            "alternate".to_owned()
        },
        root_uuid: branch.root_uuid,
        leaf_uuid: branch.leaf_uuid,
        fork_point_uuid,
        is_active: branch.is_active,
        event_count: events.len(),
        turn_count: turns.len(),
        started_at,
        ended_at,
        compacted,
    };
    ParsedBranch {
        summary,
        events,
        turns,
        tool_stats,
        turn_insights,
    }
}

fn collect_tool_result_statuses(
    content: &Value,
    statuses: &mut Vec<ToolResultStatusRecord>,
    source_uuid: Option<String>,
    sequence: i64,
) {
    if let Value::Array(blocks) = content {
        for block in blocks {
            if block.get("type").and_then(Value::as_str) == Some("tool_result") {
                collect_tool_result_status(block, statuses, source_uuid.clone(), sequence);
            }
        }
    }
}

fn collect_tool_result_status(
    value: &Value,
    statuses: &mut Vec<ToolResultStatusRecord>,
    source_uuid: Option<String>,
    sequence: i64,
) {
    let Some(tool_use_id) = value.get("tool_use_id").and_then(Value::as_str) else {
        return;
    };
    let status = if value.get("is_error").and_then(Value::as_bool) == Some(true)
        || value.get("error").is_some_and(|error| !error.is_null())
    {
        ToolResultStatus::Failure
    } else if value.get("is_error").and_then(Value::as_bool) == Some(false) {
        ToolResultStatus::Success
    } else {
        ToolResultStatus::Unknown
    };
    statuses.push(ToolResultStatusRecord {
        source_uuid,
        sequence,
        tool_use_id: tool_use_id.to_owned(),
        status,
    });
}

fn branch_tool_result_statuses(
    events: &[TimelineEvent],
    records: &[ToolResultStatusRecord],
    branch_is_uuid_lineaged: bool,
) -> HashMap<i64, ToolResultStatus> {
    let mut statuses = HashMap::new();
    for tool_use in events.iter().filter(|event| event.kind == "tool_use") {
        let Some(tool_use_id) = tool_use.tool_use_id.as_deref() else {
            continue;
        };
        let result_events = events
            .iter()
            .filter(|event| {
                event.kind == "tool_result"
                    && event.tool_use_id.as_deref() == Some(tool_use_id)
                    && records.iter().any(|record| {
                        record.tool_use_id == tool_use_id
                            && record.source_uuid == event.uuid
                            && record.sequence == event.sequence
                    })
            })
            .collect::<Vec<_>>();
        let linked = result_events
            .iter()
            .filter(|result| {
                result.parent_uuid.as_deref() == tool_use.uuid.as_deref()
                    || result.parent_tool_use_id.as_deref() == Some(tool_use_id)
            })
            .copied()
            .collect::<Vec<_>>();
        let linked_count = linked.len();
        let candidates = if linked.is_empty() {
            result_events
        } else {
            linked
        };
        let same_id_use_count = events
            .iter()
            .filter(|event| {
                event.kind == "tool_use" && event.tool_use_id.as_deref() == Some(tool_use_id)
            })
            .count();
        let Some(result) = (candidates.len() == 1
            && (linked_count == 1
                || (same_id_use_count == 1
                    && (!branch_is_uuid_lineaged || tool_use.uuid.is_some()))))
        .then(|| candidates[0]) else {
            statuses.insert(tool_use.id, ToolResultStatus::Unknown);
            continue;
        };
        let status = records
            .iter()
            .find(|record| {
                record.tool_use_id == tool_use_id
                    && record.source_uuid == result.uuid
                    && record.sequence == result.sequence
            })
            .map(|record| record.status)
            .unwrap_or(ToolResultStatus::Unknown);
        statuses.insert(tool_use.id, status);
    }
    statuses
}

fn annotate_branch_tool_result_names(events: &mut [TimelineEvent]) {
    let mut names: HashMap<String, HashSet<String>> = HashMap::new();
    for event in events.iter().filter(|event| event.kind == "tool_use") {
        if let (Some(id), Some(name)) = (event.tool_use_id.as_deref(), event.tool_name.as_deref()) {
            names
                .entry(id.to_owned())
                .or_default()
                .insert(name.to_owned());
        }
    }
    for event in events
        .iter_mut()
        .filter(|event| event.kind == "tool_result")
    {
        if event.tool_name.is_some() {
            continue;
        }
        let Some(id) = event.tool_use_id.as_deref() else {
            continue;
        };
        if let Some(known) = names.get(id).filter(|known| known.len() == 1) {
            event.tool_name = known.iter().next().cloned();
        }
    }
}

#[cfg(test)]
mod tests {
    use super::parse_session;
    use std::io::Write;
    use tempfile::NamedTempFile;

    #[test]
    fn canonical_visibility_filters_meta_and_team_records_without_uuid() {
        let mut file = NamedTempFile::new().unwrap();
        writeln!(
            file,
            r#"{{"type":"user","isMeta":true,"message":{{"role":"user","content":"hidden meta"}}}}
{{"type":"assistant","teamName":"red","message":{{"role":"assistant","content":"hidden team"}}}}
{{"type":"user","isSidechain":true,"message":{{"role":"user","content":"hidden sidechain"}}}}
{{"type":"user","message":{{"role":"user","content":"visible"}}}}"#
        )
        .unwrap();

        let parsed = parse_session(file.path()).expect("parse transcript");
        assert_eq!(parsed.events.len(), 1);
        assert_eq!(parsed.events[0].content, "visible");
        assert!(parsed.diagnostics.is_empty());
    }
}
