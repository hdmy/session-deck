pub mod claude;
pub mod claude_graph;
pub mod claude_live;
pub mod codex;
pub mod gemini;
pub mod insights;
pub mod opencode;

use crate::domain::{ParsedSession, Result};
use std::path::{Path, PathBuf};

pub use crate::domain::ProviderCapabilities;

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
    fn descriptor(&self) -> ProviderDescriptor {
        ProviderDescriptor {
            provider_id: self.id().to_owned(),
            name: display_name(self.id()),
            capabilities: self.capabilities(),
        }
    }
}

fn display_name(provider_id: &str) -> String {
    match provider_id {
        "claude" => "Claude",
        "codex" => "Codex",
        "gemini" => "Gemini",
        "opencode" => "OpenCode",
        _ => provider_id,
    }
    .to_owned()
}

/// Stable metadata contract used by the read model.  Provider construction
/// and process launching intentionally remain outside this descriptor.
#[derive(Debug, Clone, serde::Serialize, serde::Deserialize)]
pub struct ProviderDescriptor {
    pub provider_id: String,
    pub name: String,
    pub capabilities: ProviderCapabilities,
}

#[derive(Debug, Clone, Default)]
pub struct ProviderRegistry {
    descriptors: Vec<ProviderDescriptor>,
}

impl ProviderRegistry {
    pub fn builtin_session_providers() -> [&'static dyn SessionProvider; 3] {
        [
            &claude::ClaudeProvider,
            &codex::CodexProvider,
            &gemini::GeminiProvider,
        ]
    }

    pub fn builtin() -> Self {
        let mut descriptors = Self::builtin_session_providers()
            .iter()
            .map(|provider| provider.descriptor())
            .collect::<Vec<_>>();
        // OpenCode is read-only and indexed by the shared scan path.
        descriptors.push(ProviderDescriptor {
            provider_id: opencode::PROVIDER_ID.to_owned(),
            name: display_name(opencode::PROVIDER_ID),
            capabilities: ProviderCapabilities {
                supports_reader: true,
                supports_search: true,
                ..ProviderCapabilities::default()
            },
        });
        Self { descriptors }
    }

    pub fn from_providers(providers: &[&dyn SessionProvider]) -> Self {
        Self {
            descriptors: providers
                .iter()
                .map(|provider| provider.descriptor())
                .collect(),
        }
    }

    pub fn descriptors(&self) -> &[ProviderDescriptor] {
        &self.descriptors
    }

    pub fn get(&self, id: &str) -> Option<&ProviderDescriptor> {
        self.descriptors
            .iter()
            .find(|descriptor| descriptor.provider_id == id)
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn builtin_descriptors_use_provider_id_wire_field() {
        let registry = ProviderRegistry::builtin();
        let value = serde_json::to_value(registry.descriptors()).unwrap();
        assert_eq!(value[0]["provider_id"], "claude");
        assert!(value[0].get("id").is_none());
        assert!(
            registry
                .get("opencode")
                .unwrap()
                .capabilities
                .supports_reader
        );
        assert!(
            registry
                .get("opencode")
                .unwrap()
                .capabilities
                .supports_search
        );
        let provider_ids = ProviderRegistry::builtin_session_providers()
            .into_iter()
            .map(|provider| provider.id())
            .collect::<Vec<_>>();
        assert_eq!(
            registry
                .descriptors()
                .iter()
                .take(provider_ids.len())
                .map(|descriptor| descriptor.provider_id.as_str())
                .collect::<Vec<_>>(),
            provider_ids
        );
    }
}
