//! AI provider, sub-agent, network, and API-key settings.
//!
//! Holds the structs that route requests to LLM providers (`AiSettings` +
//! its per-provider sibling structs in [`super::llm`]), the optional
//! per-sub-agent model overrides, the global HTTP proxy/network settings,
//! and the dedicated [`ApiKeysSettings`] for non-LLM external services.

use std::collections::HashMap;

use serde::{Deserialize, Serialize};
use ts_rs::TS;

use super::enums::{AiProvider, ReasoningEffort};
use super::llm::{
    AnthropicSettings, GeminiSettings, GroqSettings, NvidiaSettings, OllamaSettings,
    OpenAiSettings, OpenRouterSettings, VertexAiSettings, VertexGeminiSettings, XaiSettings,
    ZaiSdkSettings,
};

/// Per-sub-agent model configuration.
///
/// Allows overriding the model and LLM parameters for specific sub-agents
/// (e.g., "coder", "analyzer"). When fields are `None`, the sub-agent
/// inherits from the main agent's defaults.
#[derive(Debug, Clone, Serialize, Deserialize, Default, TS)]
#[ts(export, export_to = "generated/")]
pub struct SubAgentModelConfig {
    /// Provider override (None = inherit from main agent).
    #[serde(skip_serializing_if = "Option::is_none")]
    pub provider: Option<AiProvider>,

    /// Model override (None = inherit from main agent).
    #[serde(skip_serializing_if = "Option::is_none")]
    pub model: Option<String>,

    /// Temperature override (0.0 - 2.0). Lower = more deterministic, higher = more creative.
    /// None = use the model's default (typically 0.3 for agents).
    #[serde(skip_serializing_if = "Option::is_none")]
    pub temperature: Option<f32>,

    /// Max output tokens override.
    /// None = use the model's default (typically 16384).
    #[serde(skip_serializing_if = "Option::is_none")]
    pub max_tokens: Option<u32>,

    /// Top-p (nucleus sampling) override (0.0 - 1.0).
    /// None = not sent to the provider (uses their default).
    #[serde(skip_serializing_if = "Option::is_none")]
    pub top_p: Option<f32>,
}

/// AI provider configuration.
#[derive(Debug, Clone, Serialize, Deserialize, TS)]
#[serde(default)]
#[ts(export, export_to = "generated/")]
pub struct AiSettings {
    /// Default AI provider.
    pub default_provider: AiProvider,

    /// Default model for the selected provider.
    pub default_model: String,

    /// Default reasoning effort for models that support it.
    #[serde(skip_serializing_if = "Option::is_none")]
    pub default_reasoning_effort: Option<ReasoningEffort>,

    /// Per-sub-agent model overrides (key = sub-agent id: "coder", "analyzer", etc.).
    ///
    /// Example in `settings.toml`:
    /// ```toml
    /// [ai.sub_agent_models.coder]
    /// provider = "openai"
    /// model = "gpt-4o"
    /// ```
    #[serde(default)]
    pub sub_agent_models: HashMap<String, SubAgentModelConfig>,

    /// Model to use for the summarizer agent.
    /// If not specified, uses the session's current model.
    /// Example: `"claude-sonnet-4-20250514"`.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub summarizer_model: Option<String>,

    /// Provider for KB research agent. Falls back to `default_provider` if not set.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub research_provider: Option<AiProvider>,

    /// Model for KB research agent. Falls back to `default_model` if not set.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub research_model: Option<String>,

    /// Vertex AI (Anthropic) specific settings.
    pub vertex_ai: VertexAiSettings,

    /// Vertex AI Gemini specific settings.
    pub vertex_gemini: VertexGeminiSettings,

    /// OpenRouter specific settings.
    pub openrouter: OpenRouterSettings,

    /// Direct Anthropic API settings.
    pub anthropic: AnthropicSettings,

    /// OpenAI settings.
    pub openai: OpenAiSettings,

    /// Ollama settings.
    pub ollama: OllamaSettings,

    /// Gemini settings.
    pub gemini: GeminiSettings,

    /// Groq settings.
    pub groq: GroqSettings,

    /// xAI (Grok) settings.
    pub xai: XaiSettings,

    /// Z.AI native SDK settings.
    #[serde(alias = "z_ai_sdk")]
    pub zai_sdk: ZaiSdkSettings,

    /// NVIDIA NIM settings.
    pub nvidia: NvidiaSettings,
}

impl Default for AiSettings {
    fn default() -> Self {
        Self {
            default_provider: AiProvider::default(),
            default_model: "claude-opus-4-5@20251101".to_string(),
            default_reasoning_effort: None,
            sub_agent_models: HashMap::new(),
            summarizer_model: None,
            research_provider: None,
            research_model: None,
            vertex_ai: VertexAiSettings::default(),
            vertex_gemini: VertexGeminiSettings::default(),
            openrouter: OpenRouterSettings::default(),
            anthropic: AnthropicSettings::default(),
            openai: OpenAiSettings::default(),
            ollama: OllamaSettings::default(),
            gemini: GeminiSettings::default(),
            groq: GroqSettings::default(),
            xai: XaiSettings::default(),
            zai_sdk: ZaiSdkSettings::default(),
            nvidia: NvidiaSettings::default(),
        }
    }
}

/// Network settings (HTTP proxy, etc.).
///
/// When configured, proxy settings are applied to all outgoing HTTP requests
/// including LLM API calls, web fetch, and Tavily search.
#[derive(Debug, Clone, Serialize, Deserialize, Default, TS)]
#[serde(default)]
#[ts(export, export_to = "generated/")]
pub struct NetworkSettings {
    /// HTTP/HTTPS proxy URL (e.g., `"http://127.0.0.1:7890"` or `"socks5://proxy:1080"`).
    #[serde(skip_serializing_if = "Option::is_none")]
    pub proxy_url: Option<String>,

    /// Comma-separated list of hosts that should bypass the proxy
    /// (e.g., `"localhost,127.0.0.1,.local"`).
    #[serde(skip_serializing_if = "Option::is_none")]
    pub no_proxy: Option<String>,

    /// GitHub Personal Access Token for higher API rate limits (5000/hour vs 60/hour).
    /// Used for tool downloads that fetch GitHub release information.
    #[serde(skip_serializing_if = "Option::is_none")]
    pub github_token: Option<String>,
}

/// API keys for external services.
#[derive(Debug, Clone, Serialize, Deserialize, Default, TS)]
#[serde(default)]
#[ts(export, export_to = "generated/")]
pub struct ApiKeysSettings {
    /// Tavily API key for web search.
    #[serde(skip_serializing_if = "Option::is_none")]
    pub tavily: Option<String>,

    /// GitHub token for repository access.
    #[serde(skip_serializing_if = "Option::is_none")]
    pub github: Option<String>,

    /// Brave Search API key.
    #[serde(skip_serializing_if = "Option::is_none")]
    pub brave: Option<String>,
}
