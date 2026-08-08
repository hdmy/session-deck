pub mod claude;
pub mod claude_graph;
pub mod claude_live;
pub mod insights;

use crate::domain::{ParsedSession, Result};
use std::path::{Path, PathBuf};

#[derive(Debug, Default)]
pub struct SourceDiscovery {
    pub paths: Vec<PathBuf>,
    pub diagnostics: Vec<String>,
    pub complete: bool,
}

/// Provider boundary for source discovery and transcript normalization.
///
/// Implementations own their on-disk layout and parser. Downstream indexing,
/// search, and viewer modules consume only the normalized domain records.
pub trait SessionProvider {
    fn id(&self) -> &'static str;
    fn default_root(&self) -> PathBuf;
    fn discover(&self, root: &Path) -> Result<SourceDiscovery>;
    fn parse(&self, path: &Path) -> Result<ParsedSession>;
    fn capabilities(&self) -> ProviderCapabilities {
        ProviderCapabilities::default()
    }
}

#[derive(Debug, Clone, Copy, Default, serde::Serialize, serde::Deserialize)]
pub struct ProviderCapabilities {
    pub supports_resume: bool,
    pub supports_changes: bool,
    pub supports_worktree: bool,
    pub supports_branching: bool,
}
