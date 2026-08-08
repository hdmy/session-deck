//! Incremental, read-only tailing for a Claude session transcript.
//!
//! A tail starts at the file's current EOF.  This is deliberate: continuation
//! owns the historical transcript, while the tail only reports bytes appended
//! after continuation was started.

use super::claude::{
    content_text, is_filtered_event, parse_assistant_content, parse_user_content, timestamp,
};
use crate::domain::{AppError, LiveTranscriptEvent, Result, TimelineEvent};
use serde_json::Value;
use std::{
    collections::HashMap,
    fs::File,
    io::{Read, Seek, SeekFrom},
    path::{Path, PathBuf},
};

const MAX_READ_PER_POLL: usize = 256 * 1024;
const MAX_PENDING_LINE: usize = 1024 * 1024;
const READ_CHUNK: usize = 64 * 1024;
const BOUNDARY_FINGERPRINT: u64 = 4 * 1024;

#[derive(Debug, Clone, Default, serde::Serialize, serde::Deserialize)]
pub struct ClaudeLiveSnapshot {
    pub events: Vec<LiveTranscriptEvent>,
    pub partial: bool,
    pub diagnostics: usize,
    /// True when this poll observed the source EOF.  A partial JSONL line is
    /// independent from this value and must not hold the continuation gate.
    pub caught_up: bool,
}

/// Provider-neutral name for callers that do not need to mention Claude.
pub type LiveTranscriptSnapshot = ClaudeLiveSnapshot;

/// Errors raised while preparing a tail for a transcript that will be
/// created by a continuation process.  The collision is kept distinct from
/// ordinary source-read failures so callers cannot accidentally overwrite or
/// attach to an existing transcript.
#[derive(Debug, Clone, Copy, PartialEq, Eq, thiserror::Error)]
pub enum PendingTailError {
    #[error("live transcript source already exists")]
    SourceCollision,
    #[error("live transcript source is not a regular file")]
    SourceNotRegular,
    #[error("live transcript source is unreadable")]
    SourceUnreadable,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
struct FileIdentity {
    #[cfg(unix)]
    device: u64,
    #[cfg(unix)]
    inode: u64,
}

fn file_identity(file: &File) -> Option<FileIdentity> {
    #[cfg(unix)]
    {
        use std::os::unix::fs::MetadataExt;
        let metadata = file.metadata().ok()?;
        Some(FileIdentity {
            device: metadata.dev(),
            inode: metadata.ino(),
        })
    }
    #[cfg(not(unix))]
    {
        let _ = file;
        None
    }
}

fn metadata_identity(metadata: &std::fs::Metadata) -> Option<FileIdentity> {
    #[cfg(unix)]
    {
        use std::os::unix::fs::MetadataExt;
        Some(FileIdentity {
            device: metadata.dev(),
            inode: metadata.ino(),
        })
    }
    #[cfg(not(unix))]
    {
        let _ = metadata;
        None
    }
}

#[derive(Debug)]
pub struct ClaudeLiveTail {
    path: PathBuf,
    session_id: String,
    awaiting_source: bool,
    offset: u64,
    pending_start: u64,
    pending: Vec<u8>,
    discarding_oversized: bool,
    identity: Option<FileIdentity>,
    boundary: Vec<u8>,
    diagnostics: usize,
    caught_up: bool,
    tool_names: HashMap<String, String>,
}

impl ClaudeLiveTail {
    /// Open the source once and capture its current EOF without reading or
    /// modifying any historical bytes.
    pub fn new(path: impl AsRef<Path>, session_id: impl Into<String>) -> Result<Self> {
        let path = path.as_ref().to_path_buf();
        let path_metadata =
            std::fs::symlink_metadata(&path).map_err(|_| AppError::LiveSourceUnreadable)?;
        if path_metadata.file_type().is_symlink() || !path_metadata.file_type().is_file() {
            return Err(AppError::LiveSourceUnreadable);
        }
        let mut file = File::open(&path).map_err(|_| AppError::LiveSourceUnreadable)?;
        if let (Some(expected), Some(actual)) =
            (metadata_identity(&path_metadata), file_identity(&file))
        {
            if expected != actual {
                return Err(AppError::LiveSourceReplaced);
            }
        }
        let offset = file
            .metadata()
            .map_err(|_| AppError::LiveSourceUnreadable)?
            .len();
        let boundary = read_boundary(&mut file, offset)?;
        Ok(Self {
            path,
            session_id: session_id.into(),
            awaiting_source: false,
            offset,
            pending_start: offset,
            pending: Vec::new(),
            discarding_oversized: false,
            identity: file_identity(&file),
            boundary,
            diagnostics: 0,
            caught_up: true,
            tool_names: HashMap::new(),
        })
    }

