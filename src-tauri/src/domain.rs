use serde::{Deserialize, Serialize};
use std::path::PathBuf;

#[derive(Debug, Clone, Serialize, Deserialize, Default)]
pub struct Project {
    pub id: String,
    #[serde(default)]
    pub workspace_id: String,
    pub name: String,
    pub path: String,
    #[serde(default)]
    pub alias: Option<String>,
    #[serde(default)]
    pub cwd_paths: Vec<String>,
    #[serde(default)]
    pub worktree_paths: Vec<String>,
    pub latest_activity: Option<i64>,
    pub sessions: Vec<SessionSummary>,
}
#[derive(Debug, Clone, Serialize, Deserialize, Default)]
pub struct SessionSummary {
    pub id: String,
    pub native_session_id: Option<String>,
    pub provider_id: String,
    pub project_id: String,
    #[serde(default)]
    pub workspace_id: String,
    #[serde(default)]
    pub project_path: Option<String>,
    #[serde(default)]
    pub worktree_path: Option<String>,
    pub title: String,
    /// Provider-derived title before any local rename is applied.
    pub source_title: String,
    pub hidden: bool,
    pub pinned: bool,
    pub last_used_at: Option<i64>,
    pub started_at: Option<i64>,
    pub ended_at: Option<i64>,
    pub branch: Option<String>,
    pub first_prompt: Option<String>,
    pub last_prompt: Option<String>,
    pub cwd: Option<String>,
    pub models: Vec<String>,
    pub tool_count: i64,
    pub source_mtime: i64,
    pub partial: bool,
}

#[derive(Debug, Clone)]
pub struct ParsedSession {
    pub summary: SessionSummary,
    pub events: Vec<TimelineEvent>,
    pub turns: Vec<ConversationTurn>,
    pub branches: Vec<ParsedBranch>,
    pub diagnostics: Vec<Diagnostic>,
    pub source_path: PathBuf,
    pub source_size: i64,
    pub source_hash: String,
    pub cwd_history: Vec<ObservedCwd>,
}

#[derive(Debug, Clone)]
pub struct ParsedBranch {
    pub summary: BranchSummary,
    pub events: Vec<TimelineEvent>,
    pub turns: Vec<ConversationTurn>,
    pub tool_stats: Vec<ToolStat>,
    pub turn_insights: Vec<TurnInsight>,
}

#[derive(Debug, Clone, Serialize, Deserialize, PartialEq, Eq, Default)]
pub struct FileChangeSummary {
    pub path: String,
    pub kind: String,
    pub additions: usize,
    pub deletions: usize,
    pub turn_id: i64,
    pub event_id: i64,
    pub tool_use_id: Option<String>,
}

#[derive(Debug, Clone, Serialize, Deserialize, PartialEq, Eq, Default)]
pub struct ToolStat {
    pub name: String,
    pub count: usize,
    pub successes: usize,
    pub failures: usize,
    pub unknown: usize,
    pub files_changed: usize,
    pub additions: usize,
    pub deletions: usize,
}

