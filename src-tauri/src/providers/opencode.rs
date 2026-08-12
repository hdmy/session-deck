//! Read-only OpenCode SQLite adapter.
//!
//! OpenCode stores many sessions in one database.  The current
//! `SessionProvider` contract identifies a session by one source path, so this
//! adapter intentionally does not implement that trait: `OpenCodeStore` keeps
//! the database path and native session id as separate values until the shared
//! source identity contract can represent both.

use crate::domain::{
    AppError, BranchSummary, ConversationTurn, Diagnostic, ParsedBranch, ParsedSession,
    SessionSummary, TimelineEvent, TurnActivity,
};
use crate::scanner::SourceFingerprint;
use rusqlite::{Connection, OpenFlags, Row};
use serde_json::Value;
use sha2::{Digest, Sha256};
#[cfg(unix)]
use std::os::unix::fs::MetadataExt;
use std::{
    fs::{self, File},
    io::{Read, Seek},
    path::{Path, PathBuf},
    time::{SystemTime, UNIX_EPOCH},
};

#[cfg(test)]
use std::cell::Cell;

#[cfg(test)]
thread_local! {
    static HASH_FILE_CALLS: Cell<usize> = const { Cell::new(0) };
}

pub const PROVIDER_ID: &str = "opencode";

#[derive(Debug, Clone)]
pub struct OpenCodeStore {
    database_path: PathBuf,
}

#[derive(Debug)]
pub struct OpenCodeScan {
    pub sessions: Vec<ParsedSession>,
    pub diagnostics: Vec<String>,
    pub complete: bool,
    pub source: SourceFingerprint,
}

#[derive(Debug, Clone, PartialEq, Eq)]
struct FileStamp {
    len: u64,
    modified: Option<SystemTime>,
    #[cfg(unix)]
    dev: u64,
    #[cfg(unix)]
    ino: u64,
}

#[derive(Debug, Clone, PartialEq, Eq)]
struct OpenCodeSourceStamp {
    database: FileStamp,
    wal: Option<FileStamp>,
}

struct OpenCodeSourceState {
    fingerprint: SourceFingerprint,
    stamp: OpenCodeSourceStamp,
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct OpenCodeSession {
    pub native_id: String,
    pub project_id: String,
    pub parent_id: Option<String>,
    pub directory: Option<String>,
    pub title: Option<String>,
    pub workspace_id: Option<String>,
    pub time_created: Option<i64>,
    pub time_updated: Option<i64>,
}

impl OpenCodeStore {
    pub fn open(path: impl AsRef<Path>) -> crate::domain::Result<Self> {
        let path = path.as_ref();
        if !path.is_file() {
            return Err(AppError::InvalidRoot(path.display().to_string()));
        }
        let store = Self {
            database_path: path.to_path_buf(),
        };
        // Open now so callers get a clear read-only/database error before a
        // scan starts.  Every operation opens the same way to avoid sharing a
        // mutable connection across reader calls.
        let _ = store.connection()?;
        Ok(store)
    }

    #[allow(dead_code)]
    pub fn default_path() -> PathBuf {
        let data_home = std::env::var_os("XDG_DATA_HOME")
            .filter(|value| !value.is_empty())
            .map(PathBuf::from);
        let home = directories::BaseDirs::new().map(|dirs| dirs.home_dir().to_path_buf());
        default_path_with_data_home(data_home.as_deref(), home.as_deref())
    }

    #[allow(dead_code)]
    pub fn database_path(&self) -> &Path {
        &self.database_path
    }

    pub fn discover(&self) -> crate::domain::Result<Vec<OpenCodeSession>> {
        let connection = self.connection()?;
        self.discover_with_connection(&connection, None)
    }

    pub fn scan_all(&self) -> crate::domain::Result<OpenCodeScan> {
        self.scan_since(None)
    }

