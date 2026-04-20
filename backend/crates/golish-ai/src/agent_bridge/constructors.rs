//! AgentBridge constructor methods for all supported LLM providers.
//!
//! Each provider typically has three constructor variants:
//! - `new_{provider}_with_runtime` — minimal config, uses defaults
//! - `new_{provider}_with_context` — adds optional context config
//! - `new_{provider}_with_shared_config` — full config with shared components

use std::path::PathBuf;
use std::sync::atomic::{AtomicBool, AtomicU64};
use std::sync::Arc;

use anyhow::Result;
use tokio::sync::RwLock;

use golish_context::{CompactionState, ContextManagerConfig};
use golish_core::runtime::GolishRuntime;
use golish_core::ApiRequestStats;
use super::AgentBridge;
use crate::agent_mode::AgentMode;
use crate::event_coordinator::EventCoordinator;
use crate::llm_client::{
    create_anthropic_components, create_gemini_components, create_groq_components,
    create_nvidia_components, create_ollama_components, create_openai_components,
    create_openrouter_components, create_vertex_components, create_vertex_gemini_components,
    create_xai_components, create_zai_sdk_components, AgentBridgeComponents,
    AnthropicClientConfig, GeminiClientConfig, GroqClientConfig, NvidiaClientConfig,
    OllamaClientConfig, OpenAiClientConfig, OpenRouterClientConfig, SharedComponentsConfig,
    VertexAnthropicClientConfig, VertexGeminiClientConfig, XaiClientConfig, ZaiSdkClientConfig,
};
use crate::planner::PlanManager;
use crate::tool_definitions::ToolConfig;

impl AgentBridge {
    // ========================================================================
    // Constructor Methods
    // ========================================================================

    pub async fn new_with_runtime(
        workspace: PathBuf,
        _provider: &str,
        model: &str,
        api_key: &str,
        runtime: Arc<dyn GolishRuntime>,
    ) -> Result<Self> {
        Self::new_openrouter_with_runtime(workspace, model, api_key, None, runtime).await
    }

    pub async fn new_openrouter_with_runtime(
        workspace: PathBuf,
        model: &str,
        api_key: &str,
        context_config: Option<ContextManagerConfig>,
        runtime: Arc<dyn GolishRuntime>,
    ) -> Result<Self> {
        let shared_config = SharedComponentsConfig {
            context_config,
            settings: golish_settings::GolishSettings::default(),
        };
        Self::new_openrouter_with_shared_config(
            workspace,
            model,
            api_key,
            None,
            shared_config,
            runtime,
            "",
        )
        .await
    }

    pub async fn new_openrouter_with_shared_config(
        workspace: PathBuf,
        model: &str,
        api_key: &str,
        provider_preferences: Option<serde_json::Value>,
        shared_config: SharedComponentsConfig,
        runtime: Arc<dyn GolishRuntime>,
        event_session_id: &str,
    ) -> Result<Self> {
        let config = OpenRouterClientConfig {
            workspace,
            model,
            api_key,
            provider_preferences,
        };

        let components = create_openrouter_components(config, shared_config).await?;

        Ok(Self::from_components_with_runtime(
            components,
            runtime,
            event_session_id.to_string(),
        ))
    }

    pub async fn new_vertex_anthropic_with_runtime(
        workspace: PathBuf,
        credentials_path: Option<&str>,
        project_id: &str,
        location: &str,
        model: &str,
        runtime: Arc<dyn GolishRuntime>,
    ) -> Result<Self> {
        Self::new_vertex_anthropic_with_context(
            workspace,
            credentials_path,
            project_id,
            location,
            model,
            None,
            runtime,
        )
        .await
    }

    pub async fn new_vertex_anthropic_with_context(
        workspace: PathBuf,
        credentials_path: Option<&str>,
        project_id: &str,
        location: &str,
        model: &str,
        context_config: Option<ContextManagerConfig>,
        runtime: Arc<dyn GolishRuntime>,
    ) -> Result<Self> {
        let shared_config = SharedComponentsConfig {
            context_config,
            settings: golish_settings::GolishSettings::default(),
        };
        Self::new_vertex_anthropic_with_shared_config(
            workspace,
            credentials_path,
            project_id,
            location,
            model,
            shared_config,
            runtime,
            "",
        )
        .await
    }

