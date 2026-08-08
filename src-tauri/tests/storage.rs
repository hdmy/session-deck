use context_vault_lib::{providers::claude::parse_session, scanner, search, storage};
use std::io::Write;
use tempfile::tempdir;

fn write_session(path: &std::path::Path, body: &str) {
    let mut file = std::fs::File::create(path).expect("create session");
    file.write_all(body.as_bytes()).expect("write session");
}

#[test]
fn search_covers_titles_projects_human_prompts_and_assistant_replies() {
    let source = tempdir().expect("create source root");
    let project = source.path().join("birds-project");
    std::fs::create_dir_all(&project).expect("create project");
    let path = project.join("a.jsonl");
    write_session(
        &path,
        r#"{"type":"user","cwd":"/work/birds-project","customTitle":"Repair BirdNET deployment","message":{"role":"user","content":"CloudBase 云开发 部署"}}
{"type":"assistant","message":{"role":"assistant","content":"镜像构建已经完成"}}
"#,
    );

    let parsed = parse_session(&path).expect("parse session");
    let database = tempdir().expect("create database root");
    let mut connection = storage::open(&database.path().join("index.db")).expect("open database");
    storage::index(&mut connection, "claude", &[parsed]).expect("index session");

    assert_eq!(
        search::query(&connection, "BirdNET")
            .expect("title search")
            .len(),
        1
    );
    assert_eq!(
        search::query(&connection, "birds-project")
            .expect("project search")
            .len(),
        1
    );
    assert_eq!(
        search::query(&connection, "CloudBase")
            .expect("prompt search")
            .len(),
        1
    );
    assert_eq!(
        search::query(&connection, "云开发")
            .expect("CJK search")
            .len(),
        1
    );
    assert_eq!(
        search::query(&connection, "部署")
            .expect("short CJK search")
            .len(),
        1
    );
    let reply_hits = search::query(&connection, "镜像构建").expect("assistant search");
    assert_eq!(reply_hits.len(), 1);
    assert!(reply_hits[0].event_id > 0);
    assert!(search::query(&connection, "missing")
        .expect("empty search")
        .is_empty());
}

#[test]
fn successful_reconciliation_removes_deleted_sources_but_failed_scan_preserves_rows() {
    let source = tempdir().expect("create source root");
    let project = source.path().join("project");
    std::fs::create_dir_all(&project).expect("create project");
    let path = project.join("session.jsonl");
    write_session(
        &path,
        "{\"type\":\"user\",\"message\":{\"role\":\"user\",\"content\":\"keep me\"}}\n",
    );

    let database = tempdir().expect("create database root");
    let mut connection = storage::open(&database.path().join("index.db")).expect("open database");
    let parsed = parse_session(&path).expect("parse session");
    let session_id = parsed.summary.id.clone();
    let removed = storage::index(&mut connection, "claude", &[parsed]).expect("index session");
    assert_eq!(removed, 0);
    assert_eq!(
        storage::source_path_for_session(&connection, &session_id).expect("source path"),
        path
    );
    assert_eq!(storage::count(&connection).expect("count rows"), 1);
    storage::set_session_hidden(&connection, &session_id, true).expect("hide session");

    let missing_root = source.path().join("missing");
    assert!(scanner::scan_root(&missing_root).is_err());
    assert_eq!(storage::count(&connection).expect("count rows"), 1);

    storage::index(&mut connection, "codex", &[]).expect("reconcile another provider");
    assert_eq!(storage::count(&connection).expect("count rows"), 1);

    std::fs::remove_file(&path).expect("remove source");
    let (sessions, diagnostics, complete) = scanner::scan_root(source.path()).expect("scan root");
    assert!(complete);
    assert!(diagnostics.is_empty());
    let removed = storage::index(&mut connection, "claude", &sessions).expect("reconcile index");
    assert_eq!(removed, 1);
    assert_eq!(storage::count(&connection).expect("count rows"), 0);
    assert!(storage::hidden_sessions(&connection)
        .expect("hidden sessions")
        .is_empty());
}

