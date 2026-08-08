use crate::domain::{
    AppError, BranchSummary, ConversationTurn, Diagnostic, ParsedSession, Project, Result,
    SearchHit, SessionDetail, SessionRelation, SessionSummary, TimelineEvent, ToolStat,
    TurnActivity, TurnInsight,
};
use rusqlite::{params, Connection, OptionalExtension};
use serde::{Deserialize, Serialize};
use std::{
    collections::{HashMap, HashSet},
    path::{Path, PathBuf},
};

pub const SCHEMA_VERSION: i64 = 9;

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize, Default)]
pub struct ClaudeSettings {
    pub executable_override: Option<String>,
    pub dangerously_skip_permissions: bool,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize, Default)]
pub struct ClaudeScanSettings {
    pub source_root: Option<String>,
    pub scan_interval_seconds: i64,
}

pub fn open(path: &Path) -> Result<Connection> {
    let connection = Connection::open(path)?;
    migrate(&connection)?;
    Ok(connection)
}

pub fn migrate(connection: &Connection) -> Result<()> {
    connection.execute_batch(
        "PRAGMA foreign_keys = ON; PRAGMA journal_mode = WAL; SAVEPOINT migrate_v8_v9;",
    )?;
    let mut migration_guard = MigrationGuard {
        connection,
        committed: false,
    };
    connection.execute_batch(
        "
        CREATE TABLE IF NOT EXISTS sessions (
            id TEXT PRIMARY KEY,
            provider_id TEXT NOT NULL,
            native_session_id TEXT,
            project_id TEXT NOT NULL,
            workspace_id TEXT,
            project_path TEXT,
            worktree_path TEXT,
            title TEXT NOT NULL,
            local_title TEXT,
            hidden INTEGER NOT NULL DEFAULT 0,
            pinned INTEGER NOT NULL DEFAULT 0,
            last_used_at INTEGER,
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

        CREATE TABLE IF NOT EXISTS timeline (
            id INTEGER PRIMARY KEY AUTOINCREMENT,
            session_id TEXT NOT NULL REFERENCES sessions(id) ON DELETE CASCADE,
            kind TEXT NOT NULL,
            role TEXT,
            content TEXT NOT NULL,
            timestamp INTEGER,
            tool_name TEXT,
            collapsed INTEGER NOT NULL,
            uuid TEXT,
            parent_uuid TEXT,
            logical_parent_uuid TEXT,
            message_id TEXT,
            parent_tool_use_id TEXT,
            tool_use_id TEXT,
            sequence INTEGER NOT NULL DEFAULT 0,
            is_sidechain INTEGER NOT NULL DEFAULT 0,
            is_meta INTEGER NOT NULL DEFAULT 0,
            turn_id INTEGER,
            final_response INTEGER NOT NULL DEFAULT 0,
            compact_boundary INTEGER NOT NULL DEFAULT 0,
            compact_preserved TEXT NOT NULL DEFAULT '[]'
        );

        CREATE INDEX IF NOT EXISTS timeline_session_id ON timeline(session_id, id);

        CREATE TABLE IF NOT EXISTS session_branches (
            session_id TEXT NOT NULL REFERENCES sessions(id) ON DELETE CASCADE,
            id TEXT NOT NULL,
            label TEXT NOT NULL,
            kind TEXT NOT NULL,
            root_uuid TEXT,
            leaf_uuid TEXT,
            fork_point_uuid TEXT,
            is_active INTEGER NOT NULL DEFAULT 0,
            event_count INTEGER NOT NULL DEFAULT 0,
            turn_count INTEGER NOT NULL DEFAULT 0,
            started_at INTEGER,
            ended_at INTEGER,
            compacted INTEGER NOT NULL DEFAULT 0,
            tool_stats TEXT NOT NULL DEFAULT '[]',
            turn_insights TEXT NOT NULL DEFAULT '[]',
            PRIMARY KEY (session_id, id)
        );
        CREATE INDEX IF NOT EXISTS session_branches_session_active
            ON session_branches(session_id, is_active, id);

        CREATE TABLE IF NOT EXISTS session_relations (
            provider_id TEXT NOT NULL DEFAULT 'claude',
            parent_session_id TEXT NOT NULL,
            child_session_id TEXT NOT NULL,
            relation_type TEXT NOT NULL CHECK (relation_type = 'fork'),
            created_at INTEGER NOT NULL,
            status TEXT NOT NULL DEFAULT 'pending'
                CHECK (status IN ('pending', 'indexed', 'source_removed', 'unknown')),
            PRIMARY KEY (provider_id, parent_session_id, child_session_id, relation_type)
        );
        CREATE TABLE IF NOT EXISTS branch_timeline (
            id INTEGER PRIMARY KEY AUTOINCREMENT,
            session_id TEXT NOT NULL,
            branch_id TEXT NOT NULL,
            kind TEXT NOT NULL,
            role TEXT,
            content TEXT NOT NULL,
            timestamp INTEGER,
            tool_name TEXT,
            collapsed INTEGER NOT NULL,
            uuid TEXT,
            parent_uuid TEXT,
            logical_parent_uuid TEXT,
            message_id TEXT,
            parent_tool_use_id TEXT,
            tool_use_id TEXT,
            sequence INTEGER NOT NULL DEFAULT 0,
            is_sidechain INTEGER NOT NULL DEFAULT 0,
            is_meta INTEGER NOT NULL DEFAULT 0,
            turn_id INTEGER,
            final_response INTEGER NOT NULL DEFAULT 0,
            compact_boundary INTEGER NOT NULL DEFAULT 0,
            compact_preserved TEXT NOT NULL DEFAULT '[]',
            FOREIGN KEY (session_id, branch_id)
                REFERENCES session_branches(session_id, id) ON DELETE CASCADE,
            UNIQUE (session_id, branch_id, id)
        );
        CREATE INDEX IF NOT EXISTS branch_timeline_session_branch
            ON branch_timeline(session_id, branch_id, id);

        CREATE TABLE IF NOT EXISTS branch_turns (
            id INTEGER NOT NULL,
            session_id TEXT NOT NULL,
            branch_id TEXT NOT NULL,
            user_prompt TEXT,
            final_response TEXT,
            timestamp INTEGER,
            completed INTEGER NOT NULL DEFAULT 0,
            PRIMARY KEY (session_id, branch_id, id),
            FOREIGN KEY (session_id, branch_id)
                REFERENCES session_branches(session_id, id) ON DELETE CASCADE
        );

        CREATE TABLE IF NOT EXISTS branch_turn_activities (
            id INTEGER PRIMARY KEY AUTOINCREMENT,
            session_id TEXT NOT NULL,
            branch_id TEXT NOT NULL,
            turn_id INTEGER NOT NULL,
            event_id INTEGER NOT NULL,
            kind TEXT NOT NULL,
            role TEXT,
            content TEXT NOT NULL,
            timestamp INTEGER,
            tool_name TEXT,
            tool_use_id TEXT,
            parent_tool_use_id TEXT,
            collapsed INTEGER NOT NULL,
            final_response INTEGER NOT NULL DEFAULT 0,
            FOREIGN KEY (session_id, branch_id, turn_id)
                REFERENCES branch_turns(session_id, branch_id, id) ON DELETE CASCADE,
            FOREIGN KEY (session_id, branch_id, event_id)
                REFERENCES branch_timeline(session_id, branch_id, id) ON DELETE CASCADE
        );
        CREATE INDEX IF NOT EXISTS branch_turn_activities_turn
            ON branch_turn_activities(session_id, branch_id, turn_id, id);

        CREATE TABLE IF NOT EXISTS turns (
            id INTEGER NOT NULL,
            session_id TEXT NOT NULL REFERENCES sessions(id) ON DELETE CASCADE,
            user_prompt TEXT,
            final_response TEXT,
            timestamp INTEGER,
            completed INTEGER NOT NULL DEFAULT 0,
            PRIMARY KEY (session_id, id)
        );

        CREATE TABLE IF NOT EXISTS turn_activities (
            id INTEGER PRIMARY KEY AUTOINCREMENT,
            session_id TEXT NOT NULL,
            turn_id INTEGER NOT NULL,
            event_id INTEGER NOT NULL,
            kind TEXT NOT NULL,
            role TEXT,
            content TEXT NOT NULL,
            timestamp INTEGER,
            tool_name TEXT,
            tool_use_id TEXT,
            parent_tool_use_id TEXT,
            collapsed INTEGER NOT NULL,
            final_response INTEGER NOT NULL DEFAULT 0,
            FOREIGN KEY(session_id, turn_id) REFERENCES turns(session_id, id) ON DELETE CASCADE
        );

        CREATE TABLE IF NOT EXISTS diagnostics (
            id INTEGER PRIMARY KEY AUTOINCREMENT,
            session_id TEXT NOT NULL REFERENCES sessions(id) ON DELETE CASCADE,
            line INTEGER NOT NULL,
            code TEXT NOT NULL
        );

        CREATE TABLE IF NOT EXISTS source_files (
            path TEXT PRIMARY KEY,
            session_id TEXT NOT NULL REFERENCES sessions(id) ON DELETE CASCADE,
            size INTEGER NOT NULL,
            mtime INTEGER NOT NULL,
            hash TEXT NOT NULL
        );

        CREATE TABLE IF NOT EXISTS session_cwds (
            session_id TEXT NOT NULL REFERENCES sessions(id) ON DELETE CASCADE,
            cwd TEXT NOT NULL,
            first_sequence INTEGER NOT NULL,
            last_sequence INTEGER NOT NULL,
            first_timestamp INTEGER,
            last_timestamp INTEGER,
            resume INTEGER NOT NULL DEFAULT 0,
            PRIMARY KEY (session_id, cwd)
        );
        CREATE TABLE IF NOT EXISTS project_overrides (
            provider_id TEXT NOT NULL DEFAULT 'claude',
            workspace_id TEXT NOT NULL,
            alias TEXT
            ,PRIMARY KEY (provider_id, workspace_id)
        );
        CREATE TABLE IF NOT EXISTS scan_runs (
            id INTEGER PRIMARY KEY AUTOINCREMENT,
            started_at INTEGER NOT NULL,
            ended_at INTEGER,
            trigger TEXT NOT NULL,
            outcome TEXT NOT NULL,
            committed INTEGER NOT NULL DEFAULT 0,
            sessions INTEGER NOT NULL DEFAULT 0,
            diagnostics INTEGER NOT NULL DEFAULT 0
        );
        CREATE TABLE IF NOT EXISTS scan_diagnostic_counts (
            scan_run_id INTEGER NOT NULL REFERENCES scan_runs(id) ON DELETE CASCADE,
            code TEXT NOT NULL,
            count INTEGER NOT NULL,
            PRIMARY KEY (scan_run_id, code)
        );

        CREATE TABLE IF NOT EXISTS claude_settings (
            id INTEGER PRIMARY KEY CHECK (id = 1),
            executable_override TEXT,
            dangerously_skip_permissions INTEGER NOT NULL DEFAULT 0,
            source_root TEXT,
            scan_interval_seconds INTEGER NOT NULL DEFAULT 0
        );
        INSERT OR IGNORE INTO claude_settings (id, executable_override, dangerously_skip_permissions)
        VALUES (1, NULL, 0);
        ",
    )?;

    let version = connection.query_row("PRAGMA user_version", [], |row| row.get::<_, i64>(0))?;
    if version > SCHEMA_VERSION {
        return Err(AppError::Message(format!(
            "database schema version {version} is newer than supported {SCHEMA_VERSION}"
        )));
    }
    let has_provider_id = connection.query_row(
        "SELECT EXISTS(SELECT 1 FROM pragma_table_info('sessions') WHERE name = 'provider_id')",
        [],
        |row| row.get::<_, bool>(0),
    )?;
    if !has_provider_id {
        connection.execute(
            "ALTER TABLE sessions ADD COLUMN provider_id TEXT NOT NULL DEFAULT 'claude'",
            [],
        )?;
    }
    let has_native_session_id = connection.query_row(
        "SELECT EXISTS(SELECT 1 FROM pragma_table_info('sessions') WHERE name = 'native_session_id')",
        [],
        |row| row.get::<_, bool>(0),
    )?;
    if !has_native_session_id {
        connection.execute("ALTER TABLE sessions ADD COLUMN native_session_id TEXT", [])?;
    }
    for (name, definition) in [
        ("local_title", "TEXT"),
        ("hidden", "INTEGER NOT NULL DEFAULT 0"),
        ("pinned", "INTEGER NOT NULL DEFAULT 0"),
        ("last_used_at", "INTEGER"),
    ] {
        let exists = connection.query_row(
            &format!(
                "SELECT EXISTS(SELECT 1 FROM pragma_table_info('sessions') WHERE name = '{name}')"
            ),
            [],
            |row| row.get::<_, bool>(0),
        )?;
        if !exists {
            connection.execute(
                &format!("ALTER TABLE sessions ADD COLUMN {name} {definition}"),
                [],
            )?;
        }
    }

    for (name, definition) in [
        ("workspace_id", "TEXT"),
        ("project_path", "TEXT"),
        ("worktree_path", "TEXT"),
    ] {
        let exists = connection.query_row(
            &format!(
                "SELECT EXISTS(SELECT 1 FROM pragma_table_info('sessions') WHERE name = '{name}')"
            ),
            [],
            |row| row.get::<_, bool>(0),
        )?;
        if !exists {
            connection.execute(
                &format!("ALTER TABLE sessions ADD COLUMN {name} {definition}"),
                [],
            )?;
        }
    }
    connection.execute_batch(
        "CREATE TABLE IF NOT EXISTS session_cwds (
            session_id TEXT NOT NULL REFERENCES sessions(id) ON DELETE CASCADE,
            cwd TEXT NOT NULL, first_sequence INTEGER NOT NULL, last_sequence INTEGER NOT NULL,
            first_timestamp INTEGER, last_timestamp INTEGER, resume INTEGER NOT NULL DEFAULT 0,
            PRIMARY KEY(session_id,cwd));
         CREATE TABLE IF NOT EXISTS project_overrides (provider_id TEXT NOT NULL DEFAULT 'claude', workspace_id TEXT NOT NULL, alias TEXT, PRIMARY KEY(provider_id,workspace_id));
         CREATE TABLE IF NOT EXISTS scan_runs (
            id INTEGER PRIMARY KEY AUTOINCREMENT, started_at INTEGER NOT NULL, ended_at INTEGER,
            trigger TEXT NOT NULL, outcome TEXT NOT NULL, committed INTEGER NOT NULL DEFAULT 0,
            sessions INTEGER NOT NULL DEFAULT 0, diagnostics INTEGER NOT NULL DEFAULT 0);
         CREATE TABLE IF NOT EXISTS scan_diagnostic_counts (
            scan_run_id INTEGER NOT NULL REFERENCES scan_runs(id) ON DELETE CASCADE,
            code TEXT NOT NULL, count INTEGER NOT NULL, PRIMARY KEY(scan_run_id,code));"
    )?;
    for (name, definition) in [
        ("source_root", "TEXT"),
        ("scan_interval_seconds", "INTEGER NOT NULL DEFAULT 0"),
    ] {
        let exists = connection.query_row(
            &format!("SELECT EXISTS(SELECT 1 FROM pragma_table_info('claude_settings') WHERE name = '{name}')"),
            [], |row| row.get::<_, bool>(0))?;
        if !exists {
            connection.execute(
                &format!("ALTER TABLE claude_settings ADD COLUMN {name} {definition}"),
                [],
            )?;
        }
    }
    for (name, definition) in [
        ("uuid", "TEXT"),
        ("parent_uuid", "TEXT"),
        ("logical_parent_uuid", "TEXT"),
        ("message_id", "TEXT"),
        ("parent_tool_use_id", "TEXT"),
        ("tool_use_id", "TEXT"),
        ("sequence", "INTEGER NOT NULL DEFAULT 0"),
        ("is_sidechain", "INTEGER NOT NULL DEFAULT 0"),
        ("is_meta", "INTEGER NOT NULL DEFAULT 0"),
        ("turn_id", "INTEGER"),
        ("final_response", "INTEGER NOT NULL DEFAULT 0"),
        ("compact_boundary", "INTEGER NOT NULL DEFAULT 0"),
        ("compact_preserved", "TEXT NOT NULL DEFAULT '[]'"),
    ] {
        let exists = connection.query_row(
            &format!(
                "SELECT EXISTS(SELECT 1 FROM pragma_table_info('timeline') WHERE name = '{name}')"
            ),
            [],
            |row| row.get::<_, bool>(0),
        )?;
        if !exists {
            connection.execute(
                &format!("ALTER TABLE timeline ADD COLUMN {name} {definition}"),
                [],
            )?;
        }
    }

    if version < 2 {
        // Search content is derived and can always be rebuilt from sessions.
        connection.execute_batch(
            "
            DROP TABLE IF EXISTS search_chunks;
            CREATE VIRTUAL TABLE search_chunks USING fts5(
                session_id UNINDEXED,
                event_id UNINDEXED,
                field UNINDEXED,
                content,
                tokenize = 'trigram'
            );
            ",
        )?;
    } else {
        connection.execute_batch(
            "
            CREATE VIRTUAL TABLE IF NOT EXISTS search_chunks USING fts5(
                session_id UNINDEXED,
                event_id UNINDEXED,
                field UNINDEXED,
                content,
                tokenize = 'trigram'
            );
            ",
        )?;
    }
    // Branch read-model tables are additive and rebuildable.  Keep this
    // CREATE IF NOT EXISTS path for databases upgraded from v5 as well as
    // databases created by the initial schema batch above.
    if version < 6 {
        connection.execute_batch(
            "
            CREATE TABLE IF NOT EXISTS session_branches (
                session_id TEXT NOT NULL REFERENCES sessions(id) ON DELETE CASCADE,
                id TEXT NOT NULL,
                label TEXT NOT NULL,
                kind TEXT NOT NULL,
                root_uuid TEXT,
                leaf_uuid TEXT,
                fork_point_uuid TEXT,
                is_active INTEGER NOT NULL DEFAULT 0,
                event_count INTEGER NOT NULL DEFAULT 0,
                turn_count INTEGER NOT NULL DEFAULT 0,
                started_at INTEGER,
                ended_at INTEGER,
                compacted INTEGER NOT NULL DEFAULT 0,
                tool_stats TEXT NOT NULL DEFAULT '[]',
                turn_insights TEXT NOT NULL DEFAULT '[]',
                PRIMARY KEY (session_id, id)
            );
            CREATE INDEX IF NOT EXISTS session_branches_session_active
                ON session_branches(session_id, is_active, id);
            CREATE TABLE IF NOT EXISTS branch_timeline (
                id INTEGER PRIMARY KEY AUTOINCREMENT,
                session_id TEXT NOT NULL,
                branch_id TEXT NOT NULL,
                kind TEXT NOT NULL,
                role TEXT,
                content TEXT NOT NULL,
                timestamp INTEGER,
                tool_name TEXT,
                collapsed INTEGER NOT NULL,
                uuid TEXT,
                parent_uuid TEXT,
                logical_parent_uuid TEXT,
                message_id TEXT,
                parent_tool_use_id TEXT,
                tool_use_id TEXT,
                sequence INTEGER NOT NULL DEFAULT 0,
                is_sidechain INTEGER NOT NULL DEFAULT 0,
                is_meta INTEGER NOT NULL DEFAULT 0,
                turn_id INTEGER,
                final_response INTEGER NOT NULL DEFAULT 0,
                compact_boundary INTEGER NOT NULL DEFAULT 0,
                compact_preserved TEXT NOT NULL DEFAULT '[]',
                FOREIGN KEY (session_id, branch_id)
                    REFERENCES session_branches(session_id, id) ON DELETE CASCADE,
                UNIQUE (session_id, branch_id, id)
            );
            CREATE INDEX IF NOT EXISTS branch_timeline_session_branch
                ON branch_timeline(session_id, branch_id, id);
            CREATE TABLE IF NOT EXISTS branch_turns (
                id INTEGER NOT NULL,
                session_id TEXT NOT NULL,
                branch_id TEXT NOT NULL,
                user_prompt TEXT,
                final_response TEXT,
                timestamp INTEGER,
                completed INTEGER NOT NULL DEFAULT 0,
                PRIMARY KEY (session_id, branch_id, id),
                FOREIGN KEY (session_id, branch_id)
                    REFERENCES session_branches(session_id, id) ON DELETE CASCADE
            );
            CREATE TABLE IF NOT EXISTS branch_turn_activities (
                id INTEGER PRIMARY KEY AUTOINCREMENT,
                session_id TEXT NOT NULL,
                branch_id TEXT NOT NULL,
                turn_id INTEGER NOT NULL,
                event_id INTEGER NOT NULL,
                kind TEXT NOT NULL,
                role TEXT,
                content TEXT NOT NULL,
                timestamp INTEGER,
                tool_name TEXT,
                tool_use_id TEXT,
                parent_tool_use_id TEXT,
                collapsed INTEGER NOT NULL,
                final_response INTEGER NOT NULL DEFAULT 0,
                FOREIGN KEY (session_id, branch_id, turn_id)
                    REFERENCES branch_turns(session_id, branch_id, id) ON DELETE CASCADE,
                FOREIGN KEY (session_id, branch_id, event_id)
                    REFERENCES branch_timeline(session_id, branch_id, id) ON DELETE CASCADE
            );
            CREATE INDEX IF NOT EXISTS branch_turn_activities_turn
                ON branch_turn_activities(session_id, branch_id, turn_id, id);
            ",
        )?;
    }
    for (name, definition) in [
        ("tool_stats", "TEXT NOT NULL DEFAULT '[]'"),
        ("turn_insights", "TEXT NOT NULL DEFAULT '[]'"),
    ] {
        let exists = connection.query_row(
            &format!(
                "SELECT EXISTS(SELECT 1 FROM pragma_table_info('session_branches') WHERE name = '{name}')"
            ),
            [],
            |row| row.get::<_, bool>(0),
        )?;
        if !exists {
            connection.execute(
                &format!("ALTER TABLE session_branches ADD COLUMN {name} {definition}"),
                [],
            )?;
        }
    }
    let relation_has_provider = connection.query_row(
        "SELECT EXISTS(SELECT 1 FROM pragma_table_info('session_relations') WHERE name = 'provider_id')",
        [],
        |row| row.get::<_, bool>(0),
    )?;
    if !relation_has_provider {
        // Keep the complete v7→v8 conversion atomic: table rebuild, indexes,
        // and user_version must commit or roll back together.
        connection.execute_batch("SAVEPOINT migrate_relations_v7_v8;")?;
        let migration = (|| -> Result<()> {
            connection.execute_batch(
                "
                DROP INDEX IF EXISTS session_relations_parent;
                DROP INDEX IF EXISTS session_relations_child;
                ALTER TABLE session_relations RENAME TO session_relations_v7;
                CREATE TABLE session_relations (
                    provider_id TEXT NOT NULL DEFAULT 'claude',
                    parent_session_id TEXT NOT NULL,
                    child_session_id TEXT NOT NULL,
                    relation_type TEXT NOT NULL CHECK (relation_type = 'fork'),
                    created_at INTEGER NOT NULL,
                    status TEXT NOT NULL DEFAULT 'pending'
                        CHECK (status IN ('pending', 'indexed', 'source_removed', 'unknown')),
                    PRIMARY KEY (provider_id, parent_session_id, child_session_id, relation_type)
                );
                INSERT INTO session_relations
                    (provider_id, parent_session_id, child_session_id, relation_type, created_at, status)
                    SELECT 'claude', parent_session_id, child_session_id, relation_type, created_at, status
                    FROM session_relations_v7;
                DROP TABLE session_relations_v7;
                CREATE INDEX session_relations_parent
                    ON session_relations(provider_id, parent_session_id);
                CREATE INDEX session_relations_child
                    ON session_relations(provider_id, child_session_id);
                ",
            )?;
            connection.pragma_update(None, "user_version", SCHEMA_VERSION)?;
            Ok(())
        })();
        if let Err(error) = migration {
            let _ = connection.execute_batch(
                "ROLLBACK TO SAVEPOINT migrate_relations_v7_v8; RELEASE SAVEPOINT migrate_relations_v7_v8;",
            );
            return Err(error);
        }
        connection.execute_batch("RELEASE SAVEPOINT migrate_relations_v7_v8;")?;
    } else {
        connection.execute_batch(
            "
            CREATE INDEX IF NOT EXISTS session_relations_parent
                ON session_relations(provider_id, parent_session_id);
            CREATE INDEX IF NOT EXISTS session_relations_child
                ON session_relations(provider_id, child_session_id);
            ",
        )?;
        connection.pragma_update(None, "user_version", SCHEMA_VERSION)?;
    }

    connection.execute_batch("RELEASE SAVEPOINT migrate_v8_v9;")?;
    migration_guard.committed = true;
    Ok(())
}

