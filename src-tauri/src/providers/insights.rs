//! Provider-neutral summaries derived from normalized conversation turns.
//!
//! This module deliberately only consumes the serialized tool input already
//! attached to a `TurnActivity`. It never opens paths mentioned by a tool.

use crate::domain::{ConversationTurn, FileChangeSummary, ToolStat, TurnActivity, TurnInsight};
use serde_json::Value;
use std::collections::HashMap;

/// The status of a Claude tool result, kept separate from provider-neutral
/// statistics so that malformed or missing result events remain visible as
/// `unknown` rather than being guessed as success.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum ToolResultStatus {
    Success,
    Failure,
    Unknown,
}

/// Derive aggregate and per-turn tool/file insights without touching the
/// filesystem. The returned tuple is `(session_tool_stats, turn_insights)`.
pub fn derive_insights(
    turns: &[ConversationTurn],
    cwd: Option<&str>,
    tool_result_status: &HashMap<i64, ToolResultStatus>,
) -> (Vec<ToolStat>, Vec<TurnInsight>) {
    let mut aggregate: Vec<ToolStat> = Vec::new();
    let mut turn_insights = Vec::with_capacity(turns.len());

    for turn in turns {
        let mut turn_stats: Vec<ToolStat> = Vec::new();
        let mut file_changes = Vec::new();

        for activity in &turn.activities {
            if activity.kind != "tool_use" {
                continue;
            }
            let name = normalize_tool_name(activity.tool_name.as_deref().unwrap_or("unknown"));
            let status = tool_result_status
                .get(&activity.event_id)
                .copied()
                .unwrap_or(ToolResultStatus::Unknown);
            let changes = extract_file_changes(activity, turn.id, cwd);

            update_tool_stat(&mut turn_stats, &name, status, &changes);
            update_tool_stat(&mut aggregate, &name, status, &changes);
            file_changes.extend(changes);
        }

        turn_insights.push(TurnInsight {
            turn_id: turn.id,
            file_changes,
            tool_stats: turn_stats,
        });
    }

    (aggregate, turn_insights)
}

/// Keep provider tool labels safe and bounded before they become aggregate
/// keys. This is lexical normalization only; no provider input is inspected.
pub fn normalize_tool_name(raw: &str) -> String {
    let normalized = raw
        .chars()
        .filter(|character| !character.is_control())
        .collect::<String>();
    let trimmed = normalized.trim();
    if trimmed.is_empty() {
        return "unknown".to_owned();
    }
    trimmed.chars().take(80).collect()
}

fn update_tool_stat(
    stats: &mut Vec<ToolStat>,
    name: &str,
    status: ToolResultStatus,
    changes: &[FileChangeSummary],
) {
    let stat = if let Some(stat) = stats.iter_mut().find(|stat| stat.name == name) {
        stat
    } else {
        stats.push(ToolStat {
            name: name.to_owned(),
            ..ToolStat::default()
        });
        stats.last_mut().expect("just inserted tool stat")
    };
    stat.count += 1;
    match status {
        ToolResultStatus::Success => stat.successes += 1,
        ToolResultStatus::Failure => stat.failures += 1,
        ToolResultStatus::Unknown => stat.unknown += 1,
    }
    stat.files_changed += changes.len();
    stat.additions += changes.iter().map(|change| change.additions).sum::<usize>();
    stat.deletions += changes.iter().map(|change| change.deletions).sum::<usize>();
}

