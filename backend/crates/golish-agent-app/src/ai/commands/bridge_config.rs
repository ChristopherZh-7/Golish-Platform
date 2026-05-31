//! Agent-bridge wiring: assembles shared services (sidecar, db, graph,
//! memory, sub-agents, pentest/MCP tools) onto a per-session [`AgentBridge`].

use std::sync::Arc;

use super::super::agent_bridge::AgentBridge;
use crate::state::AgentState;

/// Configure the agent bridge with shared services from AgentState.
///
/// This also looks up and sets the memory file path for project instructions
/// based on the workspace path and indexed codebases in settings.
///
/// Sub-agent model overrides from settings are applied to the registry.
///
/// IMPORTANT: Each session gets its own SidecarState instance to enable
/// per-session isolation and avoid blocking between tabs when agents run concurrently.
pub async fn configure_bridge(
    bridge: &mut AgentBridge,
    state: &AgentState,
    session_id: &str,
    app_handle: Option<tauri::AppHandle>,
) {
    let is_title_gen = golish_core::is_title_gen_session_id(session_id);

    if is_title_gen {
        configure_title_gen(bridge).await;
    }

    configure_core_services(bridge, state).await;
    configure_domain_hooks(bridge, state);

    let settings = state.settings_manager.get().await;
    configure_memory_and_embeddings(bridge, state, &settings).await;
    configure_sub_agents(bridge, &settings).await;

    if !is_title_gen {
        setup_bridge_mcp_tools(bridge, state).await;
        register_pentest_tools(bridge, state, app_handle).await;
        register_visible_pty_tool(bridge, state).await;
    }
}

async fn configure_title_gen(bridge: &mut AgentBridge) {
    bridge.set_tool_config(
        golish_agent_kit::tool_definitions::ToolSelectionConfig::with_preset(
            golish_agent_kit::tool_definitions::ToolPreset::None,
        ),
    );
    let mut registry = bridge.tool_registry().write().await;
    registry.clear();
    drop(registry);
    tracing::info!("[configure_bridge] Title-gen session: disabled all tools");
}

async fn configure_core_services(bridge: &mut AgentBridge, state: &AgentState) {
    let workspace_path = bridge.workspace().read().await.clone();
    let sidecar_state = std::sync::Arc::new(golish_sidecar::SidecarState::with_config(
        state.sidecar_config.clone(),
    ));
    if let Err(e) = sidecar_state.initialize(workspace_path).await {
        tracing::warn!("Failed to initialize per-session sidecar: {}", e);
    }
    let sidecar_backend: std::sync::Arc<
        dyn golish_agent_kit::sidecar_trait::SessionCaptureBackend,
    > = std::sync::Arc::new(crate::ai::sidecar_bridge::SidecarCaptureBackend::new(
        sidecar_state,
    ));

    // db tracking + readiness + chain persistence travel together and use
    // a generic readiness-gate bound that can't go through `BridgeBackends`,
    // so call `set_db_backend` directly first — `db_repo` / `embedder`
    // applied via `apply_backends` below need the live tracker to exist.
    let tracking_backend: std::sync::Arc<dyn golish_agent_kit::db_traits::DbTrackingBackend> =
        std::sync::Arc::new(crate::ai::tracking_bridge::PgTrackingBackend::new(
            state.db_pool.clone(),
        ));
    let chain_persistence: std::sync::Arc<dyn golish_sub_agents::SubAgentChainPersistence> =
        std::sync::Arc::new(crate::ai::tracking_bridge::PgChainPersistence::new(
            state.db_pool.clone(),
        ));
    let ready_gate = crate::ai::tracking_bridge::CoreDbReadyGate(state.db_ready.clone());
    bridge.set_db_backend(tracking_backend, ready_gate, chain_persistence);

    let graph_backend = std::sync::Arc::new(crate::ai::graph_bridge::GraphClientBackend::new(
        state.db_pool.clone(),
    ));
    let db_repo: std::sync::Arc<dyn golish_agent_kit::db_traits::DbRepoProvider> =
        std::sync::Arc::new(crate::ai::db_bridge::GolishDbRepoProvider::new(
            state.db_pool.clone(),
        ));

    bridge.apply_backends(golish_agent_bridge::BridgeBackends {
        indexer: Some(state.indexer_state.clone()),
        sidecar: Some(sidecar_backend),
        settings: Some(state.settings_manager.clone()),
        graph: Some(graph_backend),
        db_repo: Some(db_repo),
        ..Default::default()
    });
}