struct MigrationGuard<'a> {
    connection: &'a Connection,
    committed: bool,
}
impl Drop for MigrationGuard<'_> {
    fn drop(&mut self) {
        if !self.committed {
            let _ = self.connection.execute_batch(
                "ROLLBACK TO SAVEPOINT migrate_v8_v9; RELEASE SAVEPOINT migrate_v8_v9;",
            );
        }
    }
}

pub fn claude_settings(connection: &Connection) -> Result<ClaudeSettings> {
    connection
        .query_row(
            "SELECT executable_override, dangerously_skip_permissions FROM claude_settings WHERE id = 1",
            [],
            |row| {
                Ok(ClaudeSettings {
                    executable_override: row.get(0)?,
                    dangerously_skip_permissions: row.get::<_, i64>(1)? != 0,
                })
            },
        )
        .map_err(Into::into)
}

pub fn claude_scan_settings(connection: &Connection) -> Result<ClaudeScanSettings> {
    connection
        .query_row(
            "SELECT source_root, scan_interval_seconds FROM claude_settings WHERE id = 1",
            [],
            |row| {
                Ok(ClaudeScanSettings {
                    source_root: row.get(0)?,
                    scan_interval_seconds: row.get(1)?,
                })
            },
        )
        .map_err(Into::into)
}

