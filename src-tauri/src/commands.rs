use crate::{
    domain::{
        AppError, AppStatus, KnowledgeCard, KnowledgeCardPatch, Project, RelatedSession, Result,
        ScanReport, SearchHit, SessionDetail, SessionSummary,
    },
    indexer,
    providers::{
        claude::{ClaudeProvider, PROVIDER_ID},
        claude_live::ClaudeLiveTail,
        codex::CodexProvider,
        gemini::{self, GeminiProvider},
        opencode::{OpenCodeStore, PROVIDER_ID as OPENCODE_PROVIDER_ID},
        ProviderDescriptor, ProviderRegistry, SessionProvider,
    },
    runtime::{self, ClaudeResumeSpec, PtyEvent, PtyHandle, PtyManager, PtySize, RuntimeError},
    scanner, storage,
};
use rusqlite::Connection;
use serde::{Deserialize, Serialize};
use std::{
    collections::{BTreeMap, HashMap, HashSet},
    fs,
    path::{Path, PathBuf},
    sync::{Arc, Mutex},
};
use tauri::State;
use uuid::Uuid;

const DEFAULT_ROWS: u16 = 24;
const DEFAULT_COLS: u16 = 80;
const MAX_WRITE_BYTES: usize = 64 * 1024;
const MAX_POLL_EVENTS: usize = 64;
const MAX_POLL_BYTES: usize = 64 * 1024;
const MILLIS_PER_DAY: i64 = 86_400_000;
const ALLOWED_PROVIDER_LOOKBACK_DAYS: [i64; 5] = [7, 30, 90, 180, 365];
type ScanWork = (
    Vec<crate::domain::ParsedSession>,
    Vec<String>,
    bool,
    usize,
    usize,
    usize,
    Vec<PathBuf>,
);

fn prepare_scan(
    root: &Path,
    manifest: &HashMap<PathBuf, scanner::SourceFingerprint>,
    modified_since: Option<i64>,
) -> Result<ScanWork> {
    prepare_provider_scan(&ClaudeProvider, root, manifest, modified_since)
}

fn prepare_provider_scan(
    provider: &dyn SessionProvider,
    root: &Path,
    manifest: &HashMap<PathBuf, scanner::SourceFingerprint>,
    modified_since: Option<i64>,
) -> Result<ScanWork> {
    let plan = scanner::plan_provider_scan(provider, root, manifest, modified_since)?;
    let mut sessions = Vec::new();
    let mut diagnostics = plan.diagnostics;
    let mut complete = plan.complete;
    for fp in &plan.parse {
        match provider.parse(&fp.path) {
            Ok(parsed) => {
                let stable = scanner::fingerprint(&fp.path)
                    .map(|after| {
                        after.size == fp.size
                            && after.mtime == fp.mtime
                            && after.hash == fp.hash
                            && {
                                #[cfg(unix)]
                                {
                                    after.dev == fp.dev && after.ino == fp.ino
                                }
                                #[cfg(not(unix))]
                                {
                                    true
                                }
                            }
                    })
                    .unwrap_or(false);
                let parser_changed = parsed
                    .diagnostics
                    .iter()
                    .any(|diagnostic| diagnostic.code == "source_changed_during_scan");
                let graph_cycle_only = !parsed.diagnostics.is_empty()
                    && parsed
                        .diagnostics
                        .iter()
                        .all(|diagnostic| diagnostic.code == "conversation_graph_cycle");
                // Graph-cycle annotations preserve readable events. Other
                // partial parses remain unsafe for deletion reconciliation.
                let parser_incomplete = (parsed.summary.partial && !graph_cycle_only)
                    || parsed
                        .diagnostics
                        .iter()
                        .any(|diagnostic| diagnostic.code != "conversation_graph_cycle");
                let source_changed = !stable
                    || parsed.source_size != fp.size
                    || parsed.source_hash != fp.hash
                    || parsed.summary.source_mtime != fp.mtime;
                if source_changed || parser_incomplete {
                    complete = false;
                    if source_changed && !parser_changed {
                        diagnostics.push("source_changed_during_scan".into());
                    }
                }
                diagnostics.extend(parsed.diagnostics.iter().map(|d| d.code.clone()));
                sessions.push(parsed);
            }
            Err(_) => {
                complete = false;
                diagnostics.push("session_file_unreadable".into());
            }
        }
    }
    Ok((
        sessions,
        diagnostics,
        complete,
        plan.new_files,
        plan.changed_files,
        plan.unchanged,
        plan.discovered.into_iter().map(|fp| fp.path).collect(),
    ))
}

fn diagnostic_counts(codes: &[String]) -> Vec<(String, usize)> {
    let mut grouped = BTreeMap::<String, usize>::new();
    for code in codes {
        *grouped.entry(code.clone()).or_default() += 1;
    }
    grouped.into_iter().collect()
}

#[derive(Debug, Default)]
struct ProviderScanSummary {
    sessions: usize,
    diagnostics: Vec<String>,
    committed: bool,
    partial: bool,
    removed_sessions: usize,
    new_files: usize,
    changed_files: usize,
    unchanged_files: usize,
    removed_files: usize,
    partial_sessions: usize,
}

struct PreparedFileProvider {
    provider_id: &'static str,
    summary: ProviderScanSummary,
    reconciliation: Option<(Vec<crate::domain::ParsedSession>, Vec<PathBuf>)>,
}

fn prepare_file_provider(
    provider: &dyn SessionProvider,
    root: &Path,
    manifest: Result<HashMap<PathBuf, scanner::SourceFingerprint>>,
    modified_since: Option<i64>,
) -> PreparedFileProvider {
    let provider_id = provider.id();
    let unavailable = |summary| PreparedFileProvider {
        provider_id,
        summary,
        reconciliation: None,
    };
    match fs::symlink_metadata(root) {
        Ok(metadata) if metadata.file_type().is_symlink() || !metadata.is_dir() => {
            return unavailable(ProviderScanSummary {
                diagnostics: vec![format!("{provider_id}:root_invalid")],
                partial: true,
                ..Default::default()
            });
        }
        Ok(_) => {}
        Err(error) if error.kind() == std::io::ErrorKind::NotFound => {
            return unavailable(ProviderScanSummary::default());
        }
        Err(_) => {
            return unavailable(ProviderScanSummary {
                diagnostics: vec![format!("{provider_id}:root_unreadable")],
                partial: true,
                ..Default::default()
            });
        }
    }
    let manifest = match manifest {
        Ok(manifest) => manifest,
        Err(_) => {
            return unavailable(ProviderScanSummary {
                diagnostics: vec![format!("{provider_id}:manifest_unreadable")],
                partial: true,
                ..Default::default()
            });
        }
    };
    let (sessions, diagnostics, complete, new_files, changed_files, unchanged_files, discovered) =
        match prepare_provider_scan(provider, root, &manifest, modified_since) {
            Ok(value) => value,
            Err(_) => {
                return unavailable(ProviderScanSummary {
                    diagnostics: vec![format!("{provider_id}:provider_unavailable")],
                    partial: true,
                    ..Default::default()
                });
            }
        };
    let partial_sessions = sessions
        .iter()
        .filter(|session| session.summary.partial)
        .count();
    let summary = ProviderScanSummary {
        sessions: discovered.len(),
        diagnostics: diagnostics
            .into_iter()
            .map(|code| format!("{provider_id}:{code}"))
            .collect(),
        partial: !complete || partial_sessions > 0,
        new_files,
        changed_files,
        unchanged_files,
        removed_files: manifest
            .keys()
            .filter(|path| !discovered.contains(path))
            .count(),
        partial_sessions,
        ..Default::default()
    };
    PreparedFileProvider {
        provider_id,
        summary,
        reconciliation: complete.then_some((sessions, discovered)),
    }
}

fn reconcile_file_provider(
    connection: &mut Connection,
    mut prepared: PreparedFileProvider,
) -> ProviderScanSummary {
    if let Some((mut sessions, discovered)) = prepared.reconciliation.take() {
        if prepared.provider_id == gemini::PROVIDER_ID {
            let Ok(known_paths) = storage::known_project_paths(connection) else {
                prepared.summary.partial = true;
                prepared
                    .summary
                    .diagnostics
                    .push("gemini:project_paths_unreadable".into());
                return prepared.summary;
            };
            gemini::resolve_legacy_project_paths(&mut sessions, &known_paths);
        }
        match indexer::reconcile_incremental(
            connection,
            prepared.provider_id,
            &sessions,
            &discovered,
        ) {
            Ok(removed_sessions) => {
                prepared.summary.committed = true;
                prepared.summary.removed_sessions = removed_sessions;
            }
            Err(_) => {
                prepared.summary.partial = true;
                prepared
                    .summary
                    .diagnostics
                    .push(format!("{}:reconcile_failed", prepared.provider_id));
            }
        }
    }
    prepared.summary
}

#[cfg(test)]
fn scan_file_provider(
    connection: &mut Connection,
    provider: &dyn SessionProvider,
    root: &Path,
) -> ProviderScanSummary {
    let manifest = storage::source_manifest(connection, provider.id());
    reconcile_file_provider(
        connection,
        prepare_file_provider(provider, root, manifest, None),
    )
}

struct PreparedOpenCodeProvider {
    summary: ProviderScanSummary,
    sessions: Option<Vec<crate::domain::ParsedSession>>,
}

#[derive(Debug, Default)]
struct OpenCodeIndexSnapshot {
    source: Option<scanner::SourceFingerprint>,
    sessions: usize,
    partial_sessions: usize,
}

fn prepare_opencode_provider(
    path: &Path,
    indexed: &OpenCodeIndexSnapshot,
    modified_since: Option<i64>,
) -> PreparedOpenCodeProvider {
    if !path.is_file() {
        return PreparedOpenCodeProvider {
            summary: ProviderScanSummary::default(),
            sessions: None,
        };
    }
    let store = match OpenCodeStore::open(path) {
        Ok(store) => store,
        Err(_) => {
            return PreparedOpenCodeProvider {
                summary: ProviderScanSummary {
                    diagnostics: vec![format!("{OPENCODE_PROVIDER_ID}:database_unreadable")],
                    partial: true,
                    ..Default::default()
                },
                sessions: None,
            }
        }
    };
    if modified_since.is_none() {
        let fingerprint = match store.source_fingerprint() {
            Ok(fingerprint) => fingerprint,
            Err(_) => {
                return PreparedOpenCodeProvider {
                    summary: ProviderScanSummary {
                        diagnostics: vec![format!(
                            "{OPENCODE_PROVIDER_ID}:database_fingerprint_failed"
                        )],
                        partial: true,
                        ..Default::default()
                    },
                    sessions: None,
                }
            }
        };
        if indexed.source.as_ref().is_some_and(|source| {
            source.path == fingerprint.path
                && source.size == fingerprint.size
                && source.mtime == fingerprint.mtime
                && source.hash == fingerprint.hash
        }) {
            return PreparedOpenCodeProvider {
                summary: ProviderScanSummary {
                    sessions: indexed.sessions,
                    committed: true,
                    partial: indexed.partial_sessions > 0,
                    unchanged_files: 1,
                    partial_sessions: indexed.partial_sessions,
                    ..Default::default()
                },
                sessions: None,
            };
        }
    }
    let result = match store.scan_since(modified_since) {
        Ok(result) => result,
        Err(_) => {
            return PreparedOpenCodeProvider {
                summary: ProviderScanSummary {
                    diagnostics: vec![format!("{OPENCODE_PROVIDER_ID}:database_scan_failed")],
                    partial: true,
                    ..Default::default()
                },
                sessions: None,
            }
        }
    };
    let complete = result.complete;
    let source_unchanged = indexed.source.as_ref().is_some_and(|source| {
        source.path == result.source.path
            && source.size == result.source.size
            && source.mtime == result.source.mtime
            && source.hash == result.source.hash
    });
    let partial_sessions = result
        .sessions
        .iter()
        .filter(|session| session.summary.partial)
        .count();
    let summary = ProviderScanSummary {
        sessions: result.sessions.len(),
        diagnostics: result
            .diagnostics
            .into_iter()
            .map(|code| format!("{OPENCODE_PROVIDER_ID}:{code}"))
            .collect(),
        partial: !result.complete || partial_sessions > 0,
        new_files: usize::from(indexed.source.is_none()),
        changed_files: usize::from(indexed.source.is_some() && !source_unchanged),
        unchanged_files: usize::from(source_unchanged),
        partial_sessions,
        ..Default::default()
    };

    PreparedOpenCodeProvider {
        summary,
        sessions: complete.then_some(result.sessions),
    }
}

