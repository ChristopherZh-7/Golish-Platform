//! Per-provider configuration structs and the unified `ProviderConfig` enum.

use serde::Deserialize;
use std::path::PathBuf;

use golish_settings::schema::ModelOverride;

/// Configuration for creating an AgentBridge with OpenRouter
pub struct OpenRouterClientConfig<'a> {
    pub workspace: PathBuf,
    pub model: &'a str,
    pub api_key: &'a str,
    /// Provider preferences for routing and filtering (optional).
    pub provider_preferences: Option<serde_json::Value>,
}

/// Configuration for creating an AgentBridge with Vertex AI Anthropic
pub struct VertexAnthropicClientConfig<'a> {
    pub workspace: PathBuf,
    /// Path to service account JSON file. If None, uses application default credentials.
    pub credentials_path: Option<&'a str>,
    pub project_id: &'a str,
    pub location: &'a str,
    pub model: &'a str,
}

/// Configuration for creating an AgentBridge with Vertex AI Gemini
pub struct VertexGeminiClientConfig<'a> {
    pub workspace: PathBuf,
    /// Path to service account JSON file. If None, uses application default credentials.
    pub credentials_path: Option<&'a str>,
    pub project_id: &'a str,
    pub location: &'a str,
    pub model: &'a str,
    /// Whether to include thoughts in the response (for thinking models)
    pub include_thoughts: bool,
}

/// Configuration for creating an AgentBridge with OpenAI
pub struct OpenAiClientConfig<'a> {
    pub workspace: PathBuf,
    pub model: &'a str,
    pub api_key: &'a str,
    pub base_url: Option<&'a str>,
    /// Reasoning effort level for reasoning models (e.g., "low", "medium", "high").
    /// Reserved for future use with models that support reasoning effort configuration.
    pub reasoning_effort: Option<&'a str>,
    /// Enable OpenAI's native web search tool (web_search_preview).
    pub enable_web_search: bool,
    /// Web search context size: "low", "medium", or "high".
    pub web_search_context_size: &'a str,
}

/// Configuration for creating an AgentBridge with direct Anthropic API
pub struct AnthropicClientConfig<'a> {
    pub workspace: PathBuf,
    pub model: &'a str,
    pub api_key: &'a str,
}

/// Configuration for creating an AgentBridge with Ollama
pub struct OllamaClientConfig<'a> {
    pub workspace: PathBuf,
    pub model: &'a str,
    pub base_url: Option<&'a str>,
}

/// Configuration for creating an AgentBridge with Gemini
pub struct GeminiClientConfig<'a> {
    pub workspace: PathBuf,
    pub model: &'a str,
    pub api_key: &'a str,
}

/// Configuration for creating an AgentBridge with Groq
pub struct GroqClientConfig<'a> {
    pub workspace: PathBuf,
    pub model: &'a str,
    pub api_key: &'a str,
}

/// Configuration for creating an AgentBridge with xAI (Grok)
pub struct XaiClientConfig<'a> {
    pub workspace: PathBuf,
    pub model: &'a str,
    pub api_key: &'a str,
}

/// Configuration for creating an AgentBridge with Z.AI via native SDK
pub struct ZaiSdkClientConfig<'a> {
    pub workspace: PathBuf,
    pub model: &'a str,
    pub api_key: &'a str,
    /// Custom base URL (if None, uses default Z.AI endpoint)
    pub base_url: Option<&'a str>,
    /// Source channel identifier for request tracking
    pub source_channel: Option<&'a str>,
}

/// Configuration for creating an AgentBridge with NVIDIA NIM
pub struct NvidiaClientConfig<'a> {
    pub workspace: PathBuf,
    pub model: &'a str,
    pub api_key: &'a str,
    /// Custom base URL (if None, uses https://integrate.api.nvidia.com/v1)
    pub base_url: Option<&'a str>,
}

fn default_web_search_context_size() -> String {
    "medium".to_string()
}

fn default_include_thoughts() -> bool {
    true
}