#[test]
fn scanning_and_parsing_do_not_modify_source_files() {
    let source = tempdir().expect("create source root");
    let project = source.path().join("project");
    std::fs::create_dir_all(&project).expect("create project");
    let path = project.join("session.jsonl");
    write_session(
        &path,
        "{\"type\":\"user\",\"message\":{\"role\":\"user\",\"content\":\"read only\"}}\n",
    );

    let before_metadata = std::fs::metadata(&path).expect("read metadata");
    let before_hash = scanner::file_hash(&path).expect("hash source");
    let (sessions, diagnostics, complete) = scanner::scan_root(source.path()).expect("scan root");
    let after_metadata = std::fs::metadata(&path).expect("read metadata");
    let after_hash = scanner::file_hash(&path).expect("hash source");

    assert!(complete);
    assert!(diagnostics.is_empty());
    assert_eq!(sessions.len(), 1);
    assert_eq!(before_hash, after_hash);
    assert_eq!(before_metadata.len(), after_metadata.len());
    assert_eq!(
        before_metadata.modified().expect("before mtime"),
        after_metadata.modified().expect("after mtime")
    );

    let database = tempdir().expect("create database root");
    let mut connection = storage::open(&database.path().join("index.db")).expect("open database");
    let session_id = sessions[0].summary.id.clone();
    storage::index(&mut connection, "claude", &sessions).expect("index session");
    storage::rename_session(&connection, &session_id, Some("Local read only"))
        .expect("rename session");
    storage::set_session_hidden(&connection, &session_id, true).expect("hide session");
    storage::set_session_pinned(&connection, &session_id, true).expect("pin session");
    storage::touch_session(&connection, &session_id, 42).expect("touch session");

    let final_metadata = std::fs::metadata(&path).expect("read final metadata");
    let final_hash = scanner::file_hash(&path).expect("hash final source");
    assert_eq!(before_hash, final_hash);
    assert_eq!(before_metadata.len(), final_metadata.len());
    assert_eq!(
        before_metadata.modified().expect("before mtime"),
        final_metadata.modified().expect("after mtime")
    );
}

#[test]
fn incremental_unchanged_keeps_projection_row_ids_and_local_state() {
    let source = tempdir().expect("source");
    let project = source.path().join("p");
    std::fs::create_dir_all(&project).unwrap();
    let path = project.join("s.jsonl");
    write_session(
        &path,
        "{\"type\":\"user\",\"message\":{\"role\":\"user\",\"content\":\"keep\"}}\n",
    );
    let dbdir = tempdir().unwrap();
    let mut db = storage::open(&dbdir.path().join("i.db")).unwrap();
    let parsed = parse_session(&path).unwrap();
    let id = parsed.summary.id.clone();
    storage::index(&mut db, "claude", &[parsed]).unwrap();
    storage::rename_session(&db, &id, Some("local")).unwrap();
    let event_id: i64 = db
        .query_row("SELECT id FROM timeline WHERE session_id=?", [&id], |r| {
            r.get(0)
        })
        .unwrap();
    storage::index_incremental(&mut db, "claude", &[], std::slice::from_ref(&path)).unwrap();
    let after: i64 = db
        .query_row("SELECT id FROM timeline WHERE session_id=?", [&id], |r| {
            r.get(0)
        })
        .unwrap();
    assert_eq!(event_id, after);
    assert_eq!(storage::session_summary(&db, &id).unwrap().title, "local");
}

#[test]
fn duplicate_source_identity_is_rejected_before_projection_changes() {
    let source = tempdir().unwrap();
    let project = source.path().join("p");
    std::fs::create_dir_all(&project).unwrap();
    let a = project.join("a.jsonl");
    let b = project.join("b.jsonl");
    let body = "{\"type\":\"user\",\"sessionId\":\"same\",\"message\":{\"role\":\"user\",\"content\":\"x\"}}\n";
    write_session(&a, body);
    write_session(&b, body);
    let mut db = storage::open(&source.path().join("i.db")).unwrap();
    let parsed = vec![parse_session(&a).unwrap(), parse_session(&b).unwrap()];
    assert!(storage::index(&mut db, "claude", &parsed).is_err());
    assert_eq!(storage::count(&db).unwrap(), 0);
}

#[test]
fn incremental_parse_cannot_reuse_an_unchanged_source_identity() {
    let source = tempdir().unwrap();
    let project = source.path().join("p");
    std::fs::create_dir_all(&project).unwrap();
    let unchanged = project.join("unchanged.jsonl");
    let added = project.join("added.jsonl");
    let body = "{\"type\":\"user\",\"sessionId\":\"same\",\"message\":{\"role\":\"user\",\"content\":\"x\"}}\n";
    write_session(&unchanged, body);
    let mut db = storage::open(&source.path().join("i.db")).unwrap();
    storage::index(&mut db, "claude", &[parse_session(&unchanged).unwrap()]).unwrap();

    write_session(&added, body);
    let error = storage::index_incremental(
        &mut db,
        "claude",
        &[parse_session(&added).unwrap()],
        &[unchanged.clone(), added],
    )
    .unwrap_err();

    assert!(error.to_string().contains("another discovered source"));
    assert_eq!(storage::count(&db).unwrap(), 1);
    assert_eq!(
        storage::source_path_for_session(&db, "claude:same").unwrap(),
        unchanged
    );
}

