//! OpenAI-compatible and direct API provider settings: Anthropic, OpenAI,
//! Ollama, Groq, xAI, Z.AI SDK, NVIDIA NIM, DeepSeek, and Xiaomi MiMo.

use super::super::defaults::*;
use serde::{Deserialize, Serialize};

/// Direct Anthropic API settings.
#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(default)]
pub struct AnthropicSettings {
    /// Anthropic API key (supports $ENV_VAR syntax)
    #[serde(skip_serializing_if = "Option::is_none")]
    pub api_key: Option<String>,

    /// Whether to show this provider's models in the model selector
    #[serde(default = "default_true")]
    pub show_in_selector: bool,
}

/// OpenAI API settings.
#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(default)]
pub struct OpenAiSettings {
    /// OpenAI API key (supports $ENV_VAR syntax)
    #[serde(skip_serializing_if = "Option::is_none")]
    pub api_key: Option<String>,

    /// Custom base URL for OpenAI-compatible APIs
    #[serde(skip_serializing_if = "Option::is_none")]
    pub base_url: Option<String>,

    /// Whether to show this provider's models in the model selector
    #[serde(default = "default_true")]
    pub show_in_selector: bool,

    /// Enable OpenAI's native web search tool (web_search_preview).
    ///
    /// When enabled, OpenAI models will use server-side web search
    /// similar to Claude's native web tools, instead of Tavily.
    #[serde(default)]
    pub enable_web_search: bool,

    /// Web search context size: "low", "medium", or "high".
    ///
    /// - "low": Faster and cheaper, but may be less accurate
    /// - "medium": Balanced (default)
    /// - "high": Better results, but slower and more expensive
    #[serde(default = "default_web_search_context_size")]
    pub web_search_context_size: String,
}

/// Ollama local LLM settings.
#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(default)]
pub struct OllamaSettings {
    /// Ollama server URL
    pub base_url: String,

    /// Whether to show this provider's models in the model selector
    #[serde(default = "default_true")]
    pub show_in_selector: bool,
}

/// Groq API settings.
#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(default)]
pub struct GroqSettings {
    /// Groq API key (supports $ENV_VAR syntax)
    #[serde(skip_serializing_if = "Option::is_none")]
    pub api_key: Option<String>,

    /// Whether to show this provider's models in the model selector
    #[serde(default = "default_true")]
    pub show_in_selector: bool,
}

/// xAI (Grok) API settings.
#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(default)]
pub struct XaiSettings {
    /// xAI API key (supports $ENV_VAR syntax)
    #[serde(skip_serializing_if = "Option::is_none")]
    pub api_key: Option<String>,

    /// Whether to show this provider's models in the model selector
    #[serde(default = "default_true")]
    pub show_in_selector: bool,
}

/// Z.AI native SDK settings.
///
/// Uses the native Z.AI API via the rig-zai-sdk crate.
/// Default endpoint: https://api.z.ai/api/paas/v4
/// Coding endpoint: https://api.z.ai/api/coding/paas/v4 (for GLM Coding Plan)
#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(default)]
pub struct ZaiSdkSettings {
    /// Z.AI API key (supports $ENV_VAR syntax)
    #[serde(skip_serializing_if = "Option::is_none")]
    pub api_key: Option<String>,

    /// Custom base URL (if None, uses default Z.AI endpoint)
    /// Use "https://api.z.ai/api/coding/paas/v4" for the coding-optimized endpoint
    #[serde(skip_serializing_if = "Option::is_none")]
    pub base_url: Option<String>,

    /// Default model to use
    #[serde(skip_serializing_if = "Option::is_none")]
    pub model: Option<String>,

    /// Whether to show this provider's models in the model selector
    #[serde(default = "default_true")]
    pub show_in_selector: bool,
}

/// NVIDIA NIM API settings.
///
/// Uses the OpenAI-compatible API at https://integrate.api.nvidia.com/v1
#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(default)]
pub struct NvidiaSettings {
    /// NVIDIA API key (supports $ENV_VAR syntax)
    #[serde(skip_serializing_if = "Option::is_none")]
    pub api_key: Option<String>,