#[derive(Debug, Clone, Serialize, Deserialize, PartialEq, Eq, Default)]
pub struct TurnInsight {
    pub turn_id: i64,
    pub file_changes: Vec<FileChangeSummary>,
    pub tool_stats: Vec<ToolStat>,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct BranchSummary {
    pub id: String,
    pub session_id: String,
    pub label: String,
    pub kind: String,
    pub root_uuid: Option<String>,
    pub leaf_uuid: Option<String>,
    pub fork_point_uuid: Option<String>,
    pub is_active: bool,
    pub event_count: usize,
    pub turn_count: usize,
    pub started_at: Option<i64>,
    pub ended_at: Option<i64>,
    pub compacted: bool,
}
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct TimelineEvent {
    pub id: i64,
    pub session_id: String,
    pub kind: String,
    pub role: Option<String>,
    pub content: String,
    pub timestamp: Option<i64>,
    pub tool_name: Option<String>,
    pub collapsed: bool,
    #[serde(default)]
    pub uuid: Option<String>,
    #[serde(default)]
    pub parent_uuid: Option<String>,
    #[serde(default)]
    pub logical_parent_uuid: Option<String>,
    #[serde(default)]
    pub message_id: Option<String>,
    #[serde(default)]
    pub parent_tool_use_id: Option<String>,
    #[serde(default)]
    pub tool_use_id: Option<String>,
    #[serde(default)]
    pub sequence: i64,
    #[serde(default)]
    pub is_sidechain: bool,
    #[serde(default)]
    pub is_meta: bool,
    #[serde(default)]
    pub turn_id: Option<i64>,
    #[serde(default)]
    pub final_response: bool,
    #[serde(default)]
    pub compact_boundary: bool,
    #[serde(default)]
    pub compact_preserved_ids: Vec<String>,
}

#[derive(Debug, Clone, Serialize, Deserialize, PartialEq, Eq)]
pub struct LiveTranscriptEvent {
    pub id: String,
    pub kind: String,
    pub role: Option<String>,
    pub content: String,
    pub timestamp: Option<i64>,
    pub tool_name: Option<String>,
    pub collapsed: bool,
}

#[derive(Debug, Clone, Serialize, Deserialize, Default)]
pub struct ConversationTurn {
    pub id: i64,
    pub session_id: String,
    pub user_prompt: Option<String>,
    pub activities: Vec<TurnActivity>,
    pub final_response: Option<String>,
    pub timestamp: Option<i64>,
    pub completed: bool,
}

pub type Turn = ConversationTurn;

#[derive(Debug, Clone, Serialize, Deserialize, Default)]
pub struct TurnActivity {
    pub event_id: i64,
    pub kind: String,
    pub role: Option<String>,
    pub content: String,
    pub timestamp: Option<i64>,
    pub tool_name: Option<String>,
    pub tool_use_id: Option<String>,
    pub parent_tool_use_id: Option<String>,
    pub collapsed: bool,
    pub final_response: bool,
}
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct SessionDetail {
    pub summary: SessionSummary,
    pub timeline: Vec<TimelineEvent>,
    pub turns: Vec<ConversationTurn>,
    pub diagnostics: Vec<Diagnostic>,
    #[serde(default)]
    pub branches: Vec<BranchSummary>,
    #[serde(default)]
    pub active_branch_id: Option<String>,
    #[serde(default)]
    pub selected_branch_id: Option<String>,
    #[serde(default)]
    pub tool_stats: Vec<ToolStat>,
    #[serde(default)]
    pub turn_insights: Vec<TurnInsight>,
    #[serde(default)]
    pub relations: Vec<SessionRelation>,
    #[serde(default)]
    pub cwd_history: Vec<ObservedCwd>,
}

#[derive(Debug, Clone, Serialize, Deserialize, PartialEq, Eq, Default)]
pub struct ObservedCwd {
    pub cwd: String,
    pub first_sequence: i64,
    pub last_sequence: i64,
    pub first_timestamp: Option<i64>,
    pub last_timestamp: Option<i64>,
    pub resume: bool,
}

#[derive(Debug, Clone, Serialize, Deserialize, PartialEq, Eq)]
pub struct SessionRelation {
    pub provider_id: String,
    pub parent_session_id: String,
    pub child_session_id: String,
    pub relation_type: String,
    pub created_at: i64,
    pub status: String,
    pub parent_present: bool,
    pub child_present: bool,
}
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct Diagnostic {
    pub line: usize,
    pub code: String,
}
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct SearchHit {
    pub session: SessionSummary,
    pub snippet: String,
    pub event_id: i64,
}
#[derive(Debug, Clone, Serialize, Deserialize, Default)]
pub struct ScanReport {
    pub root: String,
    pub sessions: usize,
    pub diagnostics: usize,
    pub partial: bool,
    pub removed_sessions: usize,
    #[serde(default)]
    pub trigger: String,
    #[serde(default)]
    pub outcome: String,
    #[serde(default)]
    pub committed: bool,
    #[serde(default)]
    pub new_files: usize,
    #[serde(default)]
    pub changed_files: usize,
    #[serde(default)]
    pub unchanged_files: usize,
    #[serde(default)]
    pub removed_files: usize,
    #[serde(default)]
    pub partial_sessions: usize,
}
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct AppStatus {
    pub root: String,
    pub indexed_sessions: usize,
    pub last_scan_at: Option<i64>,
    pub scan_in_progress: bool,
}

#[derive(Debug, thiserror::Error)]
pub enum AppError {
    #[error("io error: {0}")]
    Io(#[from] std::io::Error),
    #[error("database error: {0}")]
    Database(#[from] rusqlite::Error),
    #[error("invalid root: {0}")]
    InvalidRoot(String),
    #[error("serialization error: {0}")]
    Serialization(#[from] serde_json::Error),
    #[error("live transcript source is unreadable")]
    LiveSourceUnreadable,
    #[error("live transcript source was replaced")]
    LiveSourceReplaced,
    #[error("live transcript source was truncated")]
    LiveSourceTruncated,
    #[error("{0}")]
    Message(String),
}
impl serde::Serialize for AppError {
    fn serialize<S: serde::Serializer>(
        &self,
        serializer: S,
    ) -> std::result::Result<S::Ok, S::Error> {
        serializer.serialize_str(&self.to_string())
    }
}
pub type Result<T> = std::result::Result<T, AppError>;