#[test]
fn schema_migration_namespaces_legacy_rows_with_a_provider() {
    let database = tempdir().expect("create database root");
    let path = database.path().join("legacy.db");
    let connection = rusqlite::Connection::open(&path).expect("open legacy database");
    connection
        .execute_batch(
            "
            CREATE TABLE sessions (
                id TEXT PRIMARY KEY,
                project_id TEXT NOT NULL,
                title TEXT NOT NULL,
                started_at INTEGER,
                ended_at INTEGER,
                branch TEXT,
                first_prompt TEXT,
                last_prompt TEXT,
                cwd TEXT,
                models TEXT NOT NULL,
                tool_count INTEGER NOT NULL,
                source_mtime INTEGER NOT NULL,
                partial INTEGER NOT NULL DEFAULT 0
            );
            INSERT INTO sessions (
                id, project_id, title, models, tool_count, source_mtime, partial
            ) VALUES ('legacy', '/work/project', 'Legacy', '[]', 0, 0, 0);
            CREATE VIRTUAL TABLE search_chunks USING fts5(
                session_id UNINDEXED,
                event_id UNINDEXED,
                field UNINDEXED,
                content,
                tokenize = 'trigram'
            );
            PRAGMA user_version = 2;
            ",
        )
        .expect("create legacy schema");
    drop(connection);

    let migrated = storage::open(&path).expect("migrate database");
    let provider = migrated
        .query_row(
            "SELECT provider_id FROM sessions WHERE id = 'legacy'",
            [],
            |row| row.get::<_, String>(0),
        )
        .expect("read migrated provider");
    let version = migrated
        .query_row("PRAGMA user_version", [], |row| row.get::<_, i64>(0))
        .expect("read schema version");

    assert_eq!(provider, "claude");
    assert_eq!(version, 9);
}

#[test]
fn v7_relation_migration_preserves_rows_and_is_idempotent() {
    let database = tempdir().expect("create database root");
    let path = database.path().join("v7.db");
    let connection = rusqlite::Connection::open(&path).expect("open v7 database");
    connection
        .execute_batch(
            "
            CREATE TABLE session_relations (
                parent_session_id TEXT NOT NULL,
                child_session_id TEXT NOT NULL,
                relation_type TEXT NOT NULL CHECK (relation_type = 'fork'),
                created_at INTEGER NOT NULL,
                status TEXT NOT NULL DEFAULT 'pending'
            );
            INSERT INTO session_relations (parent_session_id, child_session_id, relation_type, created_at, status)
                VALUES ('parent', 'child', 'fork', 42, 'unknown');
            CREATE INDEX session_relations_parent ON session_relations(parent_session_id);
            CREATE INDEX session_relations_child ON session_relations(child_session_id);
            PRAGMA user_version = 7;
            ",
        )
        .expect("create v7 fixture");
    drop(connection);

    let migrated = storage::open(&path).expect("migrate v7 fixture");
    let row = migrated
        .query_row(
            "SELECT provider_id, parent_session_id, child_session_id, status FROM session_relations",
            [],
            |row| {
                Ok((
                    row.get::<_, String>(0)?,
                    row.get::<_, String>(1)?,
                    row.get::<_, String>(2)?,
                    row.get::<_, String>(3)?,
                ))
            },
        )
        .expect("read migrated relation");
    assert_eq!(
        row,
        (
            "claude".into(),
            "parent".into(),
            "child".into(),
            "unknown".into()
        )
    );
    drop(migrated);

    let rerun = storage::open(&path).expect("rerun migration");
    let count: i64 = rerun
        .query_row("SELECT COUNT(*) FROM session_relations", [], |row| {
            row.get(0)
        })
        .expect("count migrated relations");
    assert_eq!(count, 1);
}