fn reconcile_opencode_provider(
    connection: &mut Connection,
    mut prepared: PreparedOpenCodeProvider,
) -> ProviderScanSummary {
    if let Some(sessions) = prepared.sessions.take() {
        match indexer::reconcile(connection, OPENCODE_PROVIDER_ID, &sessions) {
            Ok(removed_sessions) => {
                prepared.summary.committed = true;
                prepared.summary.removed_sessions = removed_sessions;
            }
            Err(_) => {
                prepared.summary.partial = true;
                prepared
                    .summary
                    .diagnostics
                    .push(format!("{OPENCODE_PROVIDER_ID}:reconcile_failed"));
            }
        }
    }
    prepared.summary
}

fn scan_all_providers(
    connection: &mut Connection,
    prepared_file_providers: Vec<PreparedFileProvider>,
    enabled_provider_ids: &[String],
    prepared_opencode: Option<PreparedOpenCodeProvider>,
) -> Vec<ProviderScanSummary> {
    let mut summaries = prepared_file_providers
        .into_iter()
        .map(|prepared| reconcile_file_provider(connection, prepared))
        .collect::<Vec<_>>();
    if enabled_provider_ids
        .iter()
        .any(|id| id == OPENCODE_PROVIDER_ID)
    {
        summaries.push(
            prepared_opencode
                .map(|prepared| reconcile_opencode_provider(connection, prepared))
                .unwrap_or_else(|| ProviderScanSummary {
                    diagnostics: vec![format!("{OPENCODE_PROVIDER_ID}:database_scan_not_prepared")],
                    partial: true,
                    ..Default::default()
                }),
        );
    }
    summaries
}

fn validate_enabled_provider_ids(provider_ids: &[String]) -> Result<Vec<String>> {
    let requested = provider_ids.iter().cloned().collect::<HashSet<_>>();
    if requested.len() != provider_ids.len() {
        return Err(AppError::Message("duplicate enabled provider".into()));
    }
    let registry = ProviderRegistry::builtin();
    if let Some(provider_id) = requested
        .iter()
        .find(|provider_id| registry.get(provider_id).is_none())
    {
        return Err(AppError::Message(format!(
            "unknown provider: {provider_id}"
        )));
    }
    Ok(registry
        .descriptors()
        .iter()
        .filter(|descriptor| requested.contains(&descriptor.provider_id))
        .map(|descriptor| descriptor.provider_id.clone())
        .collect())
}

fn validate_provider_lookback_days(
    values: &BTreeMap<String, i64>,
) -> Result<BTreeMap<String, i64>> {
    let registry = ProviderRegistry::builtin();
    for (provider_id, days) in values {
        if registry.get(provider_id).is_none() {
            return Err(AppError::Message(format!(
                "unknown provider lookback: {provider_id}"
            )));
        }
        if !ALLOWED_PROVIDER_LOOKBACK_DAYS.contains(days) {
            return Err(AppError::Message(format!(
                "invalid provider lookback days: {days}"
            )));
        }
    }
    Ok(values.clone())
}

fn provider_modified_since(
    values: &BTreeMap<String, i64>,
    provider_id: &str,
    now: i64,
) -> Option<i64> {
    values
        .get(provider_id)
        .map(|days| now.saturating_sub(days.saturating_mul(MILLIS_PER_DAY)))
}

#[derive(Debug, Default)]
pub struct LifecycleState {
    pub scanning: bool,
    pub active_terminal: Option<PtyHandle>,
    pub active_session_id: Option<String>,
}

struct ScanGuard {
    lifecycle: Arc<Mutex<LifecycleState>>,
}

impl Drop for ScanGuard {
    fn drop(&mut self) {
        if let Ok(mut lifecycle) = self.lifecycle.lock() {
            lifecycle.scanning = false;
        }
    }
}

pub struct AppState {
    pub db: Arc<Mutex<Connection>>,
    pub root: Arc<Mutex<PathBuf>>,
    pub last_scan: Arc<Mutex<Option<i64>>>,
    /// The scan/continuation gate. Checking and starting a PTY happen while
    /// holding this same mutex, so no scan can slip between those operations.
    pub lifecycle: Arc<Mutex<LifecycleState>>,
    pub pty: Arc<PtyManager>,
    pub live_tails: Arc<Mutex<HashMap<String, LiveTailEntry>>>,
}

impl AppState {
    pub fn new() -> Result<Self> {
        let data = directories::ProjectDirs::from("dev", "context-vault", "Context Vault")
            .ok_or_else(|| AppError::Message("cannot resolve data directory".into()))?;
        std::fs::create_dir_all(data.data_dir())?;
        let mut db = storage::open(&data.data_dir().join("index.db"))?;
        let mut scan_settings = storage::claude_scan_settings(&db)?;
        scan_settings.enabled_provider_ids =
            validate_enabled_provider_ids(&scan_settings.enabled_provider_ids)?;
        scan_settings.provider_lookback_days =
            validate_provider_lookback_days(&scan_settings.provider_lookback_days)?;
        storage::update_claude_scan_settings(&mut db, &scan_settings)?;
        let last_scan_at = storage::last_success_at(&db)?;
        let configured_root = scan_settings
            .source_root
            .map(PathBuf::from)
            .unwrap_or_else(scanner::default_root);
        Ok(Self {
            db: Arc::new(Mutex::new(db)),
            root: Arc::new(Mutex::new(configured_root)),
            last_scan: Arc::new(Mutex::new(last_scan_at)),
            lifecycle: Arc::new(Mutex::new(LifecycleState::default())),
            pty: Arc::new(PtyManager::new()),
            live_tails: Arc::new(Mutex::new(HashMap::new())),
        })
    }
}

#[derive(Debug, Deserialize, Default)]
pub struct ScanArgs {
    pub trigger: Option<String>,
}

#[tauri::command]
pub async fn scan(state: State<'_, AppState>, args: Option<ScanArgs>) -> Result<ScanReport> {
    let scan_started_at = chrono::Utc::now().timestamp_millis();
    let trigger = args
        .and_then(|a| a.trigger)
        .unwrap_or_else(|| "manual".into());
    if !matches!(
        trigger.as_str(),
        "manual" | "scheduled" | "post_continuation"
    ) {
        return Err(AppError::Message("invalid scan trigger".into()));
    }
    let root = state.root.lock().expect("root lock").clone();
    {
        let mut lifecycle = state.lifecycle.lock().expect("lifecycle lock");
        clear_finished_terminal(&state.pty, &state.live_tails, &mut lifecycle)?;
        if lifecycle.scanning {
            if trigger == "scheduled" {
                let db = state
                    .db
                    .lock()
                    .map_err(|_| AppError::Message("database lock poisoned".into()))?;
                storage::record_scan_run(
                    &db,
                    scan_started_at,
                    &trigger,
                    "skipped_lifecycle",
                    false,
                    0,
                    &[],
                )?;
                return Ok(ScanReport {
                    root: root.display().to_string(),
                    trigger,
                    outcome: "skipped_lifecycle".into(),
                    ..Default::default()
                });
            }
            return Err(AppError::Message("a scan is already in progress".into()));
        }
        if lifecycle.active_terminal.is_some() {
            if trigger == "scheduled" {
                let db = state
                    .db
                    .lock()
                    .map_err(|_| AppError::Message("database lock poisoned".into()))?;
                storage::record_scan_run(
                    &db,
                    scan_started_at,
                    &trigger,
                    "skipped_lifecycle",
                    false,
                    0,
                    &[],
                )?;
                return Ok(ScanReport {
                    root: root.display().to_string(),
                    trigger,
                    outcome: "skipped_lifecycle".into(),
                    ..Default::default()
                });
            }
            return Err(AppError::Message(continuation_in_progress_message(
                &lifecycle,
            )));
        }
        lifecycle.scanning = true;
    }
    let _scan_guard = ScanGuard {
        lifecycle: Arc::clone(&state.lifecycle),
    };

    let codex_root = CodexProvider.default_root();
    let gemini_root = GeminiProvider.default_root();
    let opencode_path = OpenCodeStore::default_path();
    let (enabled_provider_ids, file_provider_inputs, opencode_index, opencode_modified_since) = {
        let db = state
            .db
            .lock()
            .map_err(|_| AppError::Message("database lock poisoned".into()))?;
        let scan_settings = storage::claude_scan_settings(&db)?;
        let enabled_provider_ids =
            validate_enabled_provider_ids(&scan_settings.enabled_provider_ids)?;
        let provider_lookback_days =
            validate_provider_lookback_days(&scan_settings.provider_lookback_days)?;
        let file_provider_inputs = ProviderRegistry::builtin_session_providers()
            .into_iter()
            .filter(|provider| enabled_provider_ids.iter().any(|id| id == provider.id()))
            .filter_map(|provider| {
                let provider_root = match provider.id() {
                    PROVIDER_ID => &root,
                    "codex" => &codex_root,
                    "gemini" => &gemini_root,
                    _ => return None,
                };
                Some((
                    provider,
                    provider_root.clone(),
                    storage::source_manifest(&db, provider.id()),
                    provider_modified_since(
                        &provider_lookback_days,
                        provider.id(),
                        scan_started_at,
                    ),
                ))
            })
            .collect::<Vec<_>>();
        let opencode_index = if enabled_provider_ids
            .iter()
            .any(|id| id == OPENCODE_PROVIDER_ID)
        {
            let mut manifest = storage::source_manifest(&db, OPENCODE_PROVIDER_ID)?;
            let (sessions, partial_sessions) =
                storage::provider_session_counts(&db, OPENCODE_PROVIDER_ID)?;
            OpenCodeIndexSnapshot {
                source: manifest.remove(&opencode_path),
                sessions,
                partial_sessions,
            }
        } else {
            OpenCodeIndexSnapshot::default()
        };
        let opencode_modified_since = provider_modified_since(
            &provider_lookback_days,
            OPENCODE_PROVIDER_ID,
            scan_started_at,
        );
        (
            enabled_provider_ids,
            file_provider_inputs,
            opencode_index,
            opencode_modified_since,
        )
    };
    let prepared_file_providers = file_provider_inputs
        .into_iter()
        .map(|(provider, provider_root, manifest, modified_since)| {
            prepare_file_provider(provider, &provider_root, manifest, modified_since)
        })
        .collect();
    let prepared_opencode = enabled_provider_ids
        .iter()
        .any(|id| id == OPENCODE_PROVIDER_ID)
        .then(|| {
            prepare_opencode_provider(&opencode_path, &opencode_index, opencode_modified_since)
        });
    let summaries = {
        let mut db = state
            .db
            .lock()
            .map_err(|_| AppError::Message("database lock poisoned".into()))?;
        scan_all_providers(
            &mut db,
            prepared_file_providers,
            &enabled_provider_ids,
            prepared_opencode,
        )
    };

    let mut report = ScanReport {
        root: root.display().to_string(),
        trigger: trigger.clone(),
        ..Default::default()
    };
    let no_providers_enabled = summaries.is_empty();
    let claude_committed = enabled_provider_ids.iter().any(|id| id == PROVIDER_ID)
        && summaries.first().is_some_and(|summary| summary.committed);
    let mut attempt_codes = Vec::new();
    for summary in summaries {
        report.sessions += summary.sessions;
        report.diagnostics += summary.diagnostics.len();
        report.committed |= summary.committed;
        report.partial |= summary.partial;
        report.removed_sessions += summary.removed_sessions;
        report.new_files += summary.new_files;
        report.changed_files += summary.changed_files;
        report.unchanged_files += summary.unchanged_files;
        report.removed_files += summary.removed_files;
        report.partial_sessions += summary.partial_sessions;
        attempt_codes.extend(summary.diagnostics);
    }
    report.outcome = if no_providers_enabled {
        report.committed = true;
        "committed".into()
    } else if report.committed {
        if report.partial {
            "partial".into()
        } else {
            "committed".into()
        }
    } else {
        report.partial = true;
        report.diagnostics = report.diagnostics.max(1);
        attempt_codes.push("all_providers_unavailable".into());
        "failed".into()
    };
    if report.committed {
        *state.last_scan.lock().expect("time lock") = Some(chrono::Utc::now().timestamp_millis());
    }
    if claude_committed {
        *state.root.lock().expect("root lock") = root.clone();
    }
    {
        let db = state
            .db
            .lock()
            .map_err(|_| AppError::Message("database lock poisoned".into()))?;
        storage::record_scan_run(
            &db,
            scan_started_at,
            &report.trigger,
            &report.outcome,
            report.committed,
            report.sessions,
            &diagnostic_counts(&attempt_codes),
        )?;
    }
    Ok(report)
}