pub fn update_claude_scan_settings(
    connection: &Connection,
    settings: &ClaudeScanSettings,
) -> Result<()> {
    connection.execute(
        "UPDATE claude_settings SET source_root = ?, scan_interval_seconds = ? WHERE id = 1",
        params![settings.source_root, settings.scan_interval_seconds],
    )?;
    Ok(())
}

pub fn update_claude_settings(connection: &Connection, settings: &ClaudeSettings) -> Result<()> {
    connection.execute(
        "UPDATE claude_settings SET executable_override = ?, dangerously_skip_permissions = ? WHERE id = 1",
        params![settings.executable_override, settings.dangerously_skip_permissions],
    )?;
    Ok(())
}

pub fn index(
    connection: &mut Connection,
    provider_id: &str,
    parsed: &[ParsedSession],
) -> Result<usize> {
    index_with_paths(connection, provider_id, parsed, None, None)
}

/// Apply only parsed new/changed projections while reconciling removals from
/// the complete discovered path set. Unchanged sessions are deliberately not
/// touched, preserving row ids and local metadata.
pub fn index_incremental(
    connection: &mut Connection,
    provider_id: &str,
    parsed: &[ParsedSession],
    discovered_paths: &[PathBuf],
) -> Result<usize> {
    let paths = discovered_paths.iter().cloned().collect::<HashSet<_>>();
    index_with_paths(connection, provider_id, parsed, Some(&paths), None)
}

pub fn index_incremental_and_set_root(
    connection: &mut Connection,
    provider_id: &str,
    parsed: &[ParsedSession],
    discovered_paths: &[PathBuf],
    source_root: Option<&str>,
) -> Result<usize> {
    let paths = discovered_paths.iter().cloned().collect::<HashSet<_>>();
    index_with_paths(
        connection,
        provider_id,
        parsed,
        Some(&paths),
        Some(source_root.unwrap_or("")),
    )
}

