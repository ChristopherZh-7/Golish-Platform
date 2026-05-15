//! AI provider, sub-agent, network, and API-key settings.
//!
//! Holds the structs that route requests to LLM providers (`AiSettings` +
//! its per-provider sibling structs in [`super::llm`]), the optional
//! per-sub-agent model overrides, the global HTTP proxy/network settings,
//! and the dedicated [`ApiKeysSettings`] for non-LLM external services.

use std::collections::HashMap;

use super::enums::{AiProvider, ReasoningEffort};
use super::llm::{
    AnthropicSettings, DeepSeekSettings, GeminiSettings, GroqSettings, NvidiaSettings,
    OllamaSettings, OpenAiSettings, OpenRouterSettings, VertexAiSettings, VertexGeminiSettings,
    XaiSettings, ZaiSdkSettings,
};
use serde::{Deserialize, Serialize};

/// Per-model user override sourced from the model settings popover in the UI.
///
/// The key in the parent `HashMap` is `"<provider>::<model_id>"`
/// (e.g. `"nvidia::qwen/qwen3.5-122b-a10b"`). `None`/`false` fields mean
/// "use the provider-default behavior" — they don't override anything.
///
/// Stored verbatim in `settings.toml` and forwarded to the LLM provider
/// layer via [`ProviderConfig::model_override`][super::super::provider_config]
/// where it influences:
///
/// - `quirks.reasoning_handling` (Standard / AlwaysContent)
/// - request-time `chat_template_kwargs.enable_thinking`
/// - request-time `reasoning.effort` / `max_tokens`
/// - per-stream debug verbosity
#[derive(Debug, Clone, Default, PartialEq, Eq, Serialize, Deserialize)]
pub struct ModelOverride {
    /// `Some(true)` enables the model's thinking mode; `Some(false)` disables
    /// it; `None` defers to the provider default.
    #[serde(skip_serializing_if = "Option::is_none")]
    pub thinking: Option<bool>,

    /// Reasoning effort: `"low" | "medium" | "high" | "max"`.
    #[serde(skip_serializing_if = "Option::is_none")]
    pub reasoning_effort: Option<String>,

    /// Max output tokens override.
    #[serde(skip_serializing_if = "Option::is_none")]
    pub max_tokens: Option<u32>,

    /// Context window override (for models with multiple context-window sizes).
    #[serde(skip_serializing_if = "Option::is_none")]
    pub context_window: Option<usize>,

    /// When `true`, emit verbose per-chunk debug events to the frontend so
    /// the user can see how reasoning / text chunks are being routed.
    #[serde(default, skip_serializing_if = "is_false")]
    pub stream_debug: bool,
}

fn is_false(b: &bool) -> bool {
    !*b
}

/// Per-sub-agent model configuration.
///
/// Allows overriding the model and LLM parameters for specific sub-agents
/// (e.g., "coder", "analyzer"). When fields are `None`, the sub-agent
/// inherits from the main agent's defaults.
#[derive(Debug, Clone, Serialize, Deserialize, Default)]
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
#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(default)]
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

    /// Per-model user overrides keyed by `"<provider>::<model_id>"`.
    ///
    /// Sourced from the model settings popover in the chat panel. Forwarded
    /// to the LLM provider layer to control thinking, reasoning effort,
    /// max_tokens, etc. See [`ModelOverride`] for the field list.
    ///
    /// Example in `settings.toml`:
    /// ```toml
    /// [ai.model_overrides."nvidia::qwen/qwen3.5-122b-a10b"]
    /// thinking = false
    /// reasoning_effort = "medium"
    /// max_tokens = 8192
    /// ```
    #[serde(default)]
    pub model_overrides: HashMap<String, ModelOverride>,

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

    /// DeepSeek direct API settings.
    pub deepseek: DeepSeekSettings,
}

impl Default for AiSettings {
    fn default() -> Self {
        Self {
            default_provider: AiProvider::default(),
            default_model: "claude-opus-4-5@20251101".to_string(),
            default_reasoning_effort: None,
            sub_agent_models: HashMap::new(),
            model_overrides: HashMap::new(),
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
            deepseek: DeepSeekSettings::default(),
        }
    }
}

/// Network settings (HTTP proxy, etc.).
///
/// When configured, proxy settings are applied to all outgoing HTTP requests
/// including LLM API calls, web fetch, and Tavily search.
#[derive(Debug, Clone, Serialize, Deserialize, Default)]
#[serde(default)]
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
#[derive(Debug, Clone, Serialize, Deserialize, Default)]
#[serde(default)]
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
