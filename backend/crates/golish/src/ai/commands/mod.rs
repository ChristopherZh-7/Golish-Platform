// Commands module for AI agent interaction.
//
// This module provides Tauri command handlers for the AI agent system,
// organized into logical submodules for maintainability.

use std::collections::HashMap;
use std::sync::Arc;
use tokio::sync::RwLock;

use super::agent_bridge::AgentBridge;
use crate::state::AppState;
use golish_core::runtime::GolishRuntime;

pub mod agents;
pub mod analytics;
pub mod commit_writer;
pub mod config;
pub mod context;
pub mod core;
pub mod debug;
pub mod hitl;
pub mod loop_detection;
pub mod mode;
pub mod plan;
pub mod policy;
pub mod session;
pub mod summarizer;
pub mod workflow;

// Re-export all commands for easier access
pub use agents::*;
pub use analytics::*;
pub use commit_writer::*;
pub use config::*;
pub use context::*;
pub use core::*;
pub use debug::*;
pub use hitl::*;
pub use loop_detection::*;
pub use mode::*;
pub use plan::*;
pub use policy::*;
pub use session::*;
pub use summarizer::*;
pub use workflow::*;

/// Shared AI state supporting multiple per-session agents.
/// Uses tokio RwLock for async compatibility with AgentBridge methods.
///
/// IMPORTANT: Bridges are wrapped in Arc to allow cloning references without
/// holding the map lock during long-running operations like execute().
/// This enables concurrent agent execution across multiple tabs.
pub struct AiState {
    /// Map of session_id -> Arc<AgentBridge> for per-tab AI isolation.
    /// The Arc wrapper allows commands to clone the bridge reference and
    /// release the map lock before calling long-running async methods.
    pub bridges: Arc<RwLock<HashMap<String, Arc<AgentBridge>>>>,
    /// Legacy single bridge for backwards compatibility during migration.
    /// TODO: Remove once all commands use session-specific bridges.
    pub bridge: Arc<RwLock<Option<AgentBridge>>>,
    /// Runtime abstraction for event emission and approval handling.
    /// Stored here for later phases when AgentBridge will use it directly.
    /// Currently created during init but the existing event_tx path is used.
    pub runtime: Arc<RwLock<Option<Arc<dyn GolishRuntime>>>>,
}

impl Default for AiState {
    fn default() -> Self {
        Self {
            bridges: Arc::new(RwLock::new(HashMap::new())),
            bridge: Arc::new(RwLock::new(None)),
            runtime: Arc::new(RwLock::new(None)),
        }
    }
}

/// Error message for uninitialized AI agent.
pub const AI_NOT_INITIALIZED_ERROR: &str = "AI agent not initialized. Call init_ai_agent first.";

/// Error message for session without AI agent.
pub fn ai_session_not_initialized_error(session_id: &str) -> String {
    format!(
        "AI agent not initialized for session '{}'. Call init_ai_session first.",
        session_id
    )
}

impl AiState {
    pub fn new() -> Self {
        Self::default()
    }

    // ========== Session-specific bridge methods ==========

    /// Get an Arc clone of a session's bridge.
    ///
    /// This is the preferred method for accessing bridges as it allows releasing
    /// the map lock immediately. Use this for long-running operations like execute().
    pub async fn get_session_bridge(&self, session_id: &str) -> Option<Arc<AgentBridge>> {
        self.bridges.read().await.get(session_id).cloned()
    }