    pub fn scan_since(&self, modified_since: Option<i64>) -> crate::domain::Result<OpenCodeScan> {
        let source = self.source_state()?;
        let mut connection = self.connection()?;
        let tx = connection.transaction()?;
        let sessions = self.discover_with_connection(&tx, modified_since)?;
        let mut parsed = Vec::with_capacity(sessions.len());
        let mut diagnostics = Vec::new();
        let mut complete = true;
        for session in sessions {
            match self.parse_session_with_connection(&tx, &session.native_id, &source.fingerprint) {
                Ok(value) => {
                    diagnostics.extend(value.diagnostics.iter().map(|d| d.code.clone()));
                    parsed.push(value);
                }
                Err(_) => {
                    complete = false;
                    diagnostics.push("session_unreadable".into());
                }
            }
        }
        if source.stamp != source_stamp(&self.database_path)? {
            complete = false;
            diagnostics.push("source_changed_during_scan".into());
        }
        tx.commit()?;
        Ok(OpenCodeScan {
            sessions: parsed,
            diagnostics,
            complete,
            source: source.fingerprint,
        })
    }

    pub fn source_fingerprint(&self) -> crate::domain::Result<SourceFingerprint> {
        self.source_state().map(|source| source.fingerprint)
    }

    fn discover_with_connection(
        &self,
        connection: &Connection,
        modified_since: Option<i64>,
    ) -> crate::domain::Result<Vec<OpenCodeSession>> {
        let workspace = if has_session_column(connection, "workspace_id")? {
            "workspace_id"
        } else {
            "NULL"
        };
        let query = format!(
            "SELECT id, project_id, parent_id, directory, title, time_created, time_updated, {workspace}
             FROM session
             WHERE parent_id IS NULL
               AND (?1 IS NULL OR COALESCE(time_updated, time_created, 0) >= ?1)
             ORDER BY time_updated DESC, time_created DESC, id"
        );
        let mut statement = connection.prepare(&query)?;
        let rows = statement.query_map([modified_since], decode_session)?;
        rows.collect::<rusqlite::Result<Vec<_>>>()
            .map_err(AppError::from)
    }

    pub fn parse_session(&self, native_id: &str) -> crate::domain::Result<ParsedSession> {
        if native_id.trim().is_empty() {
            return Err(AppError::Message("OpenCode session id is empty".into()));
        }
        let source = self.source_state()?;
        let connection = self.connection()?;
        let mut parsed =
            self.parse_session_with_connection(&connection, native_id, &source.fingerprint)?;
        if source.stamp != source_stamp(&self.database_path)? {
            parsed.diagnostics.push(Diagnostic {
                line: 0,
                code: "source_changed_during_scan".into(),
            });
            parsed.summary.partial = true;
        }
        Ok(parsed)
    }