fn configure_domain_hooks(bridge: &mut AgentBridge, state: &AgentState) {
    let pool = state.db_pool.clone();
    bridge.set_post_shell_hook(std::sync::Arc::new(move |cmd, stdout, project_path| {
        let pool = pool.clone();
        Box::pin(async move {
            let store = golish_pentest::output_store::PgPentestStore::new(&pool);
            let _ = golish_pentest::output_store::maybe_detect_and_store_via(
                &store,
                &cmd,
                &stdout,
                project_path.as_deref(),
            )
            .await;
        })
    }));
    bridge.set_output_classifier(std::sync::Arc::new(|cmd, stdout| {
        golish_pentest::output_store::has_structured_storage(cmd, stdout)
    }));
}

async fn configure_memory_and_embeddings(
    bridge: &mut AgentBridge,
    state: &AgentState,
    settings: &golish_settings::GolishSettings,
) {
    if let Some(ref key) = settings.ai.openai.api_key {
        if !key.is_empty() {
            let base = settings
                .ai
                .openai
                .base_url
                .as_deref()
                .unwrap_or("https://api.openai.com/v1");
            let embedder =
                golish_db::embeddings::HttpEmbedder::new(base, key, "text-embedding-3-small", 1536);
            let bridged = crate::ai::embedder_bridge::EmbedderBridge::new(embedder);
            bridge.set_embedder(std::sync::Arc::new(bridged));
            tracing::info!("[agent] Semantic memory enabled (text-embedding-3-small)");
        }
    }

    let workspace_path = bridge.workspace().read().await.clone();
    let memory_file_path = find_memory_file_for_workspace(&workspace_path, &settings.codebases);
    if let Some(ref path) = memory_file_path {
        tracing::info!(
            "[agent] Using memory file from codebase settings: {}",
            path.display()
        );
    }
    bridge.set_memory_file_path(memory_file_path).await;

    let model_factory =
        golish_agent_kit::llm_client::LlmClientFactory::new(state.settings_manager.clone());
    bridge.set_model_factory(std::sync::Arc::new(model_factory));
}

async fn configure_sub_agents(bridge: &AgentBridge, settings: &golish_settings::GolishSettings) {
    apply_sub_agent_model_settings(bridge, &settings.ai).await;
}

async fn register_pentest_tools(
    bridge: &AgentBridge,
    state: &AgentState,
    app_handle: Option<tauri::AppHandle>,
) {
    {
        let pentest_tools = golish_pentest_app::pentest_ai::create_pentest_ai_tools(
            state.pentest_config_manager.clone(),
            state.pty_manager.clone(),
            state.pty_output_tap.clone(),
            state.active_terminal_session.clone(),
            state.pentest_busy_sessions.clone(),
            state.ai_state.runtime.clone(),
            state.db_pool.clone(),
        );
        let mut registry = bridge.tool_registry().write().await;
        for tool in pentest_tools {
            tracing::info!("[pentest-ai] Registered tool: {}", tool.name());
            registry.register_tool(tool);
        }
    }

    {
        let bridge_tools = golish_pentest_app::pentest_bridge::create_pentest_bridge_tools(
            state.db_pool.clone(),
            state.pentest_config_manager.clone(),
            app_handle,
        );
        let mut registry = bridge.tool_registry().write().await;
        for tool in bridge_tools {
            tracing::info!("[pentest-bridge] Registered tool: {}", tool.name());
            registry.register_tool(tool);
        }
    }
}

async fn register_visible_pty_tool(bridge: &AgentBridge, state: &AgentState) {
    let visible_cmd_tool = golish_app_core::pty_interactive::VisibleRunPtyCmdTool::new(
        state.pty_manager.clone(),
        state.pty_output_tap.clone(),
        state.active_terminal_session.clone(),
    );
    let mut registry = bridge.tool_registry().write().await;
    registry.register_tool(Arc::new(visible_cmd_tool));
    tracing::info!(
        "[configure_bridge] Registered VisibleRunPtyCmdTool for visible terminal execution"
    );
}