#[tauri::command]
pub fn list_projects(state: State<'_, AppState>) -> Result<Vec<Project>> {
    storage::projects(&state.db.lock().expect("db lock"))
}

#[tauri::command]
pub fn list_provider_descriptors() -> Vec<ProviderDescriptor> {
    ProviderRegistry::builtin().descriptors().to_vec()
}

#[derive(Debug, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct SetProjectAliasArgs {
    pub provider_id: String,
    pub workspace_id: String,
    pub alias: Option<String>,
}

#[tauri::command]
pub fn set_project_alias(
    state: State<'_, AppState>,
    args: SetProjectAliasArgs,
) -> Result<Vec<Project>> {
    storage::set_project_alias(
        &state.db.lock().expect("db lock"),
        &args.provider_id,
        &args.workspace_id,
        args.alias.as_deref(),
    )?;
    storage::projects(&state.db.lock().expect("db lock"))
}

#[tauri::command]
pub fn get_session(state: State<'_, AppState>, session_id: String) -> Result<SessionDetail> {
    storage::detail(&state.db.lock().expect("db lock"), &session_id)
}

#[derive(Debug, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct GetSessionBranchArgs {
    pub session_id: String,
    pub branch_id: String,
}

#[tauri::command]
pub fn get_session_branch(
    state: State<'_, AppState>,
    args: GetSessionBranchArgs,
) -> Result<SessionDetail> {
    storage::detail_branch(
        &state.db.lock().expect("db lock"),
        &args.session_id,
        &args.branch_id,
    )
}

#[tauri::command]
pub fn global_search(
    state: State<'_, AppState>,
    query: String,
    provider_id: Option<String>,
) -> Result<Vec<SearchHit>> {
    storage::search_filtered(
        &state.db.lock().expect("db lock"),
        &query,
        provider_id.as_deref(),
    )
}

#[tauri::command]
pub fn get_knowledge_card(state: State<'_, AppState>, session_id: String) -> Result<KnowledgeCard> {
    storage::get_knowledge_card(&state.db.lock().expect("db lock"), &session_id)
}

#[derive(Debug, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct UpdateKnowledgeCardArgs {
    pub session_id: String,
    pub patch: KnowledgeCardPatch,
}

#[tauri::command]
pub fn update_knowledge_card(
    state: State<'_, AppState>,
    args: UpdateKnowledgeCardArgs,
) -> Result<KnowledgeCard> {
    storage::update_knowledge_card(
        &mut state.db.lock().expect("db lock"),
        &args.session_id,
        &args.patch,
    )
}

#[derive(Debug, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct RelatedSessionsArgs {
    pub session_id: String,
    pub provider_id: Option<String>,
    pub limit: usize,
}

#[tauri::command]
pub fn related_sessions(
    state: State<'_, AppState>,
    args: RelatedSessionsArgs,
) -> Result<Vec<RelatedSession>> {
    storage::related_sessions(
        &state.db.lock().expect("db lock"),
        &args.session_id,
        args.provider_id.as_deref(),
        args.limit,
    )
}

#[derive(Debug, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct SemanticSearchArgs {
    pub query: String,
    pub provider_id: Option<String>,
    pub limit: usize,
}

#[tauri::command]
pub fn semantic_search(
    state: State<'_, AppState>,
    args: SemanticSearchArgs,
) -> Result<Vec<RelatedSession>> {
    storage::semantic_search(
        &state.db.lock().expect("db lock"),
        &args.query,
        args.provider_id.as_deref(),
        args.limit,
    )
}

#[derive(Debug, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct SetSessionHiddenArgs {
    pub session_id: String,
    pub hidden: bool,
}

#[derive(Debug, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct RenameSessionArgs {
    pub session_id: String,
    pub title: Option<String>,
}

#[derive(Debug, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct SetSessionPinnedArgs {
    pub session_id: String,
    pub pinned: bool,
}

#[derive(Debug, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct TouchSessionArgs {
    pub session_id: String,
}

#[tauri::command]
pub fn list_hidden_sessions(state: State<'_, AppState>) -> Result<Vec<SessionSummary>> {
    storage::hidden_sessions(&state.db.lock().expect("db lock"))
}

#[tauri::command]
pub fn set_session_hidden(
    state: State<'_, AppState>,
    args: SetSessionHiddenArgs,
) -> Result<SessionSummary> {
    storage::set_session_hidden(
        &state.db.lock().expect("db lock"),
        &args.session_id,
        args.hidden,
    )
}

#[tauri::command]
pub fn rename_session(
    state: State<'_, AppState>,
    args: RenameSessionArgs,
) -> Result<SessionSummary> {
    storage::rename_session(
        &state.db.lock().expect("db lock"),
        &args.session_id,
        args.title.as_deref(),
    )
}

#[tauri::command]
pub fn set_session_pinned(
    state: State<'_, AppState>,
    args: SetSessionPinnedArgs,
) -> Result<SessionSummary> {
    storage::set_session_pinned(
        &state.db.lock().expect("db lock"),
        &args.session_id,
        args.pinned,
    )
}

#[tauri::command]
pub fn touch_session(state: State<'_, AppState>, args: TouchSessionArgs) -> Result<SessionSummary> {
    storage::touch_session(
        &state.db.lock().expect("db lock"),
        &args.session_id,
        chrono::Utc::now().timestamp_millis(),
    )
}

#[tauri::command]
pub fn status(state: State<'_, AppState>) -> Result<AppStatus> {
    // Snapshot lifecycle/root/time independently before touching the DB.  No
    // path holds DB and lifecycle locks together, avoiding the DB→lifecycle
    // order used by older status code.
    let scan_in_progress = state.lifecycle.lock().expect("lifecycle lock").scanning;
    let root = state.root.lock().expect("root lock").display().to_string();
    let last_scan_at = *state.last_scan.lock().expect("time lock");
    let indexed_sessions = {
        let db = state.db.lock().expect("db lock");
        storage::count(&db)?
    };
    Ok(AppStatus {
        root,
        indexed_sessions,
        last_scan_at,
        scan_in_progress,
    })
}

#[derive(Debug, Clone, Serialize)]
pub struct IndexDiagnosticsDto {
    pub effective_root: String,
    pub scan_interval_seconds: i64,
    pub last_success_at: Option<i64>,
    pub last_attempt_at: Option<i64>,
    pub last_outcome: Option<String>,
    pub indexed_sessions: usize,
    pub last_run: Option<ScanRunDto>,
    pub diagnostic_counts: Vec<DiagnosticCountDto>,
}
#[derive(Debug, Clone, Serialize)]
pub struct ScanRunDto {
    pub id: i64,
    pub started_at: i64,
    pub trigger: String,
    pub outcome: String,
    pub committed: bool,
}
#[derive(Debug, Clone, Serialize)]
pub struct DiagnosticCountDto {
    pub code: String,
    pub count: i64,
    pub last_occurred_at: i64,
    pub last_run_id: i64,
}

#[tauri::command]
pub fn get_index_diagnostics(state: State<'_, AppState>) -> Result<IndexDiagnosticsDto> {
    let db = state.db.lock().expect("db lock");
    let settings = storage::claude_scan_settings(&db)?;
    let run = storage::last_scan_run(&db)?;
    let last_success_at = storage::last_success_at(&db)?;
    let counts = storage::diagnostic_counts(&db)?;
    Ok(IndexDiagnosticsDto {
        effective_root: settings
            .source_root
            .clone()
            .unwrap_or_else(|| scanner::default_root().display().to_string()),
        scan_interval_seconds: settings.scan_interval_seconds,
        last_success_at,
        last_attempt_at: run.as_ref().map(|r| r.1),
        last_outcome: run.as_ref().map(|r| r.3.clone()),
        indexed_sessions: storage::count(&db)?,
        last_run: run.map(|r| ScanRunDto {
            id: r.0,
            started_at: r.1,
            trigger: r.2,
            outcome: r.3,
            committed: r.4,
        }),
        diagnostic_counts: counts
            .into_iter()
            .map(|(code, count, occurred, last_run)| DiagnosticCountDto {
                code,
                count,
                last_occurred_at: occurred,
                last_run_id: last_run,
            })
            .collect(),
    })
}

#[derive(Debug, Clone, Serialize)]
pub struct ClaudeSettingsDto {
    pub executable_override: Option<String>,
    pub dangerously_skip_permissions: bool,
}

impl From<storage::ClaudeSettings> for ClaudeSettingsDto {
    fn from(value: storage::ClaudeSettings) -> Self {
        Self {
            executable_override: value.executable_override,
            dangerously_skip_permissions: value.dangerously_skip_permissions,
        }
    }
}

#[derive(Debug, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct ClaudeSettingsUpdate {
    /// Absent leaves the override unchanged; null resets to the default `claude`.
    #[serde(default, deserialize_with = "deserialize_optional_optional_string")]
    pub executable_override: Option<Option<String>>,
    pub dangerously_skip_permissions: Option<bool>,
    #[serde(default)]
    pub risk_acknowledged: bool,
}

fn deserialize_optional_optional_string<'de, D>(
    deserializer: D,
) -> std::result::Result<Option<Option<String>>, D::Error>
where
    D: serde::Deserializer<'de>,
{
    Ok(Some(Option::<String>::deserialize(deserializer)?))
}

#[tauri::command]
pub fn get_claude_settings(state: State<'_, AppState>) -> Result<ClaudeSettingsDto> {
    Ok(storage::claude_settings(&state.db.lock().expect("db lock"))?.into())
}

#[tauri::command]
pub fn update_claude_settings(
    state: State<'_, AppState>,
    update: ClaudeSettingsUpdate,
) -> Result<ClaudeSettingsDto> {
    let db = state.db.lock().expect("db lock");
    let mut current = storage::claude_settings(&db)?;
    apply_claude_settings_update(&mut current, &update)?;
    storage::update_claude_settings(&db, &current)?;
    Ok(current.into())
}

#[derive(Debug, Clone, Serialize)]
pub struct ScanSettingsDto {
    pub source_root: Option<String>,
    pub effective_root: String,
    pub scan_interval_seconds: i64,
    pub enabled_provider_ids: Vec<String>,
    pub provider_lookback_days: BTreeMap<String, i64>,
}

#[derive(Debug, Clone, Serialize)]
pub struct SourceRootActivationReportDto {
    pub settings: ScanSettingsDto,
    pub scan: ScanReport,
}

#[tauri::command]
pub fn get_scan_settings(state: State<'_, AppState>) -> Result<ScanSettingsDto> {
    let settings = storage::claude_scan_settings(&state.db.lock().expect("db lock"))?;
    Ok(ScanSettingsDto {
        source_root: settings.source_root.clone(),
        effective_root: settings
            .source_root
            .clone()
            .unwrap_or_else(|| scanner::default_root().display().to_string()),
        scan_interval_seconds: settings.scan_interval_seconds,
        enabled_provider_ids: settings.enabled_provider_ids,
        provider_lookback_days: settings.provider_lookback_days,
    })
}

#[derive(Debug, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct ScanSettingsUpdate {
    pub scan_interval_seconds: i64,
    pub enabled_provider_ids: Vec<String>,
    pub provider_lookback_days: BTreeMap<String, i64>,
}

fn validate_scan_interval(seconds: i64) -> Result<()> {
    if seconds == 0 || (60..=3600).contains(&seconds) {
        Ok(())
    } else {
        Err(AppError::Message(
            "scan interval must be 0 or between 60 and 3600 seconds".into(),
        ))
    }
}

#[tauri::command]
pub fn update_scan_settings(
    state: State<'_, AppState>,
    update: ScanSettingsUpdate,
) -> Result<ScanSettingsDto> {
    validate_scan_interval(update.scan_interval_seconds)?;
    let enabled_provider_ids = validate_enabled_provider_ids(&update.enabled_provider_ids)?;
    let provider_lookback_days = validate_provider_lookback_days(&update.provider_lookback_days)?;
    let lifecycle = state.lifecycle.lock().expect("lifecycle lock");
    if lifecycle.scanning || lifecycle.active_terminal.is_some() {
        return Err(AppError::Message("lifecycle busy".into()));
    }
    let mut db = state.db.lock().expect("db lock");
    let mut settings = storage::claude_scan_settings(&db)?;
    settings.scan_interval_seconds = update.scan_interval_seconds;
    settings.enabled_provider_ids = enabled_provider_ids;
    settings.provider_lookback_days = provider_lookback_days;
    storage::update_claude_scan_settings(&mut db, &settings)?;
    drop(lifecycle);
    Ok(ScanSettingsDto {
        source_root: settings.source_root.clone(),
        effective_root: settings
            .source_root
            .clone()
            .unwrap_or_else(|| scanner::default_root().display().to_string()),
        scan_interval_seconds: settings.scan_interval_seconds,
        enabled_provider_ids: settings.enabled_provider_ids,
        provider_lookback_days: settings.provider_lookback_days,
    })
}