fn index_with_paths(
    connection: &mut Connection,
    provider_id: &str,
    parsed: &[ParsedSession],
    discovered_paths: Option<&HashSet<PathBuf>>,
    source_root: Option<&str>,
) -> Result<usize> {
    let mut ids = HashMap::<String, PathBuf>::new();
    let mut paths = HashMap::<PathBuf, String>::new();
    let parsed_paths = parsed
        .iter()
        .map(|session| session.source_path.clone())
        .collect::<HashSet<_>>();
    let protected_paths = discovered_paths.unwrap_or(&parsed_paths);
    for session in parsed {
        if session.summary.provider_id != provider_id {
            continue;
        }
        let path = session.source_path.clone();
        if ids
            .insert(session.summary.id.clone(), path.clone())
            .is_some()
            || paths.insert(path, session.summary.id.clone()).is_some()
        {
            return Err(AppError::Message(
                "duplicate source session identity".into(),
            ));
        }
        if let Some(native) = &session.summary.native_session_id {
            let key = format!("native:{native}");
            if ids.insert(key, session.source_path.clone()).is_some() {
                return Err(AppError::Message(
                    "duplicate native session identity".into(),
                ));
            }
        }
        let existing = connection
            .query_row(
                "SELECT session_id FROM source_files WHERE path = ?",
                [session.source_path.to_string_lossy().as_ref()],
                |row| row.get::<_, String>(0),
            )
            .optional()?;
        if existing.is_some_and(|existing| existing != session.summary.id) {
            return Err(AppError::Message(
                "source path has conflicting session identity".into(),
            ));
        }

        let mut statement = connection.prepare(
            "SELECT sf.path FROM source_files sf
             JOIN sessions s ON s.id = sf.session_id
             WHERE s.provider_id = ? AND s.id = ? AND sf.path <> ?",
        )?;
        let existing_paths = statement
            .query_map(
                params![
                    provider_id,
                    session.summary.id,
                    session.source_path.to_string_lossy().as_ref()
                ],
                |row| row.get::<_, String>(0),
            )?
            .collect::<rusqlite::Result<Vec<_>>>()?;
        if existing_paths
            .iter()
            .map(PathBuf::from)
            .any(|path| protected_paths.contains(&path))
        {
            return Err(AppError::Message(
                "session identity is already mapped to another discovered source".into(),
            ));
        }

        if let Some(native) = &session.summary.native_session_id {
            let mut statement = connection.prepare(
                "SELECT sf.path FROM source_files sf
                 JOIN sessions s ON s.id = sf.session_id
                 WHERE s.provider_id = ? AND s.native_session_id = ? AND s.id <> ?",
            )?;
            let existing_paths = statement
                .query_map(params![provider_id, native, session.summary.id], |row| {
                    row.get::<_, String>(0)
                })?
                .collect::<rusqlite::Result<Vec<_>>>()?;
            if existing_paths
                .iter()
                .map(PathBuf::from)
                .any(|path| protected_paths.contains(&path))
            {
                return Err(AppError::Message(
                    "native session identity is already mapped to another discovered source".into(),
                ));
            }
        }
    }
    if parsed
        .iter()
        .any(|session| session.summary.provider_id != provider_id)
    {
        return Err(AppError::Message(
            "provider reconciliation received a foreign session".to_owned(),
        ));
    }

    let transaction = connection.transaction()?;
    let mut seen = HashSet::with_capacity(parsed.len());

    for session in parsed {
        let summary = &session.summary;
        let source_title = if summary.source_title.trim().is_empty() {
            &summary.title
        } else {
            &summary.source_title
        };
        seen.insert(summary.id.clone());
        transaction.execute(
            "
            INSERT INTO sessions (
                id, provider_id, native_session_id, project_id, workspace_id, project_path, worktree_path, title, started_at, ended_at, branch,
                first_prompt, last_prompt, cwd, models, tool_count,
                source_mtime, partial
            ) VALUES (?, ?, ?, ?, ?, ?, ?, ?, ?, ?, ?, ?, ?, ?, ?, ?, ?, ?)
            ON CONFLICT(id) DO UPDATE SET
                provider_id = excluded.provider_id,
                native_session_id = excluded.native_session_id,
                project_id = excluded.project_id,
                workspace_id = excluded.workspace_id,
                project_path = excluded.project_path,
                worktree_path = excluded.worktree_path,
                title = excluded.title,
                started_at = excluded.started_at,
                ended_at = excluded.ended_at,
                branch = excluded.branch,
                first_prompt = excluded.first_prompt,
                last_prompt = excluded.last_prompt,
                cwd = excluded.cwd,
                models = excluded.models,
                tool_count = excluded.tool_count,
                source_mtime = excluded.source_mtime,
                partial = excluded.partial
            ",
            params![
                summary.id,
                summary.provider_id,
                summary.native_session_id,
                summary.project_id,
                if summary.workspace_id.is_empty() { Some(summary.project_id.clone()) } else { Some(summary.workspace_id.clone()) },
                summary.project_path,
                summary.worktree_path,
                source_title,
                summary.started_at,
                summary.ended_at,
                summary.branch,
                summary.first_prompt,
                summary.last_prompt,
                summary.cwd,
                serde_json::to_string(&summary.models)?,
                summary.tool_count,
                summary.source_mtime,
                summary.partial,
            ],
        )?;

        let effective_title: String = transaction.query_row(
            "SELECT COALESCE(local_title, title) FROM sessions WHERE id = ?",
            [&summary.id],
            |row| row.get(0),
        )?;

        transaction.execute(
            "DELETE FROM turn_activities WHERE session_id = ?",
            [&summary.id],
        )?;
        transaction.execute("DELETE FROM turns WHERE session_id = ?", [&summary.id])?;
        transaction.execute("DELETE FROM timeline WHERE session_id = ?", [&summary.id])?;
        transaction.execute(
            "DELETE FROM diagnostics WHERE session_id = ?",
            [&summary.id],
        )?;
        transaction.execute(
            "DELETE FROM search_chunks WHERE session_id = ?",
            [&summary.id],
        )?;
        transaction.execute(
            "DELETE FROM source_files WHERE session_id = ?",
            [&summary.id],
        )?;
        // Branch rows are a complete per-session snapshot.  Removing the
        // roots first lets the composite foreign keys cascade all branch
        // timeline/turn/activity rows before the replacement is inserted.
        transaction.execute(
            "DELETE FROM session_branches WHERE session_id = ?",
            [&summary.id],
        )?;

        insert_search_chunk(&transaction, &summary.id, 0, "title", &effective_title)?;
        insert_search_chunk(
            &transaction,
            &summary.id,
            0,
            "project",
            &project_display_name(&summary.project_id),
        )?;
        if let Some(cwd) = &summary.cwd {
            insert_search_chunk(&transaction, &summary.id, 0, "project", cwd)?;
        }

        let mut event_row_ids = HashMap::with_capacity(session.events.len());
        for event in &session.events {
            transaction.execute(
                "
                INSERT INTO timeline (
                    session_id, kind, role, content, timestamp, tool_name, collapsed,
                    uuid, parent_uuid, logical_parent_uuid, message_id, parent_tool_use_id,
                    tool_use_id, sequence, is_sidechain, is_meta, turn_id, final_response,
                    compact_boundary, compact_preserved
                ) VALUES (?, ?, ?, ?, ?, ?, ?, ?, ?, ?, ?, ?, ?, ?, ?, ?, ?, ?, ?, ?)
                ",
                params![
                    summary.id,
                    event.kind,
                    event.role,
                    event.content,
                    event.timestamp,
                    event.tool_name,
                    event.collapsed,
                    event.uuid,
                    event.parent_uuid,
                    event.logical_parent_uuid,
                    event.message_id,
                    event.parent_tool_use_id,
                    event.tool_use_id,
                    event.sequence,
                    event.is_sidechain,
                    event.is_meta,
                    event.turn_id,
                    event.final_response,
                    event.compact_boundary,
                    serde_json::to_string(&event.compact_preserved_ids)?,
                ],
            )?;
            let event_id = transaction.last_insert_rowid();
            event_row_ids.insert(event.id, event_id);
            if !event.content.is_empty() && matches!(event.kind.as_str(), "user" | "assistant") {
                insert_search_chunk(
                    &transaction,
                    &summary.id,
                    event_id,
                    &event.kind,
                    &event.content,
                )?;
            }
        }

        for turn in &session.turns {
            transaction.execute(
                "INSERT INTO turns (id, session_id, user_prompt, final_response, timestamp, completed) VALUES (?, ?, ?, ?, ?, ?)",
                params![turn.id, summary.id, turn.user_prompt, turn.final_response, turn.timestamp, turn.completed],
            )?;
            for activity in &turn.activities {
                let db_event_id =
                    event_row_ids
                        .get(&activity.event_id)
                        .copied()
                        .ok_or_else(|| {
                            AppError::Message(
                                "turn activity references missing timeline event".to_owned(),
                            )
                        })?;
                transaction.execute(
                    "INSERT INTO turn_activities (session_id, turn_id, event_id, kind, role, content, timestamp, tool_name, tool_use_id, parent_tool_use_id, collapsed, final_response) VALUES (?, ?, ?, ?, ?, ?, ?, ?, ?, ?, ?, ?)",
                    params![summary.id, turn.id, db_event_id, activity.kind, activity.role, activity.content, activity.timestamp, activity.tool_name, activity.tool_use_id, activity.parent_tool_use_id, activity.collapsed, activity.final_response],
                )?;
            }
        }

        for branch in &session.branches {
            let branch_summary = &branch.summary;
            if branch_summary.session_id != summary.id {
                return Err(AppError::Message(
                    "branch summary references a foreign session".to_owned(),
                ));
            }
            let tool_stats = serde_json::to_string(&branch.tool_stats)?;
            let turn_insights = serde_json::to_string(&branch.turn_insights)?;
            transaction.execute(
                "INSERT INTO session_branches (session_id, id, label, kind, root_uuid, leaf_uuid, fork_point_uuid, is_active, event_count, turn_count, started_at, ended_at, compacted, tool_stats, turn_insights) VALUES (?, ?, ?, ?, ?, ?, ?, ?, ?, ?, ?, ?, ?, ?, ?)",
                params![
                    summary.id,
                    branch_summary.id,
                    branch_summary.label,
                    branch_summary.kind,
                    branch_summary.root_uuid,
                    branch_summary.leaf_uuid,
                    branch_summary.fork_point_uuid,
                    branch_summary.is_active,
                    branch_summary.event_count as i64,
                    branch_summary.turn_count as i64,
                    branch_summary.started_at,
                    branch_summary.ended_at,
                    branch_summary.compacted,
                    tool_stats,
                    turn_insights,
                ],
            )?;

            let mut branch_event_row_ids = HashMap::with_capacity(branch.events.len());
            for event in &branch.events {
                transaction.execute(
                    "INSERT INTO branch_timeline (session_id, branch_id, kind, role, content, timestamp, tool_name, collapsed, uuid, parent_uuid, logical_parent_uuid, message_id, parent_tool_use_id, tool_use_id, sequence, is_sidechain, is_meta, turn_id, final_response, compact_boundary, compact_preserved) VALUES (?, ?, ?, ?, ?, ?, ?, ?, ?, ?, ?, ?, ?, ?, ?, ?, ?, ?, ?, ?, ?)",
                    params![
                        summary.id,
                        branch_summary.id,
                        event.kind,
                        event.role,
                        event.content,
                        event.timestamp,
                        event.tool_name,
                        event.collapsed,
                        event.uuid,
                        event.parent_uuid,
                        event.logical_parent_uuid,
                        event.message_id,
                        event.parent_tool_use_id,
                        event.tool_use_id,
                        event.sequence,
                        event.is_sidechain,
                        event.is_meta,
                        event.turn_id,
                        event.final_response,
                        event.compact_boundary,
                        serde_json::to_string(&event.compact_preserved_ids)?,
                    ],
                )?;
                branch_event_row_ids.insert(event.id, transaction.last_insert_rowid());
            }

            for turn in &branch.turns {
                transaction.execute(
                    "INSERT INTO branch_turns (id, session_id, branch_id, user_prompt, final_response, timestamp, completed) VALUES (?, ?, ?, ?, ?, ?, ?)",
                    params![
                        turn.id,
                        summary.id,
                        branch_summary.id,
                        turn.user_prompt,
                        turn.final_response,
                        turn.timestamp,
                        turn.completed,
                    ],
                )?;
                for activity in &turn.activities {
                    let db_event_id = branch_event_row_ids
                        .get(&activity.event_id)
                        .copied()
                        .ok_or_else(|| {
                            AppError::Message(
                                "branch turn activity references missing timeline event".to_owned(),
                            )
                        })?;
                    transaction.execute(
                        "INSERT INTO branch_turn_activities (session_id, branch_id, turn_id, event_id, kind, role, content, timestamp, tool_name, tool_use_id, parent_tool_use_id, collapsed, final_response) VALUES (?, ?, ?, ?, ?, ?, ?, ?, ?, ?, ?, ?, ?)",
                        params![
                            summary.id,
                            branch_summary.id,
                            turn.id,
                            db_event_id,
                            activity.kind,
                            activity.role,
                            activity.content,
                            activity.timestamp,
                            activity.tool_name,
                            activity.tool_use_id,
                            activity.parent_tool_use_id,
                            activity.collapsed,
                            activity.final_response,
                        ],
                    )?;
                }
            }
        }

        for diagnostic in &session.diagnostics {
            transaction.execute(
                "INSERT INTO diagnostics (session_id, line, code) VALUES (?, ?, ?)",
                params![summary.id, diagnostic.line, diagnostic.code],
            )?;
        }

        transaction.execute(
            "
            INSERT INTO source_files (path, session_id, size, mtime, hash)
            VALUES (?, ?, ?, ?, ?)
            ON CONFLICT(path) DO UPDATE SET
                session_id = excluded.session_id,
                size = excluded.size,
                mtime = excluded.mtime,
                hash = excluded.hash
            ",
            params![
                session.source_path.to_string_lossy(),
                summary.id,
                session.source_size,
                summary.source_mtime,
                session.source_hash,
            ],
        )?;
        transaction.execute(
            "DELETE FROM session_cwds WHERE session_id = ?",
            [&summary.id],
        )?;
        for cwd in &session.cwd_history {
            transaction.execute(
                "INSERT INTO session_cwds (session_id,cwd,first_sequence,last_sequence,first_timestamp,last_timestamp,resume) VALUES (?,?,?,?,?,?,?)",
                params![summary.id, cwd.cwd, cwd.first_sequence, cwd.last_sequence, cwd.first_timestamp, cwd.last_timestamp, cwd.resume],
            )?;
        }
    }

    let indexed_ids = {
        let mut statement = transaction.prepare("SELECT s.id, sf.path FROM sessions s LEFT JOIN source_files sf ON sf.session_id = s.id WHERE s.provider_id = ?")?;
        let rows = statement
            .query_map([provider_id], |row| {
                Ok((row.get::<_, String>(0)?, row.get::<_, Option<String>>(1)?))
            })?
            .collect::<rusqlite::Result<Vec<_>>>()?;
        rows
    };
    let mut removed_sessions = 0;
    for (id, source_path) in indexed_ids {
        let keep = discovered_paths
            .map(|paths| {
                source_path
                    .as_ref()
                    .is_some_and(|path| paths.contains(&PathBuf::from(path)))
            })
            .unwrap_or_else(|| seen.contains(&id));
        if !keep {
            removed_sessions += transaction.execute("DELETE FROM sessions WHERE id = ?", [&id])?;
        }
    }

    // Relations are explicit local metadata and intentionally survive source
    // reconciliation. A child that was observed is indexed; if it was later
    // removed after having been indexed, preserve the edge as source_removed.
    transaction.execute(
        "UPDATE session_relations
         SET status = CASE
             WHEN EXISTS (SELECT 1 FROM sessions WHERE sessions.id = session_relations.child_session_id
                 AND sessions.provider_id = session_relations.provider_id)
                 THEN 'indexed'
             WHEN status = 'indexed' THEN 'source_removed'
             WHEN status IN ('pending', 'unknown') THEN 'unknown'
             WHEN status = 'source_removed' THEN 'source_removed'
             ELSE 'unknown'
         END
         WHERE provider_id = ?",
        [provider_id],
    )?;

    if let Some(source_root) = source_root {
        if source_root.is_empty() {
            transaction.execute(
                "UPDATE claude_settings SET source_root = NULL WHERE id = 1",
                [],
            )?;
        } else {
            transaction.execute(
                "UPDATE claude_settings SET source_root = ? WHERE id = 1",
                [source_root],
            )?;
        }
    }
    transaction.commit()?;
    Ok(removed_sessions)
}

