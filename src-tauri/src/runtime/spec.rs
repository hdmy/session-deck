use std::{
    ffi::{OsStr, OsString},
    fs,
    path::{Path, PathBuf},
    process::{Command, Output, Stdio},
    thread,
    time::{Duration, Instant},
};

use uuid::Uuid;

/// Errors raised before any provider process is started.
#[derive(Debug, thiserror::Error)]
pub enum RuntimeError {
    #[error("invalid native session id")]
    InvalidNativeSessionId,
    #[error("fork parent and child native session ids must differ")]
    SameNativeSessionId,
    #[error("working directory is not an accessible directory: {0}")]
    InvalidCwd(PathBuf),
    #[error("executable is not an executable file: {0}")]
    InvalidExecutable(PathBuf),
    #[error("only the configured Claude executable may be selected by name")]
    InvalidExecutableName,
    #[error("argument contains a NUL byte")]
    InvalidArgument,
    #[error("process error: {0}")]
    Process(#[from] std::io::Error),
    #[error("PTY error: {0}")]
    Pty(String),
    #[error("unknown PTY handle")]
    UnknownHandle,
    #[error("invalid PTY handle")]
    InvalidHandle,
    #[error("preflight {flag} timed out after {timeout_ms}ms")]
    PreflightTimeout { flag: String, timeout_ms: u64 },
    #[error("preflight {flag} exited unsuccessfully: {status:?}")]
    PreflightFailed { flag: String, status: Option<i32> },
    #[error("preflight {flag} output did not contain {expected}")]
    PreflightMissingOutput { flag: String, expected: String },
}

/// A provider-neutral process specification.  `args` are passed directly to
/// `Command`; this type intentionally has no shell-command representation.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct ResumeSpec {
    pub executable: PathBuf,
    pub args: Vec<OsString>,
    pub cwd: PathBuf,
}

impl ResumeSpec {
    pub fn new(
        executable: impl Into<PathBuf>,
        args: impl IntoIterator<Item = impl Into<OsString>>,
        cwd: impl Into<PathBuf>,
    ) -> Result<Self, RuntimeError> {
        let executable = validate_executable(&executable.into())?;
        let cwd = validate_cwd(&cwd.into())?;
        let args = args
            .into_iter()
            .map(Into::into)
            .map(|arg| {
                if arg.to_string_lossy().contains('\0') {
                    Err(RuntimeError::InvalidArgument)
                } else {
                    Ok(arg)
                }
            })
            .collect::<Result<Vec<_>, _>>()?;
        Ok(Self {
            executable,
            args,
            cwd,
        })
    }
}

/// Validates a Claude native session identifier without changing its spelling.
pub fn validate_native_session_id(value: &str) -> Result<(), RuntimeError> {
    Uuid::parse_str(value)
        .ok()
        .filter(|id| id.hyphenated().to_string() == value)
        .map(|_| ())
        .ok_or(RuntimeError::InvalidNativeSessionId)
}

/// Returns a canonical, existing directory for process execution.
pub fn validate_cwd(path: &Path) -> Result<PathBuf, RuntimeError> {
    let metadata = fs::metadata(path).map_err(|_| RuntimeError::InvalidCwd(path.to_path_buf()))?;
    if !metadata.is_dir() {
        return Err(RuntimeError::InvalidCwd(path.to_path_buf()));
    }
    fs::canonicalize(path).map_err(|_| RuntimeError::InvalidCwd(path.to_path_buf()))
}

/// Resolves and validates an executable.  Absolute/relative paths must point
/// to an executable file.  Bare names are intentionally limited to `claude`
/// (or `claude.exe`) and resolved through PATH.
pub fn validate_executable(path: &Path) -> Result<PathBuf, RuntimeError> {
    if path.as_os_str().is_empty() || path.as_os_str().to_string_lossy().contains('\0') {
        return Err(RuntimeError::InvalidExecutable(path.to_path_buf()));
    }

    let has_separator = path.components().count() > 1 || path.is_absolute();
    let candidate = if has_separator {
        path.to_path_buf()
    } else {
        let name = path.to_string_lossy().to_ascii_lowercase();
        if name != "claude" && name != "claude.exe" {
            return Err(RuntimeError::InvalidExecutableName);
        }
        let mut found = None;
        if let Some(search_path) = std::env::var_os("PATH") {
            for directory in std::env::split_paths(&search_path) {
                let candidate = directory.join(path);
                if is_executable_file(&candidate) {
                    found = Some(candidate);
                    break;
                }
            }
        }
        found.ok_or_else(|| RuntimeError::InvalidExecutable(path.to_path_buf()))?
    };

    if !is_executable_file(&candidate) {
        return Err(RuntimeError::InvalidExecutable(path.to_path_buf()));
    }
    fs::canonicalize(&candidate).map_err(|_| RuntimeError::InvalidExecutable(path.to_path_buf()))
}

