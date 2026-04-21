use anyhow::{bail, Result};
use serde::{Deserialize, Serialize};

use golish_settings::schema::{
    SidecarSettings, SynthesisGrokSettings, SynthesisOpenAiSettings, SynthesisVertexSettings,
};


/// Synthesis backend type
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize, Default)]
#[serde(rename_all = "snake_case")]
pub enum SynthesisBackend {
    /// Rule-based template generation (no API calls)
    #[default]
    Template,
    /// Anthropic Claude via Vertex AI
    VertexAnthropic,
    /// OpenAI API (or compatible)
    OpenAi,
    /// Grok API (xAI)
    Grok,
}

impl std::str::FromStr for SynthesisBackend {
    type Err = anyhow::Error;

    fn from_str(s: &str) -> Result<Self, Self::Err> {
        match s.to_lowercase().as_str() {
            "template" => Ok(SynthesisBackend::Template),
            "vertex_anthropic" | "vertex" => Ok(SynthesisBackend::VertexAnthropic),
            "openai" => Ok(SynthesisBackend::OpenAi),
            "grok" => Ok(SynthesisBackend::Grok),
            _ => bail!("Unknown synthesis backend: {}", s),
        }
    }
}

impl std::fmt::Display for SynthesisBackend {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        match self {
            SynthesisBackend::Template => write!(f, "template"),
            SynthesisBackend::VertexAnthropic => write!(f, "vertex_anthropic"),
            SynthesisBackend::OpenAi => write!(f, "openai"),
            SynthesisBackend::Grok => write!(f, "grok"),
        }
    }
}

/// Configuration for synthesis operations
#[derive(Debug, Clone)]
pub struct SynthesisConfig {
    /// Whether synthesis is enabled
    pub enabled: bool,
    /// Which backend to use
    pub backend: SynthesisBackend,
    /// Vertex AI settings (when backend = vertex_anthropic)
    pub vertex: SynthesisVertexSettings,
    /// OpenAI settings (when backend = openai)
    pub openai: SynthesisOpenAiSettings,
    /// Grok settings (when backend = grok)
    pub grok: SynthesisGrokSettings,
}

impl Default for SynthesisConfig {
    fn default() -> Self {
        Self {
            enabled: true,
            backend: SynthesisBackend::Template,
            vertex: SynthesisVertexSettings::default(),
            openai: SynthesisOpenAiSettings::default(),
            grok: SynthesisGrokSettings::default(),
        }
    }
}

impl SynthesisConfig {
    /// Create config from SidecarSettings
    pub fn from_sidecar_settings(settings: &SidecarSettings) -> Self {
        let backend = settings.synthesis_backend.parse().unwrap_or_default();

        Self {
            enabled: settings.synthesis_enabled,
            backend,
            vertex: settings.synthesis_vertex.clone(),
            openai: settings.synthesis_openai.clone(),
            grok: settings.synthesis_grok.clone(),
        }
    }
}