    #[allow(clippy::too_many_arguments)]
    pub async fn new_vertex_anthropic_with_shared_config(
        workspace: PathBuf,
        credentials_path: Option<&str>,
        project_id: &str,
        location: &str,
        model: &str,
        shared_config: SharedComponentsConfig,
        runtime: Arc<dyn GolishRuntime>,
        event_session_id: &str,
    ) -> Result<Self> {
        let config = VertexAnthropicClientConfig {
            workspace,
            credentials_path,
            project_id,
            location,
            model,
        };

        let components = create_vertex_components(config, shared_config).await?;

        Ok(Self::from_components_with_runtime(
            components,
            runtime,
            event_session_id.to_string(),
        ))
    }

    pub async fn new_vertex_gemini_with_runtime(
        workspace: PathBuf,
        credentials_path: Option<&str>,
        project_id: &str,
        location: &str,
        model: &str,
        include_thoughts: bool,
        runtime: Arc<dyn GolishRuntime>,
    ) -> Result<Self> {
        Self::new_vertex_gemini_with_context(
            workspace,
            credentials_path,
            project_id,
            location,
            model,
            include_thoughts,
            None,
            runtime,
        )
        .await
    }

    #[allow(clippy::too_many_arguments)]
    pub async fn new_vertex_gemini_with_context(
        workspace: PathBuf,
        credentials_path: Option<&str>,
        project_id: &str,
        location: &str,
        model: &str,
        include_thoughts: bool,
        context_config: Option<ContextManagerConfig>,
        runtime: Arc<dyn GolishRuntime>,
    ) -> Result<Self> {
        let shared_config = SharedComponentsConfig {
            context_config,
            settings: golish_settings::GolishSettings::default(),
        };
        Self::new_vertex_gemini_with_shared_config(
            workspace,
            credentials_path,
            project_id,
            location,
            model,
            include_thoughts,
            shared_config,
            runtime,
            "",
        )
        .await
    }

    #[allow(clippy::too_many_arguments)]
    pub async fn new_vertex_gemini_with_shared_config(
        workspace: PathBuf,
        credentials_path: Option<&str>,
        project_id: &str,
        location: &str,
        model: &str,
        include_thoughts: bool,
        shared_config: SharedComponentsConfig,
        runtime: Arc<dyn GolishRuntime>,
        event_session_id: &str,
    ) -> Result<Self> {
        let config = VertexGeminiClientConfig {
            workspace,
            credentials_path,
            project_id,
            location,
            model,
            include_thoughts,
        };

        let components = create_vertex_gemini_components(config, shared_config).await?;

        Ok(Self::from_components_with_runtime(
            components,
            runtime,
            event_session_id.to_string(),
        ))
    }

    pub async fn new_openai_with_runtime(
        workspace: PathBuf,
        model: &str,
        api_key: &str,
        base_url: Option<&str>,
        reasoning_effort: Option<&str>,
        runtime: Arc<dyn GolishRuntime>,
    ) -> Result<Self> {
        Self::new_openai_with_context(
            workspace,
            model,
            api_key,
            base_url,
            reasoning_effort,
            None,
            runtime,
        )
        .await
    }

    pub async fn new_openai_with_context(
        workspace: PathBuf,
        model: &str,
        api_key: &str,
        base_url: Option<&str>,
        reasoning_effort: Option<&str>,
        context_config: Option<ContextManagerConfig>,
        runtime: Arc<dyn GolishRuntime>,
    ) -> Result<Self> {
        let shared_config = SharedComponentsConfig {
            context_config,
            settings: golish_settings::GolishSettings::default(),
        };
        Self::new_openai_with_shared_config(
            workspace,
            model,
            api_key,
            base_url,
            reasoning_effort,
            shared_config,
            runtime,
            "",
        )
        .await
    }