#[test]
fn v7_relation_migration_rolls_back_table_indexes_and_version_on_failure() {
    let database = tempdir().expect("create database root");
    let connection =
        rusqlite::Connection::open(database.path().join("broken-v7.db")).expect("open v7 database");
    connection
        .execute_batch(
            "
            CREATE TABLE session_relations (
                parent_session_id TEXT NOT NULL,
                child_session_id TEXT NOT NULL,
                relation_type TEXT NOT NULL CHECK (relation_type = 'fork'),
                created_at INTEGER NOT NULL,
                status TEXT NOT NULL DEFAULT 'pending'
            );
            INSERT INTO session_relations (parent_session_id, child_session_id, relation_type, created_at, status)
                VALUES ('parent', 'child', 'fork', 42, 'unknown'),
                       ('parent', 'child', 'fork', 43, 'pending');
            CREATE INDEX session_relations_parent ON session_relations(parent_session_id);
            CREATE INDEX session_relations_child ON session_relations(child_session_id);
            PRAGMA user_version = 7;
            ",
        )
        .expect("create broken v7 fixture");

    assert!(storage::migrate(&connection).is_err());
    let version = connection
        .query_row("PRAGMA user_version", [], |row| row.get::<_, i64>(0))
        .expect("read unchanged schema version");
    assert_eq!(version, 7);
    let columns = connection
        .prepare("PRAGMA table_info(session_relations)")
        .expect("inspect old relation table")
        .query_map([], |row| row.get::<_, String>(1))
        .expect("read old relation columns")
        .collect::<rusqlite::Result<Vec<_>>>()
        .expect("collect old relation columns");
    assert!(!columns.iter().any(|column| column == "provider_id"));
    let index_count: i64 = connection
        .query_row(
            "SELECT COUNT(*) FROM sqlite_master WHERE type = 'index' AND name IN ('session_relations_parent', 'session_relations_child')",
            [],
            |row| row.get(0),
        )
        .expect("count restored relation indexes");
    assert_eq!(index_count, 2);

    connection
        .execute("DELETE FROM session_relations WHERE created_at = 43", [])
        .expect("repair duplicate relation");
    storage::migrate(&connection).expect("retry migration on the same connection");
    let version = connection
        .query_row("PRAGMA user_version", [], |row| row.get::<_, i64>(0))
        .expect("read migrated schema version");
    let provider: String = connection
        .query_row("SELECT provider_id FROM session_relations", [], |row| {
            row.get(0)
        })
        .expect("read migrated provider");
    assert_eq!(version, storage::SCHEMA_VERSION);
    assert_eq!(provider, "claude");
}

#[test]
fn v8_to_current_migration_preserves_local_rows_and_is_idempotent() {
    let database = tempdir().expect("create database root");
    let path = database.path().join("v8.db");
    let connection = storage::open(&path).expect("create current database");
    connection
        .execute_batch(
            "INSERT INTO sessions (
                id, provider_id, native_session_id, project_id, title, local_title,
                models, tool_count, source_mtime, partial
             ) VALUES ('legacy', 'claude', 'legacy', '/work', 'Source', 'Local', '[]', 0, 0, 0);
             DROP TABLE session_cwds;
             DROP TABLE project_overrides;
             DROP TABLE scan_diagnostic_counts;
             DROP TABLE scan_runs;
             ALTER TABLE sessions DROP COLUMN workspace_id;
             ALTER TABLE sessions DROP COLUMN project_path;
             ALTER TABLE sessions DROP COLUMN worktree_path;
             ALTER TABLE claude_settings DROP COLUMN source_root;
             ALTER TABLE claude_settings DROP COLUMN scan_interval_seconds;
             PRAGMA user_version = 8;",
        )
        .expect("downgrade fixture to v8 shape");

    storage::migrate(&connection).expect("migrate v8 fixture");
    storage::migrate(&connection).expect("rerun migration on same connection");

    let row = connection
        .query_row(
            "SELECT local_title, workspace_id FROM sessions WHERE id = 'legacy'",
            [],
            |row| Ok((row.get::<_, String>(0)?, row.get::<_, Option<String>>(1)?)),
        )
        .expect("read preserved local row");
    let version = connection
        .query_row("PRAGMA user_version", [], |row| row.get::<_, i64>(0))
        .expect("read schema version");
    assert_eq!(row, ("Local".into(), None));
    assert_eq!(version, storage::SCHEMA_VERSION);
    assert_eq!(
        storage::claude_scan_settings(&connection)
            .unwrap()
            .scan_interval_seconds,
        0
    );
}

#[test]
fn future_schema_version_is_rejected_without_schema_changes() {
    let database = tempdir().expect("create database root");
    let connection = rusqlite::Connection::open(database.path().join("future.db"))
        .expect("open future database");
    connection
        .execute_batch("CREATE TABLE sentinel (value TEXT); PRAGMA user_version = 10;")
        .expect("create future schema");

    let error = storage::migrate(&connection).unwrap_err();

    let version = connection
        .query_row("PRAGMA user_version", [], |row| row.get::<_, i64>(0))
        .expect("read future version");
    let sessions_exist: bool = connection
        .query_row(
            "SELECT EXISTS(SELECT 1 FROM sqlite_master WHERE type='table' AND name='sessions')",
            [],
            |row| row.get(0),
        )
        .expect("check rollback");
    assert!(error.to_string().contains("newer than supported"));
    assert_eq!(version, 10);
    assert!(!sessions_exist);
}