    /// Prepare a tail for a fork transcript that does not exist yet.
    ///
    /// The path is checked without creating or modifying anything.  Once the
    /// source first appears, the first observed EOF becomes the baseline; this
    /// intentionally avoids replaying the parent's copied history.
    pub fn new_pending(
        path: impl AsRef<Path>,
        session_id: impl Into<String>,
    ) -> std::result::Result<Self, PendingTailError> {
        let path = path.as_ref().to_path_buf();
        match std::fs::symlink_metadata(&path) {
            Ok(metadata) => {
                if metadata.file_type().is_symlink() || !metadata.file_type().is_file() {
                    return Err(PendingTailError::SourceNotRegular);
                }
                return Err(PendingTailError::SourceCollision);
            }
            Err(error) if error.kind() == std::io::ErrorKind::NotFound => {}
            Err(_) => return Err(PendingTailError::SourceUnreadable),
        }
        Ok(Self {
            path,
            session_id: session_id.into(),
            awaiting_source: true,
            offset: 0,
            pending_start: 0,
            pending: Vec::new(),
            discarding_oversized: false,
            identity: None,
            boundary: Vec::new(),
            diagnostics: 0,
            caught_up: true,
            tool_names: HashMap::new(),
        })
    }

    /// Alias for callers that use the shorter constructor name.
    pub fn pending(
        path: impl AsRef<Path>,
        session_id: impl Into<String>,
    ) -> std::result::Result<Self, PendingTailError> {
        Self::new_pending(path, session_id)
    }

    pub fn session_id(&self) -> &str {
        &self.session_id
    }

    /// Return the tail's accumulated status without attempting another read.
    /// Callers use this when a source error prevents `poll` from producing a
    /// new snapshot, so diagnostics and partial input are not reset.
    pub fn current_snapshot(&self) -> ClaudeLiveSnapshot {
        ClaudeLiveSnapshot {
            events: Vec::new(),
            partial: !self.pending.is_empty() || self.discarding_oversized,
            diagnostics: self.diagnostics,
            caught_up: self.caught_up,
        }
    }