    /// Get a read guard to the bridges map.
    ///
    /// WARNING: Only use for short operations. For long-running async operations,
    /// use get_session_bridge() instead to avoid blocking other sessions.
    pub async fn get_bridges(
        &self,
    ) -> tokio::sync::RwLockReadGuard<'_, HashMap<String, Arc<AgentBridge>>> {
        self.bridges.read().await
    }

    /// Check if a session has an initialized AI agent.
    pub async fn has_session_bridge(&self, session_id: &str) -> bool {
        self.bridges.read().await.contains_key(session_id)
    }

    /// Insert a bridge for a session.
    ///
    /// The bridge is wrapped in Arc for concurrent access.
    pub async fn insert_session_bridge(&self, session_id: String, bridge: AgentBridge) {
        self.bridges
            .write()
            .await
            .insert(session_id, Arc::new(bridge));
    }

    /// Remove and return the bridge for a session.
    ///
    /// Returns the Arc-wrapped bridge if it existed.
    pub async fn remove_session_bridge(&self, session_id: &str) -> Option<Arc<AgentBridge>> {
        self.bridges.write().await.remove(session_id)
    }

    // ========== Legacy single bridge methods (for backwards compatibility) ==========

    /// Get a read guard to the legacy bridge, returning an error if not initialized.
    ///
    /// DEPRECATED: Use with_session_bridge instead.
    /// This helper reduces boilerplate in command handlers by providing
    /// a consistent way to access the bridge with proper error handling.
    pub async fn get_bridge(
        &self,
    ) -> Result<tokio::sync::RwLockReadGuard<'_, Option<AgentBridge>>, String> {
        let guard = self.bridge.read().await;
        if guard.is_none() {
            return Err(AI_NOT_INITIALIZED_ERROR.to_string());
        }
        Ok(guard)
    }

    /// Execute a closure with access to the legacy bridge reference.
    ///
    /// DEPRECATED: Use with_session_bridge instead.
    /// This helper eliminates the two-step pattern of `get_bridge().await?.as_ref().unwrap()`.
    /// Only use for synchronous operations. For async operations, use `get_bridge()` directly.
    pub async fn with_bridge<F, T>(&self, f: F) -> Result<T, String>
    where
        F: FnOnce(&AgentBridge) -> T,
    {
        let guard = self.bridge.read().await;
        let bridge = guard.as_ref().ok_or(AI_NOT_INITIALIZED_ERROR)?;
        Ok(f(bridge))
    }
}

/// Configure the agent bridge with shared services from AppState.
///
/// This also looks up and sets the memory file path for project instructions
/// based on the workspace path and indexed codebases in settings.
///
/// Sub-agent model overrides from settings are applied to the registry.
///
/// IMPORTANT: Each session gets its own SidecarState instance to enable
/// per-session isolation and avoid blocking between tabs when agents run concurrently.
pub async fn configure_bridge(bridge: &mut AgentBridge, state: &AppState, session_id: &str, app_handle: Option<tauri::AppHandle>) {
    let is_title_gen = session_id.starts_with("title-gen-");

    if is_title_gen {
        configure_title_gen(bridge).await;
    }

    configure_core_services(bridge, state).await;
    configure_domain_hooks(bridge);

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
    bridge.set_tool_config(golish_ai::tool_definitions::ToolConfig::with_preset(
        golish_ai::tool_definitions::ToolPreset::None,
    ));
    let mut registry = bridge.tool_registry().write().await;
    registry.clear();
    drop(registry);
    tracing::info!("[configure_bridge] Title-gen session: disabled all tools");
}

async fn configure_core_services(bridge: &mut AgentBridge, state: &AppState) {
    bridge.set_pty_manager(state.pty_manager.clone());
    bridge.set_indexer_state(state.indexer_state.clone());

    let workspace_path = bridge.workspace().read().await.clone();
    let sidecar_state = std::sync::Arc::new(golish_sidecar::SidecarState::with_config(
        state.sidecar_config.clone(),
    ));
    if let Err(e) = sidecar_state.initialize(workspace_path).await {
        tracing::warn!("Failed to initialize per-session sidecar: {}", e);
    }
    bridge.set_sidecar_state(sidecar_state);
    bridge.set_settings_manager(state.settings_manager.clone());
    bridge.set_db_pool(state.db_pool.clone(), state.db_ready.clone());
}

