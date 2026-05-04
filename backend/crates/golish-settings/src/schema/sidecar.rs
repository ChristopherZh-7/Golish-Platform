//! Sidecar context capture and synthesis backend settings.
//!
//! The sidecar captures tool calls and reasoning during AI sessions and can
//! synthesise summaries / commit messages via a configurable LLM backend.
//! This module gathers the parent [`SidecarSettings`] together with the
//! per-backend settings (`SynthesisVertexSettings`, `SynthesisOpenAiSettings`,
//! `SynthesisGrokSettings`) so the synthesis ecosystem lives in one place.

use serde::{Deserialize, Serialize};

/// Sidecar context capture settings.
#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(default)]
pub struct SidecarSettings {
    /// Enable context capture during AI sessions.
    pub enabled: bool,

    /// Enable LLM synthesis for commit messages and summaries.
    pub synthesis_enabled: bool,

    /// Synthesis backend: `"local"` | `"vertex_anthropic"` | `"openai"` | `"grok"` | `"template"`.
    pub synthesis_backend: String,

    /// Vertex AI settings for synthesis (when `synthesis_backend = "vertex_anthropic"`).
    pub synthesis_vertex: SynthesisVertexSettings,

    /// OpenAI settings for synthesis (when `synthesis_backend = "openai"`).
    pub synthesis_openai: SynthesisOpenAiSettings,

    /// Grok settings for synthesis (when `synthesis_backend = "grok"`).
    pub synthesis_grok: SynthesisGrokSettings,

    /// Event retention in days (0 = forever).
    pub retention_days: u32,

    /// Capture tool call events.
    pub capture_tool_calls: bool,

    /// Capture agent reasoning events.
    pub capture_reasoning: bool,
}

impl Default for SidecarSettings {
    fn default() -> Self {
        Self {
            enabled: false,
            synthesis_enabled: true,
            synthesis_backend: "template".to_string(),
            synthesis_vertex: SynthesisVertexSettings::default(),
            synthesis_openai: SynthesisOpenAiSettings::default(),
            synthesis_grok: SynthesisGrokSettings::default(),
            retention_days: 30,
            capture_tool_calls: true,
            capture_reasoning: true,
        }
    }
}

/// Vertex AI settings for sidecar synthesis.
#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(default)]
pub struct SynthesisVertexSettings {
    /// Google Cloud project ID (falls back to `ai.vertex_ai.project_id` if not set).
    #[serde(skip_serializing_if = "Option::is_none")]
    pub project_id: Option<String>,

    /// Vertex AI region (falls back to `ai.vertex_ai.location` if not set).
    #[serde(skip_serializing_if = "Option::is_none")]
    pub location: Option<String>,

    /// Model to use for synthesis (default: `claude-sonnet-4-20250514`).
    pub model: String,

    /// Path to credentials (falls back to `ai.vertex_ai.credentials_path` if not set).
    #[serde(skip_serializing_if = "Option::is_none")]
    pub credentials_path: Option<String>,
}

impl Default for SynthesisVertexSettings {
    fn default() -> Self {
        Self {
            project_id: None,
            location: None,
            model: "claude-haiku-4-5@20251001".to_string(),
            credentials_path: None,
        }
    }
}

/// OpenAI settings for sidecar synthesis.
#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(default)]
pub struct SynthesisOpenAiSettings {
    /// API key (falls back to `api_keys` or env var).
    #[serde(skip_serializing_if = "Option::is_none")]
    pub api_key: Option<String>,

    /// Model to use for synthesis (default: `gpt-4o-mini`).
    pub model: String,

    /// Custom base URL for OpenAI-compatible APIs.
    #[serde(skip_serializing_if = "Option::is_none")]
    pub base_url: Option<String>,
}

impl Default for SynthesisOpenAiSettings {
    fn default() -> Self {
        Self {
            api_key: None,
            model: "gpt-4o-mini".to_string(),
            base_url: None,
        }
    }
}

/// Grok settings for sidecar synthesis.
#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(default)]
pub struct SynthesisGrokSettings {
    /// API key (falls back to env var `GROK_API_KEY`).
    #[serde(skip_serializing_if = "Option::is_none")]
    pub api_key: Option<String>,

    /// Model to use for synthesis (default: `grok-2`).
    pub model: String,
}

impl Default for SynthesisGrokSettings {
    fn default() -> Self {
        Self {
            api_key: None,
            model: "grok-2".to_string(),
        }
    }
}
