//! LLM provider settings: per-provider config blocks for Vertex Anthropic,
//! Vertex Gemini, OpenRouter (incl. provider preferences), Anthropic, OpenAI,
//! Ollama, Gemini, Groq, xAI, Z.AI SDK, and NVIDIA NIM.

use serde::{Deserialize, Serialize};

use super::defaults::*;


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

/// OpenRouter API settings.
#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(default)]
pub struct OpenRouterSettings {
    /// OpenRouter API key (supports $ENV_VAR syntax)
    #[serde(skip_serializing_if = "Option::is_none")]
    pub api_key: Option<String>,

    /// Whether to show this provider's models in the model selector
    #[serde(default = "default_true")]
    pub show_in_selector: bool,

    /// Provider preferences for routing and filtering (optional).
    /// See https://openrouter.ai/docs/guides/routing/provider-selection
    #[serde(
        default,
        skip_serializing_if = "provider_preferences_is_empty"
    )]
    pub provider_preferences: Option<OpenRouterProviderPreferences>,
}

/// Custom skip-serialization check: skip only if None or all-empty.
/// This ensures non-empty preferences are ALWAYS serialized, preventing
/// data loss when the settings file is rewritten (e.g., on window resize).
fn provider_preferences_is_empty(prefs: &Option<OpenRouterProviderPreferences>) -> bool {
    match prefs {
        None => true,
        Some(p) => p.is_empty(),
    }
}

/// OpenRouter provider preferences for routing, filtering, and prioritization.
///
/// Maps to OpenRouter's Provider Routing API:
/// <https://openrouter.ai/docs/guides/routing/provider-selection>
///
/// All fields are optional. Only non-None fields are sent to the API.
#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(default)]
pub struct OpenRouterProviderPreferences {
    /// Provider priority ordering. Try these providers first, in order.
    /// Example: ["deepinfra", "deepseek"]
    #[serde(skip_serializing_if = "Option::is_none")]
    pub order: Option<Vec<String>>,

    /// Hard allowlist: only use these providers.
    /// Example: ["deepinfra", "atlascloud"]
    #[serde(skip_serializing_if = "Option::is_none")]
    pub only: Option<Vec<String>>,

    /// Blocklist: never use these providers.
    /// Example: ["google vertex"]
    #[serde(skip_serializing_if = "Option::is_none")]
    pub ignore: Option<Vec<String>>,

    /// Whether to allow fallback to other providers when preferred ones are unavailable.
    /// Defaults to true if not specified.
    #[serde(skip_serializing_if = "Option::is_none")]
    pub allow_fallbacks: Option<bool>,

    /// Only route to providers that support all request parameters.
    #[serde(skip_serializing_if = "Option::is_none")]
    pub require_parameters: Option<bool>,

    /// Data collection policy: "allow" or "deny".
    /// "deny" restricts to providers that do not store user data non-transiently.
    #[serde(skip_serializing_if = "Option::is_none")]
    pub data_collection: Option<String>,

    /// Require Zero Data Retention endpoints only.
    #[serde(skip_serializing_if = "Option::is_none")]
    pub zdr: Option<bool>,

    /// Sort providers by: "price", "throughput", or "latency".
    #[serde(skip_serializing_if = "Option::is_none")]
    pub sort: Option<String>,

    /// Minimum throughput threshold in tokens/sec.
    #[serde(skip_serializing_if = "Option::is_none")]
    pub preferred_min_throughput: Option<f64>,

    /// Maximum latency threshold in seconds.
    #[serde(skip_serializing_if = "Option::is_none")]
    pub preferred_max_latency: Option<f64>,

    /// Maximum price per prompt token (in USD per million tokens).
    #[serde(skip_serializing_if = "Option::is_none")]
    pub max_price_prompt: Option<f64>,

    /// Maximum price per completion token (in USD per million tokens).
    #[serde(skip_serializing_if = "Option::is_none")]
    pub max_price_completion: Option<f64>,

    /// Filter by quantization levels.
    /// Valid values: "int4", "int8", "fp8", "fp16", "bf16", "fp32", "unknown"
    #[serde(skip_serializing_if = "Option::is_none")]
    pub quantizations: Option<Vec<String>>,
}

impl Default for OpenRouterProviderPreferences {
    fn default() -> Self {
        Self {
            order: None,
            only: None,
            ignore: None,
            allow_fallbacks: None,
            require_parameters: None,
            data_collection: None,
            zdr: None,
            sort: None,
            preferred_min_throughput: None,
            preferred_max_latency: None,
            max_price_prompt: None,
            max_price_completion: None,
            quantizations: None,
        }
    }
}

impl OpenRouterProviderPreferences {
    /// Check if any preferences are set.
    pub fn is_empty(&self) -> bool {
        self.order.is_none()
            && self.only.is_none()
            && self.ignore.is_none()
            && self.allow_fallbacks.is_none()
            && self.require_parameters.is_none()
            && self.data_collection.is_none()
            && self.zdr.is_none()
            && self.sort.is_none()
            && self.preferred_min_throughput.is_none()
            && self.preferred_max_latency.is_none()
            && self.max_price_prompt.is_none()
            && self.max_price_completion.is_none()
            && self.quantizations.is_none()
    }
}

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

impl Default for OpenRouterSettings {
    fn default() -> Self {
        Self {
            api_key: None,
            show_in_selector: true,
            provider_preferences: None,
        }
    }
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

impl Default for GeminiSettings {
    fn default() -> Self {
        Self {
            api_key: None,
            show_in_selector: true,
            include_thoughts: false,
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
