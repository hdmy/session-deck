use context_vault_lib::{providers::opencode::OpenCodeStore, scanner, storage};
use rusqlite::Connection;
use std::{fs, os::unix::fs::PermissionsExt};
use tempfile::NamedTempFile;

fn synthetic_db() -> NamedTempFile {
    let file = NamedTempFile::new().unwrap();
    let connection = Connection::open(file.path()).unwrap();
    connection
        .execute_batch(
            r#"CREATE TABLE session (
                id TEXT PRIMARY KEY, project_id TEXT NOT NULL, parent_id TEXT,
                directory TEXT, title TEXT, time_created INTEGER, time_updated INTEGER,
                workspace_id TEXT
            );
            CREATE TABLE message (id TEXT PRIMARY KEY, session_id TEXT NOT NULL, data TEXT NOT NULL);
            CREATE TABLE part (id TEXT PRIMARY KEY, message_id TEXT NOT NULL, data TEXT NOT NULL);
            INSERT INTO session VALUES
                ('top', 'project', NULL, '/tmp/project', 'A title', 1000, 2000, 'workspace'),
                ('child', 'project', 'top', '/tmp/project', 'child', 1001, 2001, 'workspace');
            INSERT INTO message VALUES
                ('m1', 'top', '{"role":"user","time":1100}'),
                ('m2', 'top', '{"role":"assistant","time":1200,"model":"model-a"}'),
                ('m3', 'top', '{"role":"assistant","time":1300}');
            INSERT INTO part VALUES
                ('p1', 'm1', '{"type":"text","text":"hello"}'),
                ('p2', 'm2', '{"type":"reasoning","text":"thinking"}'),
                ('p3', 'm2', '{"type":"tool","tool":"shell","callID":"call-1","state":{"input":{"cmd":"pwd"}}}'),
                ('p4', 'm3', '{"type":"text","text":"done"}');"#,
        )
        .unwrap();
    file
}

#[test]
fn discovers_only_top_level_sessions_and_normalizes_turns() {
    let file = synthetic_db();
    let store = OpenCodeStore::open(file.path()).unwrap();
    let sessions = store.discover().unwrap();
    assert_eq!(sessions.len(), 1);
    assert_eq!(sessions[0].native_id, "top");

    let parsed = store.parse_session("top").unwrap();
    assert_eq!(parsed.summary.id, "opencode:top");
    assert_eq!(parsed.summary.cwd.as_deref(), Some("/tmp/project"));
    assert_eq!(parsed.summary.title, "A title");
    assert_eq!(parsed.summary.models, vec!["model-a"]);
    assert_eq!(parsed.summary.tool_count, 1);
    assert_eq!(parsed.turns.len(), 1);
    assert_eq!(parsed.turns[0].user_prompt.as_deref(), Some("hello"));
    assert_eq!(parsed.turns[0].final_response.as_deref(), Some("done"));
    assert!(parsed
        .events
        .iter()
        .any(|event| event.kind == "user" && event.content == "hello"));
    assert!(parsed.events.iter().any(|event| event.kind == "tool_use"));
    assert!(parsed.events.iter().any(|event| event.kind == "thinking"));
}

#[test]
fn malformed_and_unknown_parts_are_diagnostics_without_raw_payloads() {
    let file = synthetic_db();
    let connection = Connection::open(file.path()).unwrap();
    connection
        .execute("INSERT INTO message VALUES ('bad', 'top', '{not-json')", [])
        .unwrap();
    connection
        .execute(
            "INSERT INTO part VALUES ('unknown', 'm1', '{\"type\":\"future\",\"secret\":\"do-not-log\"}')",
            [],
        )
        .unwrap();
    drop(connection);
    let parsed = OpenCodeStore::open(file.path())
        .unwrap()
        .parse_session("top")
        .unwrap();
    let codes = parsed
        .diagnostics
        .iter()
        .map(|diagnostic| diagnostic.code.as_str())
        .collect::<Vec<_>>();
    assert!(codes.contains(&"malformed_message_json"));
    assert!(codes.contains(&"unknown_part_type"));
    assert!(!codes.iter().any(|code| code.contains("do-not-log")));
}

