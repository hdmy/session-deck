//! Read-only, provider-neutral repository identity.  This deliberately does
//! not invoke git or inspect HEAD; it only accepts a bounded `.git` marker.
use crate::domain::Result;
#[cfg(unix)]
use std::os::unix::fs::MetadataExt;
use std::{
    fs,
    io::{ErrorKind, Read},
    path::{Path, PathBuf},
};

const MAX_METADATA_BYTES: u64 = 4096;

fn read_bounded(path: &Path) -> Result<String> {
    let before = fs::symlink_metadata(path)?;
    if before.file_type().is_symlink() || !before.is_file() || before.len() > MAX_METADATA_BYTES {
        return Err(crate::domain::AppError::InvalidRoot(
            "unsafe repository metadata".into(),
        ));
    }
    let mut file = fs::File::open(path)?;
    let opened = file.metadata()?;
    if !opened.is_file() || opened.len() > MAX_METADATA_BYTES {
        return Err(crate::domain::AppError::InvalidRoot(
            "unsafe repository metadata".into(),
        ));
    }
    #[cfg(unix)]
    if before.dev() != opened.dev() || before.ino() != opened.ino() {
        return Err(crate::domain::AppError::InvalidRoot(
            "repository metadata changed".into(),
        ));
    }
    let mut bytes = Vec::with_capacity(opened.len() as usize);
    file.by_ref()
        .take(MAX_METADATA_BYTES + 1)
        .read_to_end(&mut bytes)?;
    if bytes.len() as u64 > MAX_METADATA_BYTES {
        return Err(crate::domain::AppError::InvalidRoot(
            "unsafe repository metadata".into(),
        ));
    }
    let opened_after = file.metadata()?;
    let after = fs::symlink_metadata(path)?;
    if after.file_type().is_symlink()
        || !after.is_file()
        || before.len() != opened.len()
        || opened.len() != opened_after.len()
        || opened.len() != after.len()
        || opened.modified().ok() != opened_after.modified().ok()
    {
        return Err(crate::domain::AppError::InvalidRoot(
            "repository metadata changed".into(),
        ));
    }
    #[cfg(unix)]
    if opened.dev() != opened_after.dev()
        || opened.ino() != opened_after.ino()
        || opened.dev() != after.dev()
        || opened.ino() != after.ino()
    {
        return Err(crate::domain::AppError::InvalidRoot(
            "repository metadata changed".into(),
        ));
    }
    String::from_utf8(bytes)
        .map_err(|_| crate::domain::AppError::InvalidRoot("repository metadata is not utf8".into()))
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct ProjectIdentity {
    pub workspace_id: String,
    pub project_path: PathBuf,
    pub worktree_path: PathBuf,
}

fn lexical(path: &Path) -> PathBuf {
    let mut out = PathBuf::new();
    for component in path.components() {
        match component {
            std::path::Component::CurDir => {}
            std::path::Component::ParentDir => {
                out.pop();
            }
            other => out.push(other.as_os_str()),
        }
    }
    out
}

fn single_line(value: &str) -> Result<&str> {
    let value = value
        .strip_suffix("\r\n")
        .or_else(|| value.strip_suffix('\n'))
        .unwrap_or(value);
    if value.chars().any(char::is_control) {
        return Err(crate::domain::AppError::InvalidRoot(
            "invalid repository metadata".into(),
        ));
    }
    Ok(value)
}

fn find_marker(cwd: &Path) -> Result<(PathBuf, PathBuf)> {
    let mut probe = cwd.to_path_buf();
    loop {
        let marker = probe.join(".git");
        match fs::symlink_metadata(&marker) {
            Ok(_) => return Ok((probe, marker)),
            Err(error) if error.kind() == ErrorKind::NotFound => {}
            Err(error) => return Err(error.into()),
        }
        if !probe.pop() {
            return Err(crate::domain::AppError::InvalidRoot(
                "repository marker not found".into(),
            ));
        }
    }
}

fn validate_directory(path: &Path) -> Result<()> {
    let before = fs::symlink_metadata(path)?;
    if before.file_type().is_symlink() || !before.is_dir() {
        return Err(crate::domain::AppError::InvalidRoot(
            "unsafe repository directory".into(),
        ));
    }
    #[cfg(unix)]
    {
        let opened = fs::File::open(path)?;
        let opened = opened.metadata()?;
        let after = fs::symlink_metadata(path)?;
        if !opened.is_dir()
            || after.file_type().is_symlink()
            || !after.is_dir()
            || before.dev() != opened.dev()
            || before.ino() != opened.ino()
            || opened.dev() != after.dev()
            || opened.ino() != after.ino()
        {
            return Err(crate::domain::AppError::InvalidRoot(
                "repository directory changed".into(),
            ));
        }
    }
    Ok(())
}

pub fn identify(cwd: &Path) -> Result<ProjectIdentity> {
    let cwd = lexical(cwd);
    let (probe, marker) = find_marker(&cwd)?;
    let meta = fs::symlink_metadata(&marker)?;
    if meta.file_type().is_symlink() {
        return Err(crate::domain::AppError::InvalidRoot(
            "unsafe .git symlink".into(),
        ));
    }
    if meta.is_dir() {
        validate_directory(&marker)?;
        return Ok(ProjectIdentity {
            workspace_id: marker.display().to_string(),
            project_path: probe.clone(),
            worktree_path: cwd,
        });
    }
    if !meta.is_file() || meta.len() > 4096 {
        return Err(crate::domain::AppError::InvalidRoot(
            "unsafe .git marker".into(),
        ));
    }
    let text = read_bounded(&marker)?;
    let text = single_line(&text)?;
    let Some(raw) = text.strip_prefix("gitdir:") else {
        return Err(crate::domain::AppError::InvalidRoot(
            "invalid .git marker".into(),
        ));
    };
    let gitdir = lexical(&probe.join(raw.trim()));
    validate_directory(&gitdir)?;
    let common = gitdir.join("commondir");
    let workspace = if let Ok(meta) = fs::symlink_metadata(&common) {
        if meta.file_type().is_symlink() || !meta.is_file() || meta.len() > 4096 {
            return Err(crate::domain::AppError::InvalidRoot(
                "unsafe commondir".into(),
            ));
        }
        let raw = read_bounded(&common)?;
        let raw = single_line(&raw)?;
        lexical(&gitdir.join(raw.trim()))
    } else {
        gitdir
    };
    validate_directory(&workspace)?;
    let project_path = workspace
        .parent()
        .map(Path::to_path_buf)
        .unwrap_or_else(|| cwd.clone());
    Ok(ProjectIdentity {
        workspace_id: workspace.display().to_string(),
        project_path,
        worktree_path: cwd,
    })
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::io::Write;

    #[test]
    fn identifies_normal_repository_from_a_nested_cwd() {
        let root = tempfile::tempdir().unwrap();
        let repository = root.path().join("repository");
        let nested = repository.join("packages/app");
        fs::create_dir_all(repository.join(".git")).unwrap();
        fs::create_dir_all(&nested).unwrap();

        let identity = identify(&nested).unwrap();
        assert_eq!(
            identity.workspace_id,
            repository.join(".git").display().to_string()
        );
        assert_eq!(identity.project_path, repository);
        assert_eq!(identity.worktree_path, nested);
    }

    #[test]
    fn identifies_linked_worktree_with_standard_line_endings() {
        let root = tempfile::tempdir().unwrap();
        let repository = root.path().join("repository");
        let worktree = root.path().join("topic");
        let nested = worktree.join("packages/app");
        let gitdir = repository.join(".git/worktrees/topic");
        fs::create_dir_all(&gitdir).unwrap();
        fs::create_dir_all(&nested).unwrap();
        fs::write(
            worktree.join(".git"),
            "gitdir: ../repository/.git/worktrees/topic\r\n",
        )
        .unwrap();
        fs::write(gitdir.join("commondir"), "../..\n").unwrap();

        let identity = identify(&nested).unwrap();
        assert_eq!(
            identity.workspace_id,
            repository.join(".git").display().to_string()
        );
        assert_eq!(identity.project_path, repository);
        assert_eq!(identity.worktree_path, nested);
    }

    #[test]
    fn rejects_oversized_or_multiline_repository_metadata() {
        let root = tempfile::tempdir().unwrap();
        let worktree = root.path().join("worktree");
        fs::create_dir_all(&worktree).unwrap();
        fs::write(worktree.join(".git"), vec![b'x'; 4097]).unwrap();
        assert!(identify(&worktree).is_err());

        fs::write(worktree.join(".git"), "gitdir: target\nsecond line\n").unwrap();
        assert!(identify(&worktree).is_err());
    }

    #[cfg(unix)]
    #[test]
    fn rejects_symlinked_repository_metadata() {
        use std::os::unix::fs::symlink;

        let root = tempfile::tempdir().unwrap();
        let worktree = root.path().join("worktree");
        let target = root.path().join("target");
        fs::create_dir_all(&worktree).unwrap();
        fs::create_dir_all(&target).unwrap();
        let mut marker = fs::File::create(root.path().join("marker")).unwrap();
        writeln!(marker, "gitdir: {}", target.display()).unwrap();
        symlink(root.path().join("marker"), worktree.join(".git")).unwrap();

        assert!(identify(&worktree).is_err());
    }
}
