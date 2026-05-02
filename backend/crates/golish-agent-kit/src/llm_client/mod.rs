//! LLM client abstraction for the agent system.
//!
//! Re-exports types from `golish-llm-providers` and provides per-provider
//! component builders used by `agent_bridge::constructors`.
//!
//! ## Module layout
//!
//! - [`providers`] — one module per provider (`openai`, `anthropic`, `ollama`, …).
//!   Each exposes a single `create_*_components` builder. They are flattened up
//!   to `crate::llm_client::*` so callers keep using the original paths.
//! - [`factory`]   — [`LlmClientFactory`] for caching sub-agent model overrides.
//!
//! Shared helpers live here:
//! - [`AgentBridgeComponents`]   — common return type for every provider builder
//! - [`SharedComponentsConfig`]  — input config for shared init
//! - `SharedComponents`          — internal bag, hidden from outside
//! - `create_shared_components`  — internal constructor for the bag

mod factory;
mod providers;

use std::path::Path;
use std::path::PathBuf;
use std::sync::Arc;

use tokio::sync::RwLock;

use golish_tools::{ToolRegistry, ToolRegistryConfig};

use crate::hitl::ApprovalRecorder;
use crate::loop_detection::LoopDetector;
use crate::tool_policy::ToolPolicyManager;
use golish_context::ContextManager;
use golish_sub_agents::SubAgentRegistry;

// Re-export provider builders flat so `crate::llm_client::create_*` keeps working.
pub use factory::LlmClientFactory;
pub use providers::*;

// Re-export types from golish-llm-providers for backward compatibility
pub use golish_llm_providers::{
    rig_gemini_vertex, rig_zai_sdk, AnthropicClientConfig, GeminiClientConfig, GroqClientConfig,
    LlmClient, NvidiaClientConfig, OllamaClientConfig, OpenAiClientConfig, OpenRouterClientConfig,
    ProviderConfig, VertexAnthropicClientConfig, VertexGeminiClientConfig, XaiClientConfig,
    ZaiSdkClientConfig,
};

// Re-export ContextManagerConfig for convenience (also used internally)
pub use golish_context::ContextManagerConfig;

/// Common initialization result containing shared components
pub struct AgentBridgeComponents {
    pub workspace: Arc<RwLock<PathBuf>>,
    pub provider_name: String,
    pub model_name: String,
    pub tool_registry: Arc<RwLock<ToolRegistry>>,
    pub client: Arc<RwLock<LlmClient>>,
    pub sub_agent_registry: Arc<RwLock<SubAgentRegistry>>,
    pub approval_recorder: Arc<ApprovalRecorder>,
    pub tool_policy_manager: Arc<ToolPolicyManager>,
    pub context_manager: Arc<ContextManager>,
    pub loop_detector: Arc<RwLock<LoopDetector>>,
    /// OpenAI web search configuration (if enabled)
    pub openai_web_search_config: Option<golish_llm_providers::OpenAiWebSearchConfig>,
    /// OpenAI reasoning effort level (if set)
    pub openai_reasoning_effort: Option<String>,
    /// Factory for creating sub-agent model override clients (optional, lazy-init)
    pub model_factory: Option<Arc<LlmClientFactory>>,
    /// OpenRouter provider preferences JSON for routing and filtering (optional)
    pub openrouter_provider_preferences: Option<serde_json::Value>,
}

/// Shared components that are common to all LLM providers.
pub(crate) struct SharedComponents {
    pub(crate) tool_registry: Arc<RwLock<ToolRegistry>>,
    pub(crate) sub_agent_registry: Arc<RwLock<SubAgentRegistry>>,
    pub(crate) approval_recorder: Arc<ApprovalRecorder>,
    pub(crate) tool_policy_manager: Arc<ToolPolicyManager>,
    pub(crate) context_manager: Arc<ContextManager>,
    pub(crate) loop_detector: Arc<RwLock<LoopDetector>>,
}

/// Configuration for shared components.
#[derive(Default, Clone)]
pub struct SharedComponentsConfig {
    /// Settings instance.
    pub settings: golish_settings::GolishSettings,
    /// Context manager configuration.
    pub context_config: Option<ContextManagerConfig>,
}

/// Initialize shared components from a workspace path and model name.
///
/// If `context_config` is provided, the ContextManager will be created with those settings.
/// Otherwise, it will use the model's defaults (with context management disabled by default).
pub(crate) async fn create_shared_components(
    workspace: &Path,
    model: &str,
    config: SharedComponentsConfig,
) -> SharedComponents {
    // Create prompt registry (embedded templates) and populate sub-agent registry
    let prompt_registry = golish_sub_agents::PromptRegistry::new();
    let mut sub_agent_registry = SubAgentRegistry::new();
    sub_agent_registry.register_multiple(
        golish_sub_agents::defaults::create_default_sub_agents_from_registry(&prompt_registry).await,
    );

    // Create context manager with config if provided, otherwise use model defaults
    let context_manager = match config.context_config {
        Some(ctx_config) => {
            tracing::debug!(
                "[context] Creating ContextManager with config: enabled={}, threshold={:.2}, protected_turns={}, cooldown={}s",
                ctx_config.enabled,
                ctx_config.compaction_threshold,
                ctx_config.protected_turns,
                ctx_config.cooldown_seconds
            );
            ContextManager::with_config(model, ctx_config)
        }
        None => {
            tracing::debug!(
                "[context] Creating ContextManager with model defaults (context management disabled)"
            );
            ContextManager::for_model(model)
        }
    };

    let tool_registry_config = ToolRegistryConfig {
        settings: config.settings.clone(),
    };
    if config.settings.terminal.shell.is_some() {
        tracing::debug!("[tools] Creating ToolRegistry with shell override from settings");
    }

    SharedComponents {
        tool_registry: Arc::new(RwLock::new(
            ToolRegistry::with_config(workspace.to_path_buf(), tool_registry_config).await,
        )),
        sub_agent_registry: Arc::new(RwLock::new(sub_agent_registry)),
        approval_recorder: Arc::new(
            ApprovalRecorder::new(workspace.join(".golish").join("hitl")).await,
        ),
        tool_policy_manager: Arc::new(ToolPolicyManager::new(workspace).await),
        context_manager: Arc::new(context_manager),
        loop_detector: Arc::new(RwLock::new(LoopDetector::with_defaults())),
    }
}