#[test]
fn source_remains_read_only_and_unchanged() {
    let file = synthetic_db();
    let before = fs::read(file.path()).unwrap();
    let mode = fs::metadata(file.path()).unwrap().permissions().mode();
    fs::set_permissions(file.path(), fs::Permissions::from_mode(mode & !0o222)).unwrap();
    let store = OpenCodeStore::open(file.path()).unwrap();
    let _ = store.parse_session("top").unwrap();
    assert_eq!(fs::read(file.path()).unwrap(), before);
}

#[test]
fn scans_multiple_top_level_sessions_from_one_database_and_indexes_each_identity() {
    let file = synthetic_db();
    let connection = Connection::open(file.path()).unwrap();
    connection
        .execute(
            "INSERT INTO session VALUES ('top-2', 'project', NULL, '/tmp/project', 'second', 1002, 2002, 'workspace')",
            [],
        )
        .unwrap();
    drop(connection);

    let scan = OpenCodeStore::open(file.path())
        .unwrap()
        .scan_all()
        .unwrap();
    assert!(scan.complete);
    assert!(scan.diagnostics.is_empty());
    assert_eq!(scan.sessions.len(), 2);
    assert!(scan
        .sessions
        .iter()
        .all(|session| !session.summary.workspace_id.is_empty()));

    let dbdir = tempfile::tempdir().unwrap();
    let mut index = storage::open(&dbdir.path().join("index.db")).unwrap();
    let removed = storage::index(&mut index, "opencode", &scan.sessions).unwrap();
    assert_eq!(removed, 0);
    let count: i64 = index
        .query_row(
            "SELECT COUNT(*) FROM source_files WHERE provider_id='opencode'",
            [],
            |row| row.get(0),
        )
        .unwrap();
    assert_eq!(count, 2);
}

#[test]
fn partial_sessions_commit_and_successful_reconcile_removes_missing_session() {
    let file = synthetic_db();
    let connection = Connection::open(file.path()).unwrap();
    connection
        .execute(
            "INSERT INTO session VALUES ('top-2', 'project', NULL, '/tmp/project', 'second', 1002, 2002, 'workspace')",
            [],
        )
        .unwrap();
    drop(connection);
    let store = OpenCodeStore::open(file.path()).unwrap();
    let dbdir = tempfile::tempdir().unwrap();
    let mut index = storage::open(&dbdir.path().join("index.db")).unwrap();
    let first = store.scan_all().unwrap();
    storage::index(&mut index, "opencode", &first.sessions).unwrap();
    let indexed: i64 = index
        .query_row(
            "SELECT COUNT(*) FROM sessions WHERE provider_id='opencode'",
            [],
            |row| row.get(0),
        )
        .unwrap();
    assert_eq!(indexed, 2);

    let connection = Connection::open(file.path()).unwrap();
    connection
        .execute("INSERT INTO message VALUES ('bad', 'top', '{not-json')", [])
        .unwrap();
    drop(connection);
    let before_partial = fs::read(file.path()).unwrap();
    let before_partial_meta = fs::metadata(file.path()).unwrap();
    let before_partial_hash = scanner::file_hash(file.path()).unwrap();
    let partial = store.scan_all().unwrap();
    assert!(partial.complete);
    assert!(!partial.diagnostics.is_empty());
    assert!(partial
        .sessions
        .iter()
        .any(|session| session.summary.partial));
    assert_eq!(
        storage::index(&mut index, "opencode", &partial.sessions).unwrap(),
        0
    );
    assert_eq!(
        index
            .query_row(
                "SELECT COUNT(*) FROM sessions WHERE provider_id='opencode'",
                [],
                |row| row.get::<_, i64>(0)
            )
            .unwrap(),
        indexed
    );
    let after_partial = fs::metadata(file.path()).unwrap();
    assert_eq!(before_partial, fs::read(file.path()).unwrap());
    assert_eq!(before_partial_meta.len(), after_partial.len());
    assert_eq!(
        before_partial_meta.modified().unwrap(),
        after_partial.modified().unwrap()
    );
    assert_eq!(
        before_partial_hash,
        scanner::file_hash(file.path()).unwrap()
    );

    let connection = Connection::open(file.path()).unwrap();
    connection
        .execute("DELETE FROM message WHERE id='bad'", [])
        .unwrap();
    connection
        .execute("DELETE FROM session WHERE id='top-2'", [])
        .unwrap();
    drop(connection);
    let before_success_meta = fs::metadata(file.path()).unwrap();
    let before_success_hash = scanner::file_hash(file.path()).unwrap();
    let success = store.scan_all().unwrap();
    assert!(success.complete);
    let after_success_meta = fs::metadata(file.path()).unwrap();
    assert_eq!(before_success_meta.len(), after_success_meta.len());
    assert_eq!(
        before_success_meta.modified().unwrap(),
        after_success_meta.modified().unwrap()
    );
    assert_eq!(
        before_success_hash,
        scanner::file_hash(file.path()).unwrap()
    );
    let removed = storage::index(&mut index, "opencode", &success.sessions).unwrap();
    assert_eq!(removed, 1);
    assert_eq!(
        index
            .query_row(
                "SELECT COUNT(*) FROM sessions WHERE provider_id='opencode'",
                [],
                |row| row.get::<_, i64>(0)
            )
            .unwrap(),
        1
    );
}