#[derive(Debug, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct ActivateClaudeSourceRootArgs {
    pub source_root: Option<String>,
    pub replace_active_index_acknowledged: bool,
}

#[tauri::command]
pub fn activate_claude_source_root(
    state: State<'_, AppState>,
    args: ActivateClaudeSourceRootArgs,
) -> Result<SourceRootActivationReportDto> {
    let scan_started_at = chrono::Utc::now().timestamp_millis();
    let candidate = args
        .source_root
        .map(PathBuf::from)
        .unwrap_or_else(scanner::default_root);
    let metadata = fs::symlink_metadata(&candidate)
        .map_err(|_| AppError::InvalidRoot(candidate.display().to_string()))?;
    if !metadata.is_dir() || metadata.file_type().is_symlink() {
        return Err(AppError::InvalidRoot(candidate.display().to_string()));
    }
    let canonical = fs::canonicalize(&candidate)?;
    {
        let mut lifecycle = state.lifecycle.lock().expect("lifecycle lock");
        clear_finished_terminal(&state.pty, &state.live_tails, &mut lifecycle)?;
        if lifecycle.scanning || lifecycle.active_terminal.is_some() {
            return Err(AppError::Message("lifecycle busy".into()));
        }
        lifecycle.scanning = true;
    }
    let _scan_guard = ScanGuard {
        lifecycle: Arc::clone(&state.lifecycle),
    };
    // The authoritative replacement check happens only after this command
    // owns the scan gate. A concurrent scan therefore cannot populate the
    // index between the count check and root replacement.
    let db = state.db.lock().expect("db lock");
    let settings = storage::claude_scan_settings(&db)?;
    if !settings
        .enabled_provider_ids
        .iter()
        .any(|id| id == PROVIDER_ID)
    {
        return Err(AppError::Message(
            "Claude import is disabled in scan settings".into(),
        ));
    }
    let current = settings
        .source_root
        .as_ref()
        .map(PathBuf::from)
        .and_then(|p| fs::canonicalize(p).ok());
    let default_canonical = fs::canonicalize(scanner::default_root()).ok();
    let effective_current = if settings.source_root.is_some() {
        current
    } else {
        default_canonical
    };
    if effective_current.as_ref() != Some(&canonical)
        && storage::count(&db)? > 0
        && !args.replace_active_index_acknowledged
    {
        return Err(AppError::Message(
            "replacing active index requires acknowledgement".into(),
        ));
    }
    let modified_since = provider_modified_since(
        &validate_provider_lookback_days(&settings.provider_lookback_days)?,
        PROVIDER_ID,
        scan_started_at,
    );
    drop(db);
    let manifest = storage::source_manifest(&state.db.lock().expect("db lock"), PROVIDER_ID)?;
    let prepared = match prepare_scan(&canonical, &manifest, modified_since) {
        Ok(prepared) => prepared,
        Err(error) => {
            let db = state.db.lock().expect("db lock");
            storage::record_scan_run(
                &db,
                scan_started_at,
                "root_activation",
                "failed",
                false,
                0,
                &[("root_activation_failed".to_owned(), 1)],
            )?;
            return Err(error);
        }
    };
    let (
        sessions,
        mut diagnostics,
        complete,
        new_files,
        changed_files,
        unchanged_files,
        discovered_paths,
    ) = prepared;
    let session_count = discovered_paths.len();
    let partial_sessions = sessions
        .iter()
        .filter(|session| session.summary.partial)
        .count();
    let removed_files = manifest
        .keys()
        .filter(|path| !discovered_paths.contains(path))
        .count();
    if !complete && diagnostics.is_empty() {
        diagnostics.push("root_activation_incomplete".to_owned());
    }
    let grouped_diagnostics = diagnostic_counts(&diagnostics);
    let mut db = state.db.lock().expect("db lock");
    if !complete {
        storage::record_scan_run(
            &db,
            scan_started_at,
            "root_activation",
            "partial",
            false,
            session_count,
            &grouped_diagnostics,
        )?;
        let current = storage::claude_scan_settings(&db)?;
        return Ok(SourceRootActivationReportDto {
            settings: ScanSettingsDto {
                source_root: current.source_root.clone(),
                effective_root: current
                    .source_root
                    .clone()
                    .unwrap_or_else(|| scanner::default_root().display().to_string()),
                scan_interval_seconds: current.scan_interval_seconds,
                enabled_provider_ids: current.enabled_provider_ids,
                provider_lookback_days: current.provider_lookback_days,
            },
            scan: ScanReport {
                root: canonical.display().to_string(),
                trigger: "root_activation".into(),
                outcome: "partial".into(),
                committed: false,
                sessions: session_count,
                diagnostics: diagnostics.len(),
                partial: true,
                removed_sessions: 0,
                new_files,
                changed_files,
                unchanged_files,
                removed_files,
                partial_sessions,
            },
        });
    }

    let persisted_root = match fs::canonicalize(scanner::default_root()) {
        Ok(default) if default == canonical => None,
        _ => Some(canonical.display().to_string()),
    };
    let removed_sessions = match storage::index_incremental_and_set_root(
        &mut db,
        PROVIDER_ID,
        &sessions,
        &discovered_paths,
        persisted_root.as_deref(),
    ) {
        Ok(removed) => removed,
        Err(error) => {
            let mut failed_codes = grouped_diagnostics.clone();
            failed_codes.push(("root_activation_index_failed".to_owned(), 1));
            storage::record_scan_run(
                &db,
                scan_started_at,
                "root_activation",
                "failed",
                false,
                session_count,
                &failed_codes,
            )?;
            return Err(error);
        }
    };
    *state.root.lock().expect("root lock") = canonical.clone();
    *state.last_scan.lock().expect("time lock") = Some(chrono::Utc::now().timestamp_millis());
    storage::record_scan_run(
        &db,
        scan_started_at,
        "root_activation",
        "committed",
        true,
        session_count,
        &grouped_diagnostics,
    )?;
    let current = storage::claude_scan_settings(&db)?;
    let scan = ScanReport {
        root: canonical.display().to_string(),
        trigger: "root_activation".into(),
        outcome: "committed".into(),
        committed: true,
        sessions: session_count,
        diagnostics: diagnostics.len(),
        partial: !diagnostics.is_empty(),
        removed_sessions,
        new_files,
        changed_files,
        unchanged_files,
        removed_files,
        partial_sessions,
    };
    Ok(SourceRootActivationReportDto {
        settings: ScanSettingsDto {
            source_root: current.source_root.clone(),
            effective_root: current
                .source_root
                .clone()
                .unwrap_or_else(|| scanner::default_root().display().to_string()),
            scan_interval_seconds: current.scan_interval_seconds,
            enabled_provider_ids: current.enabled_provider_ids,
            provider_lookback_days: current.provider_lookback_days,
        },
        scan,
    })
}

fn apply_claude_settings_update(
    current: &mut storage::ClaudeSettings,
    update: &ClaudeSettingsUpdate,
) -> Result<()> {
    if let Some(override_value) = update.executable_override.as_ref() {
        if let Some(path) = override_value.as_deref() {
            runtime::validate_executable(PathBuf::from(path).as_path()).map_err(runtime_error)?;
            if path.trim().is_empty() {
                return Err(AppError::Message(
                    "executable override cannot be empty".into(),
                ));
            }
        }
        current.executable_override = override_value.clone();
    }
    if let Some(enabled) = update.dangerously_skip_permissions {
        if !current.dangerously_skip_permissions && enabled && !update.risk_acknowledged {
            return Err(AppError::Message(
                "enabling dangerously_skip_permissions requires risk_acknowledged".into(),
            ));
        }
        current.dangerously_skip_permissions = enabled;
    }
    Ok(())
}

#[derive(Debug, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct ResumeArgs {
    pub session_id: String,
}

#[derive(Debug, Clone, Serialize)]
pub struct ResumePreviewDto {
    pub resolved_executable: String,
    pub version: String,
    pub cwd: String,
    pub args: Vec<String>,
    pub command_preview: String,
}

#[derive(Debug, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct StartContinuationArgs {
    pub session_id: String,
    pub rows: Option<u16>,
    pub cols: Option<u16>,
}

#[derive(Debug, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct StartForkContinuationArgs {
    pub session_id: String,
    pub rows: Option<u16>,
    pub cols: Option<u16>,
}

#[derive(Debug, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct PtyHandleArgs {
    pub handle: String,
}

#[derive(Debug, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct WriteContinuationArgs {
    pub handle: String,
    pub data: Vec<u8>,
}

#[derive(Debug, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct ResizeContinuationArgs {
    pub handle: String,
    pub rows: u16,
    pub cols: u16,
    #[serde(default)]
    pub pixel_width: u16,
    #[serde(default)]
    pub pixel_height: u16,
}

#[derive(Debug, Clone, Serialize)]
pub struct ContinuationEventDto {
    pub kind: String,
    pub data: Option<Vec<u8>>,
    pub status: Option<String>,
    pub message: Option<String>,
}

#[derive(Debug, Clone, Serialize)]
pub struct ContinuationStatusDto {
    pub handle: String,
    pub session_id: String,
    pub parent_session_id: Option<String>,
    pub status: String,
    pub events: Vec<ContinuationEventDto>,
    pub live_events: Vec<crate::domain::LiveTranscriptEvent>,
    pub tail_partial: bool,
    pub tail_diagnostics: usize,
    pub tail_error: Option<String>,
    pub tail_caught_up: bool,
}

#[derive(Debug)]
pub struct LiveTailEntry {
    pub tail: ClaudeLiveTail,
    pub parent_session_id: Option<String>,
}

fn runtime_error(error: RuntimeError) -> AppError {
    AppError::Message(error.to_string())
}

fn cleanup_fork_relation(primary: AppError, cleanup: Result<()>) -> AppError {
    match cleanup {
        Ok(()) => primary,
        Err(error) => AppError::Message(format!("{primary}; relation cleanup failed: {error}")),
    }
}

fn continuation_spec(
    state: &AppState,
    session_id: &str,
) -> Result<(ClaudeResumeSpec, storage::ClaudeSettings)> {
    let db = state.db.lock().expect("db lock");
    let session = storage::session_summary(&db, session_id)?;
    if session.provider_id != PROVIDER_ID {
        return Err(AppError::Message(
            "continuation is only supported for Claude".into(),
        ));
    }
    let native_session_id = session
        .native_session_id
        .ok_or_else(|| AppError::Message("session has no native Claude session id".into()))?;
    let cwd = session
        .cwd
        .ok_or_else(|| AppError::Message("session has no historical working directory".into()))?;
    let settings = storage::claude_settings(&db)?;
    let executable = settings
        .executable_override
        .clone()
        .unwrap_or_else(|| "claude".to_owned());
    let spec = ClaudeResumeSpec::new(
        executable,
        native_session_id,
        cwd,
        settings.dangerously_skip_permissions,
    )
    .map_err(runtime_error)?;
    Ok((spec, settings))
}

fn preview(spec: &ClaudeResumeSpec) -> Result<ResumePreviewDto> {
    let preflight = spec.preflight().map_err(runtime_error)?;
    let version = bounded_version(&preflight.version);
    let args = spec
        .resume_spec()
        .args
        .iter()
        .map(|arg| arg.to_string_lossy().into_owned())
        .collect::<Vec<_>>();
    let executable_display = spec.resume_spec().executable.display().to_string();
    let command_preview = std::iter::once(shell_quote(&executable_display))
        .chain(args.iter().map(|arg| shell_quote(arg)))
        .collect::<Vec<_>>()
        .join(" ");
    Ok(ResumePreviewDto {
        resolved_executable: spec.resume_spec().executable.display().to_string(),
        version,
        cwd: spec.resume_spec().cwd.display().to_string(),
        args,
        command_preview,
    })
}

fn shell_quote(value: &str) -> String {
    format!("'{}'", value.replace('\'', "'\\''"))
}

fn bounded_version(output: &runtime::CommandOutput) -> String {
    let value = if output.stdout.trim().is_empty() {
        &output.stderr
    } else {
        &output.stdout
    };
    value.trim().chars().take(128).collect()
}

#[derive(Debug, Clone, PartialEq, Eq)]
struct SourceIdentity {
    #[cfg(unix)]
    device: u64,
    #[cfg(unix)]
    inode: u64,
    #[cfg(not(unix))]
    canonical_metadata_token: String,
    len: u64,
    modified_secs: u64,
    modified_nanos: u32,
}