fn extract_file_changes(
    activity: &TurnActivity,
    turn_id: i64,
    cwd: Option<&str>,
) -> Vec<FileChangeSummary> {
    let Some(tool_name) = activity.tool_name.as_deref() else {
        return Vec::new();
    };
    let Some(input) = serde_json::from_str::<Value>(&activity.content).ok() else {
        return Vec::new();
    };
    let mut changes = Vec::new();
    match tool_name {
        "Edit" => {
            let Some(path) = input_path(&input, &["file_path", "path"])
                .and_then(|path| normalize_path(&path, cwd))
            else {
                return changes;
            };
            push_change(
                &mut changes,
                FileChangeSummary {
                    path,
                    kind: "modified".to_owned(),
                    additions: line_count(input.get("new_string")),
                    deletions: line_count(input.get("old_string")),
                    turn_id,
                    event_id: activity.event_id,
                    tool_use_id: activity.tool_use_id.clone(),
                },
            );
        }
        "Write" => {
            let Some(path) = input_path(&input, &["file_path", "path"])
                .and_then(|path| normalize_path(&path, cwd))
            else {
                return changes;
            };
            push_change(
                &mut changes,
                FileChangeSummary {
                    path,
                    kind: "written".to_owned(),
                    additions: line_count(input.get("content")),
                    deletions: 0,
                    turn_id,
                    event_id: activity.event_id,
                    tool_use_id: activity.tool_use_id.clone(),
                },
            );
        }
        "MultiEdit" => {
            let Some(edits) = input.get("edits").and_then(Value::as_array) else {
                return changes;
            };
            for edit in edits {
                let path_value = input_path(edit, &["file_path", "path"])
                    .or_else(|| input_path(&input, &["file_path", "path"]));
                let Some(path) = path_value.and_then(|path| normalize_path(&path, cwd)) else {
                    continue;
                };
                push_change(
                    &mut changes,
                    FileChangeSummary {
                        path,
                        kind: "modified".to_owned(),
                        additions: line_count(edit.get("new_string")),
                        deletions: line_count(edit.get("old_string")),
                        turn_id,
                        event_id: activity.event_id,
                        tool_use_id: activity.tool_use_id.clone(),
                    },
                );
            }
        }
        "NotebookEdit" => {
            let Some(path) = input_path(&input, &["notebook_path", "path"])
                .and_then(|path| normalize_path(&path, cwd))
            else {
                return changes;
            };
            push_change(
                &mut changes,
                FileChangeSummary {
                    path,
                    kind: "notebook".to_owned(),
                    additions: line_count(input.get("new_source")),
                    deletions: 0,
                    turn_id,
                    event_id: activity.event_id,
                    tool_use_id: activity.tool_use_id.clone(),
                },
            );
        }
        _ => {}
    }
    changes
}

fn push_change(changes: &mut Vec<FileChangeSummary>, change: FileChangeSummary) {
    if let Some(existing) = changes
        .iter_mut()
        .find(|existing| existing.path == change.path)
    {
        existing.additions += change.additions;
        existing.deletions += change.deletions;
        return;
    }
    changes.push(change);
}

fn input_path(input: &Value, keys: &[&str]) -> Option<String> {
    keys.iter()
        .find_map(|key| input.get(*key).and_then(Value::as_str))
        .map(str::to_owned)
}

fn line_count(value: Option<&Value>) -> usize {
    let Some(text) = value.and_then(Value::as_str) else {
        return 0;
    };
    if text.is_empty() {
        0
    } else {
        text.lines().count()
    }
}

/// Lexically normalizes a path. No filesystem operation is performed.
/// Relative paths are returned relative to the configured workspace. An
/// absolute path outside it is reduced to an opaque safe basename.
pub fn normalize_path(raw: &str, cwd: Option<&str>) -> Option<String> {
    let raw = raw.trim_end_matches(['\r', '\n']);
    if raw.is_empty() || raw.chars().any(char::is_control) {
        return None;
    }
    // Claude can report Windows-looking paths even when parsed on macOS. Keep
    // this lexical and platform-independent; never touch the filesystem.
    let raw = &raw.replace('\\', "/");
    let absolute = raw.starts_with('/') || is_windows_absolute(raw);
    let raw_components = lexical_components(raw, absolute)?;
    if raw_components.is_empty() {
        return None;
    }

    if absolute {
        let cwd_components = cwd.and_then(|cwd| {
            let cwd = cwd.trim_end_matches(['\r', '\n']).replace('\\', "/");
            (cwd.starts_with('/') || is_windows_absolute(&cwd))
                .then(|| lexical_components(&cwd, true))
                .flatten()
        });
        if let Some(cwd_components) = cwd_components {
            if raw_components.starts_with(&cwd_components) {
                let relative = &raw_components[cwd_components.len()..];
                if !relative.is_empty() {
                    return Some(relative.join("/"));
                }
                return Some(".".to_owned());
            }
        }
        return raw_components
            .last()
            .map(|name| format!("outside-workspace/{name}"));
    }

    Some(raw_components.join("/"))
}

fn is_windows_absolute(raw: &str) -> bool {
    let bytes = raw.as_bytes();
    bytes.len() >= 3 && bytes[0].is_ascii_alphabetic() && bytes[1] == b':' && bytes[2] == b'/'
}

fn lexical_components(raw: &str, absolute: bool) -> Option<Vec<String>> {
    let mut components = Vec::new();
    for component in raw.split('/') {
        match component {
            "" | "." => {}
            ".." => {
                if components.pop().is_none() && !absolute {
                    return None;
                }
            }
            value if value.chars().any(char::is_control) => return None,
            value => components.push(value.to_owned()),
        }
    }
    Some(components)
}