fn is_executable_file(path: &Path) -> bool {
    let Ok(metadata) = fs::metadata(path) else {
        return false;
    };
    if !metadata.is_file() {
        return false;
    }
    #[cfg(unix)]
    {
        use std::os::unix::fs::PermissionsExt;
        metadata.permissions().mode() & 0o111 != 0
    }
    #[cfg(not(unix))]
    {
        true
    }
}

/// Claude's provider-specific continuation adapter.
#[derive(Debug, Clone)]
pub struct ClaudeResumeSpec {
    resume: ResumeSpec,
    native_session_id: String,
    dangerously_skip_permissions: bool,
}

impl ClaudeResumeSpec {
    pub fn new(
        executable: impl Into<PathBuf>,
        native_session_id: impl AsRef<str>,
        cwd: impl Into<PathBuf>,
        dangerously_skip_permissions: bool,
    ) -> Result<Self, RuntimeError> {
        let native_session_id = native_session_id.as_ref();
        validate_native_session_id(native_session_id)?;
        let mut args = vec![
            OsString::from("--resume"),
            OsString::from(native_session_id),
        ];
        if dangerously_skip_permissions {
            args.push(OsString::from("--dangerously-skip-permissions"));
        }
        let resume = ResumeSpec::new(executable, args, cwd)?;
        Ok(Self {
            resume,
            native_session_id: native_session_id.to_owned(),
            dangerously_skip_permissions,
        })
    }

    pub fn resume_spec(&self) -> &ResumeSpec {
        &self.resume
    }

    pub fn native_session_id(&self) -> &str {
        &self.native_session_id
    }

    pub fn preflight(&self) -> Result<CliPreflight, RuntimeError> {
        preflight_with_options(&self.resume.executable, self.dangerously_skip_permissions)
    }
}

/// A validated Claude continuation that forks a native session into a new
/// native session id.  Arguments are passed directly to the executable; this
/// type intentionally does not expose a shell command representation.
#[derive(Debug, Clone)]
pub struct ClaudeForkSpec {
    resume: ResumeSpec,
    parent_native_session_id: String,
    new_native_session_id: String,
    dangerously_skip_permissions: bool,
}

impl ClaudeForkSpec {
    pub fn new(
        executable: impl Into<PathBuf>,
        parent_native_session_id: impl AsRef<str>,
        new_native_session_id: impl AsRef<str>,
        cwd: impl Into<PathBuf>,
        dangerously_skip_permissions: bool,
    ) -> Result<Self, RuntimeError> {
        let parent_native_session_id = parent_native_session_id.as_ref();
        let new_native_session_id = new_native_session_id.as_ref();
        validate_native_session_id(parent_native_session_id)?;
        validate_native_session_id(new_native_session_id)?;
        if parent_native_session_id == new_native_session_id {
            return Err(RuntimeError::SameNativeSessionId);
        }
        let mut args = vec![
            OsString::from("--resume"),
            OsString::from(parent_native_session_id),
            OsString::from("--fork-session"),
            OsString::from("--session-id"),
            OsString::from(new_native_session_id),
        ];
        if dangerously_skip_permissions {
            args.push(OsString::from("--dangerously-skip-permissions"));
        }
        let resume = ResumeSpec::new(executable, args, cwd)?;
        Ok(Self {
            resume,
            parent_native_session_id: parent_native_session_id.to_owned(),
            new_native_session_id: new_native_session_id.to_owned(),
            dangerously_skip_permissions,
        })
    }

    pub fn resume_spec(&self) -> &ResumeSpec {
        &self.resume
    }

    pub fn parent_native_session_id(&self) -> &str {
        &self.parent_native_session_id
    }

    pub fn new_native_session_id(&self) -> &str {
        &self.new_native_session_id
    }

