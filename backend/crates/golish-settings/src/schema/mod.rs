//! Settings schema definitions for Golish configuration.
//!
//! All settings structs use `#[serde(default)]` to allow partial configuration files.
//! Missing fields are filled with sensible defaults.

use serde::{Deserialize, Serialize};
use std::collections::HashMap;

mod enums;
mod defaults;

use defaults::*;
pub use enums::*;

mod llm;
pub use llm::*;

#[cfg(test)]
mod tests;

// =============================================================================
// Settings structs
// =============================================================================

/// Root settings structure for Golish.
///
/// Loaded from `~/.golish/settings.toml` with environment variable interpolation support.
/// Version field enables future migrations.
#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(default)]
pub struct GolishSettings {
    /// Schema version for migrations
    pub version: u32,

    /// AI provider configuration
    pub ai: AiSettings,

    /// API keys for external services
    pub api_keys: ApiKeysSettings,

    /// Tool enablement settings
    #[serde(default)]
    pub tools: ToolsSettings,

    /// User interface preferences
    pub ui: UiSettings,

    /// Terminal configuration
    pub terminal: TerminalSettings,

    /// Agent behavior settings
    pub agent: AgentSettings,

    /// MCP server definitions
    #[serde(default)]
    pub mcp_servers: HashMap<String, McpServerConfig>,

    /// Repository trust levels
    #[serde(default)]
    pub trust: TrustSettings,

    /// Privacy and telemetry settings
    pub privacy: PrivacySettings,

    /// Advanced/debug settings
    pub advanced: AdvancedSettings,

    /// Sidecar context capture settings
    pub sidecar: SidecarSettings,

    /// Code indexer settings
    pub indexer: IndexerSettings,

    /// Context window management settings
    pub context: ContextSettings,

    /// Telemetry and observability settings
    pub telemetry: TelemetrySettings,

    /// Network settings (proxy, etc.)
    #[serde(default)]
    pub network: NetworkSettings,

    /// Native OS notification settings
    #[serde(default)]
    pub notifications: NotificationsSettings,

    /// List of indexed codebase paths (deprecated, migrated to `codebases`)
    #[serde(default, skip_serializing_if = "Vec::is_empty")]
    pub indexed_codebases: Vec<String>,

    /// Indexed codebases with configuration (new format)
    #[serde(default)]
    pub codebases: Vec<CodebaseConfig>,
}

/// Per-sub-agent model configuration.
///
/// Allows overriding the model and LLM parameters for specific sub-agents
/// (e.g., "coder", "analyzer"). When fields are None, the sub-agent inherits
/// from the main agent's defaults.
#[derive(Debug, Clone, Serialize, Deserialize, Default)]
pub struct SubAgentModelConfig {
    /// Provider override (None = inherit from main agent)
    #[serde(skip_serializing_if = "Option::is_none")]
    pub provider: Option<AiProvider>,

    /// Model override (None = inherit from main agent)
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
    /// Default AI provider
    pub default_provider: AiProvider,

    /// Default model for the selected provider
    pub default_model: String,

    /// Default reasoning effort for models that support it
    #[serde(skip_serializing_if = "Option::is_none")]
    pub default_reasoning_effort: Option<ReasoningEffort>,

    /// Per-sub-agent model overrides (key = sub-agent id: "coder", "analyzer", etc.)
    ///
    /// Example in settings.toml:
    /// ```toml
    /// [ai.sub_agent_models.coder]
    /// provider = "openai"
    /// model = "gpt-4o"
    /// ```
    #[serde(default)]
    pub sub_agent_models: HashMap<String, SubAgentModelConfig>,

    /// Model to use for the summarizer agent.
    /// If not specified, uses the session's current model.
    /// Example: "claude-sonnet-4-20250514"
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub summarizer_model: Option<String>,

    /// Provider for KB research agent. Falls back to default_provider if not set.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub research_provider: Option<AiProvider>,

    /// Model for KB research agent. Falls back to default_model if not set.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub research_model: Option<String>,

    /// Vertex AI (Anthropic) specific settings
    pub vertex_ai: VertexAiSettings,

