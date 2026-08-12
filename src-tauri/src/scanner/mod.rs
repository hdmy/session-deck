use crate::{
    domain::{ParsedSession, Result},
    providers::{claude::ClaudeProvider, SessionProvider},
};
use sha2::{Digest, Sha256};
#[cfg(unix)]
use std::os::unix::fs::MetadataExt;
use std::{
    collections::{HashMap, HashSet},
    fs::File,
    io::{BufReader, ErrorKind, Read},
    path::{Path, PathBuf},
};

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct SourceFingerprint {
    pub path: PathBuf,
    pub size: i64,
    pub mtime: i64,
    pub hash: String,
    #[cfg(unix)]
    pub dev: u64,
    #[cfg(unix)]
    pub ino: u64,
}

#[derive(Debug, Default)]
pub struct ScanPlan {
    pub discovered: Vec<SourceFingerprint>,
    pub parse: Vec<SourceFingerprint>,
    pub unchanged: usize,
    pub new_files: usize,
    pub changed_files: usize,
    pub diagnostics: Vec<String>,
    pub complete: bool,
}

fn mtime(metadata: &std::fs::Metadata) -> i64 {
    metadata
        .modified()
        .ok()
        .and_then(|time| time.duration_since(std::time::UNIX_EPOCH).ok())
        .map(|d| d.as_millis() as i64)
        .unwrap_or_default()
}

/// Hashes a source while proving that size and mtime did not change during the read.
pub fn fingerprint(path: &Path) -> Result<SourceFingerprint> {
    let entry = std::fs::symlink_metadata(path)?;
    if entry.file_type().is_symlink() || !entry.is_file() {
        return Err(crate::domain::AppError::InvalidRoot(
            path.display().to_string(),
        ));
    }
    let before = std::fs::metadata(path)?;
    if !before.is_file() {
        return Err(crate::domain::AppError::InvalidRoot(
            path.display().to_string(),
        ));
    }
    let mut opened = File::open(path)?;
    let opened_metadata = opened.metadata()?;
    #[cfg(unix)]
    if opened_metadata.dev() != before.dev() || opened_metadata.ino() != before.ino() {
        return Err(crate::domain::AppError::Message(
            "source_changed_during_scan".into(),
        ));
    }
    let mut hasher = Sha256::new();
    let mut buffer = [0_u8; 32 * 1024];
    loop {
        let read = opened.read(&mut buffer)?;
        if read == 0 {
            break;
        }
        hasher.update(&buffer[..read]);
    }
    let hash = format!("{:x}", hasher.finalize());
    let after = std::fs::metadata(path)?;
    let final_entry = std::fs::symlink_metadata(path)?;
    if final_entry.file_type().is_symlink() || !final_entry.is_file() {
        return Err(crate::domain::AppError::Message(
            "source_changed_during_scan".into(),
        ));
    }
    #[cfg(unix)]
    if final_entry.dev() != opened_metadata.dev() || final_entry.ino() != opened_metadata.ino() {
        return Err(crate::domain::AppError::Message(
            "source_changed_during_scan".into(),
        ));
    }
    if before.len() != after.len() || before.modified().ok() != after.modified().ok() {
        return Err(crate::domain::AppError::Message(
            "source_changed_during_scan".into(),
        ));
    }
    Ok(SourceFingerprint {
        path: path.to_path_buf(),
        size: after.len() as i64,
        mtime: mtime(&after),
        hash,
        #[cfg(unix)]
        dev: opened_metadata.dev(),
        #[cfg(unix)]
        ino: opened_metadata.ino(),
    })
}