#[derive(Debug, Clone, PartialEq, Eq)]
struct ValidatedSource {
    canonical_path: PathBuf,
    identity: SourceIdentity,
}

#[derive(Debug, Clone, PartialEq, Eq)]
struct ValidatedForkPaths {
    source: ValidatedSource,
    target: PathBuf,
}

fn source_identity(metadata: &fs::Metadata, canonical_path: &Path) -> Option<SourceIdentity> {
    use std::time::UNIX_EPOCH;
    let modified = metadata.modified().ok()?.duration_since(UNIX_EPOCH).ok()?;
    #[cfg(unix)]
    {
        use std::os::unix::fs::MetadataExt;
        let _ = canonical_path;
        Some(SourceIdentity {
            device: metadata.dev(),
            inode: metadata.ino(),
            len: metadata.len(),
            modified_secs: modified.as_secs(),
            modified_nanos: modified.subsec_nanos(),
        })
    }
    #[cfg(not(unix))]
    {
        Some(SourceIdentity {
            canonical_metadata_token: format!(
                "{}:{}:{}:{}",
                canonical_path.display(),
                metadata.len(),
                modified.as_secs(),
                modified.subsec_nanos()
            ),
            len: metadata.len(),
            modified_secs: modified.as_secs(),
            modified_nanos: modified.subsec_nanos(),
        })
    }
}

/// Validate the immutable source mapping used by both resume and fork.  This
/// intentionally checks the raw directory entry before canonicalization so a
/// symlink cannot be smuggled into the provider root.
fn validate_continuation_source(
    root: &Path,
    source_raw: &Path,
    native_session_id: &str,
) -> Result<ValidatedSource> {
    let source_meta = fs::symlink_metadata(source_raw)
        .map_err(|_| AppError::Message("parent transcript source is unavailable".into()))?;
    if source_meta.file_type().is_symlink() || !source_meta.file_type().is_file() {
        return Err(AppError::Message(
            "parent transcript source is not a regular file".into(),
        ));
    }
    let canonical_root = fs::canonicalize(root)
        .map_err(|_| AppError::Message("configured root is unavailable".into()))?;
    let source = fs::canonicalize(source_raw)
        .map_err(|_| AppError::Message("parent transcript source is unavailable".into()))?;
    let source_parent = source
        .parent()
        .ok_or_else(|| AppError::Message("parent transcript source has no project root".into()))?;
    if !source.starts_with(&canonical_root) || !source_parent.starts_with(&canonical_root) {
        return Err(AppError::Message(
            "parent transcript source is outside configured root".into(),
        ));
    }
    let filename = source.file_name().and_then(|value| value.to_str());
    let expected = format!("{native_session_id}.jsonl");
    if filename != Some(expected.as_str())
        || source.extension().and_then(|ext| ext.to_str()) != Some("jsonl")
        || uuid::Uuid::parse_str(native_session_id)
            .ok()
            .is_none_or(|uuid| uuid.hyphenated().to_string() != native_session_id)
    {
        return Err(AppError::Message(
            "parent transcript filename is invalid".into(),
        ));
    }
    let opened = fs::File::open(source_raw)
        .map_err(|_| AppError::Message("parent transcript source is unavailable".into()))?;
    let opened_meta = opened
        .metadata()
        .map_err(|_| AppError::Message("parent transcript source is unavailable".into()))?;
    let expected = source_identity(&source_meta, &source)
        .ok_or_else(|| AppError::Message("parent transcript source identity unavailable".into()))?;
    let actual = source_identity(&opened_meta, &source)
        .ok_or_else(|| AppError::Message("parent transcript source identity unavailable".into()))?;
    if expected != actual {
        return Err(AppError::Message(
            "parent transcript source was replaced".into(),
        ));
    }
    Ok(ValidatedSource {
        canonical_path: source,
        identity: expected,
    })
}

fn validate_fork_paths(
    root: &Path,
    source_raw: &Path,
    parent_native: &str,
    child_native: &str,
) -> Result<ValidatedForkPaths> {
    let source = validate_continuation_source(root, source_raw, parent_native)?;
    let source_parent = source
        .canonical_path
        .parent()
        .ok_or_else(|| AppError::Message("parent transcript source has no project root".into()))?;
    let target = source_parent.join(format!("{child_native}.jsonl"));
    match fs::symlink_metadata(&target) {
        Ok(_) => {
            return Err(AppError::Message(
                "fork transcript target already exists".into(),
            ))
        }
        Err(error) if error.kind() == std::io::ErrorKind::NotFound => {}
        Err(_) => {
            return Err(AppError::Message(
                "fork transcript target is unavailable".into(),
            ))
        }
    }
    Ok(ValidatedForkPaths { source, target })
}

#[tauri::command]
pub fn resume_preflight(state: State<'_, AppState>, args: ResumeArgs) -> Result<ResumePreviewDto> {
    let (spec, _) = continuation_spec(&state, &args.session_id)?;
    preview(&spec)
}

#[tauri::command]
pub fn start_continuation(
    state: State<'_, AppState>,
    args: StartContinuationArgs,
) -> Result<ContinuationStatusDto> {
    // Reject a competing start before doing provider preflight. The final
    // check below remains authoritative because this early check is only a
    // fast-path and cannot reserve the slot across the preflight call.
    {
        let mut lifecycle = state.lifecycle.lock().expect("lifecycle lock");
        clear_finished_terminal(&state.pty, &state.live_tails, &mut lifecycle)?;
        if lifecycle.scanning {
            return Err(AppError::Message("a scan is in progress".into()));
        }
        if lifecycle.active_terminal.is_some() {
            return Err(AppError::Message(continuation_in_progress_message(
                &lifecycle,
            )));
        }
    }
    let (initial_spec, _) = continuation_spec(&state, &args.session_id)?;
    // Probe capabilities before taking the gate; a slow or broken CLI cannot
    // mark a continuation active.
    let _initial_preflight = initial_spec.preflight().map_err(runtime_error)?;
    let size = clamped_size(
        args.rows.unwrap_or(DEFAULT_ROWS),
        args.cols.unwrap_or(DEFAULT_COLS),
        0,
        0,
    );
    let mut lifecycle = state.lifecycle.lock().expect("lifecycle lock");
    clear_finished_terminal(&state.pty, &state.live_tails, &mut lifecycle)?;
    if lifecycle.scanning {
        return Err(AppError::Message("a scan is in progress".into()));
    }
    if lifecycle.active_terminal.is_some() {
        return Err(AppError::Message(continuation_in_progress_message(
            &lifecycle,
        )));
    }
    // Preflight may be slow. Re-read the indexed session and source only after
    // taking the final lifecycle gate so a concurrent scan cannot leave us
    // with a stale cwd, executable, or transcript path.
    let (final_spec, _) = continuation_spec(&state, &args.session_id)?;
    let raw_source_path = {
        let db = state.db.lock().expect("db lock");
        storage::source_path_for_session(&db, &args.session_id)?
    };
    // The spec is immutable after this point. Settings updates may proceed in
    // parallel, but can only affect a later start; this launch uses the spec
    // that was probed and captured while holding the lifecycle gate.
    let final_preflight = final_spec.preflight().map_err(runtime_error)?;
    let configured_root = state.root.lock().expect("root lock").clone();
    let validated_source = validate_continuation_source(
        &configured_root,
        &raw_source_path,
        final_spec.native_session_id(),
    )?;
    let mut live_tails = state
        .live_tails
        .lock()
        .map_err(|_| AppError::Message("transcript tail registry lock poisoned".into()))?;
    // Open the tail only after final preflight and registry acquisition. This
    // captures an EOF without ever mutating the provider source.
    let tail = ClaudeLiveTail::new(&raw_source_path, args.session_id.clone())
        .map_err(|_| AppError::Message("cannot start transcript tail".into()))?;
    let revalidated_source = validate_continuation_source(
        &configured_root,
        &raw_source_path,
        final_spec.native_session_id(),
    )?;
    if revalidated_source != validated_source {
        return Err(AppError::Message(
            "parent transcript source changed before continuation spawn".into(),
        ));
    }
    let handle = state
        .pty
        .start(final_spec.resume_spec(), size)
        .map_err(runtime_error)?;
    let handle_string = handle.to_string();
    live_tails.insert(
        handle_string.clone(),
        LiveTailEntry {
            tail,
            parent_session_id: None,
        },
    );
    lifecycle.active_terminal = Some(handle);
    lifecycle.active_session_id = Some(args.session_id.clone());
    let version = bounded_version(&final_preflight.version);
    Ok(ContinuationStatusDto {
        handle: handle_string,
        session_id: args.session_id,
        parent_session_id: None,
        status: format!("started ({version})"),
        events: Vec::new(),
        live_events: Vec::new(),
        tail_partial: false,
        tail_diagnostics: 0,
        tail_error: None,
        tail_caught_up: true,
    })
}

/// Start a native Claude fork while keeping the parent transcript untouched.
/// The child transcript is expected at the provider's canonical project root
/// beside the parent source and is tailed only after a pending-source check.
#[tauri::command]
pub fn start_fork_continuation(
    state: State<'_, AppState>,
    args: StartForkContinuationArgs,
) -> Result<ContinuationStatusDto> {
    {
        let mut lifecycle = state.lifecycle.lock().expect("lifecycle lock");
        clear_finished_terminal(&state.pty, &state.live_tails, &mut lifecycle)?;
        if lifecycle.scanning {
            return Err(AppError::Message("a scan is in progress".into()));
        }
        if lifecycle.active_terminal.is_some() {
            return Err(AppError::Message(continuation_in_progress_message(
                &lifecycle,
            )));
        }
    }

    // Preliminary capability probe uses the existing parent continuation
    // contract and does not reserve the lifecycle gate.
    let (initial_parent_spec, _) = continuation_spec(&state, &args.session_id)?;
    initial_parent_spec.preflight().map_err(runtime_error)?;
    let size = clamped_size(
        args.rows.unwrap_or(DEFAULT_ROWS),
        args.cols.unwrap_or(DEFAULT_COLS),
        0,
        0,
    );

    let mut lifecycle = state.lifecycle.lock().expect("lifecycle lock");
    clear_finished_terminal(&state.pty, &state.live_tails, &mut lifecycle)?;
    if lifecycle.scanning {
        return Err(AppError::Message("a scan is in progress".into()));
    }
    if lifecycle.active_terminal.is_some() {
        return Err(AppError::Message(continuation_in_progress_message(
            &lifecycle,
        )));
    }

    let (parent_spec, settings) = continuation_spec(&state, &args.session_id)?;
    let parent_native = parent_spec.native_session_id().to_owned();
    let parent_source = {
        let db = state.db.lock().expect("db lock");
        storage::source_path_for_session(&db, &args.session_id)?
    };
    let child_native = Uuid::new_v4().hyphenated().to_string();
    let child_session_id = format!("claude:{child_native}");
    let final_spec = runtime::ClaudeForkSpec::new(
        settings
            .executable_override
            .clone()
            .unwrap_or_else(|| "claude".to_owned()),
        parent_native,
        &child_native,
        parent_spec.resume_spec().cwd.clone(),
        settings.dangerously_skip_permissions,
    )
    .map_err(runtime_error)?;
    let final_preflight = final_spec.preflight().map_err(runtime_error)?;
    let configured_root = state.root.lock().expect("root lock").clone();
    // Construct the pending tail only after the longest provider preflight;
    // the final identity/target check is repeated immediately before spawn.
    let first_paths = validate_fork_paths(
        &configured_root,
        &parent_source,
        final_spec.parent_native_session_id(),
        &child_native,
    )?;
    // Reserve the relation before spawning.  This makes a successful spawn
    // observable as pending immediately and guarantees failures cannot leave
    // an untracked child process.
    {
        let db = state.db.lock().expect("db lock");
        storage::register_fork_relation(
            &db,
            PROVIDER_ID,
            &args.session_id,
            &child_session_id,
            chrono::Utc::now().timestamp_millis(),
        )?;
    }
    let mut live_tails = match state.live_tails.lock() {
        Ok(guard) => guard,
        Err(_) => {
            let db = state.db.lock().expect("db lock");
            return Err(cleanup_fork_relation(
                AppError::Message("transcript tail registry lock poisoned".into()),
                storage::remove_fork_relation(
                    &db,
                    PROVIDER_ID,
                    &args.session_id,
                    &child_session_id,
                ),
            ));
        }
    };
    // This is the final raw-path check.  The remaining target race is only the
    // provider's random-UUID file creation at spawn; Session Deck never
    // creates or writes provider transcript files.
    let second_paths = match validate_fork_paths(
        &configured_root,
        &parent_source,
        final_spec.parent_native_session_id(),
        &child_native,
    ) {
        Ok(paths) => paths,
        Err(error) => {
            drop(live_tails);
            let db = state.db.lock().expect("db lock");
            return Err(cleanup_fork_relation(
                error,
                storage::remove_fork_relation(
                    &db,
                    PROVIDER_ID,
                    &args.session_id,
                    &child_session_id,
                ),
            ));
        }
    };
    if second_paths != first_paths {
        drop(live_tails);
        let db = state.db.lock().expect("db lock");
        return Err(cleanup_fork_relation(
            AppError::Message(
                "parent transcript source or fork target changed before spawn".into(),
            ),
            storage::remove_fork_relation(&db, PROVIDER_ID, &args.session_id, &child_session_id),
        ));
    }
    // Rebuild from the second validator result; never continue with a stale
    // target if validation had to be repeated.
    let tail = match ClaudeLiveTail::new_pending(&second_paths.target, child_session_id.clone()) {
        Ok(tail) => tail,
        Err(error) => {
            drop(live_tails);
            let db = state.db.lock().expect("db lock");
            return Err(cleanup_fork_relation(
                AppError::Message(error.to_string()),
                storage::remove_fork_relation(
                    &db,
                    PROVIDER_ID,
                    &args.session_id,
                    &child_session_id,
                ),
            ));
        }
    };
    let handle = match state.pty.start(final_spec.resume_spec(), size) {
        Ok(handle) => handle,
        Err(error) => {
            drop(live_tails);
            let db = state.db.lock().expect("db lock");
            return Err(cleanup_fork_relation(
                runtime_error(error),
                storage::remove_fork_relation(
                    &db,
                    PROVIDER_ID,
                    &args.session_id,
                    &child_session_id,
                ),
            ));
        }
    };
    let handle_string = handle.to_string();
    live_tails.insert(
        handle_string.clone(),
        LiveTailEntry {
            tail,
            parent_session_id: Some(args.session_id.clone()),
        },
    );
    lifecycle.active_terminal = Some(handle);
    lifecycle.active_session_id = Some(child_session_id.clone());

    let version = bounded_version(&final_preflight.version);
    Ok(ContinuationStatusDto {
        handle: handle_string,
        session_id: child_session_id,
        parent_session_id: Some(args.session_id),
        status: format!("started ({version})"),
        events: Vec::new(),
        live_events: Vec::new(),
        tail_partial: false,
        tail_diagnostics: 0,
        tail_error: None,
        tail_caught_up: true,
    })
}