    fn parse_session_with_connection(
        &self,
        connection: &Connection,
        native_id: &str,
        source: &SourceFingerprint,
    ) -> crate::domain::Result<ParsedSession> {
        let workspace = if has_session_column(connection, "workspace_id")? {
            "workspace_id"
        } else {
            "NULL"
        };
        let query = format!(
            "SELECT id, project_id, parent_id, directory, title, time_created, time_updated, {workspace}
             FROM session WHERE id = ?1"
        );
        let session = connection
            .query_row(&query, [native_id], decode_session)
            .map_err(|error| match error {
                rusqlite::Error::QueryReturnedNoRows => {
                    AppError::Message(format!("OpenCode session not found: {native_id}"))
                }
                other => AppError::Database(other),
            })?;

        let mut events: Vec<TimelineEvent> = Vec::new();
        let mut turns: Vec<ConversationTurn> = Vec::new();
        let mut diagnostics = Vec::new();
        let mut models = Vec::new();
        let mut tool_count = 0_i64;
        let mut first_prompt = None;
        let mut last_prompt = None;
        let mut sequence = 0_i64;
        let mut message_statement = connection
            .prepare("SELECT id, data FROM message WHERE session_id = ?1 ORDER BY rowid")?;
        let messages = message_statement.query_map([native_id], |row| {
            Ok((row.get::<_, String>(0)?, row.get::<_, String>(1)?))
        })?;
        for (message_index, message_result) in messages.enumerate() {
            let (message_id, raw_message) = message_result?;
            let message: Value = match serde_json::from_str(&raw_message) {
                Ok(value) => value,
                Err(_) => {
                    diagnostics.push(Diagnostic {
                        line: message_index + 1,
                        code: "malformed_message_json".into(),
                    });
                    sequence += 1;
                    continue;
                }
            };
            let role = match message.get("role").and_then(Value::as_str) {
                Some("user") | Some("assistant") | Some("tool") => {
                    message.get("role").and_then(Value::as_str)
                }
                Some(_) => {
                    diagnostics.push(Diagnostic {
                        line: message_index + 1,
                        code: "unknown_message_role".into(),
                    });
                    None
                }
                None => {
                    diagnostics.push(Diagnostic {
                        line: message_index + 1,
                        code: "message_missing_role".into(),
                    });
                    None
                }
            };
            if let Some(model) = message.get("model").and_then(Value::as_str) {
                if !models.iter().any(|value: &String| value == model) {
                    models.push(model.to_owned());
                }
            }
            let timestamp = message_timestamp(&message);
            let mut part_statement =
                connection.prepare("SELECT data FROM part WHERE message_id = ?1 ORDER BY rowid")?;
            let parts = part_statement.query_map([&message_id], |row| row.get::<_, String>(0))?;
            let mut part_count = 0_usize;
            for part_result in parts {
                let raw_part = part_result?;
                let part: Value = match serde_json::from_str(&raw_part) {
                    Ok(value) => value,
                    Err(_) => {
                        diagnostics.push(Diagnostic {
                            line: message_index + 1,
                            code: "malformed_part_json".into(),
                        });
                        part_count += 1;
                        continue;
                    }
                };
                let part_type = part.get("type").and_then(Value::as_str).unwrap_or("");
                let Some((part_kind, content, tool_name, tool_use_id, collapsed)) =
                    normalize_part(part_type, &part)
                else {
                    diagnostics.push(Diagnostic {
                        line: message_index + 1,
                        code: "unknown_part_type".into(),
                    });
                    part_count += 1;
                    continue;
                };
                let kind = match (role, part_kind) {
                    (Some("user"), "assistant") => "user",
                    (Some("tool"), "assistant") => "tool_result",
                    _ => part_kind,
                };
                if content.trim().is_empty() && kind != "tool_use" {
                    part_count += 1;
                    continue;
                }
                let event_id = events.len() as i64 + 1;
                let final_response = role == Some("assistant") && kind == "assistant";
                if final_response {
                    for previous in events.iter_mut().rev() {
                        if previous.kind == "user" {
                            break;
                        }
                        if previous.kind == "assistant"
                            && previous.role.as_deref() == Some("assistant")
                        {
                            previous.final_response = false;
                            break;
                        }
                    }
                }
                events.push(TimelineEvent {
                    id: event_id,
                    session_id: format!("{PROVIDER_ID}:{native_id}"),
                    kind: kind.into(),
                    role: role.map(str::to_owned),
                    content: content.clone(),
                    timestamp,
                    tool_name: tool_name.clone(),
                    collapsed,
                    uuid: None,
                    parent_uuid: None,
                    logical_parent_uuid: None,
                    message_id: Some(message_id.clone()),
                    parent_tool_use_id: None,
                    tool_use_id: tool_use_id.clone(),
                    sequence,
                    is_sidechain: false,
                    is_meta: false,
                    turn_id: None,
                    final_response,
                    compact_boundary: false,
                    compact_preserved_ids: Vec::new(),
                });
                append_turn(
                    TurnInput {
                        role,
                        kind,
                        content: &content,
                        timestamp,
                        tool_name,
                        tool_use_id,
                        event_id,
                        final_response,
                    },
                    &mut turns,
                    &mut first_prompt,
                    &mut last_prompt,
                );
                if kind == "tool_use" {
                    tool_count += 1;
                }
                part_count += 1;
            }
            if part_count == 0 && role.is_some() {
                diagnostics.push(Diagnostic {
                    line: message_index + 1,
                    code: "message_without_parts".into(),
                });
            }
            sequence += 1;
        }
        drop(message_statement);

        let title = session
            .title
            .clone()
            .filter(|value| !value.trim().is_empty())
            .or_else(|| first_prompt.clone())
            .unwrap_or_else(|| {
                format!(
                    "Untitled session · {}",
                    native_id.chars().take(8).collect::<String>()
                )
            });
        let id = format!("{PROVIDER_ID}:{native_id}");
        let partial = !diagnostics.is_empty();
        for turn in &mut turns {
            turn.session_id = id.clone();
            for activity in &mut turn.activities {
                activity.event_id = activity.event_id.max(1);
            }
        }
        let branch_summary = BranchSummary {
            id: "main".into(),
            session_id: id.clone(),
            label: "main".into(),
            kind: "main".into(),
            root_uuid: None,
            leaf_uuid: None,
            fork_point_uuid: None,
            is_active: true,
            event_count: events.len(),
            turn_count: turns.len(),
            started_at: session.time_created,
            ended_at: session.time_updated,
            compacted: false,
        };
        let summary = SessionSummary {
            id: id.clone(),
            native_session_id: Some(native_id.to_owned()),
            provider_id: PROVIDER_ID.into(),
            project_id: format!("{PROVIDER_ID}:{}", session.project_id),
            workspace_id: stable_workspace(&session),
            project_path: session.directory.clone(),
            worktree_path: None,
            source_title: title.clone(),
            title,
            hidden: false,
            pinned: false,
            last_used_at: session.time_updated,
            started_at: session.time_created,
            ended_at: session.time_updated,
            branch: None,
            first_prompt,
            last_prompt,
            cwd: session.directory,
            models,
            tool_count,
            source_mtime: source.mtime,
            partial,
        };
        let partial = !diagnostics.is_empty();
        let summary = SessionSummary { partial, ..summary };
        Ok(ParsedSession {
            summary,
            events: events.clone(),
            turns: turns.clone(),
            branches: vec![ParsedBranch {
                summary: BranchSummary {
                    event_count: events.len(),
                    turn_count: turns.len(),
                    ..branch_summary
                },
                events,
                turns,
                tool_stats: Vec::new(),
                turn_insights: Vec::new(),
            }],
            diagnostics,
            source_path: self.database_path.clone(),
            source_size: source.size,
            source_hash: source.hash.clone(),
            cwd_history: Vec::new(),
        })
    }

