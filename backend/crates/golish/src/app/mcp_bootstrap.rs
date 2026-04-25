//! Background initialisation of MCP (Model Context Protocol) servers.
//!
//! The body is a lift-and-shift of the MCP-init spawn that used to live
//! inline in `run_gui::setup`.

use std::sync::Arc;

use tauri::{async_runtime, AppHandle, Emitter, Manager};

use crate::app::workspace::resolve_workspace_path;
use crate::state::AppState;

/// Spawn the background MCP initialization task. Emits `mcp-event` payloads
/// to the frontend (`initializing` / `error` / `ready`) and refreshes MCP
/// tools on any agent bridges that were created before init finished.
pub(crate) fn spawn_mcp_initialization(
    mcp_manager_slot: Arc<tokio::sync::RwLock<Option<Arc<golish_mcp::McpManager>>>>,
    app_handle: AppHandle,
) {
    async_runtime::spawn(async move {
        let workspace = resolve_workspace_path();

        tracing::info!(
            "[mcp] Starting background MCP initialization for workspace: {:?}",
            workspace
        );

        let _ = app_handle.emit(
            "mcp-event",
            serde_json::json!({
                "type": "initializing",
                "message": "Connecting to MCP servers..."
            }),
        );

        let config = match golish_mcp::load_mcp_config(&workspace) {
            Ok(c) => c,
            Err(e) => {
                tracing::warn!("[mcp] Failed to load MCP config: {}", e);
                let _ = app_handle.emit(
                    "mcp-event",
                    serde_json::json!({
                        "type": "error",
                        "message": format!("Failed to load MCP config: {}", e)
                    }),
                );
                return;
            }
        };

        if config.mcp_servers.is_empty() {
            tracing::debug!("[mcp] No MCP servers configured, skipping initialization");
            // Store empty manager so commands don't get "not initialized" errors.
            let manager = Arc::new(golish_mcp::McpManager::new(
                std::collections::HashMap::new(),
            ));
            *mcp_manager_slot.write().await = Some(manager);
            let _ = app_handle.emit(
                "mcp-event",
                serde_json::json!({
                    "type": "ready",
                    "message": "No MCP servers configured",
                    "serverCount": 0,
                    "toolCount": 0
                }),
            );
            return;
        }

        let server_count = config.mcp_servers.len();
        let manager = Arc::new(golish_mcp::McpManager::new(config.mcp_servers));

        // Connect to all enabled servers (this is the slow part).
        if let Err(e) = manager.connect_all().await {
            tracing::warn!("[mcp] Some MCP servers failed to connect: {}", e);
            // Non-fatal: continue with whatever connected.
        }

        let tool_count = manager
            .list_tools()
            .await
            .map(|tools| tools.len())
            .unwrap_or(0);

        *mcp_manager_slot.write().await = Some(Arc::clone(&manager));

        tracing::info!(
            "[mcp] Background MCP initialization complete: {} servers, {} tools",
            server_count,
            tool_count
        );

        let _ = app_handle.emit(
            "mcp-event",
            serde_json::json!({
                "type": "ready",
                "message": format!("MCP ready: {} tools from {} servers", tool_count, server_count),
                "serverCount": server_count,
                "toolCount": tool_count
            }),
        );

        // Refresh MCP tools on any bridges that were created before MCP
        // finished loading (e.g. a session initialised during startup).
        let app_state = app_handle.state::<AppState>();
        let bridges = app_state.ai_state.bridges.read().await;
        for (session_id, bridge) in bridges.iter() {
            crate::ai::commands::setup_bridge_mcp_tools(bridge, &app_state).await;
            tracing::debug!(
                "[mcp] Refreshed MCP tools for session {} after background init",
                session_id
            );
        }
    });
}