#[test]
fn relations_are_namespaced_by_provider_during_reconciliation() {
    let database = tempdir().expect("create database root");
    let mut connection = storage::open(&database.path().join("index.db")).expect("open database");
    storage::register_fork_relation(&connection, "claude", "parent", "child", 1)
        .expect("register Claude relation");
    storage::register_fork_relation(&connection, "codex", "parent", "child", 2)
        .expect("register Codex relation");
    let before: i64 = connection
        .query_row(
            "SELECT COUNT(*) FROM session_relations WHERE provider_id = 'claude'",
            [],
            |row| row.get(0),
        )
        .expect("count Claude relations");
    storage::index(&mut connection, "codex", &[]).expect("reconcile Codex");
    let after: i64 = connection
        .query_row(
            "SELECT COUNT(*) FROM session_relations WHERE provider_id = 'claude'",
            [],
            |row| row.get(0),
        )
        .expect("count Claude relations after Codex reconcile");
    assert_eq!(before, 1);
    assert_eq!(after, before);
}

#[test]
fn relation_reconciliation_transitions_pending_unknown_indexed_removed_and_reappeared() {
    let database = tempdir().expect("create database root");
    let mut connection = storage::open(&database.path().join("index.db")).expect("open database");
    const CHILD: &str = "123e4567-e89b-12d3-a456-426614174000";
    let source = tempdir().expect("create child source");
    let project = source.path().join("project");
    std::fs::create_dir_all(&project).expect("create child project");
    let child_path = project.join(format!("{CHILD}.jsonl"));
    std::fs::write(
        &child_path,
        format!("{{\"type\":\"user\",\"sessionId\":\"{CHILD}\",\"message\":{{\"role\":\"user\",\"content\":\"child\"}}}}\n"),
    )
    .expect("write child source");
    storage::register_fork_relation(
        &connection,
        "claude",
        "parent",
        &format!("claude:{CHILD}"),
        1,
    )
    .expect("register relation");
    fn status(connection: &rusqlite::Connection, provider: &str) -> String {
        connection
            .query_row(
                "SELECT status FROM session_relations WHERE provider_id = ?",
                [provider],
                |row| row.get::<_, String>(0),
            )
            .expect("read relation status")
    }
    storage::index(&mut connection, "claude", &[]).expect("missing child reconcile");
    assert_eq!(status(&connection, "claude"), "unknown");
    storage::index(&mut connection, "claude", &[]).expect("unknown retry");
    assert_eq!(status(&connection, "claude"), "unknown");
    let child = parse_session(&child_path).expect("parse child source");
    storage::index(&mut connection, "claude", &[child]).expect("reappeared reconcile");
    assert_eq!(status(&connection, "claude"), "indexed");
    connection
        .execute(
            "DELETE FROM sessions WHERE id = ?",
            [format!("claude:{CHILD}")],
        )
        .expect("remove child");
    storage::index(&mut connection, "claude", &[]).expect("removed reconcile");
    assert_eq!(status(&connection, "claude"), "source_removed");
    let child = parse_session(&child_path).expect("reparse child source");
    storage::index(&mut connection, "claude", &[child]).expect("reappear after removal");
    assert_eq!(status(&connection, "claude"), "indexed");

    storage::register_fork_relation(
        &connection,
        "codex",
        "parent",
        &format!("claude:{CHILD}"),
        2,
    )
    .expect("register other provider relation");
    storage::index(&mut connection, "claude", &[]).expect("Claude isolation reconcile");
    assert_eq!(status(&connection, "codex"), "pending");
}

#[test]
fn branch_read_model_round_trips_active_and_alternate_details() {
    let source = tempdir().expect("create source root");
    let project = source.path().join("project");
    std::fs::create_dir_all(&project).expect("create project");
    let path = project.join("branches.jsonl");
    write_session(
        &path,
        r#"{"type":"user","sessionId":"branch-1","uuid":"u1","timestamp":1,"message":{"role":"user","content":"inspect"}}
{"type":"assistant","sessionId":"branch-1","uuid":"a-old","parentUuid":"u1","timestamp":2,"message":{"role":"assistant","content":"old response"}}
{"type":"assistant","sessionId":"branch-1","uuid":"a-new","parentUuid":"u1","timestamp":3,"message":{"role":"assistant","content":"new response"}}
"#,
    );
    let parsed = parse_session(&path).expect("parse session");
    assert_eq!(parsed.branches.len(), 2);
    let session_id = parsed.summary.id.clone();
    let active_id = parsed
        .branches
        .iter()
        .find(|branch| branch.summary.is_active)
        .expect("active branch")
        .summary
        .id
        .clone();
    let alternate = parsed
        .branches
        .iter()
        .find(|branch| !branch.summary.is_active)
        .expect("alternate branch");
    let alternate_id = alternate.summary.id.clone();
    let database = tempdir().expect("create database root");
    let mut connection = storage::open(&database.path().join("index.db")).expect("open database");
    storage::index(&mut connection, "claude", &[parsed]).expect("index session");

    let active = storage::detail(&connection, &session_id).expect("active detail");
    assert_eq!(active.active_branch_id.as_deref(), Some(active_id.as_str()));
    assert_eq!(
        active.selected_branch_id.as_deref(),
        Some(active_id.as_str())
    );
    assert_eq!(active.timeline.len(), active.branches[0].event_count);
    let alternate_detail =
        storage::detail_branch(&connection, &session_id, &alternate_id).expect("alternate detail");
    assert_eq!(
        alternate_detail.active_branch_id.as_deref(),
        Some(active_id.as_str())
    );
    assert_eq!(
        alternate_detail.selected_branch_id.as_deref(),
        Some(alternate_id.as_str())
    );
    assert!(alternate_detail
        .timeline
        .iter()
        .any(|event| event.content == "old response"));
    assert!(storage::detail_branch(&connection, &session_id, "missing").is_err());

    let branch_count: i64 = connection
        .query_row(
            "SELECT COUNT(*) FROM session_branches WHERE session_id = ?",
            [&session_id],
            |row| row.get(0),
        )
        .expect("branch rows");
    assert_eq!(branch_count, 2);
}

