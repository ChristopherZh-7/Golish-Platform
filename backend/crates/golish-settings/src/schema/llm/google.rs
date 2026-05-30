//! Google-hosted LLM provider settings: Vertex AI (Anthropic on GCP),
//! Vertex AI Gemini (native Gemini on GCP), and direct Gemini API.

use super::super::defaults::*;
use serde::{Deserialize, Serialize};

/// Vertex AI (Anthropic on Google Cloud) settings.
#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(default)]
pub struct VertexAiSettings {
    /// Path to service account JSON credentials
    #[serde(skip_serializing_if = "Option::is_none")]
    pub credentials_path: Option<String>,

    /// Google Cloud project ID
    #[serde(skip_serializing_if = "Option::is_none")]
    pub project_id: Option<String>,

    /// Vertex AI region (e.g., "us-east5")
    #[serde(skip_serializing_if = "Option::is_none")]
    pub location: Option<String>,

    /// Whether to show this provider's models in the model selector
    #[serde(default = "default_true")]
    pub show_in_selector: bool,
}

/// Vertex AI Gemini (native Google Gemini on Vertex AI) settings.
#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(default)]
pub struct VertexGeminiSettings {
    /// Path to service account JSON credentials
    #[serde(skip_serializing_if = "Option::is_none")]
    pub credentials_path: Option<String>,

    /// Google Cloud project ID
    #[serde(skip_serializing_if = "Option::is_none")]
    pub project_id: Option<String>,

    /// Vertex AI region (e.g., "us-central1")
    #[serde(skip_serializing_if = "Option::is_none")]
    pub location: Option<String>,

    /// Whether to show this provider's models in the model selector
    #[serde(default = "default_true")]
    pub show_in_selector: bool,

    /// Whether to include thoughts in the response (for thinking models)
    #[serde(default)]
    pub include_thoughts: bool,
}

/// Gemini API settings.
#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(default)]
pub struct GeminiSettings {
    /// Gemini API key (supports $ENV_VAR syntax)
    #[serde(skip_serializing_if = "Option::is_none")]
    pub api_key: Option<String>,

    /// Whether to show this provider's models in the model selector
    #[serde(default = "default_true")]
    pub show_in_selector: bool,

    /// Whether to include thoughts in the response (for thinking models)
    #[serde(default)]
    pub include_thoughts: bool,
}

impl Default for VertexAiSettings {
    fn default() -> Self {
        Self {
            credentials_path: None,
            project_id: None,
            location: None,
            show_in_selector: true,
        }
    }
}

impl Default for VertexGeminiSettings {
    fn default() -> Self {
        Self {
            credentials_path: None,
            project_id: None,
            location: None,
            show_in_selector: true,
            include_thoughts: false,
        }
    }
}

impl Default for GeminiSettings {
    fn default() -> Self {
        Self {
            api_key: None,
            show_in_selector: true,
            include_thoughts: false,
        }
    }
}