fn create_v9_path_identity_database(path: &std::path::Path, duplicate_native: bool) {
    let connection = Connection::open(path).unwrap();
    let native = if duplicate_native {
        "native-1"
    } else {
        "native-2"
    };
    connection
        .execute_batch(&format!(
            "
            CREATE TABLE sessions (
                id TEXT PRIMARY KEY,
                provider_id TEXT NOT NULL,
                native_session_id TEXT,
                project_id TEXT NOT NULL,
                title TEXT NOT NULL,
                models TEXT NOT NULL,
                tool_count INTEGER NOT NULL,
                source_mtime INTEGER NOT NULL,
                partial INTEGER NOT NULL DEFAULT 0
            );
            CREATE TABLE source_files (
                path TEXT PRIMARY KEY,
                session_id TEXT NOT NULL REFERENCES sessions(id) ON DELETE CASCADE,
                size INTEGER NOT NULL,
                mtime INTEGER NOT NULL,
                hash TEXT NOT NULL
            );
            INSERT INTO sessions VALUES ('s1', 'claude', 'native-1', '/repo', 'one', '[]', 0, 1, 0);
            INSERT INTO sessions VALUES ('s2', 'claude', '{native}', '/repo', 'two', '[]', 0, 1, 0);
            INSERT INTO source_files VALUES ('/repo/one.jsonl', 's1', 1, 1, 'one');
            INSERT INTO source_files VALUES (CASE WHEN {duplicate_native} THEN NULL ELSE '/repo/two.jsonl' END, 's2', 1, 1, 'two');
            PRAGMA user_version = 9;
            "
        ))
        .unwrap();
}

