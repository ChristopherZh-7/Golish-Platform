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
use crate::llm_client::AgentBridgeComponents;
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
            prompt_registry: golish_sub_agents::PromptRegistry::new(),
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