    fn source_state(&self) -> crate::domain::Result<OpenCodeSourceState> {
        let before = source_stamp(&self.database_path)?;
        let database_hash = hash_file(&self.database_path)?;
        let wal_path = sqlite_sidecar_path(&self.database_path, "-wal");
        let wal_hash = before
            .wal
            .as_ref()
            .map(|_| hash_file(&wal_path))
            .transpose()?;
        if before != source_stamp(&self.database_path)? {
            return Err(AppError::Message("source_changed_during_scan".into()));
        }
        let hash = match wal_hash {
            Some(wal_hash) => {
                let mut combined = Sha256::new();
                combined.update(b"opencode-source-v1\0");
                combined.update(database_hash.as_bytes());
                combined.update(b"\0wal\0");
                combined.update(wal_hash.as_bytes());
                format!("{:x}", combined.finalize())
            }
            None => database_hash,
        };
        let size = before
            .database
            .len
            .saturating_add(before.wal.as_ref().map_or(0, |wal| wal.len))
            .min(i64::MAX as u64) as i64;
        let mtime = file_stamp_mtime(&before.database).max(
            before
                .wal
                .as_ref()
                .map(file_stamp_mtime)
                .unwrap_or_default(),
        );
        Ok(OpenCodeSourceState {
            fingerprint: SourceFingerprint {
                path: self.database_path.clone(),
                size,
                mtime,
                hash,
                #[cfg(unix)]
                dev: before.database.dev,
                #[cfg(unix)]
                ino: before.database.ino,
            },
            stamp: before,
        })
    }