#[tauri::command]
pub fn poll_continuation(
    state: State<'_, AppState>,
    args: PtyHandleArgs,
) -> Result<ContinuationStatusDto> {
    let handle = PtyHandle::parse(&args.handle).map_err(runtime_error)?;
    let events = state
        .pty
        .read_events_limited(handle, MAX_POLL_EVENTS, MAX_POLL_BYTES)
        .map_err(runtime_error)?;
    let observed_status = state.pty.exit_status(handle).map_err(runtime_error)?;
    let tail = poll_live_tail(&state, handle)?;
    let drained = state.pty.events_drained(handle).map_err(runtime_error)?;
    let poll_limited = events.len() >= MAX_POLL_EVENTS
        || events
            .iter()
            .filter_map(|event| match event {
                PtyEvent::Output(bytes) => Some(bytes.len()),
                _ => None,
            })
            .sum::<usize>()
            >= MAX_POLL_BYTES;
    let ready = continuation_status_ready(drained, poll_limited, tail.caught_up);
    let status = observed_status.clone().filter(|_| ready);
    if status.is_some() {
        let mut lifecycle = state.lifecycle.lock().expect("lifecycle lock");
        if lifecycle.active_terminal == Some(handle) {
            lifecycle.active_terminal = None;
            lifecycle.active_session_id = None;
        }
    }
    Ok(ContinuationStatusDto {
        handle: handle.to_string(),
        session_id: tail.session_id,
        parent_session_id: tail.parent_session_id,
        status: continuation_status_label(observed_status.as_deref(), ready),
        events: events.into_iter().map(event_dto).collect(),
        live_events: tail.events,
        tail_partial: tail.partial,
        tail_diagnostics: tail.diagnostics,
        tail_error: tail.error,
        tail_caught_up: tail.caught_up,
    })
}

fn continuation_status_ready(drained: bool, poll_limited: bool, tail_caught_up: bool) -> bool {
    drained && !poll_limited && tail_caught_up
}

fn continuation_status_label(observed_exit: Option<&str>, ready: bool) -> String {
    match (observed_exit, ready) {
        (Some(status), true) => status.to_owned(),
        (Some(_), false) => "draining".to_owned(),
        (None, _) => "running".to_owned(),
    }
}

#[tauri::command]
pub fn write_continuation(state: State<'_, AppState>, args: WriteContinuationArgs) -> Result<()> {
    if args.data.len() > MAX_WRITE_BYTES {
        return Err(AppError::Message("PTY input exceeds 64 KiB".into()));
    }
    let handle = PtyHandle::parse(&args.handle).map_err(runtime_error)?;
    state.pty.write(handle, &args.data).map_err(runtime_error)
}

#[tauri::command]
pub fn resize_continuation(state: State<'_, AppState>, args: ResizeContinuationArgs) -> Result<()> {
    let handle = PtyHandle::parse(&args.handle).map_err(runtime_error)?;
    let size = clamped_size(args.rows, args.cols, args.pixel_width, args.pixel_height);
    state.pty.resize(handle, size).map_err(runtime_error)
}

#[tauri::command]
pub fn close_continuation(state: State<'_, AppState>, args: PtyHandleArgs) -> Result<()> {
    let handle = PtyHandle::parse(&args.handle).map_err(runtime_error)?;
    // Hold the lifecycle gate across the close decision so a stale PTY cannot
    // race a final poll and turn an otherwise active gate into an error.
    let mut lifecycle = state.lifecycle.lock().expect("lifecycle lock");
    let active_match = lifecycle.active_terminal == Some(handle);
    let result = state.pty.close(handle);
    let stale_active = matches!(&result, Err(RuntimeError::UnknownHandle)) && active_match;
    let result = if stale_active { Ok(()) } else { result };
    if result.is_ok() {
        if lifecycle.active_terminal == Some(handle) {
            lifecycle.active_terminal = None;
            lifecycle.active_session_id = None;
        }
        state
            .live_tails
            .lock()
            .map_err(|_| AppError::Message("transcript tail registry lock poisoned".into()))?
            .remove(&handle.to_string());
    }
    result.map_err(runtime_error)
}

fn event_dto(event: PtyEvent) -> ContinuationEventDto {
    match event {
        PtyEvent::Output(bytes) => ContinuationEventDto {
            kind: "output".into(),
            data: Some(bytes),
            status: None,
            message: None,
        },
        PtyEvent::Exited { status } => ContinuationEventDto {
            kind: "exited".into(),
            data: None,
            status: Some(status),
            message: None,
        },
        PtyEvent::Error { message } => ContinuationEventDto {
            kind: "error".into(),
            data: None,
            status: None,
            message: Some(message),
        },
    }
}

fn clamped_size(rows: u16, cols: u16, pixel_width: u16, pixel_height: u16) -> PtySize {
    PtySize {
        rows: rows.clamp(1, 200),
        cols: cols.clamp(1, 500),
        pixel_width: pixel_width.min(10_000),
        pixel_height: pixel_height.min(10_000),
    }
}

fn continuation_in_progress_message(lifecycle: &LifecycleState) -> String {
    lifecycle.active_session_id.as_deref().map_or_else(
        || "a continuation is in progress".to_owned(),
        |session_id| format!("a continuation is in progress for session {session_id}"),
    )
}

struct LiveTailPoll {
    session_id: String,
    parent_session_id: Option<String>,
    events: Vec<crate::domain::LiveTranscriptEvent>,
    partial: bool,
    diagnostics: usize,
    error: Option<String>,
    caught_up: bool,
}

fn poll_live_tail(state: &AppState, handle: PtyHandle) -> Result<LiveTailPoll> {
    // Serialize tail polls through the registry mutex. Keeping the tailer in
    // the map avoids a concurrent poll observing a false "unavailable"
    // error and preserves it if the source reports a transient error.
    let mut tails = state
        .live_tails
        .lock()
        .map_err(|_| AppError::Message("transcript tail registry lock poisoned".into()))?;
    let entry = tails
        .get_mut(&handle.to_string())
        .ok_or_else(|| AppError::Message("continuation transcript tail is unavailable".into()))?;
    let session_id = entry.tail.session_id().to_owned();
    let parent_session_id = entry.parent_session_id.clone();
    let (events, partial, diagnostics, tail_error, caught_up) =
        poll_live_tail_result(&mut entry.tail);
    drop(tails);
    Ok(LiveTailPoll {
        session_id,
        parent_session_id,
        events,
        partial,
        diagnostics,
        error: tail_error,
        caught_up,
    })
}

fn poll_live_tail_result(
    tail: &mut ClaudeLiveTail,
) -> (
    Vec<crate::domain::LiveTranscriptEvent>,
    bool,
    usize,
    Option<String>,
    bool,
) {
    match tail.poll() {
        Ok(snapshot) => (
            snapshot.events,
            snapshot.partial,
            snapshot.diagnostics,
            None,
            snapshot.caught_up,
        ),
        Err(_) => {
            let snapshot = tail.current_snapshot();
            (
                Vec::new(),
                snapshot.partial,
                snapshot.diagnostics,
                Some("transcript tail unavailable".to_owned()),
                true,
            )
        }
    }
}