    #[allow(clippy::too_many_arguments)]
    pub async fn new_openai_with_shared_config(
        workspace: PathBuf,
        model: &str,
        api_key: &str,
        base_url: Option<&str>,
        reasoning_effort: Option<&str>,
        shared_config: SharedComponentsConfig,
        runtime: Arc<dyn GolishRuntime>,
        event_session_id: &str,
    ) -> Result<Self> {
        let config = OpenAiClientConfig {
            workspace,
            model,
            api_key,
            base_url,
            reasoning_effort,
            enable_web_search: false,
            web_search_context_size: "medium",
        };
        let components = create_openai_components(config, shared_config).await?;
        Ok(Self::from_components_with_runtime(
            components,
            runtime,
            event_session_id.to_string(),
        ))
    }

    pub async fn new_anthropic_with_runtime(
        workspace: PathBuf,
        model: &str,
        api_key: &str,
        runtime: Arc<dyn GolishRuntime>,
    ) -> Result<Self> {
        Self::new_anthropic_with_context(workspace, model, api_key, None, runtime).await
    }

    pub async fn new_anthropic_with_context(
        workspace: PathBuf,
        model: &str,
        api_key: &str,
        context_config: Option<ContextManagerConfig>,
        runtime: Arc<dyn GolishRuntime>,
    ) -> Result<Self> {
        let shared_config = SharedComponentsConfig {
            context_config,
            settings: golish_settings::GolishSettings::default(),
        };
        Self::new_anthropic_with_shared_config(
            workspace,
            model,
            api_key,
            shared_config,
            runtime,
            "",
        )
        .await
    }

    pub async fn new_anthropic_with_shared_config(
        workspace: PathBuf,
        model: &str,
        api_key: &str,
        shared_config: SharedComponentsConfig,
        runtime: Arc<dyn GolishRuntime>,
        event_session_id: &str,
    ) -> Result<Self> {
        let config = AnthropicClientConfig {
            workspace,
            model,
            api_key,
        };
        let components = create_anthropic_components(config, shared_config).await?;
        Ok(Self::from_components_with_runtime(
            components,
            runtime,
            event_session_id.to_string(),
        ))
    }

    pub async fn new_ollama_with_runtime(
        workspace: PathBuf,
        model: &str,
        base_url: Option<&str>,
        runtime: Arc<dyn GolishRuntime>,
    ) -> Result<Self> {
        Self::new_ollama_with_context(workspace, model, base_url, None, runtime).await
    }

    pub async fn new_ollama_with_context(
        workspace: PathBuf,
        model: &str,
        base_url: Option<&str>,
        context_config: Option<ContextManagerConfig>,
        runtime: Arc<dyn GolishRuntime>,
    ) -> Result<Self> {
        let shared_config = SharedComponentsConfig {
            context_config,
            settings: golish_settings::GolishSettings::default(),
        };
        Self::new_ollama_with_shared_config(workspace, model, base_url, shared_config, runtime, "")
            .await
    }

    pub async fn new_ollama_with_shared_config(
        workspace: PathBuf,
        model: &str,
        base_url: Option<&str>,
        shared_config: SharedComponentsConfig,
        runtime: Arc<dyn GolishRuntime>,
        event_session_id: &str,
    ) -> Result<Self> {
        let config = OllamaClientConfig {
            workspace,
            model,
            base_url,
        };
        let components = create_ollama_components(config, shared_config).await?;
        Ok(Self::from_components_with_runtime(
            components,
            runtime,
            event_session_id.to_string(),
        ))
    }

    pub async fn new_gemini_with_runtime(
        workspace: PathBuf,
        model: &str,
        api_key: &str,
        runtime: Arc<dyn GolishRuntime>,
    ) -> Result<Self> {
        Self::new_gemini_with_context(workspace, model, api_key, None, runtime).await
    }

    pub async fn new_gemini_with_context(
        workspace: PathBuf,
        model: &str,
        api_key: &str,
        context_config: Option<ContextManagerConfig>,
        runtime: Arc<dyn GolishRuntime>,
    ) -> Result<Self> {
        let shared_config = SharedComponentsConfig {
            context_config,
            settings: golish_settings::GolishSettings::default(),
        };
        Self::new_gemini_with_shared_config(workspace, model, api_key, shared_config, runtime, "")
            .await
    }