    fn connection(&self) -> crate::domain::Result<Connection> {
        Connection::open_with_flags(&self.database_path, OpenFlags::SQLITE_OPEN_READ_ONLY)
            .map_err(AppError::from)
    }
}

fn default_path_with_data_home(data_home: Option<&Path>, home: Option<&Path>) -> PathBuf {
    data_home
        .filter(|path| path.is_absolute())
        .map(|path| path.join("opencode/opencode.db"))
        .or_else(|| home.map(|path| path.join(".local/share/opencode/opencode.db")))
        .unwrap_or_else(|| PathBuf::from(".local/share/opencode/opencode.db"))
}

fn stable_workspace(session: &OpenCodeSession) -> String {
    session
        .directory
        .as_deref()
        .filter(|value| !value.trim().is_empty())
        .map(|value| format!("{PROVIDER_ID}:dir:{}", Path::new(value).display()))
        .or_else(|| {
            session
                .workspace_id
                .as_deref()
                .filter(|value| !value.trim().is_empty())
                .map(|value| format!("{PROVIDER_ID}:workspace:{value}"))
        })
        .unwrap_or_else(|| format!("{PROVIDER_ID}:project:{}", session.project_id))
}

fn decode_session(row: &Row<'_>) -> rusqlite::Result<OpenCodeSession> {
    Ok(OpenCodeSession {
        native_id: row.get(0)?,
        project_id: row.get(1)?,
        parent_id: row.get(2)?,
        directory: row.get(3)?,
        title: row.get(4)?,
        time_created: row.get(5)?,
        time_updated: row.get(6)?,
        workspace_id: row.get(7)?,
    })
}

fn has_session_column(connection: &Connection, name: &str) -> rusqlite::Result<bool> {
    connection.query_row(
        "SELECT EXISTS(SELECT 1 FROM pragma_table_info('session') WHERE name = ?1)",
        [name],
        |row| row.get(0),
    )
}

type NormalizedPart = (&'static str, String, Option<String>, Option<String>, bool);

fn normalize_part(part_type: &str, part: &Value) -> Option<NormalizedPart> {
    match part_type {
        "text" => Some((
            "assistant",
            part.get("text")
                .and_then(Value::as_str)
                .unwrap_or_default()
                .into(),
            None,
            None,
            false,
        )),
        "reasoning" => Some((
            "thinking",
            value_text(part.get("text").or_else(|| part.get("summary"))),
            None,
            None,
            true,
        )),
        "tool" => {
            let state = part.get("state").unwrap_or(part);
            let output = state
                .get("output")
                .or_else(|| state.get("result"))
                .or_else(|| state.get("error"));
            Some((
                if output.is_some() {
                    "tool_result"
                } else {
                    "tool_use"
                },
                output
                    .map(|value| value_text(Some(value)))
                    .unwrap_or_else(|| value_text(state.get("input").or(Some(part)))),
                part.get("tool")
                    .or_else(|| part.get("name"))
                    .and_then(Value::as_str)
                    .map(str::to_owned),
                part.get("callID")
                    .or_else(|| part.get("call_id"))
                    .or_else(|| part.get("id"))
                    .and_then(Value::as_str)
                    .map(str::to_owned),
                true,
            ))
        }
        "file" => Some((
            "file",
            value_text(part.get("text").or_else(|| part.get("path"))),
            None,
            None,
            true,
        )),
        "patch" => Some((
            "patch",
            value_text(part.get("text").or_else(|| part.get("path"))),
            None,
            None,
            true,
        )),
        "step-start" | "step-finish" => {
            Some(("step", value_text(part.get("text")), None, None, true))
        }
        "compaction" => Some((
            "compaction",
            value_text(part.get("text").or_else(|| part.get("summary"))),
            None,
            None,
            true,
        )),
        _ => None,
    }
}

struct TurnInput<'a> {
    role: Option<&'a str>,
    kind: &'a str,
    content: &'a str,
    timestamp: Option<i64>,
    tool_name: Option<String>,
    tool_use_id: Option<String>,
    event_id: i64,
    final_response: bool,
}

fn append_turn(
    input: TurnInput<'_>,
    turns: &mut Vec<ConversationTurn>,
    first_prompt: &mut Option<String>,
    last_prompt: &mut Option<String>,
) {
    let TurnInput {
        role,
        kind,
        content,
        timestamp,
        tool_name,
        tool_use_id,
        event_id,
        final_response,
    } = input;
    if role == Some("user") {
        turns.push(ConversationTurn {
            id: turns.len() as i64 + 1,
            session_id: String::new(),
            user_prompt: Some(content.to_owned()),
            activities: Vec::new(),
            final_response: None,
            timestamp,
            completed: false,
        });
        if first_prompt.is_none() {
            *first_prompt = Some(content.to_owned());
        }
        *last_prompt = Some(content.to_owned());
        return;
    }
    if turns.is_empty() {
        turns.push(ConversationTurn {
            id: 1,
            ..Default::default()
        });
    }
    let turn = turns.last_mut().expect("turn inserted above");
    if final_response {
        turn.final_response = Some(content.to_owned());
        turn.completed = true;
    } else {
        turn.activities.push(TurnActivity {
            event_id,
            kind: kind.into(),
            role: role.map(str::to_owned),
            content: content.to_owned(),
            timestamp,
            tool_name,
            tool_use_id,
            parent_tool_use_id: None,
            collapsed: matches!(
                kind,
                "tool_use" | "tool_result" | "thinking" | "file" | "patch" | "step" | "compaction"
            ),
            final_response: false,
        });
    }
}

fn message_timestamp(message: &Value) -> Option<i64> {
    message
        .get("time")
        .and_then(value_timestamp)
        .or_else(|| message.get("timestamp").and_then(value_timestamp))
}

fn value_timestamp(value: &Value) -> Option<i64> {
    value
        .as_i64()
        .or_else(|| value.get("created").and_then(Value::as_i64))
        .or_else(|| value.get("updated").and_then(Value::as_i64))
        .or_else(|| value.as_str().and_then(|value| value.parse().ok()))
}

fn value_text(value: Option<&Value>) -> String {
    let Some(value) = value else {
        return String::new();
    };
    value
        .as_str()
        .map(str::to_owned)
        .or_else(|| value.get("text").and_then(Value::as_str).map(str::to_owned))
        .unwrap_or_else(|| value.to_string())
}

fn file_stamp_mtime(stamp: &FileStamp) -> i64 {
    stamp
        .modified
        .and_then(|value| value.duration_since(UNIX_EPOCH).ok())
        .map(|value| value.as_millis() as i64)
        .unwrap_or_default()
}

fn sqlite_sidecar_path(path: &Path, suffix: &str) -> PathBuf {
    let mut value = path.as_os_str().to_os_string();
    value.push(suffix);
    value.into()
}

fn file_stamp(path: &Path) -> std::io::Result<FileStamp> {
    let metadata = fs::symlink_metadata(path)?;
    if metadata.file_type().is_symlink() || !metadata.is_file() {
        return Err(std::io::Error::new(
            std::io::ErrorKind::InvalidData,
            "OpenCode source is not a regular file",
        ));
    }
    Ok(FileStamp {
        len: metadata.len(),
        modified: metadata.modified().ok(),
        #[cfg(unix)]
        dev: metadata.dev(),
        #[cfg(unix)]
        ino: metadata.ino(),
    })
}

fn source_stamp(database_path: &Path) -> std::io::Result<OpenCodeSourceStamp> {
    let wal_path = sqlite_sidecar_path(database_path, "-wal");
    let wal = match file_stamp(&wal_path) {
        Ok(stamp) => Some(stamp),
        Err(error) if error.kind() == std::io::ErrorKind::NotFound => None,
        Err(error) => return Err(error),
    };
    Ok(OpenCodeSourceStamp {
        database: file_stamp(database_path)?,
        wal,
    })
}

fn hash_file(path: &Path) -> std::io::Result<String> {
    #[cfg(test)]
    HASH_FILE_CALLS.with(|calls| calls.set(calls.get() + 1));
    let mut file = File::open(path)?;
    file.seek(std::io::SeekFrom::Start(0))?;
    let mut hasher = Sha256::new();
    let mut buffer = [0_u8; 64 * 1024];
    loop {
        let read = file.read(&mut buffer)?;
        if read == 0 {
            break;
        }
        hasher.update(&buffer[..read]);
    }
    Ok(format!("{:x}", hasher.finalize()))
}

#[cfg(test)]
mod tests {
    use super::{default_path_with_data_home, OpenCodeStore, HASH_FILE_CALLS};
    use rusqlite::Connection;
    use std::path::Path;