    /// Vertex AI Gemini specific settings
    pub vertex_gemini: VertexGeminiSettings,

    /// OpenRouter specific settings
    pub openrouter: OpenRouterSettings,

    /// Direct Anthropic API settings
    pub anthropic: AnthropicSettings,

    /// OpenAI settings
    pub openai: OpenAiSettings,

    /// Ollama settings
    pub ollama: OllamaSettings,

    /// Gemini settings
    pub gemini: GeminiSettings,

    /// Groq settings
    pub groq: GroqSettings,

    /// xAI (Grok) settings
    pub xai: XaiSettings,

    /// Z.AI native SDK settings
    #[serde(alias = "z_ai_sdk")]
    pub zai_sdk: ZaiSdkSettings,

    /// NVIDIA NIM settings
    pub nvidia: NvidiaSettings,
}

/// Network settings (HTTP proxy, etc.)
///
/// When configured, proxy settings are applied to all outgoing HTTP requests
/// including LLM API calls, web fetch, and Tavily search.
#[derive(Debug, Clone, Serialize, Deserialize, Default)]
#[serde(default)]
pub struct NetworkSettings {
    /// HTTP/HTTPS proxy URL (e.g., "http://127.0.0.1:7890" or "socks5://proxy:1080")
    #[serde(skip_serializing_if = "Option::is_none")]
    pub proxy_url: Option<String>,

    /// Comma-separated list of hosts that should bypass the proxy
    /// (e.g., "localhost,127.0.0.1,.local")
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
    /// Tavily API key for web search
    #[serde(skip_serializing_if = "Option::is_none")]
    pub tavily: Option<String>,

    /// GitHub token for repository access
    #[serde(skip_serializing_if = "Option::is_none")]
    pub github: Option<String>,

    /// Brave Search API key
    #[serde(skip_serializing_if = "Option::is_none")]
    pub brave: Option<String>,
}

/// Tool enablement settings.
#[derive(Debug, Clone, Default, Serialize, Deserialize)]
#[serde(default)]
pub struct ToolsSettings {
    /// Enable web search tools (Tavily)
    pub web_search: bool,
}

/// User interface preferences.
#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(default)]
pub struct UiSettings {
    /// Theme
    pub theme: Theme,

    /// Show tips on startup
    pub show_tips: bool,

    /// Hide banner/welcome message
    pub hide_banner: bool,

    /// Window state (persisted on close/resize)
    #[serde(default)]
    pub window: WindowSettings,
}

/// Window state settings (persisted across sessions).
#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(default)]
pub struct WindowSettings {
    /// Window width in pixels
    pub width: u32,

    /// Window height in pixels
    pub height: u32,

    /// Window X position (None = centered)
    #[serde(skip_serializing_if = "Option::is_none")]
    pub x: Option<i32>,

    /// Window Y position (None = centered)
    #[serde(skip_serializing_if = "Option::is_none")]
    pub y: Option<i32>,

    /// Whether the window is maximized
    pub maximized: bool,
}

/// Caret (text cursor) customization for the input area.
#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(default)]
pub struct CaretSettings {
    /// Caret style: "block" or "default" (native browser caret)
    pub style: String,

    /// Block caret width in ch units (0.5-3.0)
    pub width: f64,

    /// Caret color as hex string (e.g. "#FFFFFF"). None = inherit from theme foreground.
    #[serde(skip_serializing_if = "Option::is_none")]
    pub color: Option<String>,

    /// Blink speed in milliseconds (0 = no blink)
    pub blink_speed: f64,

    /// Caret opacity (0.0-1.0)
    pub opacity: f64,
}

impl Default for CaretSettings {
    fn default() -> Self {
        Self {
            style: "default".to_string(),
            width: 1.0,
            color: None,
            blink_speed: 530.0,
            opacity: 1.0,
        }
    }
}

/// Terminal configuration.
#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(default)]
pub struct TerminalSettings {
    /// Default shell override
    #[serde(skip_serializing_if = "Option::is_none")]
    pub shell: Option<String>,

    /// Font family
    pub font_family: String,