    pub fn poll(&mut self) -> Result<ClaudeLiveSnapshot> {
        if self.awaiting_source {
            let metadata = match std::fs::symlink_metadata(&self.path) {
                Ok(metadata) => metadata,
                Err(error) if error.kind() == std::io::ErrorKind::NotFound => {
                    return Ok(self.empty_snapshot())
                }
                Err(_) => return Err(AppError::LiveSourceUnreadable),
            };
            if metadata.file_type().is_symlink() || !metadata.file_type().is_file() {
                return Err(AppError::LiveSourceUnreadable);
            }
            let mut file = match File::open(&self.path) {
                Ok(file) => file,
                Err(_) => return Err(AppError::LiveSourceUnreadable),
            };
            if let (Some(expected), Some(actual)) =
                (metadata_identity(&metadata), file_identity(&file))
            {
                if expected != actual {
                    return Err(AppError::LiveSourceReplaced);
                }
            }
            let metadata = file
                .metadata()
                .map_err(|_| AppError::LiveSourceUnreadable)?;
            let offset = metadata.len();
            let boundary = read_boundary(&mut file, offset)?;
            // Commit the identity and baseline only after all source reads
            // succeed.  A failed first observation remains safely pending.
            self.offset = offset;
            self.pending_start = offset;
            self.identity = file_identity(&file);
            self.boundary = boundary;
            self.awaiting_source = false;
            return Ok(self.empty_snapshot());
        }
        let path_metadata =
            std::fs::symlink_metadata(&self.path).map_err(|_| AppError::LiveSourceUnreadable)?;
        if path_metadata.file_type().is_symlink() || !path_metadata.file_type().is_file() {
            return Err(AppError::LiveSourceReplaced);
        }
        let mut file = File::open(&self.path).map_err(|_| AppError::LiveSourceUnreadable)?;
        let metadata = file
            .metadata()
            .map_err(|_| AppError::LiveSourceUnreadable)?;
        if let (Some(expected), Some(actual)) = (self.identity, file_identity(&file)) {
            if expected != actual {
                return Err(AppError::LiveSourceReplaced);
            }
        }
        if metadata.len() < self.offset {
            return Err(AppError::LiveSourceTruncated);
        }
        if read_boundary(&mut file, self.offset)? != self.boundary {
            // A same-inode rewrite can preserve the old length.  The bounded
            // prefix at the read boundary lets us reject that source without
            // hashing the whole transcript.
            return Err(AppError::LiveSourceReplaced);
        }

        self.caught_up = false;

        file.seek(SeekFrom::Start(self.offset))
            .map_err(|_| AppError::LiveSourceUnreadable)?;
        let mut remaining = (metadata.len() - self.offset) as usize;
        remaining = remaining.min(MAX_READ_PER_POLL);
        let mut events = Vec::new();
        let mut buffer = vec![0_u8; READ_CHUNK];

        while remaining > 0 {
            let read_len = remaining.min(buffer.len());
            let read = file
                .read(&mut buffer[..read_len])
                .map_err(|_| AppError::LiveSourceUnreadable)?;
            if read == 0 {
                break;
            }
            let chunk_start = self.offset;
            self.consume_chunk(&buffer[..read], chunk_start, &mut events);
            self.offset += read as u64;
            remaining -= read;
        }
        self.boundary = read_boundary(&mut file, self.offset)?;
        self.caught_up = self.offset >= metadata.len();

        Ok(ClaudeLiveSnapshot {
            events,
            partial: !self.pending.is_empty() || self.discarding_oversized,
            diagnostics: self.diagnostics,
            caught_up: self.caught_up,
        })
    }

    fn empty_snapshot(&self) -> ClaudeLiveSnapshot {
        ClaudeLiveSnapshot {
            events: Vec::new(),
            partial: !self.pending.is_empty() || self.discarding_oversized,
            diagnostics: self.diagnostics,
            caught_up: self.caught_up,
        }
    }

    fn consume_chunk(
        &mut self,
        bytes: &[u8],
        chunk_start: u64,
        events: &mut Vec<LiveTranscriptEvent>,
    ) {
        let mut index = 0;
        while index < bytes.len() {
            if self.discarding_oversized {
                if let Some(relative) = bytes[index..].iter().position(|byte| *byte == b'\n') {
                    index += relative + 1;
                    self.discarding_oversized = false;
                    self.pending_start = chunk_start + index as u64;
                } else {
                    break;
                }
                continue;
            }

            if self.pending.is_empty() {
                self.pending_start = chunk_start + index as u64;
            }
            let Some(relative_newline) = bytes[index..].iter().position(|byte| *byte == b'\n')
            else {
                let available = bytes.len() - index;
                let room = MAX_PENDING_LINE.saturating_sub(self.pending.len());
                let take = available.min(room);
                self.pending.extend_from_slice(&bytes[index..index + take]);
                index += take;
                if take < available {
                    self.pending.clear();
                    self.discarding_oversized = true;
                    self.diagnostics += 1;
                }
                continue;
            };

            let line_length = self.pending.len() + relative_newline;
            if line_length > MAX_PENDING_LINE {
                self.pending.clear();
                self.diagnostics += 1;
                index += relative_newline + 1;
                self.pending_start = chunk_start + index as u64;
                continue;
            }
            let end = index + relative_newline + 1;
            self.pending.extend_from_slice(&bytes[index..end]);
            index = end;
            self.consume_complete_lines(events);
        }
    }

