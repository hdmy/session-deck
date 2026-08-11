//! Provider continuation and PTY lifecycle primitives.
//!
//! This module deliberately owns no transcript or provider configuration
//! state.  Callers hand it a validated, direct process specification.

mod pty;
mod spec;

pub use pty::{PtyEvent, PtyHandle, PtyManager, PtySize};
pub use spec::{
    preflight, preflight_with_options, validate_cwd, validate_executable,
    validate_native_session_id, ClaudeForkSpec, ClaudeResumeSpec, CliPreflight, CommandOutput,
    ResumeSpec, RuntimeError,
};