fn clear_finished_terminal(
    pty: &PtyManager,
    live_tails: &Arc<Mutex<HashMap<String, LiveTailEntry>>>,
    lifecycle: &mut LifecycleState,
) -> Result<()> {
    let Some(handle) = lifecycle.active_terminal else {
        let (handles, invalid_keys) = live_tails
            .lock()
            .map_err(|_| AppError::Message("transcript tail registry lock poisoned".into()))?
            .keys()
            .fold(
                (Vec::new(), Vec::new()),
                |(mut handles, mut invalid), value| {
                    if let Ok(handle) = PtyHandle::parse(value) {
                        handles.push(handle);
                    } else {
                        invalid.push(value.clone());
                    }
                    (handles, invalid)
                },
            );
        if !invalid_keys.is_empty() {
            let mut tails = live_tails
                .lock()
                .map_err(|_| AppError::Message("transcript tail registry lock poisoned".into()))?;
            for key in invalid_keys {
                tails.remove(&key);
            }
        }
        for handle in handles {
            if pty.reap_if_inactive(handle).map_err(runtime_error)? {
                live_tails
                    .lock()
                    .map_err(|_| {
                        AppError::Message("transcript tail registry lock poisoned".into())
                    })?
                    .remove(&handle.to_string());
            }
        }
        lifecycle.active_session_id = None;
        return Ok(());
    };
    if pty.reap_if_inactive(handle).map_err(runtime_error)? {
        lifecycle.active_terminal = None;
        lifecycle.active_session_id = None;
        live_tails
            .lock()
            .map_err(|_| AppError::Message("transcript tail registry lock poisoned".into()))?
            .remove(&handle.to_string());
    }
    Ok(())
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::io::Write;

    fn prepare_test_file_providers(
        connection: &Connection,
        claude_root: &Path,
        codex_root: &Path,
        gemini_root: &Path,
        enabled_provider_ids: &[String],
    ) -> Vec<PreparedFileProvider> {
        ProviderRegistry::builtin_session_providers()
            .into_iter()
            .filter(|provider| enabled_provider_ids.iter().any(|id| id == provider.id()))
            .filter_map(|provider| {
                let root = match provider.id() {
                    PROVIDER_ID => claude_root,
                    "codex" => codex_root,
                    "gemini" => gemini_root,
                    _ => return None,
                };
                Some(prepare_file_provider(
                    provider,
                    root,
                    storage::source_manifest(connection, provider.id()),
                    None,
                ))
            })
            .collect()
    }

    #[cfg(unix)]
    struct ReplacingProvider {
        source: PathBuf,
    }

    #[cfg(unix)]
    impl SessionProvider for ReplacingProvider {
        fn id(&self) -> &'static str {
            "test"
        }

        fn default_root(&self) -> PathBuf {
            self.source.clone()
        }

        fn discover(&self, _root: &Path) -> Result<crate::providers::SourceDiscovery> {
            Ok(crate::providers::SourceDiscovery {
                paths: vec![self.source.clone()],
                diagnostics: Vec::new(),
                complete: true,
            })
        }

        fn parse(&self, path: &Path) -> Result<crate::domain::ParsedSession> {
            let before = scanner::fingerprint(path)?;
            let replacement = path.with_extension("replacement");
            std::fs::write(&replacement, b"replacement").unwrap();
            std::fs::rename(&replacement, path).unwrap();
            let summary = SessionSummary {
                source_mtime: before.mtime,
                ..Default::default()
            };
            Ok(crate::domain::ParsedSession {
                source_path: path.to_path_buf(),
                summary,
                events: Vec::new(),
                turns: Vec::new(),
                branches: Vec::new(),
                diagnostics: Vec::new(),
                source_size: before.size,
                source_hash: before.hash,
                cwd_history: Vec::new(),
            })
        }
    }

    struct RecoverableDiagnosticProvider {
        source: PathBuf,
    }

    impl SessionProvider for RecoverableDiagnosticProvider {
        fn id(&self) -> &'static str {
            "test"
        }

        fn default_root(&self) -> PathBuf {
            self.source.clone()
        }

        fn discover(&self, _root: &Path) -> Result<crate::providers::SourceDiscovery> {
            Ok(crate::providers::SourceDiscovery {
                paths: vec![self.source.clone()],
                diagnostics: Vec::new(),
                complete: true,
            })
        }

        fn parse(&self, path: &Path) -> Result<crate::domain::ParsedSession> {
            let fingerprint = scanner::fingerprint(path)?;
            Ok(crate::domain::ParsedSession {
                source_path: path.to_path_buf(),
                summary: SessionSummary {
                    id: "test:recoverable".into(),
                    provider_id: "test".into(),
                    project_id: "test-project".into(),
                    title: "Recoverable diagnostic".into(),
                    source_title: "Recoverable diagnostic".into(),
                    source_mtime: fingerprint.mtime,
                    partial: true,
                    ..Default::default()
                },
                events: Vec::new(),
                turns: Vec::new(),
                branches: Vec::new(),
                diagnostics: vec![crate::domain::Diagnostic {
                    line: 1,
                    code: "conversation_graph_cycle".into(),
                }],
                source_size: fingerprint.size,
                source_hash: fingerprint.hash,
                cwd_history: Vec::new(),
            })
        }
    }

    #[cfg(unix)]
    #[test]
    fn post_parse_path_replacement_makes_the_scan_incomplete() {
        let root = tempfile::tempdir().unwrap();
        let source = root.path().join("session.jsonl");
        std::fs::write(&source, b"original-data").unwrap();
        let provider = ReplacingProvider {
            source: source.clone(),
        };

        let (sessions, diagnostics, complete, ..) =
            prepare_provider_scan(&provider, root.path(), &HashMap::new(), None).unwrap();

        assert_eq!(sessions.len(), 1);
        assert!(!complete);
        assert!(diagnostics
            .iter()
            .any(|code| code == "source_changed_during_scan"));
    }

    #[test]
    fn stable_partial_session_commits_and_keeps_its_diagnostic() {
        let root = tempfile::tempdir().unwrap();
        let source = root.path().join("session.jsonl");
        std::fs::write(&source, b"synthetic").unwrap();
        let provider = RecoverableDiagnosticProvider {
            source: source.clone(),
        };
        let database = tempfile::tempdir().unwrap();
        let mut connection = storage::open(&database.path().join("index.db")).unwrap();

        let summary = scan_file_provider(&mut connection, &provider, root.path());

        assert!(summary.committed);
        assert!(summary.partial);
        assert_eq!(summary.partial_sessions, 1);
        assert!(summary
            .diagnostics
            .iter()
            .any(|code| code == "test:conversation_graph_cycle"));
        assert_eq!(storage::count(&connection).unwrap(), 1);
    }

    #[test]
    fn synthetic_multi_provider_scan_aggregates_without_touching_default_roots() {
        let claude = tempfile::tempdir().unwrap();
        let claude_project = claude.path().join("project");
        std::fs::create_dir_all(&claude_project).unwrap();
        std::fs::write(
            claude_project.join("session.jsonl"),
            br#"{"type":"user","sessionId":"claude-1","message":{"role":"user","content":"hello claude"}}
{"type":"assistant","message":{"role":"assistant","content":"done"}}
"#,
        )
        .unwrap();

        let codex = tempfile::tempdir().unwrap();
        let codex_day = codex.path().join("sessions/2026/01/01");
        std::fs::create_dir_all(&codex_day).unwrap();
        std::fs::write(
            codex_day.join("rollout.jsonl"),
            br#"{"type":"session_meta","payload":{"session_id":"codex-1","agent_role":"main"}}
{"type":"response_item","payload":{"message":{"role":"user","content":[{"type":"input_text","text":"hello codex"}]}}}
"#,
        )
        .unwrap();

        let gemini = tempfile::tempdir().unwrap();
        let gemini_chats = gemini.path().join("tmp/project/chats");
        std::fs::create_dir_all(&gemini_chats).unwrap();
        std::fs::write(
            gemini_chats.join("session-gemini.json"),
            br#"{"sessionId":"gemini-1","projectHash":"hash","messages":[{"type":"user","content":"hello gemini"},{"type":"gemini","content":"done"}]}
"#,
        )
        .unwrap();

        let database = tempfile::tempdir().unwrap();
        let mut connection = storage::open(&database.path().join("index.db")).unwrap();
        let missing_opencode = database.path().join("missing-opencode.db");
        let claude_ids = vec!["claude".into()];
        let prepared = prepare_test_file_providers(
            &connection,
            claude.path(),
            codex.path(),
            gemini.path(),
            &claude_ids,
        );
        let claude_only = scan_all_providers(&mut connection, prepared, &claude_ids, None);
        assert_eq!(claude_only.len(), 1);
        assert_eq!(storage::count(&connection).unwrap(), 1);
        let all_ids = vec![
            "claude".into(),
            "codex".into(),
            "gemini".into(),
            "opencode".into(),
        ];
        let prepared = prepare_test_file_providers(
            &connection,
            claude.path(),
            codex.path(),
            gemini.path(),
            &all_ids,
        );
        let summaries = scan_all_providers(
            &mut connection,
            prepared,
            &all_ids,
            Some(prepare_opencode_provider(
                &missing_opencode,
                &OpenCodeIndexSnapshot::default(),
                None,
            )),
        );
        assert_eq!(
            summaries.iter().filter(|summary| summary.committed).count(),
            3
        );
        assert_eq!(
            summaries
                .iter()
                .map(|summary| summary.sessions)
                .sum::<usize>(),
            3
        );
        assert_eq!(storage::count(&connection).unwrap(), 3);
        assert_eq!(
            storage::search_filtered(&connection, "hello", Some("codex"))
                .unwrap()
                .len(),
            1
        );
    }

    #[test]
    fn unchanged_opencode_database_skips_session_parsing() {
        let source = tempfile::NamedTempFile::new().unwrap();
        let source_connection = Connection::open(source.path()).unwrap();
        source_connection
            .execute_batch(
                "CREATE TABLE session (
                    id TEXT PRIMARY KEY, project_id TEXT NOT NULL, parent_id TEXT,
                    directory TEXT, title TEXT, time_created INTEGER, time_updated INTEGER,
                    workspace_id TEXT
                );
                CREATE TABLE message (id TEXT PRIMARY KEY, session_id TEXT NOT NULL, data TEXT NOT NULL);
                CREATE TABLE part (id TEXT PRIMARY KEY, message_id TEXT NOT NULL, data TEXT NOT NULL);
                INSERT INTO session VALUES ('one', 'project', NULL, '/tmp/project', 'One', 1, 2, NULL);",
            )
            .unwrap();
        drop(source_connection);

        let database = tempfile::tempdir().unwrap();
        let mut index = storage::open(&database.path().join("index.db")).unwrap();
        let first =
            prepare_opencode_provider(source.path(), &OpenCodeIndexSnapshot::default(), None);
        assert!(reconcile_opencode_provider(&mut index, first).committed);
        let mut manifest = storage::source_manifest(&index, OPENCODE_PROVIDER_ID).unwrap();
        let (sessions, partial_sessions) =
            storage::provider_session_counts(&index, OPENCODE_PROVIDER_ID).unwrap();
        let snapshot = OpenCodeIndexSnapshot {
            source: manifest.remove(source.path()),
            sessions,
            partial_sessions,
        };

        let unchanged = prepare_opencode_provider(source.path(), &snapshot, None);

        assert!(unchanged.summary.committed);
        assert_eq!(unchanged.summary.sessions, 1);
        assert_eq!(unchanged.summary.unchanged_files, 1);
        assert!(unchanged.sessions.is_none());
    }

    #[test]
    fn missing_or_partial_provider_scan_keeps_existing_rows() {
        let root = tempfile::tempdir().unwrap();
        let project = root.path().join("project");
        std::fs::create_dir_all(&project).unwrap();
        let source = project.join("session.jsonl");
        std::fs::write(
            &source,
            br#"{"type":"user","sessionId":"keep","message":{"role":"user","content":"keep"}}
"#,
        )
        .unwrap();
        let database = tempfile::tempdir().unwrap();
        let mut connection = storage::open(&database.path().join("index.db")).unwrap();
        let first = scan_file_provider(&mut connection, &ClaudeProvider, root.path());
        assert!(first.committed);
        assert_eq!(storage::count(&connection).unwrap(), 1);

        std::fs::remove_dir_all(root.path()).unwrap();
        let missing = scan_file_provider(&mut connection, &ClaudeProvider, root.path());
        assert!(!missing.committed);
        assert_eq!(storage::count(&connection).unwrap(), 1);

        std::fs::create_dir_all(&project).unwrap();
        std::fs::write(&source, b"not-json\n").unwrap();
        let partial = scan_file_provider(&mut connection, &ClaudeProvider, root.path());
        assert!(!partial.committed);
        assert!(partial.partial);
        assert_eq!(storage::count(&connection).unwrap(), 1);
    }

    #[test]
    fn failed_provider_does_not_block_other_commit_or_delete_old_rows() {
        let codex_good = tempfile::tempdir().unwrap();
        let codex_day = codex_good.path().join("sessions/2026/01/01");
        std::fs::create_dir_all(&codex_day).unwrap();
        std::fs::write(
            codex_day.join("rollout.jsonl"),
            br#"{"type":"session_meta","payload":{"session_id":"old","agent_role":"main"}}
{"type":"response_item","payload":{"message":{"role":"user","content":[{"type":"input_text","text":"old codex"}]}}}
"#,
        )
        .unwrap();
        let database = tempfile::tempdir().unwrap();
        let mut connection = storage::open(&database.path().join("index.db")).unwrap();
        assert!(scan_file_provider(&mut connection, &CodexProvider, codex_good.path()).committed);

        let claude = tempfile::tempdir().unwrap();
        let project = claude.path().join("project");
        std::fs::create_dir_all(&project).unwrap();
        std::fs::write(
            project.join("new.jsonl"),
            br#"{"type":"user","sessionId":"new","message":{"role":"user","content":"new claude"}}
"#,
        )
        .unwrap();
        let codex_failed = database.path().join("codex-root-file");
        std::fs::write(&codex_failed, b"not a root").unwrap();
        let enabled = vec!["claude".into(), "codex".into()];
        let prepared = prepare_test_file_providers(
            &connection,
            claude.path(),
            &codex_failed,
            &database.path().join("missing-gemini"),
            &enabled,
        );
        let summaries = scan_all_providers(&mut connection, prepared, &enabled, None);
        assert!(summaries[0].committed);
        assert!(summaries[1].partial);
        assert_eq!(
            storage::search_filtered(&connection, "old", Some("codex"))
                .unwrap()
                .len(),
            1
        );
        assert_eq!(
            storage::search_filtered(&connection, "new", Some("claude"))
                .unwrap()
                .len(),
            1
        );
    }

    #[test]
    fn dimensions_are_clamped_to_safe_bounds() {
        let size = clamped_size(0, u16::MAX, u16::MAX, u16::MAX);
        assert_eq!(size.rows, 1);
        assert_eq!(size.cols, 500);
        assert_eq!(size.pixel_width, 10_000);
        assert_eq!(size.pixel_height, 10_000);
    }

    #[test]
    fn scan_interval_accepts_disabled_and_documented_boundaries() {
        assert!(validate_scan_interval(0).is_ok());
        assert!(validate_scan_interval(60).is_ok());
        assert!(validate_scan_interval(3600).is_ok());
        assert!(validate_scan_interval(59).is_err());
        assert!(validate_scan_interval(3601).is_err());
        assert!(validate_scan_interval(-1).is_err());
    }

    #[test]
    fn enabled_providers_are_known_unique_and_registry_ordered() {
        assert_eq!(
            validate_enabled_provider_ids(&["codex".into(), "claude".into()]).unwrap(),
            vec!["claude", "codex"]
        );
        assert!(validate_enabled_provider_ids(&["claude".into(), "claude".into()]).is_err());
        assert!(validate_enabled_provider_ids(&["unknown".into()]).is_err());
    }

    #[test]
    fn provider_lookbacks_accept_only_known_agents_and_dropdown_values() {
        let values = BTreeMap::from([("gemini".to_owned(), 30)]);
        assert_eq!(validate_provider_lookback_days(&values).unwrap(), values);
        assert!(
            validate_provider_lookback_days(&BTreeMap::from([("unknown".to_owned(), 30)])).is_err()
        );
        assert!(
            validate_provider_lookback_days(&BTreeMap::from([("gemini".to_owned(), 31)])).is_err()
        );
    }

    #[test]
    fn command_preview_quotes_args_and_does_not_include_help() {
        let preview = shell_quote("--resume");
        assert_eq!(preview, "'--resume'");
    }

    #[test]
    fn fork_cleanup_preserves_primary_and_cleanup_errors() {
        let primary = AppError::Message("spawn failed".into());
        let cleanup = Err(AppError::Message("database locked".into()));
        let combined = cleanup_fork_relation(primary, cleanup);
        assert_eq!(
            combined.to_string(),
            "spawn failed; relation cleanup failed: database locked"
        );
        let primary = AppError::Message("tail failed".into());
        assert_eq!(
            cleanup_fork_relation(primary, Ok(())).to_string(),
            "tail failed"
        );
    }

    #[test]
    fn fork_target_validation_rejects_outside_and_collisions() {
        let root = tempfile::tempdir().unwrap();
        let project = root.path().join("project");
        std::fs::create_dir_all(&project).unwrap();
        let source = project.join("123e4567-e89b-12d3-a456-426614174000.jsonl");
        std::fs::write(&source, "seed\n").unwrap();
        let target = validate_fork_paths(
            root.path(),
            &source,
            "123e4567-e89b-12d3-a456-426614174000",
            "123e4567-e89b-12d3-a456-426614174001",
        )
        .unwrap();
        assert_eq!(
            target.target,
            fs::canonicalize(&project)
                .unwrap()
                .join("123e4567-e89b-12d3-a456-426614174001.jsonl")
        );
        std::fs::write(&target.target, "collision\n").unwrap();
        assert!(validate_fork_paths(
            root.path(),
            &source,
            "123e4567-e89b-12d3-a456-426614174000",
            "123e4567-e89b-12d3-a456-426614174001"
        )
        .is_err());
        let outside = tempfile::NamedTempFile::new().unwrap();
        let outside_path = outside.path().with_extension("jsonl");
        std::fs::rename(outside.path(), &outside_path).unwrap();
        assert!(validate_fork_paths(
            root.path(),
            &outside_path,
            "123e4567-e89b-12d3-a456-426614174000",
            "123e4567-e89b-12d3-a456-426614174002"
        )
        .is_err());
    }

    #[test]
    fn continuation_source_validator_rejects_filename_native_mismatch() {
        let root = tempfile::tempdir().unwrap();
        let project = root.path().join("project");
        std::fs::create_dir_all(&project).unwrap();
        let source = project.join("123e4567-e89b-12d3-a456-426614174000.jsonl");
        std::fs::write(&source, "seed\n").unwrap();
        let error = validate_continuation_source(
            root.path(),
            &source,
            "123e4567-e89b-12d3-a456-426614174001",
        )
        .unwrap_err();
        assert!(error.to_string().contains("filename is invalid"));
        assert!(!error
            .to_string()
            .contains(source.to_string_lossy().as_ref()));
    }

    #[cfg(unix)]
    #[test]
    fn fork_target_validation_rejects_parent_symlink() {
        let root = tempfile::tempdir().unwrap();
        let real = root.path().join("real");
        std::fs::create_dir_all(&real).unwrap();
        let source = real.join("123e4567-e89b-12d3-a456-426614174000.jsonl");
        std::fs::write(&source, "seed\n").unwrap();
        let linked_source = root.path().join("linked.jsonl");
        std::os::unix::fs::symlink(&source, &linked_source).unwrap();
        assert!(validate_fork_paths(
            root.path(),
            &linked_source,
            "123e4567-e89b-12d3-a456-426614174000",
            "123e4567-e89b-12d3-a456-426614174001"
        )
        .is_err());
    }

    #[test]
    fn settings_update_distinguishes_missing_from_null_override() {
        let missing: ClaudeSettingsUpdate =
            serde_json::from_str(r#"{"dangerouslySkipPermissions":false}"#).unwrap();
        assert_eq!(missing.executable_override, None);
        let reset: ClaudeSettingsUpdate =
            serde_json::from_str(r#"{"executableOverride":null}"#).unwrap();
        assert_eq!(reset.executable_override, Some(None));
        let custom: ClaudeSettingsUpdate =
            serde_json::from_str(r#"{"executableOverride":"claude"}"#).unwrap();
        assert_eq!(custom.executable_override, Some(Some("claude".to_owned())));
    }

    #[test]
    fn continuation_inputs_accept_frontend_camel_case_fields() {
        let resume: ResumeArgs = serde_json::from_str(r#"{"sessionId":"claude:session"}"#).unwrap();
        assert_eq!(resume.session_id, "claude:session");

        let start: StartContinuationArgs =
            serde_json::from_str(r#"{"sessionId":"claude:session","rows":30,"cols":100}"#).unwrap();
        assert_eq!(start.session_id, "claude:session");
        assert_eq!((start.rows, start.cols), (Some(30), Some(100)));

        let resize: ResizeContinuationArgs = serde_json::from_str(
            r#"{"handle":"123e4567-e89b-12d3-a456-426614174000","rows":24,"cols":80,"pixelWidth":900,"pixelHeight":300}"#,
        )
        .unwrap();
        assert_eq!((resize.pixel_width, resize.pixel_height), (900, 300));
    }

    #[test]
    fn session_commands_accept_camel_case_and_preserve_title_null_semantics() {
        let hidden: SetSessionHiddenArgs =
            serde_json::from_str(r#"{"sessionId":"session-1","hidden":true}"#).unwrap();
        assert_eq!(hidden.session_id, "session-1");
        assert!(hidden.hidden);

        let reset: RenameSessionArgs =
            serde_json::from_str(r#"{"sessionId":"session-1","title":null}"#).unwrap();
        assert_eq!(reset.session_id, "session-1");
        assert_eq!(reset.title, None);

        let empty: RenameSessionArgs =
            serde_json::from_str(r#"{"sessionId":"session-1","title":""}"#).unwrap();
        assert_eq!(empty.title, Some(String::new()));

        let pinned: SetSessionPinnedArgs =
            serde_json::from_str(r#"{"sessionId":"session-1","pinned":false}"#).unwrap();
        assert_eq!(pinned.session_id, "session-1");
        assert!(!pinned.pinned);

        let touch: TouchSessionArgs = serde_json::from_str(r#"{"sessionId":"session-1"}"#).unwrap();
        assert_eq!(touch.session_id, "session-1");
    }

    #[test]
    fn project_alias_wire_uses_provider_id_and_camel_case_arguments() {
        let args: SetProjectAliasArgs = serde_json::from_str(
            r#"{"providerId":"codex","workspaceId":"/repo/.git","alias":"Codex"}"#,
        )
        .unwrap();
        assert_eq!(args.provider_id, "codex");
        assert_eq!(args.workspace_id, "/repo/.git");
        assert_eq!(args.alias.as_deref(), Some("Codex"));
    }

    #[test]
    fn scan_guard_releases_scan_gate_on_drop() {
        let lifecycle = Arc::new(Mutex::new(LifecycleState {
            scanning: true,
            active_terminal: None,
            active_session_id: None,
        }));
        {
            let _guard = ScanGuard {
                lifecycle: Arc::clone(&lifecycle),
            };
        }
        assert!(!lifecycle.lock().unwrap().scanning);
    }

    #[test]
    fn continuation_conflict_names_active_session_without_source_details() {
        let lifecycle = LifecycleState {
            scanning: false,
            active_terminal: None,
            active_session_id: Some("session-42".to_owned()),
        };
        let message = continuation_in_progress_message(&lifecycle);
        assert_eq!(
            message,
            "a continuation is in progress for session session-42"
        );
        assert!(!message.contains(".jsonl"));
    }

    #[test]
    fn clear_finished_terminal_removes_orphaned_live_tails() {
        let file = tempfile::NamedTempFile::new().expect("tail source");
        let tail = ClaudeLiveTail::new(file.path(), "session-42").expect("tail");
        let tails = Arc::new(Mutex::new(HashMap::from([(
            "stale".to_owned(),
            LiveTailEntry {
                tail,
                parent_session_id: None,
            },
        )])));
        let mut lifecycle = LifecycleState::default();
        clear_finished_terminal(&PtyManager::new(), &tails, &mut lifecycle).expect("clear tails");
        assert!(tails.lock().unwrap().is_empty());
        assert!(lifecycle.active_session_id.is_none());
    }

    #[test]
    fn continuation_status_serializes_the_live_tail_contract() {
        let dto = ContinuationStatusDto {
            handle: "handle".to_owned(),
            session_id: "session".to_owned(),
            parent_session_id: None,
            status: "running".to_owned(),
            events: Vec::new(),
            live_events: Vec::new(),
            tail_partial: true,
            tail_diagnostics: 2,
            tail_error: Some("transcript tail unavailable".to_owned()),
            tail_caught_up: true,
        };
        let value = serde_json::to_value(dto).expect("serialize status");
        assert_eq!(value["session_id"], "session");
        assert_eq!(value["tail_partial"], true);
        assert_eq!(value["tail_diagnostics"], 2);
        assert_eq!(value["tail_error"], "transcript tail unavailable");
    }

    #[test]
    fn final_status_requires_pty_drain_poll_quiet_and_tail_caught_up() {
        assert!(continuation_status_ready(true, false, true));
        assert!(!continuation_status_ready(false, false, true));
        assert!(!continuation_status_ready(true, true, true));
        assert!(!continuation_status_ready(true, false, false));
        assert_eq!(continuation_status_label(Some("exit"), false), "draining");
        assert_eq!(continuation_status_label(Some("exit"), true), "exit");
        assert_eq!(continuation_status_label(None, false), "running");
    }

    #[test]
    fn tail_error_preserves_malformed_diagnostics_and_partial_state() {
        let mut file = tempfile::NamedTempFile::new().expect("tail source");
        let mut tail = ClaudeLiveTail::new(file.path(), "session").expect("tail");
        file.write_all(b"not-json\n").expect("malformed line");
        file.flush().expect("flush malformed line");

        let (_, partial, diagnostics, error, _) = poll_live_tail_result(&mut tail);
        assert!(!partial);
        assert_eq!(diagnostics, 1);
        assert!(error.is_none());

        file.as_file_mut().set_len(0).expect("truncate source");
        let (_, partial, diagnostics, error, _) = poll_live_tail_result(&mut tail);
        assert!(!partial);
        assert_eq!(diagnostics, 1);
        assert_eq!(error.as_deref(), Some("transcript tail unavailable"));
    }

    #[test]
    fn output_events_preserve_raw_utf8_chunks() {
        let dto = event_dto(PtyEvent::Output(vec![0xf0, 0x9f, 0x8c, 0x8d]));
        assert_eq!(dto.data, Some(vec![0xf0, 0x9f, 0x8c, 0x8d]));
    }

    #[test]
    fn settings_update_requires_ack_for_dangerous_mode_and_allows_reset() {
        let mut current = storage::ClaudeSettings {
            executable_override: Some("/tmp/claude".to_owned()),
            dangerously_skip_permissions: false,
        };
        let reset = ClaudeSettingsUpdate {
            executable_override: Some(None),
            dangerously_skip_permissions: None,
            risk_acknowledged: false,
        };
        apply_claude_settings_update(&mut current, &reset).unwrap();
        assert_eq!(current.executable_override, None);

        let denied = ClaudeSettingsUpdate {
            executable_override: None,
            dangerously_skip_permissions: Some(true),
            risk_acknowledged: false,
        };
        assert!(apply_claude_settings_update(&mut current, &denied).is_err());
        let accepted = ClaudeSettingsUpdate {
            risk_acknowledged: true,
            ..denied
        };
        apply_claude_settings_update(&mut current, &accepted).unwrap();
        assert!(current.dangerously_skip_permissions);
    }
}