/// Current source manifest, restricted to one provider.  The manifest is
/// derived metadata and is safe to use for incremental scan planning.
pub fn source_manifest(
    connection: &Connection,
    provider_id: &str,
) -> Result<HashMap<PathBuf, crate::scanner::SourceFingerprint>> {
    let mut statement = connection.prepare(
        "SELECT sf.path, sf.size, sf.mtime, sf.hash FROM source_files sf
         JOIN sessions s ON s.id = sf.session_id WHERE s.provider_id = ?",
    )?;
    let rows = statement
        .query_map([provider_id], |row| {
            Ok(crate::scanner::SourceFingerprint {
                path: PathBuf::from(row.get::<_, String>(0)?),
                size: row.get(1)?,
                mtime: row.get(2)?,
                hash: row.get(3)?,
                #[cfg(unix)]
                dev: 0,
                #[cfg(unix)]
                ino: 0,
            })
        })?
        .collect::<rusqlite::Result<Vec<_>>>()?;
    Ok(rows.into_iter().map(|fp| (fp.path.clone(), fp)).collect())
}

fn insert_search_chunk(
    connection: &Connection,
    session_id: &str,
    event_id: i64,
    field: &str,
    content: &str,
) -> Result<()> {
    if !content.trim().is_empty() {
        connection.execute(
            "INSERT INTO search_chunks (session_id, event_id, field, content) VALUES (?, ?, ?, ?)",
            params![session_id, event_id, field, content],
        )?;
    }
    Ok(())
}

fn decode_summary(row: &rusqlite::Row<'_>) -> rusqlite::Result<SessionSummary> {
    let source_title: String = row.get(4)?;
    let local_title: Option<String> = row.get(5)?;
    let models: String = row.get(15)?;
    Ok(SessionSummary {
        id: row.get(0)?,
        provider_id: row.get(1)?,
        native_session_id: row.get(2)?,
        project_id: row.get(3)?,
        title: local_title.unwrap_or_else(|| source_title.clone()),
        source_title,
        hidden: row.get::<_, i64>(6)? != 0,
        pinned: row.get::<_, i64>(7)? != 0,
        last_used_at: row.get(8)?,
        started_at: row.get(9)?,
        ended_at: row.get(10)?,
        branch: row.get(11)?,
        first_prompt: row.get(12)?,
        last_prompt: row.get(13)?,
        cwd: row.get(14)?,
        models: serde_json::from_str(&models).unwrap_or_default(),
        tool_count: row.get(16)?,
        source_mtime: row.get(17)?,
        partial: row.get::<_, i64>(18)? != 0,
        workspace_id: row
            .get::<_, Option<String>>(19)?
            .unwrap_or_else(|| row.get::<_, String>(3).unwrap_or_default()),
        project_path: row.get(20)?,
        worktree_path: row.get(21)?,
    })
}

fn decode_branch_summary(row: &rusqlite::Row<'_>) -> rusqlite::Result<BranchSummary> {
    Ok(BranchSummary {
        session_id: row.get(0)?,
        id: row.get(1)?,
        label: row.get(2)?,
        kind: row.get(3)?,
        root_uuid: row.get(4)?,
        leaf_uuid: row.get(5)?,
        fork_point_uuid: row.get(6)?,
        is_active: row.get::<_, i64>(7)? != 0,
        event_count: row.get::<_, i64>(8)? as usize,
        turn_count: row.get::<_, i64>(9)? as usize,
        started_at: row.get(10)?,
        ended_at: row.get(11)?,
        compacted: row.get::<_, i64>(12)? != 0,
    })
}

fn branch_summaries(connection: &Connection, session_id: &str) -> Result<Vec<BranchSummary>> {
    let mut statement = connection.prepare(
        "SELECT session_id, id, label, kind, root_uuid, leaf_uuid, fork_point_uuid,
                is_active, event_count, turn_count, started_at, ended_at, compacted
         FROM session_branches WHERE session_id = ? ORDER BY is_active DESC, id",
    )?;
    let rows = statement
        .query_map([session_id], decode_branch_summary)?
        .collect::<rusqlite::Result<Vec<_>>>()?;
    Ok(rows)
}

fn branch_insights(
    connection: &Connection,
    session_id: &str,
    branch_id: Option<&str>,
) -> Result<(Vec<ToolStat>, Vec<TurnInsight>)> {
    let Some(branch_id) = branch_id else {
        return Ok((Vec::new(), Vec::new()));
    };
    let row = connection.query_row(
        "SELECT tool_stats, turn_insights FROM session_branches WHERE session_id = ? AND id = ?",
        params![session_id, branch_id],
        |row| Ok((row.get::<_, String>(0)?, row.get::<_, String>(1)?)),
    )?;
    Ok((
        serde_json::from_str(&row.0).unwrap_or_default(),
        serde_json::from_str(&row.1).unwrap_or_default(),
    ))
}