    /// Custom base URL (defaults to https://integrate.api.nvidia.com/v1)
    #[serde(skip_serializing_if = "Option::is_none")]
    pub base_url: Option<String>,

    /// Whether to show this provider's models in the model selector
    #[serde(default = "default_true")]
    pub show_in_selector: bool,
}

/// DeepSeek direct API settings.
///
/// Uses the OpenAI-compatible API at https://api.deepseek.com.
#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(default)]
pub struct DeepSeekSettings {
    /// DeepSeek API key (supports $ENV_VAR syntax)
    #[serde(skip_serializing_if = "Option::is_none")]
    pub api_key: Option<String>,

    /// Custom base URL (defaults to https://api.deepseek.com)
    #[serde(skip_serializing_if = "Option::is_none")]
    pub base_url: Option<String>,

    /// Whether to show this provider's models in the model selector
    #[serde(default = "default_true")]
    pub show_in_selector: bool,
}

/// Xiaomi MiMo Token Plan settings.
///
/// Provides OpenAI-compatible and Anthropic-compatible endpoints sharing one
/// API key. Region selects the cluster (cn / sgp / ams); explicit base URLs
/// override the per-region defaults when set.
///
/// See `docs/design/2026-05-27-add-xiaomi-mimo-provider.md`.
#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(default)]
pub struct XiaomiSettings {
    /// Xiaomi Token Plan API key (`tp-xxxxx` for token plan, `sk-xxxxx` for pay-as-you-go).
    #[serde(skip_serializing_if = "Option::is_none")]
    pub api_key: Option<String>,

    /// Cluster region: `"cn"` (default), `"sgp"`, or `"ams"`.
    #[serde(skip_serializing_if = "Option::is_none")]
    pub region: Option<String>,

    /// Preferred protocol when both are available: `"openai"`, `"anthropic"`, or `"auto"`.
    ///
    /// `"auto"` (default) lets the model registry's transport hint decide,
    /// falling back to OpenAI-compatible when neither is hinted.
    #[serde(skip_serializing_if = "Option::is_none")]
    pub default_protocol: Option<String>,

    /// Custom OpenAI-compatible base URL (defaults to region-derived URL).
    #[serde(skip_serializing_if = "Option::is_none")]
    pub openai_base_url: Option<String>,

    /// Custom Anthropic-compatible base URL (defaults to region-derived URL).
    #[serde(skip_serializing_if = "Option::is_none")]
    pub anthropic_base_url: Option<String>,

    /// Whether to show this provider's models in the model selector.
    #[serde(default = "default_true")]
    pub show_in_selector: bool,
}

impl Default for AnthropicSettings {
    fn default() -> Self {
        Self {
            api_key: None,
            show_in_selector: true,
        }
    }
}

impl Default for OpenAiSettings {
    fn default() -> Self {
        Self {
            api_key: None,
            base_url: None,
            show_in_selector: true,
            enable_web_search: false,
            web_search_context_size: "medium".to_string(),
        }
    }
}

impl Default for OllamaSettings {
    fn default() -> Self {
        Self {
            base_url: "http://localhost:11434".to_string(),
            show_in_selector: true,
        }
    }
}

impl Default for GroqSettings {
    fn default() -> Self {
        Self {
            api_key: None,
            show_in_selector: true,
        }
    }
}

impl Default for XaiSettings {
    fn default() -> Self {
        Self {
            api_key: None,
            show_in_selector: true,
        }
    }
}

impl Default for ZaiSdkSettings {
    fn default() -> Self {
        Self {
            api_key: None,
            base_url: None,
            model: None,
            show_in_selector: true,
        }
    }
}

impl Default for NvidiaSettings {
    fn default() -> Self {
        Self {
            api_key: None,
            base_url: None,
            show_in_selector: true,
        }
    }
}

impl Default for DeepSeekSettings {
    fn default() -> Self {
        Self {
            api_key: None,
            base_url: None,
            show_in_selector: true,
        }
    }
}

impl Default for XiaomiSettings {
    fn default() -> Self {
        Self {
            api_key: None,
            region: None,
            default_protocol: None,
            openai_base_url: None,
            anthropic_base_url: None,
            show_in_selector: true,
        }
    }
}