#[test]
fn local_session_state_survives_rescan_and_controls_visibility_and_search() {
    let source = tempdir().expect("create source root");
    let project = source.path().join("project");
    std::fs::create_dir_all(&project).expect("create project");
    let first_path = project.join("first.jsonl");
    let second_path = project.join("second.jsonl");
    write_session(
        &first_path,
        r#"{"type":"user","customTitle":"Provider First","message":{"role":"user","content":"first"}}
"#,
    );
    write_session(
        &second_path,
        r#"{"type":"user","customTitle":"Provider Second","message":{"role":"user","content":"second"}}
"#,
    );

    let first = parse_session(&first_path).expect("parse first");
    let first_id = first.summary.id.clone();
    let second = parse_session(&second_path).expect("parse second");
    let second_id = second.summary.id.clone();
    let database = tempdir().expect("create database root");
    let mut connection = storage::open(&database.path().join("index.db")).expect("open database");
    storage::index(&mut connection, "claude", &[first, second]).expect("index sessions");

    let renamed = storage::rename_session(&connection, &first_id, Some("  Local First  "))
        .expect("rename session");
    assert_eq!(renamed.title, "Local First");
    assert_eq!(renamed.source_title, "Provider First");
    assert_eq!(
        search::query(&connection, "Local First")
            .expect("search local title")
            .len(),
        1
    );
    assert!(search::query(&connection, "Provider First")
        .expect("old title search")
        .is_empty());

    storage::set_session_hidden(&connection, &first_id, true).expect("hide first");
    storage::set_session_pinned(&connection, &first_id, true).expect("pin first");
    storage::touch_session(&connection, &first_id, 2_000).expect("touch first");
    let before_rescan = storage::session_summary(&connection, &first_id).expect("read first");

    // Re-indexing provider data must not erase local state.
    let first_again = parse_session(&first_path).expect("reparse first");
    let second_again = parse_session(&second_path).expect("reparse second");
    storage::index(&mut connection, "claude", &[first_again, second_again])
        .expect("re-index sessions");
    let after_rescan = storage::session_summary(&connection, &first_id).expect("read first");
    assert_eq!(after_rescan.title, before_rescan.title);
    assert_eq!(after_rescan.hidden, before_rescan.hidden);
    assert_eq!(after_rescan.pinned, before_rescan.pinned);
    assert_eq!(after_rescan.last_used_at, before_rescan.last_used_at);

    assert!(storage::projects(&connection)
        .expect("projects")
        .iter()
        .flat_map(|project| project.sessions.iter())
        .all(|session| session.id != first_id));
    assert!(search::query(&connection, "Local First")
        .expect("hidden search")
        .is_empty());
    let hidden = storage::hidden_sessions(&connection).expect("hidden sessions");
    assert_eq!(hidden.len(), 1);
    assert_eq!(hidden[0].id, first_id);

    storage::set_session_hidden(&connection, &first_id, false).expect("unhide first");
    storage::set_session_pinned(&connection, &first_id, false).expect("unpin first");
    storage::set_session_pinned(&connection, &second_id, true).expect("pin second");
    storage::touch_session(&connection, &first_id, 2_000).expect("touch first");
    storage::touch_session(&connection, &second_id, 1_000).expect("touch second");
    let sessions = &storage::projects(&connection).expect("projects")[0].sessions;
    assert_eq!(sessions[0].id, second_id);
    assert!(storage::rename_session(&connection, &first_id, Some("   ")).is_err());
    assert!(storage::rename_session(&connection, &first_id, Some(&"x".repeat(201))).is_err());

    // Resetting with None restores the provider title and search index.
    let reset = storage::rename_session(&connection, &first_id, None).expect("reset title");
    assert_eq!(reset.title, "Provider First");
    assert_eq!(reset.source_title, "Provider First");
    assert!(search::query(&connection, "Local First")
        .expect("reset old title search")
        .is_empty());
    assert_eq!(
        search::query(&connection, "Provider First")
            .expect("reset provider title search")
            .len(),
        1
    );
}