pub fn register_fork_relation(
    connection: &Connection,
    provider_id: &str,
    parent_session_id: &str,
    child_session_id: &str,
    created_at: i64,
) -> Result<()> {
    if parent_session_id == child_session_id {
        return Err(AppError::Message(
            "fork relation parent and child must differ".into(),
        ));
    }
    connection.execute(
        "INSERT INTO session_relations
            (provider_id, parent_session_id, child_session_id, relation_type, created_at, status)
         VALUES (?, ?, ?, 'fork', ?, CASE WHEN EXISTS
             (SELECT 1 FROM sessions WHERE id = ? AND provider_id = ?) THEN 'indexed' ELSE 'pending' END)
         ON CONFLICT(provider_id, parent_session_id, child_session_id, relation_type)
         DO UPDATE SET created_at = excluded.created_at,
                       status = CASE WHEN EXISTS
                           (SELECT 1 FROM sessions WHERE id = excluded.child_session_id
                               AND provider_id = excluded.provider_id)
                           THEN 'indexed' ELSE session_relations.status END",
        params![
            provider_id,
            parent_session_id,
            child_session_id,
            created_at,
            child_session_id,
            provider_id,
        ],
    )?;
    Ok(())
}

pub fn remove_fork_relation(
    connection: &Connection,
    provider_id: &str,
    parent_session_id: &str,
    child_session_id: &str,
) -> Result<()> {
    connection.execute(
        "DELETE FROM session_relations
         WHERE provider_id = ? AND parent_session_id = ? AND child_session_id = ?
           AND relation_type = 'fork'",
        params![provider_id, parent_session_id, child_session_id],
    )?;
    Ok(())
}

pub fn relations_for_session(
    connection: &Connection,
    session_id: &str,
) -> Result<Vec<SessionRelation>> {
    let mut statement = connection.prepare(
        "SELECT provider_id, parent_session_id, child_session_id, relation_type, created_at, status,
                EXISTS(SELECT 1 FROM sessions p WHERE p.id = parent_session_id AND p.provider_id = session_relations.provider_id),
                EXISTS(SELECT 1 FROM sessions c WHERE c.id = child_session_id AND c.provider_id = session_relations.provider_id)
         FROM session_relations
         WHERE provider_id = (SELECT provider_id FROM sessions WHERE id = ? LIMIT 1)
           AND (parent_session_id = ? OR child_session_id = ?)
         ORDER BY created_at, parent_session_id, child_session_id",
    )?;
    let rows = statement
        .query_map(params![session_id, session_id, session_id], |row| {
            Ok(SessionRelation {
                provider_id: row.get(0)?,
                parent_session_id: row.get(1)?,
                child_session_id: row.get(2)?,
                relation_type: row.get(3)?,
                created_at: row.get(4)?,
                status: row.get(5)?,
                parent_present: row.get(6)?,
                child_present: row.get(7)?,
            })
        })?
        .collect::<rusqlite::Result<Vec<_>>>()?;
    Ok(rows)
}

pub fn session_summary(connection: &Connection, id: &str) -> Result<SessionSummary> {
    let summary = connection.query_row(
        "
            SELECT id, provider_id, native_session_id, project_id, title, local_title, hidden, pinned,
                   last_used_at, started_at, ended_at, branch, first_prompt, last_prompt, cwd, models, tool_count,
                   source_mtime, partial, workspace_id, project_path, worktree_path
            FROM sessions WHERE id = ?
            ",
        [id],
        decode_summary,
    );
    match summary {
        Ok(summary) => Ok(summary),
        Err(rusqlite::Error::QueryReturnedNoRows) => {
            Err(AppError::Message(format!("session not found: {id}")))
        }
        Err(error) => Err(error.into()),
    }
}

/// Return the provider-owned transcript path recorded for an indexed session.
///
/// The source file table is the only trusted mapping from a local session id
/// to a transcript path.  In particular, this helper deliberately does not
/// derive a path from the session cwd or native id.
pub fn source_path_for_session(connection: &Connection, id: &str) -> Result<PathBuf> {
    connection
        .query_row(
            "SELECT path FROM source_files WHERE session_id = ? ORDER BY path LIMIT 1",
            [id],
            |row| row.get::<_, String>(0),
        )
        .map(PathBuf::from)
        .map_err(|error| match error {
            rusqlite::Error::QueryReturnedNoRows => {
                AppError::Message(format!("session source not found: {id}"))
            }
            other => other.into(),
        })
}

fn ensure_session_exists(connection: &Connection, id: &str) -> Result<()> {
    let exists = connection.query_row(
        "SELECT EXISTS(SELECT 1 FROM sessions WHERE id = ?)",
        [id],
        |row| row.get::<_, bool>(0),
    )?;
    if exists {
        Ok(())
    } else {
        Err(AppError::Message(format!("session not found: {id}")))
    }
}

const MAX_LOCAL_TITLE_CHARS: usize = 200;

fn normalize_local_title(title: &str) -> Result<String> {
    let normalized = title.trim();
    if normalized.is_empty() {
        return Err(AppError::Message(
            "session title must not be empty".to_owned(),
        ));
    }
    if normalized.chars().count() > MAX_LOCAL_TITLE_CHARS {
        return Err(AppError::Message(format!(
            "session title must be at most {MAX_LOCAL_TITLE_CHARS} characters"
        )));
    }
    Ok(normalized.to_owned())
}

pub fn hidden_sessions(connection: &Connection) -> Result<Vec<SessionSummary>> {
    let mut statement = connection.prepare(
        "
        SELECT id, provider_id, native_session_id, project_id, title, local_title, hidden, pinned,
               last_used_at, started_at, ended_at, branch, first_prompt, last_prompt, cwd, models, tool_count,
               source_mtime, partial, workspace_id, project_path, worktree_path
        FROM sessions
        WHERE hidden = 1 AND EXISTS (SELECT 1 FROM source_files WHERE source_files.session_id = sessions.id)
        ORDER BY pinned DESC,
                 COALESCE(last_used_at, ended_at, started_at, source_mtime) DESC,
                 id ASC
        ",
    )?;
    let sessions = statement
        .query_map([], decode_summary)?
        .collect::<rusqlite::Result<Vec<_>>>()?;
    Ok(sessions)
}

pub fn set_session_hidden(
    connection: &Connection,
    id: &str,
    hidden: bool,
) -> Result<SessionSummary> {
    ensure_session_exists(connection, id)?;
    connection.execute(
        "UPDATE sessions SET hidden = ? WHERE id = ?",
        params![hidden, id],
    )?;
    session_summary(connection, id)
}

pub fn rename_session(
    connection: &Connection,
    id: &str,
    title: Option<&str>,
) -> Result<SessionSummary> {
    let local_title = title.map(normalize_local_title).transpose()?;
    ensure_session_exists(connection, id)?;
    let transaction = connection.unchecked_transaction()?;
    transaction.execute(
        "UPDATE sessions SET local_title = ? WHERE id = ?",
        params![local_title, id],
    )?;
    let effective_title: String = transaction.query_row(
        "SELECT COALESCE(local_title, title) FROM sessions WHERE id = ?",
        [id],
        |row| row.get(0),
    )?;
    transaction.execute(
        "DELETE FROM search_chunks WHERE session_id = ? AND field = 'title'",
        [id],
    )?;
    insert_search_chunk(&transaction, id, 0, "title", &effective_title)?;
    transaction.commit()?;
    session_summary(connection, id)
}

pub fn set_session_pinned(
    connection: &Connection,
    id: &str,
    pinned: bool,
) -> Result<SessionSummary> {
    ensure_session_exists(connection, id)?;
    connection.execute(
        "UPDATE sessions SET pinned = ? WHERE id = ?",
        params![pinned, id],
    )?;
    session_summary(connection, id)
}

pub fn touch_session(
    connection: &Connection,
    id: &str,
    last_used_at: i64,
) -> Result<SessionSummary> {
    ensure_session_exists(connection, id)?;
    connection.execute(
        "UPDATE sessions SET last_used_at = ? WHERE id = ?",
        params![last_used_at, id],
    )?;
    session_summary(connection, id)
}

pub fn projects(connection: &Connection) -> Result<Vec<Project>> {
    let mut groups = connection.prepare("SELECT provider_id, COALESCE(workspace_id, project_id), MAX(COALESCE(last_used_at, ended_at, started_at, source_mtime)) FROM sessions WHERE hidden = 0 GROUP BY provider_id, COALESCE(workspace_id, project_id) ORDER BY MAX(COALESCE(last_used_at, ended_at, started_at, source_mtime)) DESC, provider_id, COALESCE(workspace_id, project_id)")?;
    let rows = groups
        .query_map([], |row| {
            Ok((
                row.get::<_, String>(0)?,
                row.get::<_, String>(1)?,
                row.get::<_, Option<i64>>(2)?,
            ))
        })?
        .collect::<rusqlite::Result<Vec<_>>>()?;
    let mut projects = Vec::new();
    for (provider, workspace, latest_activity) in rows {
        let mut stmt = connection.prepare("SELECT id, provider_id, native_session_id, project_id, title, local_title, hidden, pinned, last_used_at, started_at, ended_at, branch, first_prompt, last_prompt, cwd, models, tool_count, source_mtime, partial, workspace_id, project_path, worktree_path FROM sessions WHERE hidden = 0 AND provider_id = ? AND COALESCE(workspace_id, project_id) = ? ORDER BY pinned DESC, COALESCE(last_used_at, ended_at, started_at, source_mtime) DESC, id")?;
        let sessions = stmt
            .query_map(params![provider, workspace], decode_summary)?
            .collect::<rusqlite::Result<Vec<_>>>()?;
        let path = sessions
            .iter()
            .find_map(|s| s.project_path.clone().or_else(|| s.cwd.clone()))
            .unwrap_or_else(|| workspace.clone());
        let cwd_paths = distinct_session_cwds(connection, &sessions)?;
        let worktree_paths = sessions
            .iter()
            .filter_map(|s| s.worktree_path.clone())
            .collect::<HashSet<_>>()
            .into_iter()
            .collect::<Vec<_>>();
        let mut worktree_paths = worktree_paths;
        worktree_paths.sort();
        let alias = connection
            .query_row(
                "SELECT alias FROM project_overrides WHERE provider_id = ? AND workspace_id = ?",
                params![provider, workspace],
                |row| row.get(0),
            )
            .optional()?
            .flatten();
        projects.push(Project {
            id: workspace.clone(),
            workspace_id: workspace,
            name: alias.clone().unwrap_or_else(|| project_display_name(&path)),
            path,
            alias,
            cwd_paths,
            worktree_paths,
            latest_activity,
            sessions,
        });
    }
    Ok(projects)
}