    pub async fn new_gemini_with_shared_config(
        workspace: PathBuf,
        model: &str,
        api_key: &str,
        shared_config: SharedComponentsConfig,
        runtime: Arc<dyn GolishRuntime>,
        event_session_id: &str,
    ) -> Result<Self> {
        let config = GeminiClientConfig {
            workspace,
            model,
            api_key,
        };
        let components = create_gemini_components(config, shared_config).await?;
        Ok(Self::from_components_with_runtime(
            components,
            runtime,
            event_session_id.to_string(),
        ))
    }

    pub async fn new_groq_with_runtime(
        workspace: PathBuf,
        model: &str,
        api_key: &str,
        runtime: Arc<dyn GolishRuntime>,
    ) -> Result<Self> {
        Self::new_groq_with_context(workspace, model, api_key, None, runtime).await
    }

    pub async fn new_groq_with_context(
        workspace: PathBuf,
        model: &str,
        api_key: &str,
        context_config: Option<ContextManagerConfig>,
        runtime: Arc<dyn GolishRuntime>,
    ) -> Result<Self> {
        let shared_config = SharedComponentsConfig {
            context_config,
            settings: golish_settings::GolishSettings::default(),
        };
        Self::new_groq_with_shared_config(workspace, model, api_key, shared_config, runtime, "")
            .await
    }

    pub async fn new_groq_with_shared_config(
        workspace: PathBuf,
        model: &str,
        api_key: &str,
        shared_config: SharedComponentsConfig,
        runtime: Arc<dyn GolishRuntime>,
        event_session_id: &str,
    ) -> Result<Self> {
        let config = GroqClientConfig {
            workspace,
            model,
            api_key,
        };
        let components = create_groq_components(config, shared_config).await?;
        Ok(Self::from_components_with_runtime(
            components,
            runtime,
            event_session_id.to_string(),
        ))
    }

    pub async fn new_xai_with_runtime(
        workspace: PathBuf,
        model: &str,
        api_key: &str,
        runtime: Arc<dyn GolishRuntime>,
    ) -> Result<Self> {
        Self::new_xai_with_context(workspace, model, api_key, None, runtime).await
    }

    pub async fn new_xai_with_context(
        workspace: PathBuf,
        model: &str,
        api_key: &str,
        context_config: Option<ContextManagerConfig>,
        runtime: Arc<dyn GolishRuntime>,
    ) -> Result<Self> {
        let shared_config = SharedComponentsConfig {
            context_config,
            settings: golish_settings::GolishSettings::default(),
        };
        Self::new_xai_with_shared_config(workspace, model, api_key, shared_config, runtime, "")
            .await
    }

    pub async fn new_xai_with_shared_config(
        workspace: PathBuf,
        model: &str,
        api_key: &str,
        shared_config: SharedComponentsConfig,
        runtime: Arc<dyn GolishRuntime>,
        event_session_id: &str,
    ) -> Result<Self> {
        let config = XaiClientConfig {
            workspace,
            model,
            api_key,
        };
        let components = create_xai_components(config, shared_config).await?;
        Ok(Self::from_components_with_runtime(
            components,
            runtime,
            event_session_id.to_string(),
        ))
    }

    pub async fn new_zai_sdk_with_runtime(
        workspace: PathBuf,
        model: &str,
        api_key: &str,
        base_url: Option<&str>,
        source_channel: Option<&str>,
        runtime: Arc<dyn GolishRuntime>,
    ) -> Result<Self> {
        Self::new_zai_sdk_with_context(workspace, model, api_key, base_url, source_channel, None, runtime).await
    }

    #[allow(clippy::too_many_arguments)]
    pub async fn new_zai_sdk_with_context(
        workspace: PathBuf,
        model: &str,
        api_key: &str,
        base_url: Option<&str>,
        source_channel: Option<&str>,
        context_config: Option<ContextManagerConfig>,
        runtime: Arc<dyn GolishRuntime>,
    ) -> Result<Self> {
        let shared_config = SharedComponentsConfig {
            context_config,
            settings: golish_settings::GolishSettings::default(),
        };
        Self::new_zai_sdk_with_shared_config(
            workspace,
            model,
            api_key,
            base_url,
            source_channel,
            shared_config,
            runtime,
            "",
        )
        .await
    }