#[test]
fn storage_sort_ties_are_stable_and_project_path_comes_from_latest_session() {
    let source = tempdir().expect("create source root");
    let project_a = source.path().join("project-a");
    let project_b = source.path().join("project-b");
    let hidden_project = source.path().join("hidden-project");
    std::fs::create_dir_all(&project_a).expect("create project a");
    std::fs::create_dir_all(&project_b).expect("create project b");
    std::fs::create_dir_all(&hidden_project).expect("create hidden project");

    let a_session = project_a.join("a.jsonl");
    let z_session = project_a.join("z.jsonl");
    let b_session = project_b.join("b.jsonl");
    let a_hidden = hidden_project.join("a-hidden.jsonl");
    let z_hidden = hidden_project.join("z-hidden.jsonl");
    write_session(
        &a_session,
        r#"{"type":"user","sessionId":"a-session","cwd":"/work/project-a","message":{"role":"user","content":"a"}}
"#,
    );
    write_session(
        &z_session,
        r#"{"type":"user","sessionId":"z-session","cwd":"/work/project-a","message":{"role":"user","content":"z"}}
"#,
    );
    write_session(
        &b_session,
        r#"{"type":"user","sessionId":"b-session","cwd":"/work/project-b","message":{"role":"user","content":"b"}}
"#,
    );
    write_session(
        &a_hidden,
        r#"{"type":"user","sessionId":"a-hidden","cwd":"/work/hidden","message":{"role":"user","content":"a hidden"}}
"#,
    );
    write_session(
        &z_hidden,
        r#"{"type":"user","sessionId":"z-hidden","cwd":"/work/hidden","message":{"role":"user","content":"z hidden"}}
"#,
    );

    let database = tempdir().expect("create database root");
    let mut connection = storage::open(&database.path().join("index.db")).expect("open database");
    let parsed = [&a_session, &z_session, &b_session, &a_hidden, &z_hidden]
        .into_iter()
        .map(|path| parse_session(path).expect("parse session"))
        .collect::<Vec<_>>();
    storage::index(&mut connection, "claude", &parsed).expect("index sessions");

    connection
        .execute(
            "UPDATE sessions SET pinned = 0, last_used_at = 100, ended_at = 10, started_at = 1, source_mtime = 1",
            [],
        )
        .expect("tie session activity");
    connection
        .execute(
            "UPDATE sessions SET hidden = 1 WHERE id IN ('claude:a-hidden', 'claude:z-hidden')",
            [],
        )
        .expect("hide sessions");
    connection
        .execute(
            r#"UPDATE sessions SET cwd = CASE id
                WHEN 'claude:a-session' THEN '/a-newest'
                WHEN 'claude:z-session' THEN '/z-older'
                ELSE cwd
            END,
            ended_at = CASE id
                WHEN 'claude:a-session' THEN 20
                WHEN 'claude:b-session' THEN 20
                WHEN 'claude:z-session' THEN 10
                ELSE ended_at
            END"#,
            [],
        )
        .expect("set deterministic session recency");

    let hidden = storage::hidden_sessions(&connection).expect("hidden sessions");
    assert_eq!(
        hidden
            .iter()
            .map(|session| session.id.as_str())
            .collect::<Vec<_>>(),
        vec!["claude:a-hidden", "claude:z-hidden"]
    );

    let projects = storage::projects(&connection).expect("projects");
    assert_eq!(
        projects
            .iter()
            .map(|project| project.id.as_str())
            .collect::<Vec<_>>(),
        vec!["/work/project-a", "/work/project-b"]
    );
    // The latest stable project path is surfaced for the grouped workspace.
    assert_eq!(projects[0].path, "/a-newest");
    assert_eq!(
        projects[0]
            .sessions
            .iter()
            .map(|session| session.id.as_str())
            .collect::<Vec<_>>(),
        vec!["claude:a-session", "claude:z-session"]
    );

    connection
        .execute(
            "UPDATE sessions SET cwd = NULL WHERE id = 'claude:a-session'",
            [],
        )
        .expect("clear latest cwd");
    let projects = storage::projects(&connection).expect("projects after cwd removal");
    assert_eq!(projects[0].path, "/z-older");

    // A row's first available timestamp is its activity key. Aggregating each
    // column independently would incorrectly rank project-b ahead of project-a.
    connection
        .execute(
            "UPDATE sessions SET last_used_at = CASE id
                 WHEN 'claude:a-session' THEN 10
                 WHEN 'claude:z-session' THEN NULL
                 WHEN 'claude:b-session' THEN 900
                 ELSE last_used_at END,
             ended_at = CASE id
                 WHEN 'claude:a-session' THEN 1
                 WHEN 'claude:z-session' THEN 1000
                 WHEN 'claude:b-session' THEN 900
                 ELSE ended_at END",
            [],
        )
        .expect("set heterogeneous activity");
    let projects = storage::projects(&connection).expect("projects with heterogeneous activity");
    assert_eq!(projects[0].id, "/work/project-a");
    assert_eq!(projects[0].path, "/z-older");
    assert_eq!(
        projects[0]
            .sessions
            .iter()
            .map(|session| session.id.as_str())
            .collect::<Vec<_>>(),
        vec!["claude:z-session", "claude:a-session"]
    );
}