#[test]
fn old_v9_source_identity_migrates_and_reopens_idempotently() {
    let root = tempfile::tempdir().unwrap();
    let path = root.path().join("v9.db");
    create_v9_path_identity_database(&path, false);

    for _ in 0..3 {
        let connection = storage::open(&path).unwrap();
        let rows = connection
            .prepare(
                "SELECT provider_id, path, native_session_id, session_id
                 FROM source_files ORDER BY path",
            )
            .unwrap()
            .query_map([], |row| {
                Ok((
                    row.get::<_, String>(0)?,
                    row.get::<_, String>(1)?,
                    row.get::<_, String>(2)?,
                    row.get::<_, String>(3)?,
                ))
            })
            .unwrap()
            .collect::<rusqlite::Result<Vec<_>>>()
            .unwrap();
        assert_eq!(rows.len(), 2);
        assert_eq!(rows[0].0, "claude");
        assert_eq!(rows[0].2, "native-1");
        assert_eq!(rows[1].2, "native-2");
        assert_eq!(
            connection
                .query_row("PRAGMA user_version", [], |row| row.get::<_, i64>(0))
                .unwrap(),
            storage::SCHEMA_VERSION
        );
    }
}

#[test]
fn development_v9_tuple_schema_is_upgraded_without_rebuild() {
    let root = tempfile::tempdir().unwrap();
    let path = root.path().join("v9-tuple.db");
    let connection = storage::open(&path).unwrap();
    connection
        .execute_batch("PRAGMA user_version = 9;")
        .unwrap();
    drop(connection);

    let reopened = storage::open(&path).unwrap();
    assert_eq!(
        reopened
            .query_row("SELECT COUNT(*) FROM source_files", [], |row| row
                .get::<_, i64>(0))
            .unwrap(),
        0
    );
    assert_eq!(
        reopened
            .query_row("PRAGMA user_version", [], |row| row.get::<_, i64>(0))
            .unwrap(),
        storage::SCHEMA_VERSION
    );
}

#[test]
fn failed_v9_source_identity_migration_rolls_back_and_can_retry() {
    let root = tempfile::tempdir().unwrap();
    let path = root.path().join("v9-broken.db");
    create_v9_path_identity_database(&path, true);
    let connection = Connection::open(&path).unwrap();

    assert!(storage::migrate(&connection).is_err());
    assert_eq!(
        connection
            .query_row("PRAGMA user_version", [], |row| row.get::<_, i64>(0))
            .unwrap(),
        9
    );
    let columns = connection
        .prepare("PRAGMA table_info(source_files)")
        .unwrap()
        .query_map([], |row| row.get::<_, String>(1))
        .unwrap()
        .collect::<rusqlite::Result<Vec<_>>>()
        .unwrap();
    assert_eq!(columns, ["path", "session_id", "size", "mtime", "hash"]);

    connection
        .execute(
            "UPDATE source_files SET path = '/repo/two.jsonl' WHERE session_id = 's2'",
            [],
        )
        .unwrap();
    storage::migrate(&connection).unwrap();
    assert_eq!(
        connection
            .query_row("SELECT COUNT(*) FROM source_files", [], |row| row
                .get::<_, i64>(0))
            .unwrap(),
        2
    );
}

#[test]
fn source_identity_queries_ignore_cross_provider_corruption() {
    let file = synthetic_db();
    let scan = OpenCodeStore::open(file.path())
        .unwrap()
        .scan_all()
        .unwrap();
    let root = tempfile::tempdir().unwrap();
    let mut index = storage::open(&root.path().join("index.db")).unwrap();
    storage::index(&mut index, "opencode", &scan.sessions).unwrap();
    let session_id = scan.sessions[0].summary.id.clone();
    index
        .execute(
            "INSERT INTO source_files
             (provider_id, path, native_session_id, session_id, size, mtime, hash)
             VALUES ('codex', '/corrupt.jsonl', 'corrupt', ?, 1, 1, 'corrupt')",
            [&session_id],
        )
        .unwrap();

    let manifest = storage::source_manifest(&index, "opencode").unwrap();
    assert!(!manifest.contains_key(std::path::Path::new("/corrupt.jsonl")));
    storage::index_incremental(&mut index, "opencode", &[], &[file.path().to_path_buf()]).unwrap();
    assert_eq!(storage::count(&index).unwrap(), 1);
}