/// Unified configuration for all LLM providers.
///
/// Uses serde tag discrimination for clean JSON/frontend integration.
/// This enables a single Tauri command to handle all provider initialization.
#[allow(dead_code)]
#[derive(Debug, Clone, Deserialize)]
#[serde(tag = "provider", rename_all = "snake_case")]
pub enum ProviderConfig {
    VertexAi {
        workspace: String,
        model: String,
        #[serde(default)]
        credentials_path: Option<String>,
        project_id: String,
        location: String,
        #[serde(default)]
        model_override: Option<ModelOverride>,
    },
    VertexGemini {
        workspace: String,
        model: String,
        #[serde(default)]
        credentials_path: Option<String>,
        project_id: String,
        location: String,
        #[serde(default = "default_include_thoughts")]
        include_thoughts: bool,
        #[serde(default)]
        model_override: Option<ModelOverride>,
    },
    Openrouter {
        workspace: String,
        model: String,
        api_key: String,
        #[serde(default)]
        provider_preferences: Option<serde_json::Value>,
        #[serde(default)]
        model_override: Option<ModelOverride>,
    },
    Openai {
        workspace: String,
        model: String,
        api_key: String,
        #[serde(default)]
        base_url: Option<String>,
        #[serde(default)]
        reasoning_effort: Option<String>,
        #[serde(default)]
        enable_web_search: bool,
        #[serde(default = "default_web_search_context_size")]
        web_search_context_size: String,
        #[serde(default)]
        model_override: Option<ModelOverride>,
    },
    Anthropic {
        workspace: String,
        model: String,
        api_key: String,
        #[serde(default)]
        model_override: Option<ModelOverride>,
    },
    Ollama {
        workspace: String,
        model: String,
        #[serde(default)]
        base_url: Option<String>,
        #[serde(default)]
        model_override: Option<ModelOverride>,
    },
    Gemini {
        workspace: String,
        model: String,
        api_key: String,
        #[serde(default = "default_include_thoughts")]
        include_thoughts: bool,
        #[serde(default)]
        model_override: Option<ModelOverride>,
    },
    Groq {
        workspace: String,
        model: String,
        api_key: String,
        #[serde(default)]
        model_override: Option<ModelOverride>,
    },
    Xai {
        workspace: String,
        model: String,
        api_key: String,
        #[serde(default)]
        model_override: Option<ModelOverride>,
    },
    ZaiSdk {
        workspace: String,
        model: String,
        api_key: String,
        #[serde(default)]
        base_url: Option<String>,
        #[serde(default)]
        source_channel: Option<String>,
        #[serde(default)]
        model_override: Option<ModelOverride>,
    },
    Nvidia {
        workspace: String,
        model: String,
        api_key: String,
        #[serde(default)]
        base_url: Option<String>,
        #[serde(default)]
        model_override: Option<ModelOverride>,
    },
}

#[allow(dead_code)]
impl ProviderConfig {
    pub fn workspace(&self) -> &str {
        match self {
            Self::VertexAi { workspace, .. } => workspace,
            Self::VertexGemini { workspace, .. } => workspace,
            Self::Openrouter { workspace, .. } => workspace,
            Self::Openai { workspace, .. } => workspace,
            Self::Anthropic { workspace, .. } => workspace,
            Self::Ollama { workspace, .. } => workspace,
            Self::Gemini { workspace, .. } => workspace,
            Self::Groq { workspace, .. } => workspace,
            Self::Xai { workspace, .. } => workspace,
            Self::ZaiSdk { workspace, .. } => workspace,
            Self::Nvidia { workspace, .. } => workspace,
        }
    }

    pub fn model(&self) -> &str {
        match self {
            Self::VertexAi { model, .. } => model,
            Self::VertexGemini { model, .. } => model,
            Self::Openrouter { model, .. } => model,
            Self::Openai { model, .. } => model,
            Self::Anthropic { model, .. } => model,
            Self::Ollama { model, .. } => model,
            Self::Gemini { model, .. } => model,
            Self::Groq { model, .. } => model,
            Self::Xai { model, .. } => model,
            Self::ZaiSdk { model, .. } => model,
            Self::Nvidia { model, .. } => model,
        }
    }

    pub fn provider_name(&self) -> &'static str {
        match self {
            Self::VertexAi { .. } => "vertex_ai",
            Self::VertexGemini { .. } => "vertex_gemini",
            Self::Openrouter { .. } => "openrouter",
            Self::Openai { .. } => "openai",
            Self::Anthropic { .. } => "anthropic",
            Self::Ollama { .. } => "ollama",
            Self::Gemini { .. } => "gemini",
            Self::Groq { .. } => "groq",
            Self::Xai { .. } => "xai",
            Self::ZaiSdk { .. } => "zai_sdk",
            Self::Nvidia { .. } => "nvidia",
        }
    }

    /// Extract the user-supplied per-model override, if any.
    pub fn model_override(&self) -> Option<&ModelOverride> {
        match self {
            Self::VertexAi { model_override, .. }
            | Self::VertexGemini { model_override, .. }
            | Self::Openrouter { model_override, .. }
            | Self::Openai { model_override, .. }
            | Self::Anthropic { model_override, .. }
            | Self::Ollama { model_override, .. }
            | Self::Gemini { model_override, .. }
            | Self::Groq { model_override, .. }
            | Self::Xai { model_override, .. }
            | Self::ZaiSdk { model_override, .. }
            | Self::Nvidia { model_override, .. } => model_override.as_ref(),
        }
    }
}