/// Build a read-only plan. Existing manifest entries are compared by path and
/// full SHA-256 fingerprint, so same-size rewrites are still detected.
pub fn plan_provider_scan(
    provider: &dyn SessionProvider,
    root: &Path,
    manifest: &HashMap<PathBuf, SourceFingerprint>,
    modified_since: Option<i64>,
) -> Result<ScanPlan> {
    let discovery = provider.discover(root)?;
    let mut plan = ScanPlan {
        complete: discovery.complete,
        diagnostics: discovery.diagnostics,
        ..Default::default()
    };
    let mut eligible_paths = Vec::new();
    let mut excluded_paths = HashSet::new();
    for path in discovery.paths {
        let in_range = modified_since.is_none_or(|cutoff| {
            std::fs::symlink_metadata(&path).map_or(true, |metadata| {
                if metadata.file_type().is_symlink() || !metadata.is_file() {
                    return true;
                }
                metadata
                    .modified()
                    .ok()
                    .and_then(|time| time.duration_since(std::time::UNIX_EPOCH).ok())
                    .is_none_or(|duration| duration.as_millis() as i64 >= cutoff)
            })
        });
        if in_range {
            eligible_paths.push(path);
        } else {
            excluded_paths.insert(path);
        }
    }
    let discovered_paths = eligible_paths.iter().cloned().collect::<HashSet<_>>();
    for old_path in manifest
        .keys()
        .filter(|path| !discovered_paths.contains(*path))
    {
        if excluded_paths.contains(old_path) {
            continue;
        }
        if omitted_source_is_unsafe(old_path) {
            plan.complete = false;
            plan.diagnostics.push("source_entry_unsafe".into());
        }
    }
    for path in eligible_paths {
        let fp = match fingerprint(&path) {
            Ok(fp) => fp,
            Err(_) => {
                plan.complete = false;
                plan.diagnostics.push("source_changed_during_scan".into());
                continue;
            }
        };
        if manifest.get(&fp.path).is_some_and(|old| {
            old.size == fp.size && old.mtime == fp.mtime && old.hash == fp.hash && {
                #[cfg(unix)]
                {
                    (old.dev == 0 || old.dev == fp.dev) && (old.ino == 0 || old.ino == fp.ino)
                }
                #[cfg(not(unix))]
                {
                    true
                }
            }
        }) {
            plan.unchanged += 1;
        } else {
            if manifest.contains_key(&fp.path) {
                plan.changed_files += 1;
            } else {
                plan.new_files += 1;
            }
            plan.parse.push(fp.clone());
        }
        plan.discovered.push(fp);
    }
    plan.discovered.sort_by(|a, b| a.path.cmp(&b.path));
    plan.parse.sort_by(|a, b| a.path.cmp(&b.path));
    Ok(plan)
}

fn omitted_source_is_unsafe(path: &Path) -> bool {
    match std::fs::symlink_metadata(path) {
        Ok(metadata) => metadata.file_type().is_symlink() || !metadata.is_file(),
        Err(error) if error.kind() == ErrorKind::NotFound => match path.parent() {
            Some(parent) => match std::fs::symlink_metadata(parent) {
                Ok(metadata) => metadata.file_type().is_symlink() || !metadata.is_dir(),
                Err(parent_error) => parent_error.kind() != ErrorKind::NotFound,
            },
            None => false,
        },
        Err(_) => true,
    }
}

pub fn default_root() -> PathBuf {
    ClaudeProvider.default_root()
}

pub fn scan_root(root: &Path) -> Result<(Vec<ParsedSession>, Vec<String>, bool)> {
    scan_provider(&ClaudeProvider, root)
}

/// Scans any provider into the normalized domain model. Reconciliation is safe
/// only when both discovery and every source read completed successfully.
pub fn scan_provider(
    provider: &dyn SessionProvider,
    root: &Path,
) -> Result<(Vec<ParsedSession>, Vec<String>, bool)> {
    let discovery = provider.discover(root)?;
    let mut sessions = Vec::with_capacity(discovery.paths.len());
    let mut diagnostics = discovery.diagnostics;
    let mut complete = discovery.complete;

    for path in discovery.paths {
        match provider.parse(&path) {
            Ok(parsed) => {
                if parsed
                    .diagnostics
                    .iter()
                    .any(|diagnostic| diagnostic.code == "source_changed_during_scan")
                {
                    complete = false;
                    diagnostics.push("source_changed_during_scan".to_owned());
                }
                sessions.push(parsed);
            }
            Err(_) => {
                complete = false;
                diagnostics.push("session_file_unreadable".to_owned());
            }
        }
    }

    sessions.sort_by(|left, right| left.source_path.cmp(&right.source_path));
    Ok((sessions, diagnostics, complete))
}