    /// Font size in pixels
    pub font_size: u32,

    /// Scrollback buffer lines
    pub scrollback: u32,

    /// Additional commands that trigger fullterm mode.
    /// These are merged with the built-in defaults (claude, cc, codex, etc.).
    /// Most TUI apps are auto-detected via ANSI sequences; this is for edge cases.
    #[serde(default, skip_serializing_if = "Vec::is_empty")]
    pub fullterm_commands: Vec<String>,

    /// Input caret customization
    #[serde(default)]
    pub caret: CaretSettings,
}

/// Agent behavior settings.
#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(default)]
pub struct AgentSettings {
    /// Auto-save conversations
    pub session_persistence: bool,

    /// Session retention in days (0 = forever)
    pub session_retention_days: u32,

    /// Enable pattern learning for auto-approval
    pub pattern_learning: bool,

    /// Minimum approvals before auto-approve
    pub min_approvals_for_auto: u32,

    /// Approval rate threshold (0.0 - 1.0)
    pub approval_threshold: f64,
}

/// MCP (Model Context Protocol) server configuration.
#[derive(Debug, Clone, Serialize, Deserialize, Default)]
#[serde(default)]
pub struct McpServerConfig {
    /// Command to start the server
    #[serde(skip_serializing_if = "Option::is_none")]
    pub command: Option<String>,

    /// Arguments for the command
    #[serde(default)]
    pub args: Vec<String>,

    /// Environment variables for the server
    #[serde(default)]
    pub env: HashMap<String, String>,

    /// URL for HTTP-based MCP servers
    #[serde(skip_serializing_if = "Option::is_none")]
    pub url: Option<String>,
}

/// Repository trust settings.
#[derive(Debug, Clone, Serialize, Deserialize, Default)]
#[serde(default)]
pub struct TrustSettings {
    /// Paths with full trust (all tools allowed)
    #[serde(default)]
    pub full_trust: Vec<String>,

    /// Paths with read-only trust
    #[serde(default)]
    pub read_only_trust: Vec<String>,

    /// Paths that are never trusted
    #[serde(default)]
    pub never_trust: Vec<String>,

    /// Additional paths accessible outside workspace (supports glob patterns)
    /// Example: ["~/Documents/*", "/tmp/scratch"]
    #[serde(default)]
    pub allowed_paths: Vec<String>,

    /// Disable workspace path restrictions entirely (use with caution)
    #[serde(default)]
    pub disable_path_restrictions: bool,
}

/// Privacy and telemetry settings.
#[derive(Debug, Clone, Serialize, Deserialize, Default)]
#[serde(default)]
pub struct PrivacySettings {
    /// Enable anonymous usage statistics
    pub usage_statistics: bool,

    /// Log prompts for debugging (local only)
    pub log_prompts: bool,
}

/// Advanced/debug settings.
#[derive(Debug, Clone, Serialize, Deserialize, Default)]
#[serde(default)]
pub struct AdvancedSettings {
    /// Enable experimental features
    pub enable_experimental: bool,

    /// Log level
    pub log_level: LogLevel,

    /// Enable LLM API request/response logging to ./logs/api/
    /// When enabled, raw JSON request/response data is logged per session
    pub enable_llm_api_logs: bool,

    /// Extract and parse the raw SSE JSON instead of logging escaped strings
    /// When enabled, SSE chunks are logged as parsed JSON objects
    pub extract_raw_sse: bool,
}

/// Code indexer settings.
#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(default)]
pub struct IndexerSettings {
    /// Where to store index files: "global" or "local"
    pub index_location: IndexLocation,
}

/// Telemetry and observability settings.
#[derive(Debug, Clone, Default, Serialize, Deserialize)]
#[serde(default)]
pub struct TelemetrySettings {
    /// Langfuse integration settings
    pub langfuse: LangfuseSettings,
}

/// Langfuse tracing configuration.
///
/// Langfuse provides LLM observability via OpenTelemetry.
/// See: https://langfuse.com/docs/integrations/opentelemetry
#[derive(Debug, Clone, Default, Serialize, Deserialize)]
#[serde(default)]
pub struct LangfuseSettings {
    /// Enable Langfuse tracing
    pub enabled: bool,