fn distinct_session_cwds(
    connection: &Connection,
    sessions: &[SessionSummary],
) -> Result<Vec<String>> {
    let mut values = HashSet::new();
    let mut stmt = connection.prepare("SELECT cwd FROM session_cwds WHERE session_id = ?")?;
    for session in sessions {
        for cwd in stmt.query_map([&session.id], |row| row.get::<_, String>(0))? {
            values.insert(cwd?);
        }
    }
    let mut values = values.into_iter().collect::<Vec<_>>();
    values.sort();
    Ok(values)
}

pub fn projects_legacy(connection: &Connection) -> Result<Vec<Project>> {
    let mut statement = connection.prepare(
        "
        SELECT project_id,
               COALESCE(
                   (
                       SELECT latest.cwd
                       FROM sessions AS latest
                       WHERE latest.project_id = sessions.project_id AND latest.hidden = 0
                       ORDER BY latest.pinned DESC,
                                COALESCE(latest.last_used_at, latest.ended_at,
                                         latest.started_at, latest.source_mtime) DESC,
                                latest.id ASC
                       LIMIT 1
                   ),
                   sessions.project_id
               ),
               MAX(COALESCE(last_used_at, ended_at, started_at, source_mtime))
        FROM sessions
        WHERE hidden = 0
        GROUP BY project_id
        ORDER BY MAX(COALESCE(last_used_at, ended_at, started_at, source_mtime)) DESC,
                 project_id ASC
        ",
    )?;
    let rows = statement
        .query_map([], |row| {
            Ok((
                row.get::<_, String>(0)?,
                row.get::<_, String>(1)?,
                row.get::<_, Option<i64>>(2)?,
            ))
        })?
        .collect::<rusqlite::Result<Vec<_>>>()?;

    let mut projects = Vec::with_capacity(rows.len());
    for (id, path, latest_activity) in rows {
        let mut session_statement = connection.prepare(
            "
            SELECT id, provider_id, native_session_id, project_id, title, local_title, hidden, pinned,
                   last_used_at, started_at, ended_at, branch, first_prompt, last_prompt, cwd, models, tool_count,
                   source_mtime, partial, workspace_id, project_path, worktree_path
            FROM sessions
            WHERE project_id = ? AND hidden = 0
            ORDER BY pinned DESC,
                     COALESCE(last_used_at, ended_at, started_at, source_mtime) DESC,
                     id ASC
            ",
        )?;
        let sessions = session_statement
            .query_map([&id], decode_summary)?
            .collect::<rusqlite::Result<Vec<_>>>()?;

        projects.push(Project {
            id: id.clone(),
            workspace_id: id.clone(),
            name: project_display_name(&id),
            path,
            alias: connection.query_row("SELECT alias FROM project_overrides WHERE provider_id = 'claude' AND workspace_id = ?", [&id], |row| row.get(0)).unwrap_or(None),
            cwd_paths: Vec::new(),
            worktree_paths: Vec::new(),
            latest_activity,
            sessions,
        });
    }
    Ok(projects)
}

pub fn set_project_alias(
    connection: &Connection,
    workspace_id: &str,
    alias: Option<&str>,
) -> Result<()> {
    let alias = alias.map(str::trim).filter(|value| !value.is_empty());
    if alias.is_some_and(|value| value.chars().count() > 200) {
        return Err(AppError::Message("project alias is too long".into()));
    }
    connection.execute("INSERT INTO project_overrides (provider_id, workspace_id, alias) VALUES ('claude', ?, ?) ON CONFLICT(provider_id,workspace_id) DO UPDATE SET alias = excluded.alias", params![workspace_id, alias])?;
    Ok(())
}

fn project_display_name(project_id: &str) -> String {
    Path::new(project_id)
        .file_name()
        .and_then(|value| value.to_str())
        .filter(|value| !value.is_empty())
        .unwrap_or(project_id)
        .to_owned()
}

pub fn detail(connection: &Connection, id: &str) -> Result<SessionDetail> {
    let summary = session_summary(connection, id)?;
    let branches = branch_summaries(connection, id)?;
    let active_branch_id = branches
        .iter()
        .find(|branch| branch.is_active)
        .map(|branch| branch.id.clone());
    let (tool_stats, turn_insights) = branch_insights(connection, id, active_branch_id.as_deref())?;
    let relations = relations_for_session(connection, id)?;
    let mut cwd_statement = connection.prepare("SELECT cwd, first_sequence, last_sequence, first_timestamp, last_timestamp, resume FROM session_cwds WHERE session_id = ? ORDER BY first_sequence, cwd")?;
    let cwd_history = cwd_statement
        .query_map([id], |row| {
            Ok(crate::domain::ObservedCwd {
                cwd: row.get(0)?,
                first_sequence: row.get(1)?,
                last_sequence: row.get(2)?,
                first_timestamp: row.get(3)?,
                last_timestamp: row.get(4)?,
                resume: row.get::<_, i64>(5)? != 0,
            })
        })?
        .collect::<rusqlite::Result<Vec<_>>>()?;
    let mut timeline_statement = connection.prepare(
        "
        SELECT id, session_id, kind, role, content, timestamp, tool_name, collapsed,
               uuid, parent_uuid, logical_parent_uuid, message_id, parent_tool_use_id,
               tool_use_id, sequence, is_sidechain, is_meta, turn_id, final_response,
               compact_boundary, compact_preserved
        FROM timeline WHERE session_id = ? ORDER BY id
        ",
    )?;
    let timeline = timeline_statement
        .query_map([id], |row| {
            Ok(TimelineEvent {
                id: row.get(0)?,
                session_id: row.get(1)?,
                kind: row.get(2)?,
                role: row.get(3)?,
                content: row.get(4)?,
                timestamp: row.get(5)?,
                tool_name: row.get(6)?,
                collapsed: row.get::<_, i64>(7)? != 0,
                uuid: row.get(8)?,
                parent_uuid: row.get(9)?,
                logical_parent_uuid: row.get(10)?,
                message_id: row.get(11)?,
                parent_tool_use_id: row.get(12)?,
                tool_use_id: row.get(13)?,
                sequence: row.get(14)?,
                is_sidechain: row.get::<_, i64>(15)? != 0,
                is_meta: row.get::<_, i64>(16)? != 0,
                turn_id: row.get(17)?,
                final_response: row.get::<_, i64>(18)? != 0,
                compact_boundary: row.get::<_, i64>(19)? != 0,
                compact_preserved_ids: serde_json::from_str(&row.get::<_, String>(20)?)
                    .unwrap_or_default(),
            })
        })?
        .collect::<rusqlite::Result<Vec<_>>>()?;

    let mut diagnostic_statement = connection
        .prepare("SELECT line, code FROM diagnostics WHERE session_id = ? ORDER BY line")?;
    let diagnostics = diagnostic_statement
        .query_map([id], |row| {
            Ok(Diagnostic {
                line: row.get(0)?,
                code: row.get(1)?,
            })
        })?
        .collect::<rusqlite::Result<Vec<_>>>()?;

    let mut turns_statement = connection.prepare(
        "SELECT id, session_id, user_prompt, final_response, timestamp, completed FROM turns WHERE session_id = ? ORDER BY id",
    )?;
    let mut turns = turns_statement
        .query_map([id], |row| {
            Ok(ConversationTurn {
                id: row.get(0)?,
                session_id: row.get(1)?,
                user_prompt: row.get(2)?,
                final_response: row.get(3)?,
                timestamp: row.get(4)?,
                completed: row.get::<_, i64>(5)? != 0,
                activities: Vec::new(),
            })
        })?
        .collect::<rusqlite::Result<Vec<_>>>()?;
    let mut activity_statement = connection.prepare(
        "SELECT turn_id, event_id, kind, role, content, timestamp, tool_name, tool_use_id, parent_tool_use_id, collapsed, final_response FROM turn_activities WHERE session_id = ? ORDER BY id",
    )?;
    let activities = activity_statement
        .query_map([id], |row| {
            Ok((
                row.get::<_, i64>(0)?,
                TurnActivity {
                    event_id: row.get(1)?,
                    kind: row.get(2)?,
                    role: row.get(3)?,
                    content: row.get(4)?,
                    timestamp: row.get(5)?,
                    tool_name: row.get(6)?,
                    tool_use_id: row.get(7)?,
                    parent_tool_use_id: row.get(8)?,
                    collapsed: row.get::<_, i64>(9)? != 0,
                    final_response: row.get::<_, i64>(10)? != 0,
                },
            ))
        })?
        .collect::<rusqlite::Result<Vec<_>>>()?;
    for (turn_id, activity) in activities {
        if let Some(turn) = turns.iter_mut().find(|turn| turn.id == turn_id) {
            turn.activities.push(activity);
        }
    }

    Ok(SessionDetail {
        summary,
        timeline,
        turns,
        diagnostics,
        branches,
        active_branch_id: active_branch_id.clone(),
        selected_branch_id: active_branch_id,
        tool_stats,
        turn_insights,
        relations,
        cwd_history,
    })
}