fn configure_domain_hooks(bridge: &mut AgentBridge) {
    bridge.set_post_shell_hook(std::sync::Arc::new(|pool, cmd, stdout, project_path| {
        Box::pin(async move {
            let _ = golish_pentest::output_store::maybe_detect_and_store(
                &pool,
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
    state: &AppState,
    settings: &golish_settings::GolishSettings,
) {
    if let Some(ref key) = settings.ai.openai.api_key {
        if !key.is_empty() {
            let base = settings.ai.openai.base_url.as_deref().unwrap_or("https://api.openai.com/v1");
            let embedder = golish_db::embeddings::HttpEmbedder::new(
                base, key, "text-embedding-3-small", 1536,
            );
            bridge.set_embedder(std::sync::Arc::new(embedder));
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

    let model_factory = golish_ai::llm_client::LlmClientFactory::new(state.settings_manager.clone());
    bridge.set_model_factory(std::sync::Arc::new(model_factory));
}

async fn configure_sub_agents(
    bridge: &AgentBridge,
    settings: &golish_settings::GolishSettings,
) {
    apply_sub_agent_model_settings(bridge, &settings.ai).await;
}

async fn register_pentest_tools(
    bridge: &AgentBridge,
    state: &AppState,
    app_handle: Option<tauri::AppHandle>,
) {
    {
        let pentest_tools = crate::tools::pentest_ai::create_pentest_ai_tools(
            state.pentest_config_manager.clone(),
            state.pty_manager.clone(),
            state.pty_output_tap.clone(),
            state.active_terminal_session.clone(),
            state.pentest_busy_sessions.clone(),
            state.ai_state.runtime.clone(),
        );
        let mut registry = bridge.tool_registry().write().await;
        for tool in pentest_tools {
            tracing::info!("[pentest-ai] Registered tool: {}", tool.name());
            registry.register_tool(tool);
        }
    }

    {
        let bridge_tools = crate::tools::pentest_bridge::create_pentest_bridge_tools(
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

async fn register_visible_pty_tool(bridge: &AgentBridge, state: &AppState) {
    let visible_cmd_tool = crate::tools::pty_interactive::VisibleRunPtyCmdTool::new(
        state.pty_manager.clone(),
        state.pty_output_tap.clone(),
        state.active_terminal_session.clone(),
    );
    let mut registry = bridge.tool_registry().write().await;
    registry.register_tool(Arc::new(visible_cmd_tool));
    tracing::info!("[configure_bridge] Registered VisibleRunPtyCmdTool for visible terminal execution");
}

/// Set up MCP tool definitions and executor on a bridge from the global MCP manager.
/// This is called during bridge configuration and also when MCP servers change.
pub(crate) async fn setup_bridge_mcp_tools(bridge: &AgentBridge, state: &AppState) {
    let manager_guard = state.mcp_manager.read().await;
    let Some(manager) = manager_guard.as_ref() else {
        tracing::debug!("[mcp] Global MCP manager not yet initialized, skipping tool setup");
        return;
    };

    let manager = Arc::clone(manager);
    drop(manager_guard); // Release the lock

    // Get all available tools from connected servers
    match manager.list_tools().await {
        Ok(tools) => {
            let tool_definitions: Vec<rig::completion::ToolDefinition> =
                tools.iter().map(|tool| tool.to_tool_definition()).collect();

            tracing::info!(
                "[mcp] Setting {} MCP tools on bridge",
                tool_definitions.len()
            );

            // Create executor closure that routes MCP tool calls through the manager.
            let manager_clone = Arc::clone(&manager);
            let executor = Arc::new(move |name: &str, args: &serde_json::Value| {
                let manager = Arc::clone(&manager_clone);
                let name = name.to_string();
                let args = args.clone();
                Box::pin(async move {
                    if !name.starts_with("mcp__") {
                        return None;
                    }
                    match manager.call_tool(&name, args).await {
                        Ok(result) => {
                            let (value, success) =
                                golish_mcp::convert_mcp_result_to_tool_result(result);
                            Some((value, success))
                        }
                        Err(e) => {
                            tracing::error!("[mcp] Tool call failed for '{}': {}", name, e);
                            let error_result = serde_json::json!({
                                "error": e.to_string(),
                            });
                            Some((error_result, false))
                        }
                    }
                })
                    as std::pin::Pin<
                        Box<
                            dyn std::future::Future<Output = Option<(serde_json::Value, bool)>>
                                + Send,
                        >,
                    >
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
    ai_settings: &crate::settings::schema::AiSettings,
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
    codebases: &[crate::settings::schema::CodebaseConfig],
) -> Option<std::path::PathBuf> {
    // Helper to expand ~ to home directory
    fn expand_home_dir(path: &str) -> std::path::PathBuf {
        if path.starts_with("~/") {
            dirs::home_dir()
                .map(|home| home.join(&path[2..]))
                .unwrap_or_else(|| std::path::PathBuf::from(path))
        } else {
            std::path::PathBuf::from(path)
        }
    }

    // Canonicalize workspace path for comparison
    let workspace_canonical = workspace_path.canonicalize().ok()?;

    // Find matching codebase
    for config in codebases {
        let codebase_path = expand_home_dir(&config.path);
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
