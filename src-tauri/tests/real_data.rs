use context_vault_lib::{providers::claude::PROVIDER_ID, scanner, storage};

#[test]
#[ignore = "reads the current user's Claude Code source directory"]
fn scans_real_claude_data_without_indexing_or_printing_content() {
    let root = scanner::default_root();
    assert!(
        root.is_dir(),
        "Claude Code project directory is unavailable"
    );

    let (sessions, scan_diagnostics, complete) = scanner::scan_root(&root).expect("scan real data");
    let parser_diagnostics = sessions
        .iter()
        .map(|session| session.diagnostics.len())
        .sum::<usize>();
    let partial_sessions = sessions
        .iter()
        .filter(|session| session.summary.partial)
        .count();

    assert!(complete, "real source enumeration was incomplete");
    assert!(!sessions.is_empty(), "no top-level sessions were parsed");
    eprintln!(
        "real_data_smoke sessions={} partial_sessions={} scan_diagnostics={} parser_diagnostics={}",
        sessions.len(),
        partial_sessions,
        scan_diagnostics.len(),
        parser_diagnostics
    );
}

#[test]
#[ignore = "reads the current user's Claude Code source directory"]
fn indexes_real_claude_data_in_a_temporary_database() {
    let root = scanner::default_root();
    let (sessions, _, complete) = scanner::scan_root(&root).expect("scan real data");
    assert!(complete, "real source enumeration was incomplete");

    let temporary = tempfile::tempdir().expect("create temporary database directory");
    let mut connection =
        storage::open(&temporary.path().join("index.db")).expect("open temporary index");
    storage::index(&mut connection, PROVIDER_ID, &sessions).expect("index real sessions");

    let projects = storage::projects(&connection).expect("list real projects");
    let first_session = projects
        .iter()
        .flat_map(|project| &project.sessions)
        .next()
        .expect("indexed session");
    let detail = storage::detail(&connection, &first_session.id).expect("read real session detail");

    assert_eq!(
        storage::count(&connection).expect("count index"),
        sessions.len()
    );
    assert_eq!(detail.summary.id, first_session.id);
    eprintln!(
        "real_index_smoke sessions={} projects={} timeline_events={}",
        sessions.len(),
        projects.len(),
        detail.timeline.len()
    );
}