    #[test]
    fn scan_hashes_the_shared_database_once() {
        let directory = tempfile::tempdir().unwrap();
        let path = directory.path().join("opencode.db");
        let connection = Connection::open(&path).unwrap();
        connection
            .execute_batch(
                "CREATE TABLE session (
                    id TEXT PRIMARY KEY,
                    project_id TEXT NOT NULL,
                    parent_id TEXT,
                    directory TEXT,
                    title TEXT,
                    time_created INTEGER,
                    time_updated INTEGER,
                    workspace_id TEXT
                );
                CREATE TABLE message (id TEXT PRIMARY KEY, session_id TEXT NOT NULL, data TEXT NOT NULL);
                CREATE TABLE part (id TEXT PRIMARY KEY, message_id TEXT NOT NULL, data TEXT NOT NULL);
                INSERT INTO session VALUES ('one', 'project', NULL, '/tmp/project', 'One', 1, 2, NULL);
                INSERT INTO session VALUES ('two', 'project', NULL, '/tmp/project', 'Two', 3, 4, NULL);",
            )
            .unwrap();
        drop(connection);

        HASH_FILE_CALLS.with(|calls| calls.set(0));
        let scan = OpenCodeStore::open(&path).unwrap().scan_all().unwrap();

        assert_eq!(scan.sessions.len(), 2);
        HASH_FILE_CALLS.with(|calls| assert_eq!(calls.get(), 1));
    }