    fn consume_complete_lines(&mut self, events: &mut Vec<LiveTranscriptEvent>) {
        while let Some(newline) = self.pending.iter().position(|byte| *byte == b'\n') {
            let line_offset = self.pending_start;
            let mut line = self.pending.drain(..=newline).collect::<Vec<_>>();
            self.pending_start += (newline + 1) as u64;
            line.pop();
            if line.last() == Some(&b'\r') {
                line.pop();
            }
            if line.iter().all(u8::is_ascii_whitespace) {
                continue;
            }
            let outcome = parse_line(&line, line_offset, &self.session_id, &mut self.tool_names);
            self.diagnostics += outcome.diagnostics;
            events.extend(outcome.events);
        }
    }
}

fn read_boundary(file: &mut File, offset: u64) -> Result<Vec<u8>> {
    let start = offset.saturating_sub(BOUNDARY_FINGERPRINT);
    let length = (offset - start) as usize;
    if length == 0 {
        return Ok(Vec::new());
    }
    file.seek(SeekFrom::Start(start))
        .map_err(|_| AppError::LiveSourceUnreadable)?;
    let mut boundary = vec![0_u8; length];
    file.read_exact(&mut boundary).map_err(|error| {
        if error.kind() == std::io::ErrorKind::UnexpectedEof {
            AppError::LiveSourceTruncated
        } else {
            AppError::LiveSourceUnreadable
        }
    })?;
    Ok(boundary)
}

struct ParseOutcome {
    events: Vec<LiveTranscriptEvent>,
    diagnostics: usize,
}

fn parse_line(
    line: &[u8],
    line_offset: u64,
    session_id: &str,
    tool_names: &mut HashMap<String, String>,
) -> ParseOutcome {
    let value: Value = match serde_json::from_slice(line) {
        Ok(value) => value,
        Err(_) => {
            return ParseOutcome {
                events: Vec::new(),
                diagnostics: 1,
            }
        }
    };
    if is_filtered_event(&value) {
        return ParseOutcome {
            events: Vec::new(),
            diagnostics: 0,
        };
    }

    let event_type = value.get("type").and_then(Value::as_str).unwrap_or("");
    let message = value.get("message").unwrap_or(&value);
    let role = message.get("role").and_then(Value::as_str);
    let event_timestamp = timestamp(&value);
    let mut normalized = Vec::<TimelineEvent>::new();
    match (event_type, role) {
        ("user", Some("user")) => {
            if let Some(content) = message.get("content") {
                let exclude = ["isCompactSummary", "isVisibleInTranscriptOnly"]
                    .iter()
                    .any(|key| value.get(key).and_then(Value::as_bool).unwrap_or(false));
                parse_user_content(
                    content,
                    exclude,
                    &mut normalized,
                    session_id,
                    event_timestamp,
                    tool_names,
                );
            }
        }
        ("assistant", Some("assistant")) => {
            if let Some(content) = message.get("content") {
                parse_assistant_content(
                    content,
                    &mut normalized,
                    session_id,
                    event_timestamp,
                    tool_names,
                );
            }
        }
        ("tool_result", _) => {
            let content = value
                .get("content")
                .or_else(|| message.get("content"))
                .and_then(content_text)
                .unwrap_or_default();
            let tool_name = value
                .get("name")
                .and_then(Value::as_str)
                .map(str::to_owned)
                .or_else(|| {
                    value
                        .get("tool_use_id")
                        .and_then(Value::as_str)
                        .and_then(|id| tool_names.get(id))
                        .cloned()
                });
            normalized.push(TimelineEvent {
                id: 0,
                session_id: session_id.to_owned(),
                kind: "tool_result".to_owned(),
                role: Some("tool".to_owned()),
                content,
                timestamp: event_timestamp,
                tool_name,
                collapsed: true,
                uuid: None,
                parent_uuid: None,
                logical_parent_uuid: None,
                message_id: None,
                parent_tool_use_id: None,
                tool_use_id: None,
                sequence: 0,
                is_sidechain: false,
                is_meta: false,
                turn_id: None,
                final_response: false,
                compact_boundary: false,
                compact_preserved_ids: Vec::new(),
            });
        }
        _ => {}
    }

    let ignored_event = matches!(
        event_type,
        "" | "agent-name"
            | "ai-title"
            | "attachment"
            | "custom-title"
            | "file-history-delta"
            | "file-history-snapshot"
            | "last-prompt"
            | "mode"
            | "permission-mode"
            | "pr-link"
            | "queue-operation"
            | "system"
    );
    // Live preview intentionally omits system/compact records.  The full
    // parser remains canonical for those events; this is a documented preview
    // contract rather than a second visibility definition.
    let diagnostics = if normalized.is_empty() && !ignored_event {
        1
    } else {
        0
    };
    let events = normalized
        .into_iter()
        .enumerate()
        .map(|(block_index, event)| LiveTranscriptEvent {
            id: format!("{line_offset}:{block_index}"),
            kind: event.kind,
            role: event.role,
            content: event.content,
            timestamp: event.timestamp,
            tool_name: event.tool_name,
            collapsed: event.collapsed,
        })
        .collect();
    ParseOutcome {
        events,
        diagnostics,
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use sha2::{Digest, Sha256};
    use std::{
        fs,
        io::{Seek, SeekFrom, Write},
        time::SystemTime,
    };
    use tempfile::NamedTempFile;

    fn append(file: &mut NamedTempFile, value: &str) {
        file.write_all(value.as_bytes()).unwrap();
        file.flush().unwrap();
    }

    fn source_fingerprint(file: &NamedTempFile) -> (u64, Option<SystemTime>, String) {
        let metadata = file.as_file().metadata().unwrap();
        let bytes = fs::read(file.path()).unwrap();
        let mut hash = Sha256::new();
        hash.update(bytes);
        (
            metadata.len(),
            metadata.modified().ok(),
            format!("{:x}", hash.finalize()),
        )
    }

    #[test]
    fn tails_only_appended_complete_lines_and_stable_ids() {
        let mut file = NamedTempFile::new().unwrap();
        append(
            &mut file,
            "{\"type\":\"user\",\"message\":{\"role\":\"user\",\"content\":\"old\"}}\n",
        );
        let initial_len = file.as_file().metadata().unwrap().len();
        let mut tail = ClaudeLiveTail::new(file.path(), "session").unwrap();
        assert!(tail.poll().unwrap().events.is_empty());
        append(&mut file, "{\"type\":\"assistant\",\"message\":{\"role\":\"assistant\",\"content\":[{\"type\":\"text\",\"text\":\"new\"}]}}\n");
        let snapshot = tail.poll().unwrap();
        assert_eq!(snapshot.events.len(), 1);
        assert_eq!(snapshot.events[0].id, format!("{initial_len}:0"));
        assert_eq!(snapshot.events[0].content, "new");
    }

    #[test]
    fn large_backlog_stays_draining_until_eof_and_delivers_every_line() {
        let mut file = NamedTempFile::new().unwrap();
        let mut tail = ClaudeLiveTail::new(file.path(), "session").unwrap();
        let line = "{\"type\":\"assistant\",\"message\":{\"role\":\"assistant\",\"content\":\"backlog\"}}\n";
        let count = 4_000;
        for _ in 0..count {
            append(&mut file, line);
        }
        let mut delivered = 0;
        let first = tail.poll().unwrap();
        delivered += first.events.len();
        assert!(
            !first.caught_up,
            "the first poll must report a >256 KiB backlog"
        );
        while !tail.current_snapshot().caught_up {
            let snapshot = tail.poll().unwrap();
            delivered += snapshot.events.len();
        }
        assert_eq!(delivered, count);
        assert!(tail.current_snapshot().caught_up);
    }

    #[test]
    fn handles_partial_utf8_and_malformed_lines() {
        let mut file = NamedTempFile::new().unwrap();
        let mut tail = ClaudeLiveTail::new(file.path(), "session").unwrap();
        append(&mut file, "{\"type\":\"assistant\",\"message\":{\"role\":\"assistant\",\"content\":[{\"type\":\"text\",\"text\":\"你");
        let first = tail.poll().unwrap();
        assert!(first.partial);
        append(&mut file, "好\"}]}}\nnot-json\n");
        let second = tail.poll().unwrap();
        assert_eq!(second.events[0].content, "你好");
        assert_eq!(second.diagnostics, 1);
        assert_eq!(tail.poll().unwrap().diagnostics, 1);
    }

    #[test]
    fn current_snapshot_preserves_status_after_source_error() {
        let mut file = NamedTempFile::new().unwrap();
        let mut tail = ClaudeLiveTail::new(file.path(), "session").unwrap();
        append(&mut file, "not-json\n");
        let snapshot = tail.poll().unwrap();
        assert_eq!(snapshot.diagnostics, 1);

        file.as_file_mut().set_len(0).unwrap();
        assert!(matches!(tail.poll(), Err(AppError::LiveSourceTruncated)));
        let current = tail.current_snapshot();
        assert!(!current.partial);
        assert_eq!(current.diagnostics, 1);
        assert!(current.events.is_empty());
    }

    #[test]
    fn discards_oversized_lines_across_polls_and_keeps_following_line() {
        let mut file = NamedTempFile::new().unwrap();
        let mut tail = ClaudeLiveTail::new(file.path(), "session").unwrap();
        append(&mut file, &"x".repeat(900 * 1024));
        let first = tail.poll().unwrap();
        assert!(first.partial);
        assert_eq!(first.diagnostics, 0);
        while tail.pending.len() < 900 * 1024 {
            assert!(tail.poll().unwrap().partial);
        }

        append(
            &mut file,
            &format!(
                "{}\n{{\"type\":\"assistant\",\"message\":{{\"role\":\"assistant\",\"content\":\"kept\"}}}}\n",
                "x".repeat(200 * 1024)
            ),
        );
        let second = tail.poll().unwrap();
        assert_eq!(second.diagnostics, 1);
        assert_eq!(second.events.len(), 1);
        assert_eq!(second.events[0].content, "kept");
        assert!(!second.partial);
        assert!(tail.pending.len() <= MAX_PENDING_LINE);
    }

    #[test]
    fn maps_tool_blocks_and_filters_meta_and_sidechain() {
        let mut file = NamedTempFile::new().unwrap();
        let mut tail = ClaudeLiveTail::new(file.path(), "session").unwrap();
        append(&mut file, "{\"type\":\"assistant\",\"message\":{\"role\":\"assistant\",\"content\":[{\"type\":\"tool_use\",\"id\":\"t1\",\"name\":\"Bash\",\"input\":{\"cmd\":\"ls\"}},{\"type\":\"thinking\",\"thinking\":\"hmm\"}]}}\n");
        append(&mut file, "{\"type\":\"user\",\"isMeta\":true,\"message\":{\"role\":\"user\",\"content\":\"hidden\"}}\n");
        let snapshot = tail.poll().unwrap();
        assert_eq!(snapshot.events.len(), 2);
        assert_eq!(snapshot.events[0].kind, "tool_use");
        assert_eq!(snapshot.events[0].tool_name.as_deref(), Some("Bash"));
        assert!(snapshot.events[0].collapsed);
        assert_eq!(snapshot.events[1].kind, "thinking");
    }

    #[test]
    fn reports_truncation_and_replacement_without_leaking_old_content() {
        let mut file = NamedTempFile::new().unwrap();
        append(&mut file, "seed\n");
        let mut tail = ClaudeLiveTail::new(file.path(), "session").unwrap();
        file.as_file_mut().set_len(0).unwrap();
        assert!(matches!(tail.poll(), Err(AppError::LiveSourceTruncated)));

        let mut replacement = NamedTempFile::new_in(file.path().parent().unwrap()).unwrap();
        append(&mut replacement, "other\n");
        fs::rename(replacement.path(), file.path()).unwrap();
        assert!(matches!(tail.poll(), Err(AppError::LiveSourceReplaced)));
    }

    #[test]
    fn detects_same_inode_rewrite_at_or_above_old_offset() {
        let mut file = NamedTempFile::new().unwrap();
        append(&mut file, &"a".repeat(8 * 1024));
        let mut tail = ClaudeLiveTail::new(file.path(), "session").unwrap();
        file.as_file_mut().seek(SeekFrom::Start(0)).unwrap();
        let rewritten = "b".repeat(8 * 1024);
        file.as_file_mut().write_all(rewritten.as_bytes()).unwrap();
        file.as_file_mut().flush().unwrap();
        assert!(matches!(tail.poll(), Err(AppError::LiveSourceReplaced)));
    }

    #[test]
    fn tail_does_not_modify_source_metadata_or_bytes() {
        let mut file = NamedTempFile::new().unwrap();
        append(&mut file, "old\n");
        let before = source_fingerprint(&file);
        let mut tail = ClaudeLiveTail::new(file.path(), "session").unwrap();
        let _ = tail.poll();
        let after = source_fingerprint(&file);
        assert_eq!(before, after);
    }

    #[test]
    fn pending_tail_waits_for_source_then_baselines_existing_bytes() {
        let directory = tempfile::tempdir().unwrap();
        let path = directory.path().join("fork.jsonl");
        let mut tail = ClaudeLiveTail::new_pending(&path, "session").unwrap();
        assert!(tail.poll().unwrap().events.is_empty());

        fs::write(
            &path,
            "{\"type\":\"assistant\",\"message\":{\"role\":\"assistant\",\"content\":\"copied parent\"}}\n",
        )
        .unwrap();
        // The first observed EOF is the baseline, so copied parent bytes are
        // not replayed by the live tail.
        assert!(tail.poll().unwrap().events.is_empty());

        let mut file = fs::OpenOptions::new().append(true).open(&path).unwrap();
        file.write_all(
            b"{\"type\":\"assistant\",\"message\":{\"role\":\"assistant\",\"content\":\"new fork\"}}\n",
        )
        .unwrap();
        file.flush().unwrap();
        let snapshot = tail.poll().unwrap();
        assert_eq!(snapshot.events.len(), 1);
        assert_eq!(snapshot.events[0].content, "new fork");
    }

    #[test]
    fn pending_tail_reports_preexisting_collision() {
        let file = NamedTempFile::new().unwrap();
        let error = ClaudeLiveTail::new_pending(file.path(), "session").unwrap_err();
        assert_eq!(error, PendingTailError::SourceCollision);
    }

    #[test]
    fn pending_tail_does_not_modify_source_bytes_or_metadata() {
        let directory = tempfile::tempdir().unwrap();
        let path = directory.path().join("fork.jsonl");
        let mut tail = ClaudeLiveTail::new_pending(&path, "session").unwrap();
        assert!(tail.poll().unwrap().events.is_empty());
        fs::write(&path, "seed\n").unwrap();
        let before = {
            let metadata = fs::metadata(&path).unwrap();
            (
                metadata.len(),
                metadata.modified().ok(),
                fs::read(&path).unwrap(),
            )
        };
        // Establish the baseline, then verify polling itself remains read-only.
        assert!(tail.poll().unwrap().events.is_empty());
        let after = {
            let metadata = fs::metadata(&path).unwrap();
            (
                metadata.len(),
                metadata.modified().ok(),
                fs::read(&path).unwrap(),
            )
        };
        assert_eq!(before, after);
    }

    #[cfg(unix)]
    #[test]
    fn pending_source_symlink_and_directory_are_rejected() {
        let directory = tempfile::tempdir().unwrap();
        let path = directory.path().join("fork.jsonl");
        let target = directory.path().join("real.jsonl");
        fs::write(&target, "seed\n").unwrap();
        let mut tail = ClaudeLiveTail::new_pending(&path, "session").unwrap();
        std::os::unix::fs::symlink(&target, &path).unwrap();
        assert!(matches!(tail.poll(), Err(AppError::LiveSourceUnreadable)));
        fs::remove_file(&path).unwrap();
        fs::create_dir(&path).unwrap();
        assert!(matches!(tail.poll(), Err(AppError::LiveSourceUnreadable)));
    }

    #[cfg(unix)]
    #[test]
    fn active_source_replacement_with_symlink_is_rejected() {
        let directory = tempfile::tempdir().unwrap();
        let path = directory.path().join("session.jsonl");
        let target = directory.path().join("target.jsonl");
        fs::write(&path, "seed\n").unwrap();
        fs::write(&target, "other\n").unwrap();
        let mut tail = ClaudeLiveTail::new(&path, "session").unwrap();
        fs::remove_file(&path).unwrap();
        std::os::unix::fs::symlink(&target, &path).unwrap();
        assert!(matches!(tail.poll(), Err(AppError::LiveSourceReplaced)));
    }
}