/// Load one alternate branch while retaining the session-wide summary,
/// diagnostics, and branch navigation metadata.  Branch ids are scoped to a
/// session; querying with another session id therefore returns the same
/// explicit "branch not found" error rather than leaking rows across sessions.
pub fn detail_branch(
    connection: &Connection,
    session_id: &str,
    branch_id: &str,
) -> Result<SessionDetail> {
    let summary = session_summary(connection, session_id)?;
    let branches = branch_summaries(connection, session_id)?;
    let active_branch_id = branches
        .iter()
        .find(|branch| branch.is_active)
        .map(|branch| branch.id.clone());
    if !branches.iter().any(|branch| branch.id == branch_id) {
        return Err(AppError::Message(format!(
            "branch not found for session: {session_id}/{branch_id}"
        )));
    }
    let (tool_stats, turn_insights) = branch_insights(connection, session_id, Some(branch_id))?;
    let relations = relations_for_session(connection, session_id)?;
    let mut cwd_statement = connection.prepare("SELECT cwd, first_sequence, last_sequence, first_timestamp, last_timestamp, resume FROM session_cwds WHERE session_id = ? ORDER BY first_sequence, cwd")?;
    let cwd_history = cwd_statement
        .query_map([session_id], |row| {
            Ok(crate::domain::ObservedCwd {
                cwd: row.get(0)?,
                first_sequence: row.get(1)?,
                last_sequence: row.get(2)?,
                first_timestamp: row.get(3)?,
                last_timestamp: row.get(4)?,
                resume: row.get::<_, i64>(5)? != 0,
            })
        })?
        .collect::<rusqlite::Result<Vec<_>>>()?;

    let mut timeline_statement = connection.prepare(
        "SELECT id, session_id, kind, role, content, timestamp, tool_name, collapsed,
                uuid, parent_uuid, logical_parent_uuid, message_id, parent_tool_use_id,
                tool_use_id, sequence, is_sidechain, is_meta, turn_id, final_response,
                compact_boundary, compact_preserved
         FROM branch_timeline WHERE session_id = ? AND branch_id = ? ORDER BY id",
    )?;
    let timeline = timeline_statement
        .query_map(params![session_id, branch_id], |row| {
            Ok(TimelineEvent {
                id: row.get(0)?,
                session_id: row.get(1)?,
                kind: row.get(2)?,
                role: row.get(3)?,
                content: row.get(4)?,
                timestamp: row.get(5)?,
                tool_name: row.get(6)?,
                collapsed: row.get::<_, i64>(7)? != 0,
                uuid: row.get(8)?,
                parent_uuid: row.get(9)?,
                logical_parent_uuid: row.get(10)?,
                message_id: row.get(11)?,
                parent_tool_use_id: row.get(12)?,
                tool_use_id: row.get(13)?,
                sequence: row.get(14)?,
                is_sidechain: row.get::<_, i64>(15)? != 0,
                is_meta: row.get::<_, i64>(16)? != 0,
                turn_id: row.get(17)?,
                final_response: row.get::<_, i64>(18)? != 0,
                compact_boundary: row.get::<_, i64>(19)? != 0,
                compact_preserved_ids: serde_json::from_str(&row.get::<_, String>(20)?)
                    .unwrap_or_default(),
            })
        })?
        .collect::<rusqlite::Result<Vec<_>>>()?;

    let mut diagnostic_statement = connection
        .prepare("SELECT line, code FROM diagnostics WHERE session_id = ? ORDER BY line")?;
    let diagnostics = diagnostic_statement
        .query_map([session_id], |row| {
            Ok(Diagnostic {
                line: row.get(0)?,
                code: row.get(1)?,
            })
        })?
        .collect::<rusqlite::Result<Vec<_>>>()?;

    let mut turns_statement = connection.prepare(
        "SELECT id, session_id, user_prompt, final_response, timestamp, completed
         FROM branch_turns WHERE session_id = ? AND branch_id = ? ORDER BY id",
    )?;
    let mut turns = turns_statement
        .query_map(params![session_id, branch_id], |row| {
            Ok(ConversationTurn {
                id: row.get(0)?,
                session_id: row.get(1)?,
                user_prompt: row.get(2)?,
                final_response: row.get(3)?,
                timestamp: row.get(4)?,
                completed: row.get::<_, i64>(5)? != 0,
                activities: Vec::new(),
            })
        })?
        .collect::<rusqlite::Result<Vec<_>>>()?;
    let mut activity_statement = connection.prepare(
        "SELECT turn_id, event_id, kind, role, content, timestamp, tool_name,
                tool_use_id, parent_tool_use_id, collapsed, final_response
         FROM branch_turn_activities
         WHERE session_id = ? AND branch_id = ? ORDER BY id",
    )?;
    let activities = activity_statement
        .query_map(params![session_id, branch_id], |row| {
            Ok((
                row.get::<_, i64>(0)?,
                TurnActivity {
                    event_id: row.get(1)?,
                    kind: row.get(2)?,
                    role: row.get(3)?,
                    content: row.get(4)?,
                    timestamp: row.get(5)?,
                    tool_name: row.get(6)?,
                    tool_use_id: row.get(7)?,
                    parent_tool_use_id: row.get(8)?,
                    collapsed: row.get::<_, i64>(9)? != 0,
                    final_response: row.get::<_, i64>(10)? != 0,
                },
            ))
        })?
        .collect::<rusqlite::Result<Vec<_>>>()?;
    for (turn_id, activity) in activities {
        if let Some(turn) = turns.iter_mut().find(|turn| turn.id == turn_id) {
            turn.activities.push(activity);
        }
    }

    Ok(SessionDetail {
        summary,
        timeline,
        turns,
        diagnostics,
        branches,
        active_branch_id,
        selected_branch_id: Some(branch_id.to_owned()),
        tool_stats,
        turn_insights,
        relations,
        cwd_history,
    })
}

pub fn search(connection: &Connection, query: &str) -> Result<Vec<SearchHit>> {
    let query = query.trim();
    if query.is_empty() {
        return Ok(Vec::new());
    }

    let rows = if query.chars().count() < 3 {
        let escaped = escape_like(query);
        let pattern = format!("%{escaped}%");
        let mut statement = connection.prepare(
            "
            SELECT session_id, event_id, field, content
            FROM search_chunks
            WHERE content LIKE ? ESCAPE '\\'
              AND session_id IN (SELECT id FROM sessions WHERE hidden = 0)
            ORDER BY CASE field
                WHEN 'title' THEN 0
                WHEN 'project' THEN 1
                WHEN 'user' THEN 2
                ELSE 3
            END
            LIMIT 200
            ",
        )?;
        let rows = statement
            .query_map([pattern], |row| {
                Ok((
                    row.get::<_, String>(0)?,
                    row.get::<_, i64>(1)?,
                    row.get::<_, String>(2)?,
                    row.get::<_, String>(3)?,
                ))
            })?
            .collect::<rusqlite::Result<Vec<_>>>()?;
        rows
    } else {
        let literal_query = format!("\"{}\"", query.replace('"', "\"\""));
        let mut statement = connection.prepare(
            "
            SELECT session_id, event_id, field, content
            FROM search_chunks
            WHERE search_chunks MATCH ?
              AND session_id IN (SELECT id FROM sessions WHERE hidden = 0)
            ORDER BY CASE field
                WHEN 'title' THEN 0
                WHEN 'project' THEN 1
                WHEN 'user' THEN 2
                ELSE 3
            END
            LIMIT 200
            ",
        )?;
        let rows = statement
            .query_map([literal_query], |row| {
                Ok((
                    row.get::<_, String>(0)?,
                    row.get::<_, i64>(1)?,
                    row.get::<_, String>(2)?,
                    row.get::<_, String>(3)?,
                ))
            })?
            .collect::<rusqlite::Result<Vec<_>>>()?;
        rows
    };

    let mut seen = HashSet::new();
    let mut hits = Vec::new();
    for (session_id, event_id, _field, content) in rows {
        if seen.insert(session_id.clone()) {
            hits.push(SearchHit {
                session: session_summary(connection, &session_id)?,
                snippet: content.chars().take(240).collect(),
                event_id,
            });
        }
        if hits.len() == 100 {
            break;
        }
    }
    Ok(hits)
}

fn escape_like(query: &str) -> String {
    query
        .replace('\\', "\\\\")
        .replace('%', "\\%")
        .replace('_', "\\_")
}

pub fn count(connection: &Connection) -> Result<usize> {
    Ok(
        connection.query_row("SELECT COUNT(*) FROM sessions", [], |row| {
            row.get::<_, i64>(0)
        })? as usize,
    )
}

pub type ScanRunSummary = (i64, i64, String, String, bool);

pub fn last_scan_run(connection: &Connection) -> Result<Option<ScanRunSummary>> {
    connection.query_row("SELECT id, started_at, trigger, outcome, committed FROM scan_runs ORDER BY id DESC LIMIT 1", [], |row| {
        Ok((row.get(0)?, row.get(1)?, row.get(2)?, row.get(3)?, row.get::<_, i64>(4)? != 0))
    }).optional().map_err(Into::into)
}
pub fn last_success_at(connection: &Connection) -> Result<Option<i64>> {
    connection.query_row("SELECT started_at FROM scan_runs WHERE committed=1 AND outcome='committed' ORDER BY id DESC LIMIT 1", [], |row| row.get(0)).optional().map_err(Into::into)
}

pub fn diagnostic_counts(connection: &Connection) -> Result<Vec<(String, i64, i64, i64)>> {
    let mut stmt = connection.prepare("SELECT d.code, SUM(d.count), MAX(r.ended_at), MAX(d.scan_run_id) FROM scan_diagnostic_counts d JOIN scan_runs r ON r.id=d.scan_run_id GROUP BY d.code ORDER BY d.code")?;
    let rows = stmt
        .query_map([], |row| {
            Ok((row.get(0)?, row.get(1)?, row.get(2)?, row.get(3)?))
        })?
        .collect::<rusqlite::Result<Vec<_>>>()?;
    Ok(rows)
}

pub fn record_scan_run(
    connection: &Connection,
    started_at: i64,
    trigger: &str,
    outcome: &str,
    committed: bool,
    sessions: usize,
    diagnostics: &[(String, usize)],
) -> Result<i64> {
    let tx = connection.unchecked_transaction()?;
    tx.execute("INSERT INTO scan_runs (started_at, ended_at, trigger, outcome, committed, sessions, diagnostics) VALUES (?, ?, ?, ?, ?, ?, ?)", params![started_at, chrono::Utc::now().timestamp_millis(), trigger, outcome, committed, sessions as i64, diagnostics.iter().map(|(_, count)| *count as i64).sum::<i64>()])?;
    let id = tx.last_insert_rowid();
    for (code, count) in diagnostics {
        tx.execute(
            "INSERT INTO scan_diagnostic_counts (scan_run_id, code, count) VALUES (?, ?, ?)",
            params![id, code, *count as i64],
        )?;
    }
    tx.commit()?;
    Ok(id)
}