/// MCP tool executor that routes tool calls through the MCP manager.
///
/// Handles tools with the `mcp__` prefix; returns `None` for all others.
pub struct McpManagerToolExecutor {
    manager: Arc<golish_mcp::McpManager>,
}

impl McpManagerToolExecutor {
    pub fn new(manager: Arc<golish_mcp::McpManager>) -> Self {
        Self { manager }
    }
}

#[async_trait::async_trait]
impl golish_agent_runtime::agentic_loop::McpToolExecutor for McpManagerToolExecutor {
    async fn execute_tool(
        &self,
        tool_name: &str,
        args: &serde_json::Value,
    ) -> Option<(serde_json::Value, bool)> {
        if !tool_name.starts_with("mcp__") {
            return None;
        }
        match self.manager.call_tool(tool_name, args.clone()).await {
            Ok(result) => {
                let (value, success) = golish_mcp::convert_mcp_result_to_tool_result(result);
                Some((value, success))
            }
            Err(e) => {
                tracing::error!("[mcp] Tool call failed for '{}': {}", tool_name, e);
                Some((serde_json::json!({"error": e.to_string()}), false))
            }
        }
    }
}

/// Set up MCP tool definitions and executor on a bridge from the global MCP manager.
/// This is called during bridge configuration and also when MCP servers change.
pub async fn setup_bridge_mcp_tools(bridge: &AgentBridge, state: &AgentState) {
    let manager_guard = state.mcp_manager.read().await;
    let Some(manager) = manager_guard.as_ref() else {
        tracing::debug!("[mcp] Global MCP manager not yet initialized, skipping tool setup");
        return;
    };

    let manager = Arc::clone(manager);
    drop(manager_guard);

    match manager.list_tools().await {
        Ok(tools) => {
            let tool_definitions: Vec<rig::completion::ToolDefinition> =
                tools.iter().map(|tool| tool.to_tool_definition()).collect();

            tracing::info!(
                "[mcp] Setting {} MCP tools on bridge",
                tool_definitions.len()
            );

            let executor = Arc::new(McpManagerToolExecutor {
                manager: Arc::clone(&manager),
            });

            bridge.set_mcp_tools(tool_definitions).await;
            bridge.set_mcp_executor(executor).await;
        }
        Err(e) => {
            tracing::warn!("[mcp] Failed to list MCP tools: {}", e);
        }
    }
}

/// Apply sub-agent model overrides from settings to the registry.
async fn apply_sub_agent_model_settings(
    bridge: &AgentBridge,
    ai_settings: &golish_settings::schema::AiSettings,
) {
    let mut registry = bridge.sub_agent_registry().write().await;

    for (agent_id, config) in &ai_settings.sub_agent_models {
        if let Some(agent) = registry.get_mut(agent_id) {
            if let (Some(provider), Some(model)) = (&config.provider, &config.model) {
                let provider_str = provider.to_string();
                agent.set_model_override(&provider_str, model);
                tracing::info!(
                    "Sub-agent '{}' configured to use {}/{}",
                    agent_id,
                    provider_str,
                    model
                );
            }
            agent.temperature = config.temperature;
            agent.max_tokens = config.max_tokens;
            agent.top_p = config.top_p;
        } else {
            tracing::warn!(
                "Sub-agent model config for '{}' ignored: agent not found in registry",
                agent_id
            );
        }
    }
}

/// Find the memory file path for a workspace by matching against indexed codebases.
pub(crate) fn find_memory_file_for_workspace(
    workspace_path: &std::path::Path,
    codebases: &[golish_settings::schema::CodebaseConfig],
) -> Option<std::path::PathBuf> {
    // Canonicalize workspace path for comparison
    let workspace_canonical = workspace_path.canonicalize().ok()?;

    // Find matching codebase
    for config in codebases {
        let codebase_path = golish_core::paths::expand_tilde(&config.path);
        if let Ok(codebase_canonical) = codebase_path.canonicalize() {
            // Check if workspace is the codebase or a subdirectory
            if workspace_canonical == codebase_canonical
                || workspace_canonical.starts_with(&codebase_canonical)
            {
                // Found matching codebase
                if let Some(ref memory_file) = config.memory_file {
                    // Return just the filename - it will be resolved relative to workspace
                    return Some(std::path::PathBuf::from(memory_file));
                }
                // Codebase found but no memory file configured
                return None;
            }
        }
    }

    // No matching codebase found
    None
}
