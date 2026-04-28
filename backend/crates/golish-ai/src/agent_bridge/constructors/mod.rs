//! `AgentBridge` constructor methods, split by provider family.
//!
//! Each family submodule contributes additional `impl AgentBridge` methods
//! for its providers. Three layered constructors per provider:
//! - `new_*_with_runtime`: minimal config, defaults applied.
//! - `new_*_with_context`: adds an optional `ContextManagerConfig`.
//! - `new_*_with_shared_config`: full control via `SharedComponentsConfig`.
//!
//! All paths funnel into [`AgentBridge::from_components_with_runtime`] (the
//! core constructor that builds the struct from `AgentBridgeComponents` and
//! a runtime). The shared `new_with_runtime` dispatcher in this file routes
//! the legacy `provider`-string entry point onto the OpenRouter path.
//!
//! ## Unified entry point
//!
//! [`AgentBridge::from_provider_config`] dispatches to the correct
//! `*_with_shared_config` constructor based on the [`ProviderConfig`] variant,
//! eliminating duplicated match arms at every call site.

use std::path::PathBuf;
use std::sync::atomic::{AtomicBool, AtomicU64};
use std::sync::Arc;

use anyhow::Result;
use tokio::sync::RwLock;

use golish_context::CompactionState;
use golish_core::runtime::GolishRuntime;
use golish_core::ApiRequestStats;

use super::AgentBridge;
use crate::agent_mode::AgentMode;
use crate::event_coordinator::EventCoordinator;
use crate::llm_client::{AgentBridgeComponents, ProviderConfig, SharedComponentsConfig};
use crate::planner::PlanManager;
use crate::tool_definitions::ToolConfig;