    /// Langfuse host URL (defaults to https://cloud.langfuse.com)
    #[serde(skip_serializing_if = "Option::is_none")]
    pub host: Option<String>,

    /// Langfuse public key (supports $ENV_VAR syntax, or set LANGFUSE_PUBLIC_KEY env var)
    #[serde(skip_serializing_if = "Option::is_none")]
    pub public_key: Option<String>,

    /// Langfuse secret key (supports $ENV_VAR syntax, or set LANGFUSE_SECRET_KEY env var)
    #[serde(skip_serializing_if = "Option::is_none")]
    pub secret_key: Option<String>,

    /// Sampling ratio (0.0 to 1.0, default 1.0 = sample everything)
    /// Use lower values for high-traffic production deployments
    #[serde(skip_serializing_if = "Option::is_none")]
    pub sampling_ratio: Option<f64>,
}

/// Context window management settings.
#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(default)]
pub struct ContextSettings {
    /// Enable context window management
    #[serde(default = "default_context_enabled")]
    pub enabled: bool,

    /// Context utilization threshold (0.0-1.0) at which compaction is triggered
    #[serde(default = "default_compaction_threshold")]
    pub compaction_threshold: f64,

    /// DEPRECATED: No longer used. Compaction replaces pruning.
    /// Kept for backwards compatibility with existing config files.
    #[serde(default = "default_protected_turns")]
    pub protected_turns: usize,

    /// DEPRECATED: No longer used. Compaction replaces pruning.
    /// Kept for backwards compatibility with existing config files.
    #[serde(default = "default_cooldown_seconds")]
    pub cooldown_seconds: u64,
}

/// Native OS notification settings.
#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(default)]
pub struct NotificationsSettings {
    /// Enable native OS notifications for agent/command completion
    pub native_enabled: bool,

    /// Enable in-app notification sounds (independent of OS notifications).
    /// Defaults to true.
    pub sound_enabled: bool,

    /// Notification sound (macOS system sound name like "Blow" or "Ping").
    /// If None, defaults to "Blow" on macOS and no sound on other platforms.
    #[serde(skip_serializing_if = "Option::is_none")]
    pub sound: Option<String>,
}

impl Default for NotificationsSettings {
    fn default() -> Self {
        Self {
            native_enabled: false,
            sound_enabled: true,
            sound: None,
        }
    }
}

impl Default for IndexerSettings {
    fn default() -> Self {
        Self {
            index_location: IndexLocation::Global,
        }
    }
}

impl Default for ContextSettings {
    fn default() -> Self {
        Self {
            enabled: default_context_enabled(),
            compaction_threshold: default_compaction_threshold(),
            protected_turns: default_protected_turns(),
            cooldown_seconds: default_cooldown_seconds(),
        }
    }
}

/// Configuration for an indexed codebase.
#[derive(Debug, Clone, Serialize, Deserialize, Default)]
pub struct CodebaseConfig {
    /// Path to the codebase (supports ~ for home directory)
    pub path: String,

    /// Memory file associated with this codebase: "AGENTS.md", "CLAUDE.md", or None
    #[serde(skip_serializing_if = "Option::is_none")]
    pub memory_file: Option<String>,
}

/// Sidecar context capture settings.
#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(default)]
pub struct SidecarSettings {
    /// Enable context capture during AI sessions
    pub enabled: bool,

    /// Enable LLM synthesis for commit messages and summaries
    pub synthesis_enabled: bool,

    /// Synthesis backend: "local" | "vertex_anthropic" | "openai" | "grok" | "template"
    pub synthesis_backend: String,

    /// Vertex AI settings for synthesis (when synthesis_backend = "vertex_anthropic")
    pub synthesis_vertex: SynthesisVertexSettings,

    /// OpenAI settings for synthesis (when synthesis_backend = "openai")
    pub synthesis_openai: SynthesisOpenAiSettings,

    /// Grok settings for synthesis (when synthesis_backend = "grok")
    pub synthesis_grok: SynthesisGrokSettings,