    pub fn preflight(&self) -> Result<CliPreflight, RuntimeError> {
        preflight_fork_with_options(&self.resume.executable, self.dangerously_skip_permissions)
    }
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct CommandOutput {
    pub status: Option<i32>,
    pub stdout: String,
    pub stderr: String,
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct CliPreflight {
    pub version: CommandOutput,
    pub help: CommandOutput,
}

/// Runs harmless provider CLI probes using direct argument arrays.  No session
/// is resumed and no shell is involved.
pub fn preflight(executable: impl AsRef<Path>) -> Result<CliPreflight, RuntimeError> {
    preflight_with_options(executable, false)
}

pub fn preflight_with_options(
    executable: impl AsRef<Path>,
    dangerously_skip_permissions: bool,
) -> Result<CliPreflight, RuntimeError> {
    let executable = validate_executable(executable.as_ref())?;
    let version = run_probe(&executable, "--version")?;
    let help = run_probe(&executable, "--help")?;
    for (flag, output) in [("--version", &version), ("--help", &help)] {
        if output.status != Some(0) {
            return Err(RuntimeError::PreflightFailed {
                flag: flag.to_owned(),
                status: output.status,
            });
        }
    }
    let help_text = format!("{}\n{}", help.stdout, help.stderr);
    for expected in ["--resume"]
        .into_iter()
        .chain(dangerously_skip_permissions.then_some("--dangerously-skip-permissions"))
    {
        if !help_text.contains(expected) {
            return Err(RuntimeError::PreflightMissingOutput {
                flag: "--help".to_owned(),
                expected: expected.to_owned(),
            });
        }
    }
    Ok(CliPreflight { version, help })
}

fn preflight_fork_with_options(
    executable: impl AsRef<Path>,
    dangerously_skip_permissions: bool,
) -> Result<CliPreflight, RuntimeError> {
    let result = preflight_with_options(executable, dangerously_skip_permissions)?;
    let help_text = format!("{}\n{}", result.help.stdout, result.help.stderr);
    for expected in ["--fork-session", "--session-id"] {
        if !help_text.contains(expected) {
            return Err(RuntimeError::PreflightMissingOutput {
                flag: "--help".to_owned(),
                expected: expected.to_owned(),
            });
        }
    }
    Ok(result)
}

fn run_probe(executable: &Path, flag: &str) -> Result<CommandOutput, RuntimeError> {
    const TIMEOUT: Duration = Duration::from_secs(2);
    let mut child = Command::new(executable)
        .arg(OsStr::new(flag))
        .stdout(Stdio::piped())
        .stderr(Stdio::piped())
        .spawn()?;
    let deadline = Instant::now() + TIMEOUT;
    loop {
        if child.try_wait()?.is_some() {
            break;
        }
        if Instant::now() >= deadline {
            let _ = child.kill();
            let _ = child.wait();
            return Err(RuntimeError::PreflightTimeout {
                flag: flag.to_owned(),
                timeout_ms: TIMEOUT.as_millis() as u64,
            });
        }
        thread::sleep(Duration::from_millis(10));
    }
    let output = child.wait_with_output()?;
    command_output(output)
}

fn command_output(output: Output) -> Result<CommandOutput, RuntimeError> {
    Ok(CommandOutput {
        status: output.status.code(),
        stdout: String::from_utf8_lossy(&output.stdout).into_owned(),
        stderr: String::from_utf8_lossy(&output.stderr).into_owned(),
    })
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::fs;
    #[cfg(unix)]
    use std::os::unix::fs::PermissionsExt;
    use tempfile::{tempdir, NamedTempFile, TempPath};

    #[cfg(unix)]
    fn executable_script(body: &str) -> TempPath {
        let file = NamedTempFile::new().unwrap();
        fs::write(file.path(), format!("#!/bin/sh\n{body}\n")).unwrap();
        fs::set_permissions(file.path(), fs::Permissions::from_mode(0o755)).unwrap();
        file.into_temp_path()
    }

    #[test]
    fn rejects_invalid_id_and_cwd_and_executable() {
        assert!(matches!(
            validate_native_session_id("not-a-uuid"),
            Err(RuntimeError::InvalidNativeSessionId)
        ));
        assert!(validate_native_session_id("123E4567-E89B-12D3-A456-426614174000").is_err());
        assert!(validate_native_session_id("123e4567e89b12d3a456426614174000").is_err());
        assert!(validate_cwd(Path::new("/definitely/missing")).is_err());
        assert!(validate_executable(Path::new("/definitely/missing")).is_err());
    }

    #[test]
    fn builds_exact_claude_arguments() {
        let dir = tempdir().unwrap();
        let spec = ClaudeResumeSpec::new(
            "/bin/sh",
            "123e4567-e89b-12d3-a456-426614174000",
            dir.path(),
            true,
        )
        .unwrap();
        assert_eq!(
            spec.resume_spec().args,
            vec![
                OsString::from("--resume"),
                OsString::from("123e4567-e89b-12d3-a456-426614174000"),
                OsString::from("--dangerously-skip-permissions")
            ]
        );
    }

    #[test]
    fn builds_exact_claude_fork_arguments_and_exposes_native_ids() {
        let dir = tempdir().unwrap();
        let spec = ClaudeForkSpec::new(
            "/bin/sh",
            "123e4567-e89b-12d3-a456-426614174000",
            "223e4567-e89b-12d3-a456-426614174000",
            dir.path(),
            true,
        )
        .unwrap();
        assert_eq!(
            spec.resume_spec().args,
            vec![
                OsString::from("--resume"),
                OsString::from("123e4567-e89b-12d3-a456-426614174000"),
                OsString::from("--fork-session"),
                OsString::from("--session-id"),
                OsString::from("223e4567-e89b-12d3-a456-426614174000"),
                OsString::from("--dangerously-skip-permissions"),
            ]
        );
        assert_eq!(
            spec.parent_native_session_id(),
            "123e4567-e89b-12d3-a456-426614174000"
        );
        assert_eq!(
            spec.new_native_session_id(),
            "223e4567-e89b-12d3-a456-426614174000"
        );
    }

    #[test]
    fn rejects_invalid_parent_or_new_fork_id() {
        let dir = tempdir().unwrap();
        assert!(matches!(
            ClaudeForkSpec::new(
                "/bin/sh",
                "not-a-uuid",
                "223e4567-e89b-12d3-a456-426614174000",
                dir.path(),
                false,
            ),
            Err(RuntimeError::InvalidNativeSessionId)
        ));
        assert!(matches!(
            ClaudeForkSpec::new(
                "/bin/sh",
                "123e4567-e89b-12d3-a456-426614174000",
                "not-a-uuid",
                dir.path(),
                false,
            ),
            Err(RuntimeError::InvalidNativeSessionId)
        ));
        assert!(matches!(
            ClaudeForkSpec::new(
                "/bin/sh",
                "123e4567-e89b-12d3-a456-426614174000",
                "123e4567-e89b-12d3-a456-426614174000",
                dir.path(),
                false,
            ),
            Err(RuntimeError::SameNativeSessionId)
        ));
    }

    #[test]
    fn fork_without_danger_flag_has_no_hidden_arguments() {
        let dir = tempdir().unwrap();
        let spec = ClaudeForkSpec::new(
            "/bin/sh",
            "123e4567-e89b-12d3-a456-426614174000",
            "223e4567-e89b-12d3-a456-426614174000",
            dir.path(),
            false,
        )
        .unwrap();
        assert_eq!(
            spec.resume_spec()
                .args
                .iter()
                .map(|arg| arg.to_string_lossy().into_owned())
                .collect::<Vec<_>>(),
            vec![
                "--resume",
                "123e4567-e89b-12d3-a456-426614174000",
                "--fork-session",
                "--session-id",
                "223e4567-e89b-12d3-a456-426614174000",
            ]
        );
    }

    #[cfg(unix)]
    #[test]
    fn preflight_is_bounded_and_validates_capabilities() {
        let file = executable_script(
            "case \"$1\" in --version) echo version;; --help) echo '--resume --dangerously-skip-permissions';; esac",
        );
        let result = preflight_with_options(file.to_path_buf(), true).unwrap();
        assert_eq!(result.version.status, Some(0));

        let hanging = executable_script("sleep 10");
        let started = Instant::now();
        let error = preflight(hanging.to_path_buf()).unwrap_err();
        assert!(started.elapsed() < Duration::from_secs(3));
        assert!(matches!(error, RuntimeError::PreflightTimeout { .. }));
    }
}