pub fn file_hash(path: &Path) -> Result<String> {
    let mut reader = BufReader::new(File::open(path)?);
    let mut hasher = Sha256::new();
    let mut buffer = [0_u8; 32 * 1024];
    loop {
        let read = reader.read(&mut buffer)?;
        if read == 0 {
            break;
        }
        hasher.update(&buffer[..read]);
    }
    Ok(format!("{:x}", hasher.finalize()))
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::{
        domain::{Diagnostic, ParsedSession},
        providers::SourceDiscovery,
    };

    struct ChangingProvider {
        source: PathBuf,
    }

    struct StaticProvider {
        source: PathBuf,
    }

    impl SessionProvider for StaticProvider {
        fn id(&self) -> &'static str {
            "test"
        }

        fn default_root(&self) -> PathBuf {
            self.source.clone()
        }

        fn discover(&self, _root: &Path) -> Result<SourceDiscovery> {
            Ok(SourceDiscovery {
                paths: vec![self.source.clone()],
                diagnostics: Vec::new(),
                complete: true,
            })
        }

        fn parse(&self, _path: &Path) -> Result<ParsedSession> {
            unreachable!("scan planning does not parse sources")
        }
    }

    impl SessionProvider for ChangingProvider {
        fn id(&self) -> &'static str {
            "test"
        }

        fn default_root(&self) -> PathBuf {
            self.source.clone()
        }

        fn discover(&self, _root: &Path) -> Result<SourceDiscovery> {
            Ok(SourceDiscovery {
                paths: vec![self.source.clone()],
                diagnostics: Vec::new(),
                complete: true,
            })
        }

        fn parse(&self, path: &Path) -> Result<ParsedSession> {
            Ok(ParsedSession {
                source_path: path.to_path_buf(),
                summary: Default::default(),
                events: Vec::new(),
                turns: Vec::new(),
                branches: Vec::new(),
                diagnostics: vec![Diagnostic {
                    line: 1,
                    code: "source_changed_during_scan".to_owned(),
                }],
                source_size: 0,
                source_hash: String::new(),
                cwd_history: Vec::new(),
            })
        }
    }

    #[test]
    fn changing_source_makes_reconciliation_incomplete() {
        let provider = ChangingProvider {
            source: PathBuf::from("synthetic.jsonl"),
        };
        let (sessions, diagnostics, complete) =
            scan_provider(&provider, Path::new("unused")).expect("scan provider");

        assert_eq!(sessions.len(), 1);
        assert!(!complete);
        assert_eq!(diagnostics, ["source_changed_during_scan"]);
    }

    #[test]
    fn same_size_and_mtime_with_a_new_hash_is_planned_as_changed() {
        let root = tempfile::tempdir().unwrap();
        let source = root.path().join("session.jsonl");
        std::fs::write(&source, b"aaaa").unwrap();
        let original = fingerprint(&source).unwrap();
        let metadata = std::fs::metadata(&source).unwrap();
        let modified = metadata.modified().unwrap();

        std::fs::write(&source, b"bbbb").unwrap();
        let file = std::fs::OpenOptions::new()
            .write(true)
            .open(&source)
            .unwrap();
        file.set_times(std::fs::FileTimes::new().set_modified(modified))
            .unwrap();

        let provider = StaticProvider {
            source: source.clone(),
        };
        let manifest = HashMap::from([(source, original)]);
        let plan = plan_provider_scan(&provider, root.path(), &manifest, None).unwrap();

        assert_eq!(plan.changed_files, 1);
        assert_eq!(plan.new_files, 0);
        assert_eq!(plan.unchanged, 0);
        assert_eq!(plan.parse.len(), 1);
    }

    #[test]
    fn lookback_excludes_old_sources_from_scan_plan() {
        let root = tempfile::tempdir().unwrap();
        let source = root.path().join("session.jsonl");
        std::fs::write(&source, b"old").unwrap();
        let file = std::fs::OpenOptions::new()
            .write(true)
            .open(&source)
            .unwrap();
        file.set_times(
            std::fs::FileTimes::new()
                .set_modified(std::time::UNIX_EPOCH + std::time::Duration::from_secs(1)),
        )
        .unwrap();
        let original = fingerprint(&source).unwrap();
        let provider = StaticProvider {
            source: source.clone(),
        };
        let manifest = HashMap::from([(source, original)]);

        let plan = plan_provider_scan(&provider, root.path(), &manifest, Some(2_000)).unwrap();

        assert!(plan.complete);
        assert!(plan.discovered.is_empty());
        assert!(plan.parse.is_empty());
    }

    #[test]
    fn replacing_an_indexed_jsonl_with_a_directory_makes_scan_incomplete() {
        let root = tempfile::tempdir().unwrap();
        let project = root.path().join("project");
        let source = project.join("session.jsonl");
        std::fs::create_dir_all(&project).unwrap();
        std::fs::write(&source, b"original").unwrap();
        let original = fingerprint(&source).unwrap();
        let manifest = HashMap::from([(source.clone(), original)]);
        std::fs::remove_file(&source).unwrap();
        std::fs::create_dir(&source).unwrap();

        let plan = plan_provider_scan(&ClaudeProvider, root.path(), &manifest, None).unwrap();

        assert!(!plan.complete);
        assert!(plan
            .diagnostics
            .iter()
            .any(|code| code == "session_entry_unsafe" || code == "source_entry_unsafe"));
    }

    #[cfg(unix)]
    #[test]
    fn replacing_an_indexed_project_with_a_symlink_makes_scan_incomplete() {
        use std::os::unix::fs::symlink;

        let root = tempfile::tempdir().unwrap();
        let project = root.path().join("project");
        let source = project.join("session.jsonl");
        let target = root.path().join("target");
        std::fs::create_dir_all(&project).unwrap();
        std::fs::create_dir_all(&target).unwrap();
        std::fs::write(&source, b"original").unwrap();
        let original = fingerprint(&source).unwrap();
        let manifest = HashMap::from([(source, original)]);
        std::fs::remove_dir_all(&project).unwrap();
        symlink(&target, &project).unwrap();

        let plan = plan_provider_scan(&ClaudeProvider, root.path(), &manifest, None).unwrap();

        assert!(!plan.complete);
        assert!(plan
            .diagnostics
            .iter()
            .any(|code| code == "project_entry_unsafe" || code == "source_entry_unsafe"));
    }
}