    #[test]
    fn scan_since_parses_only_recent_sessions() {
        let directory = tempfile::tempdir().unwrap();
        let path = directory.path().join("opencode.db");
        let connection = Connection::open(&path).unwrap();
        connection
            .execute_batch(
                "CREATE TABLE session (
                    id TEXT PRIMARY KEY, project_id TEXT NOT NULL, parent_id TEXT,
                    directory TEXT, title TEXT, time_created INTEGER, time_updated INTEGER,
                    workspace_id TEXT
                );
                CREATE TABLE message (id TEXT PRIMARY KEY, session_id TEXT NOT NULL, data TEXT NOT NULL);
                CREATE TABLE part (id TEXT PRIMARY KEY, message_id TEXT NOT NULL, data TEXT NOT NULL);
                INSERT INTO session VALUES
                    ('old', 'project', NULL, '/tmp/project', 'Old', 1, 2, NULL),
                    ('recent', 'project', NULL, '/tmp/project', 'Recent', 3, 4, NULL);",
            )
            .unwrap();
        drop(connection);

        let scan = OpenCodeStore::open(&path)
            .unwrap()
            .scan_since(Some(3))
            .unwrap();

        assert_eq!(scan.sessions.len(), 1);
        assert_eq!(
            scan.sessions[0].summary.native_session_id.as_deref(),
            Some("recent")
        );
    }

    #[test]
    fn source_fingerprint_includes_uncheckpointed_wal_changes() {
        let directory = tempfile::tempdir().unwrap();
        let path = directory.path().join("opencode.db");
        let connection = Connection::open(&path).unwrap();
        connection
            .pragma_update(None, "journal_mode", "WAL")
            .unwrap();
        connection
            .pragma_update(None, "wal_autocheckpoint", 0)
            .unwrap();
        connection
            .execute_batch(
                "CREATE TABLE session (
                    id TEXT PRIMARY KEY,
                    project_id TEXT NOT NULL,
                    parent_id TEXT,
                    directory TEXT,
                    title TEXT,
                    time_created INTEGER,
                    time_updated INTEGER,
                    workspace_id TEXT
                );
                CREATE TABLE message (id TEXT PRIMARY KEY, session_id TEXT NOT NULL, data TEXT NOT NULL);
                CREATE TABLE part (id TEXT PRIMARY KEY, message_id TEXT NOT NULL, data TEXT NOT NULL);
                INSERT INTO session VALUES ('one', 'project', NULL, '/tmp/project', 'One', 1, 2, NULL);",
            )
            .unwrap();
        let store = OpenCodeStore::open(&path).unwrap();
        let before = store.source_fingerprint().unwrap();
        let database_hash = crate::scanner::file_hash(&path).unwrap();

        connection
            .execute(
                "INSERT INTO session VALUES ('two', 'project', NULL, '/tmp/project', 'Two', 3, 4, NULL)",
                [],
            )
            .unwrap();
        let after = store.source_fingerprint().unwrap();

        assert_eq!(database_hash, crate::scanner::file_hash(&path).unwrap());
        assert_ne!(before.hash, after.hash);
    }

    #[test]
    fn relative_xdg_data_home_falls_back_to_home() {
        let path =
            default_path_with_data_home(Some(Path::new("relative")), Some(Path::new("/home/test")));
        assert_eq!(
            path,
            Path::new("/home/test/.local/share/opencode/opencode.db")
        );
    }

    #[test]
    fn absolute_xdg_data_home_is_used() {
        let path =
            default_path_with_data_home(Some(Path::new("/data")), Some(Path::new("/home/test")));
        assert_eq!(path, Path::new("/data/opencode/opencode.db"));
    }
}