mod anthropic_family;
mod gemini_family;
mod openai_family;
mod other_family;

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

    /// Unified constructor that dispatches to the correct provider-specific
    /// builder based on the [`ProviderConfig`] variant.
    ///
    /// This replaces the duplicated 11-arm match statements that previously
    /// existed in `init_ai_session` and `init_ai_agent_unified`.
    pub async fn from_provider_config(
        config: ProviderConfig,
        shared_config: SharedComponentsConfig,
        runtime: Arc<dyn GolishRuntime>,
        event_session_id: &str,
    ) -> Result<Self> {
        let workspace_path: PathBuf = config.workspace().into();

        match config {
            ProviderConfig::VertexAi {
                model,
                credentials_path,
                project_id,
                location,
                ..
            } => {
                Self::new_vertex_anthropic_with_shared_config(
                    workspace_path,
                    credentials_path.as_deref(),
                    &project_id,
                    &location,
                    &model,
                    shared_config,
                    runtime,
                    event_session_id,
                )
                .await
            }
            ProviderConfig::Openrouter {
                model,
                api_key,
                provider_preferences,
                ..
            } => {
                Self::new_openrouter_with_shared_config(
                    workspace_path,
                    &model,
                    &api_key,
                    provider_preferences,
                    shared_config,
                    runtime,
                    event_session_id,
                )
                .await
            }
            ProviderConfig::Openai {
                model,
                api_key,
                base_url,
                reasoning_effort,
                ..
            } => {
                Self::new_openai_with_shared_config(
                    workspace_path,
                    &model,
                    &api_key,
                    base_url.as_deref(),
                    reasoning_effort.as_deref(),
                    shared_config,
                    runtime,
                    event_session_id,
                )
                .await
            }
            ProviderConfig::Anthropic {
                model, api_key, ..
            } => {
                Self::new_anthropic_with_shared_config(
                    workspace_path,
                    &model,
                    &api_key,
                    shared_config,
                    runtime,
                    event_session_id,
                )
                .await
            }
            ProviderConfig::Ollama {
                model, base_url, ..
            } => {
                Self::new_ollama_with_shared_config(
                    workspace_path,
                    &model,
                    base_url.as_deref(),
                    shared_config,
                    runtime,
                    event_session_id,
                )
                .await
            }
            ProviderConfig::Gemini {
                model, api_key, ..
            } => {
                Self::new_gemini_with_shared_config(
                    workspace_path,
                    &model,
                    &api_key,
                    shared_config,
                    runtime,
                    event_session_id,
                )
                .await
            }
            ProviderConfig::Groq {
                model, api_key, ..
            } => {
                Self::new_groq_with_shared_config(
                    workspace_path,
                    &model,
                    &api_key,
                    shared_config,
                    runtime,
                    event_session_id,
                )
                .await
            }
            ProviderConfig::Xai {
                model, api_key, ..
            } => {
                Self::new_xai_with_shared_config(
                    workspace_path,
                    &model,
                    &api_key,
                    shared_config,
                    runtime,
                    event_session_id,
                )
                .await
            }
            ProviderConfig::ZaiSdk {
                model,
                api_key,
                base_url,
                source_channel,
                ..
            } => {
                Self::new_zai_sdk_with_shared_config(
                    workspace_path,
                    &model,
                    &api_key,
                    base_url.as_deref(),
                    source_channel.as_deref(),
                    shared_config,
                    runtime,
                    event_session_id,
                )
                .await
            }
            ProviderConfig::Nvidia {
                model,
                api_key,
                base_url,
                ..
            } => {
                Self::new_nvidia_with_shared_config(
                    workspace_path,
                    &model,
                    &api_key,
                    base_url.as_deref(),
                    shared_config,
                    runtime,
                    event_session_id,
                )
                .await
            }
            ProviderConfig::VertexGemini {
                model,
                credentials_path,
                project_id,
                location,
                include_thoughts,
                ..
            } => {
                Self::new_vertex_gemini_with_shared_config(
                    workspace_path,
                    credentials_path.as_deref(),
                    &project_id,
                    &location,
                    &model,
                    include_thoughts,
                    shared_config,
                    runtime,
                    event_session_id,
                )
                .await
            }
        }
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

        use super::{
            BridgeAccessControl, BridgeEventBus, BridgeLlmConfig, BridgeServices, BridgeSession,
        };

        Self {
            events: BridgeEventBus {
                event_tx: None,
                runtime: Some(runtime),
                event_session_id: Some(event_session_id),
                event_sequence: AtomicU64::new(0),
                frontend_ready: AtomicBool::new(false),
                event_buffer: RwLock::new(Vec::new()),
                coordinator: Some(coordinator),
                transcript_writer: None,
                transcript_base_dir: None,
            },
            llm: BridgeLlmConfig {
                client,
                provider_name,
                model_name,
                model_factory,
                openai_web_search_config,
                openai_reasoning_effort,
                openrouter_provider_preferences,
            },
            services: BridgeServices {
                db_pool: None,
                db_tracker: None,
                indexer_state: None,
                sidecar_state: None,
                settings_manager: None,
                pty_manager: None,
            },
            access: BridgeAccessControl {
                approval_recorder,
                pending_approvals: Default::default(),
                tool_policy_manager,
                agent_mode: Arc::new(RwLock::new(AgentMode::default())),
                loop_detector,
            },
            session: BridgeSession {
                conversation_history: Default::default(),
                session_manager: Default::default(),
                session_persistence_enabled: Arc::new(RwLock::new(true)),
            },
            workspace,
            tool_registry,
            tool_config: ToolConfig::main_agent(),
            cancelled: Arc::new(AtomicBool::new(false)),
            api_request_stats: Arc::new(ApiRequestStats::new()),
            sub_agent_registry,
            prompt_registry: golish_sub_agents::PromptRegistry::new(),
            use_agents: Arc::new(RwLock::new(true)),
            execution_mode: Arc::new(RwLock::new(crate::execution_mode::ExecutionMode::default())),
            context_manager,
            compaction_state: Arc::new(RwLock::new(CompactionState::new())),
            plan_manager: Arc::new(PlanManager::new()),
            current_session_id: Default::default(),
            memory_file_path: Arc::new(RwLock::new(None)),
            post_shell_hook: None,
            output_classifier: None,
            skill_cache: Arc::new(RwLock::new(Vec::new())),
            mcp_tool_definitions: Arc::new(RwLock::new(Vec::new())),
            mcp_tool_executor: Arc::new(RwLock::new(None)),
        }
    }

}