#[test]
fn claude_settings_are_migrated_with_safe_defaults_and_persisted() {
    let database = tempdir().expect("create database root");
    let path = database.path().join("settings.db");
    let connection = storage::open(&path).expect("open database");
    assert_eq!(
        storage::claude_settings(&connection).expect("read defaults"),
        storage::ClaudeSettings {
            executable_override: None,
            dangerously_skip_permissions: false,
        }
    );
    let updated = storage::ClaudeSettings {
        executable_override: Some("/usr/local/bin/claude".to_owned()),
        dangerously_skip_permissions: true,
    };
    storage::update_claude_settings(&connection, &updated).expect("update settings");
    assert_eq!(
        storage::claude_settings(&connection).expect("read settings"),
        updated
    );
}

#[test]
fn detail_persists_turns_and_native_session_id() {
    let source = tempdir().expect("create source root");
    let project = source.path().join("project");
    std::fs::create_dir_all(&project).expect("create project");
    let path = project.join("native-123.jsonl");
    write_session(
        &path,
        "{\"type\":\"user\",\"sessionId\":\"native-123\",\"uuid\":\"u1\",\"message\":{\"role\":\"user\",\"content\":\"hello\"}}\n{\"type\":\"assistant\",\"sessionId\":\"native-123\",\"uuid\":\"a1\",\"parentUuid\":\"u1\",\"message\":{\"role\":\"assistant\",\"content\":\"world\"}}\n",
    );
    let parsed = parse_session(&path).expect("parse session");
    let database = tempdir().expect("create database root");
    let mut connection = storage::open(&database.path().join("index.db")).expect("open database");
    storage::index(&mut connection, "claude", &[parsed]).expect("index session");

    let detail = storage::detail(&connection, "claude:native-123").expect("detail");
    assert_eq!(
        detail.summary.native_session_id.as_deref(),
        Some("native-123")
    );
    assert_eq!(detail.turns.len(), 1);
    assert_eq!(detail.turns[0].final_response.as_deref(), Some("world"));
    assert_eq!(detail.turns[0].activities.len(), 1);

    let second_path = project.join("native-456.jsonl");
    write_session(
        &second_path,
        "{\"type\":\"user\",\"sessionId\":\"native-456\",\"uuid\":\"u2\",\"message\":{\"role\":\"user\",\"content\":\"second\"}}\n{\"type\":\"assistant\",\"sessionId\":\"native-456\",\"uuid\":\"a2\",\"parentUuid\":\"u2\",\"message\":{\"role\":\"assistant\",\"content\":\"reply\"}}\n",
    );
    storage::index(
        &mut connection,
        "claude",
        &[
            parse_session(&path).expect("first"),
            parse_session(&second_path).expect("second"),
        ],
    )
    .expect("reindex sessions");
    let second_detail = storage::detail(&connection, "claude:native-456").expect("second detail");
    let activity_event_id = second_detail.turns[0].activities[0].event_id;
    let timeline_exists: i64 = connection
        .query_row(
            "SELECT COUNT(*) FROM timeline WHERE id = ? AND session_id = ?",
            rusqlite::params![activity_event_id, "claude:native-456"],
            |row| row.get(0),
        )
        .expect("timeline mapping");
    assert_eq!(timeline_exists, 1);
}

#[test]
fn local_actions_report_nonexistent_session_ids() {
    let database = tempdir().expect("create database root");
    let connection = storage::open(&database.path().join("index.db")).expect("open database");
    for error in [
        storage::set_session_hidden(&connection, "missing", true).expect_err("missing hidden"),
        storage::rename_session(&connection, "missing", Some("title")).expect_err("missing rename"),
        storage::set_session_pinned(&connection, "missing", true).expect_err("missing pin"),
        storage::touch_session(&connection, "missing", 1).expect_err("missing touch"),
        storage::detail(&connection, "missing").expect_err("missing detail"),
    ] {
        assert!(error.to_string().contains("session not found: missing"));
    }
}