    /// Event retention in days (0 = forever)
    pub retention_days: u32,

    /// Capture tool call events
    pub capture_tool_calls: bool,

    /// Capture agent reasoning events
    pub capture_reasoning: bool,
}

/// Vertex AI settings for sidecar synthesis.
#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(default)]
pub struct SynthesisVertexSettings {
    /// Google Cloud project ID (falls back to ai.vertex_ai.project_id if not set)
    #[serde(skip_serializing_if = "Option::is_none")]
    pub project_id: Option<String>,

    /// Vertex AI region (falls back to ai.vertex_ai.location if not set)
    #[serde(skip_serializing_if = "Option::is_none")]
    pub location: Option<String>,

    /// Model to use for synthesis (default: claude-sonnet-4-20250514)
    pub model: String,

    /// Path to credentials (falls back to ai.vertex_ai.credentials_path if not set)
    #[serde(skip_serializing_if = "Option::is_none")]
    pub credentials_path: Option<String>,
}

/// OpenAI settings for sidecar synthesis.
#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(default)]
pub struct SynthesisOpenAiSettings {
    /// API key (falls back to api_keys or env var)
    #[serde(skip_serializing_if = "Option::is_none")]
    pub api_key: Option<String>,

    /// Model to use for synthesis (default: gpt-4o-mini)
    pub model: String,

    /// Custom base URL for OpenAI-compatible APIs
    #[serde(skip_serializing_if = "Option::is_none")]
    pub base_url: Option<String>,
}

/// Grok settings for sidecar synthesis.
#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(default)]
pub struct SynthesisGrokSettings {
    /// API key (falls back to env var GROK_API_KEY)
    #[serde(skip_serializing_if = "Option::is_none")]
    pub api_key: Option<String>,

    /// Model to use for synthesis (default: grok-2)
    pub model: String,
}

// =============================================================================
// Default implementations
// =============================================================================

impl Default for GolishSettings {
    fn default() -> Self {
        Self {
            version: 1,
            ai: AiSettings::default(),
            api_keys: ApiKeysSettings::default(),
            tools: ToolsSettings::default(),
            ui: UiSettings::default(),
            terminal: TerminalSettings::default(),
            agent: AgentSettings::default(),
            mcp_servers: HashMap::new(),
            trust: TrustSettings::default(),
            privacy: PrivacySettings::default(),
            advanced: AdvancedSettings::default(),
            sidecar: SidecarSettings::default(),
            indexer: IndexerSettings::default(),
            context: ContextSettings::default(),
            telemetry: TelemetrySettings::default(),
            network: NetworkSettings::default(),
            notifications: NotificationsSettings::default(),
            indexed_codebases: Vec::new(),
            codebases: Vec::new(),
        }
    }
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

impl Default for UiSettings {
    fn default() -> Self {
        Self {
            theme: Theme::default(),
            show_tips: true,
            hide_banner: false,
            window: WindowSettings::default(),
        }
    }
}

impl Default for WindowSettings {
    fn default() -> Self {
        Self {
            width: 1400,
            height: 900,
            x: None,
            y: None,
            maximized: false,
        }
    }
}

impl Default for TerminalSettings {
    fn default() -> Self {
        Self {
            shell: None,
            font_family: "SF Mono".to_string(),
            font_size: 14,
            scrollback: 10000,
            fullterm_commands: Vec::new(),
            caret: CaretSettings::default(),
        }
    }
}

impl Default for AgentSettings {
    fn default() -> Self {
        Self {
            session_persistence: true,
            session_retention_days: 30,
            pattern_learning: true,
            min_approvals_for_auto: 3,
            approval_threshold: 0.8,
        }
    }
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

impl Default for SynthesisOpenAiSettings {
    fn default() -> Self {
        Self {
            api_key: None,
            model: "gpt-4o-mini".to_string(),
            base_url: None,
        }
    }
}

impl Default for SynthesisGrokSettings {
    fn default() -> Self {
        Self {
            api_key: None,
            model: "grok-2".to_string(),
        }
    }
}