    #[allow(clippy::too_many_arguments)]
    pub async fn new_zai_sdk_with_shared_config(
        workspace: PathBuf,
        model: &str,
        api_key: &str,
        base_url: Option<&str>,
        source_channel: Option<&str>,
        shared_config: SharedComponentsConfig,
        runtime: Arc<dyn GolishRuntime>,
        event_session_id: &str,
    ) -> Result<Self> {
        let config = ZaiSdkClientConfig {
            workspace,
            model,
            api_key,
            base_url,
            source_channel,
        };
        let components = create_zai_sdk_components(config, shared_config).await?;
        Ok(Self::from_components_with_runtime(
            components,
            runtime,
            event_session_id.to_string(),
        ))
    }

    pub async fn new_nvidia_with_shared_config(
        workspace: PathBuf,
        model: &str,
        api_key: &str,
        base_url: Option<&str>,
        shared_config: SharedComponentsConfig,
        runtime: Arc<dyn GolishRuntime>,
        event_session_id: &str,
    ) -> Result<Self> {
        let config = NvidiaClientConfig {
            workspace,
            model,
            api_key,
            base_url,
        };
        let components = create_nvidia_components(config, shared_config).await?;
        Ok(Self::from_components_with_runtime(
            components,
            runtime,
            event_session_id.to_string(),
        ))
    }

    /// Core constructor: builds an AgentBridge from pre-built components.
    pub(super) fn from_components_with_runtime(
        components: AgentBridgeComponents,
        runtime: Arc<dyn GolishRuntime>,
        event_session_id: String,
    ) -> Self {
        let AgentBridgeComponents {
            workspace,
            provider_name,
            model_name,
            tool_registry,
            client,
            sub_agent_registry,
            approval_recorder,
            tool_policy_manager,
            context_manager,
            loop_detector,
            openai_web_search_config,
            openai_reasoning_effort,
            model_factory,
            openrouter_provider_preferences,
        } = components;

        let coordinator = EventCoordinator::spawn(
            event_session_id.clone(),
            runtime.clone(),
            None,
        );

        Self {
            workspace,
            provider_name,
            model_name,
            tool_registry,
            client,
            event_tx: None,
            runtime: Some(runtime),
            event_session_id: Some(event_session_id),
            event_sequence: AtomicU64::new(0),
            frontend_ready: AtomicBool::new(false),
            event_buffer: RwLock::new(Vec::new()),
            sub_agent_registry,
            api_request_stats: Arc::new(ApiRequestStats::new()),
            pty_manager: None,
            current_session_id: Default::default(),
            conversation_history: Default::default(),
            indexer_state: None,
            transcript_writer: None,
            transcript_base_dir: None,
            session_manager: Default::default(),
            session_persistence_enabled: Arc::new(RwLock::new(true)),
            approval_recorder,
            pending_approvals: Default::default(),
            tool_policy_manager,
            context_manager,
            compaction_state: Arc::new(RwLock::new(CompactionState::new())),
            loop_detector,
            tool_config: ToolConfig::main_agent(),
            agent_mode: Arc::new(RwLock::new(AgentMode::default())),
            plan_manager: Arc::new(PlanManager::new()),
            sidecar_state: None,
            memory_file_path: Arc::new(RwLock::new(None)),
            settings_manager: None,
            openai_web_search_config,
            openai_reasoning_effort,
            db_pool: None,
            db_tracker: None,
            model_factory,
            openrouter_provider_preferences,
            skill_cache: Arc::new(RwLock::new(Vec::new())),
            coordinator: Some(coordinator),
            cancelled: Arc::new(AtomicBool::new(false)),
            mcp_tool_definitions: Arc::new(RwLock::new(Vec::new())),
            mcp_tool_executor: Arc::new(RwLock::new(None)),
            use_agents: Arc::new(RwLock::new(true)),
            execution_mode: Arc::new(RwLock::new(crate::execution_mode::ExecutionMode::default())),
        }
    }
}
