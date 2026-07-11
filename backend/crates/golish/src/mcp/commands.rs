//! Tauri commands for MCP server management.
//!
//! These commands enable the frontend to:
//! - List configured MCP servers and their connection status
//! - View available tools from connected servers
//! - Connect/disconnect individual servers
//! - Trust project-specific MCP configurations
//!
//! The MCP manager is global (shared across all sessions) and initialized
//! in the background during app startup.

use crate::error::GolishError;
use serde::{Deserialize, Serialize};
use std::collections::{HashMap, HashSet};
use std::path::PathBuf;
use std::sync::Arc;
use tauri::State;

use crate::state::{AppState, McpManaged};

fn is_platform_node_available() -> bool {
    let env_status = golish_pentest::handlers::check_env_setup();
    env_status.nvm_installed
}

fn classify_mcp_server_source(
    name: &str,
    project_names: &HashSet<String>,
    user_names: &HashSet<String>,
    builtin_names: &HashSet<String>,
) -> &'static str {
    if project_names.contains(name) {
        "project"
    } else if user_names.contains(name) {
        "user"
    } else if builtin_names.contains(name) {
        "builtin"
    } else {
        "unknown"
    }
}

/// Information about a configured MCP server for the frontend.
#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct McpServerInfo {
    /// Server name (from config key)
    pub name: String,
    /// Transport type (stdio, http)
    pub transport: String,
    /// Whether the server is enabled in config
    pub enabled: bool,
    /// Connection status
    pub status: McpServerStatus,
    /// Number of tools available (if connected)
    pub tool_count: Option<usize>,
    /// Error message (if status is Error)
    pub error: Option<String>,
    /// Source: "user" for ~/.golish/mcp.json, "project" for <project>/.golish/mcp.json
    pub source: String,
    /// Setup status for built-in servers: "ready", "needs_build", "needs_node", or null
    pub setup_status: Option<String>,
}

/// Server connection status.
#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(rename_all = "lowercase")]
pub enum McpServerStatus {
    Connected,
    Disconnected,
    Connecting,
    Error,
}

/// Information about an MCP tool for the frontend.
#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct McpToolInfo {
    /// Full tool name (mcp__{server}__{tool})
    pub name: String,
    /// Server this tool belongs to
    pub server_name: String,
    /// Original tool name from the server
    pub tool_name: String,
    /// Tool description
    pub description: Option<String>,
}

/// List all configured MCP servers with their status.
///
/// Returns servers from both user-global (~/.golish/mcp.json) and
/// project-specific (<project>/.golish/mcp.json) configurations.
/// Live connection status is reported from the global MCP manager.
#[tauri::command]
pub async fn mcp_list_servers(
    workspace_path: Option<String>,
    state: State<'_, McpManaged>,
) -> Result<Vec<McpServerInfo>, GolishError> {
    use golish_mcp::{load_mcp_config, McpTransportType};

    // Get workspace path (from parameter or current dir as fallback)
    let workspace = match workspace_path {
        Some(p) => PathBuf::from(p),
        None => {
            // Fall back to current directory
            std::env::current_dir()
                .map_err(|e| format!("Failed to get current directory: {}", e))?
        }
    };

    // Load merged config
    let config = load_mcp_config(&workspace)?;

    // Check which servers are from builtin, user, or project config
    let builtin_names = golish_mcp::builtin_server_names();

    let user_config = dirs::home_dir()
        .map(|h| h.join(".golish/mcp.json"))
        .and_then(|p| std::fs::read_to_string(p).ok())
        .and_then(|s| serde_json::from_str::<golish_mcp::McpConfigFile>(&s).ok())
        .map(|c| {
            c.mcp_servers
                .keys()
                .cloned()
                .collect::<std::collections::HashSet<_>>()
        })
        .unwrap_or_default();

    let project_config = if golish_mcp::is_project_config_trusted(&workspace) {
        std::fs::read_to_string(workspace.join(".golish/mcp.json"))
            .ok()
            .and_then(|s| serde_json::from_str::<golish_mcp::McpConfigFile>(&s).ok())
            .map(|c| c.mcp_servers.into_keys().collect::<HashSet<_>>())
            .unwrap_or_default()
    } else {
        HashSet::new()
    };

    // Get live status from the global MCP manager (if initialized)
    let manager_guard = state.manager.read().await;
    let manager = manager_guard.as_ref();

    let mut servers = Vec::new();
    for (name, server_config) in config.mcp_servers {
        let transport = match server_config.transport() {
            McpTransportType::Stdio => "stdio",
            McpTransportType::Http => "http",
            McpTransportType::Sse => "sse",
        };

        let source =
            classify_mcp_server_source(&name, &project_config, &user_config, &builtin_names);

        // Get live connection status from the global manager
        let (status, tool_count, error) = if let Some(mgr) = manager {
            match mgr.server_status(&name).await {
                Some(golish_mcp::ServerStatus::Connected { tool_count }) => {
                    (McpServerStatus::Connected, Some(tool_count), None)
                }
                Some(golish_mcp::ServerStatus::Error(msg)) => {
                    (McpServerStatus::Error, None, Some(msg))
                }
                Some(golish_mcp::ServerStatus::Disconnected) | None => {
                    (McpServerStatus::Disconnected, None, None)
                }
            }
        } else {
            // Manager not yet initialized
            (McpServerStatus::Disconnected, None, None)
        };

        let setup_status = if source == "builtin"
            && matches!(
                status,
                McpServerStatus::Disconnected | McpServerStatus::Error
            ) {
            if let Some(ref cmd) = server_config.command {
                if cmd == "node" {
                    if !is_platform_node_available() {
                        Some("needs_node".to_string())
                    } else if !server_config.args.is_empty() {
                        let entry = &server_config.args[0];
                        let entry_path = std::path::Path::new(entry);
                        let tool_dir = entry_path.parent().and_then(|p| p.parent());
                        let needs_build = tool_dir.is_none_or(|d| {
                            !d.join("node_modules").exists() || !entry_path.exists()
                        });
                        if needs_build {
                            Some("needs_build".to_string())
                        } else {
                            None
                        }
                    } else {
                        None
                    }
                } else {
                    None
                }
            } else {
                None
            }
        } else {
            None
        };

        servers.push(McpServerInfo {
            name,
            transport: transport.to_string(),
            enabled: server_config.enabled,
            status,
            tool_count,
            error,
            source: source.to_string(),
            setup_status,
        });
    }

    Ok(servers)
}

/// List all tools from connected MCP servers.
///
/// This retrieves tools from the global MCP manager.
#[tauri::command]
pub async fn mcp_list_tools(state: State<'_, McpManaged>) -> Result<Vec<McpToolInfo>, GolishError> {
    let manager_guard = state.manager.read().await;
    let manager = manager_guard
        .as_ref()
        .ok_or_else(|| GolishError::Internal("MCP manager not initialized yet".into()))?;

    let tools = manager.list_tools().await?;

    let mut result = Vec::new();
    for tool in tools {
        let full_name = format!(
            "mcp__{}__{}",
            golish_mcp::sanitize_name(&tool.server_name),
            golish_mcp::sanitize_name(&tool.tool_name)
        );

        if let Ok((server_name, tool_name)) = golish_mcp::parse_mcp_tool_name(&full_name) {
            result.push(McpToolInfo {
                name: full_name,
                server_name,
                tool_name,
                description: tool.description.clone(),
            });
        }
    }

    Ok(result)
}

/// Check if a project's MCP configuration is trusted.
#[tauri::command]
pub async fn mcp_is_project_trusted(project_path: String) -> Result<bool, GolishError> {
    let path = PathBuf::from(project_path);
    Ok(golish_mcp::is_project_config_trusted(&path))
}

/// Mark a project's MCP configuration as trusted.
///
/// This should be called after the user explicitly approves a project's
/// MCP configuration in the UI.
#[tauri::command]
pub async fn mcp_trust_project_config(project_path: String) -> Result<(), GolishError> {
    let path = PathBuf::from(project_path);
    golish_mcp::trust_project_config(&path).map_err(GolishError::from)
}

/// Get MCP configuration for a workspace.
///
/// Returns the merged configuration from user-global and project-specific sources.
#[tauri::command]
pub async fn mcp_get_config(
    workspace_path: String,
) -> Result<HashMap<String, serde_json::Value>, GolishError> {
    use golish_mcp::load_mcp_config;

    let workspace = PathBuf::from(workspace_path);
    let config = load_mcp_config(&workspace)?;

    // Convert to JSON-serializable format
    let servers: HashMap<String, serde_json::Value> = config
        .mcp_servers
        .into_iter()
        .map(|(name, cfg)| {
            (
                name,
                serde_json::to_value(cfg).unwrap_or(serde_json::Value::Null),
            )
        })
        .collect();

    Ok(servers)
}

/// Check if MCP config exists for a workspace.
#[tauri::command]
pub async fn mcp_has_project_config(workspace_path: String) -> Result<bool, GolishError> {
    let path = PathBuf::from(workspace_path).join(".golish/mcp.json");
    Ok(path.exists())
}

/// Connect to an MCP server.
///
/// The server must be configured in the workspace config.
/// After connecting, all active agent sessions have their MCP tools refreshed.
#[tauri::command]
pub async fn mcp_connect(
    server_name: String,
    state: State<'_, AppState>,
) -> Result<(), GolishError> {
    // Get the global MCP manager
    let manager_guard = state.mcp_manager.read().await;
    let manager = manager_guard.as_ref().ok_or_else(|| {
        "MCP manager not initialized yet. Please wait for background initialization to complete."
            .to_string()
    })?;
    let manager = Arc::clone(manager);
    drop(manager_guard);

    // Connect to the server
    manager
        .connect(&server_name)
        .await
        .map_err(|e| format!("Failed to connect to MCP server '{}': {}", server_name, e))?;

    // Refresh MCP tools on all active bridges
    refresh_all_bridge_mcp_tools(&state).await;

    Ok(())
}

/// Disconnect from an MCP server.
///
/// After disconnecting, all active agent sessions have their MCP tools refreshed.
#[tauri::command]
pub async fn mcp_disconnect(
    server_name: String,
    state: State<'_, AppState>,
) -> Result<(), GolishError> {
    // Get the global MCP manager
    let manager_guard = state.mcp_manager.read().await;
    let manager = manager_guard.as_ref().ok_or_else(|| {
        "MCP manager not initialized yet. Please wait for background initialization to complete."
            .to_string()
    })?;
    let manager = Arc::clone(manager);
    drop(manager_guard);

    // Disconnect from the server
    manager.disconnect(&server_name).await.map_err(|e| {
        format!(
            "Failed to disconnect from MCP server '{}': {}",
            server_name, e
        )
    })?;

    // Refresh MCP tools on all active bridges
    refresh_all_bridge_mcp_tools(&state).await;

    Ok(())
}

/// Set up a built-in MCP server by running npm install + build in its tool directory.
#[tauri::command]
pub async fn mcp_setup_builtin(
    server_name: String,
    workspace_path: Option<String>,
) -> Result<McpSetupResult, GolishError> {
    let _ = workspace_path;

    if !is_platform_node_available() {
        return Ok(McpSetupResult {
            success: false,
            message: "Node.js is not installed. Please install it first in Settings > Environment."
                .to_string(),
        });
    }

    let tool_dir = golish_mcp::builtin_setup_directory(&server_name)
        .ok_or_else(|| format!("Unknown built-in MCP server '{}'", server_name))?;

    if !tool_dir.exists() {
        return Ok(McpSetupResult {
            success: false,
            message: format!("Tool directory not found: {}", tool_dir.display()),
        });
    }

    tracing::info!(
        "[mcp] Setting up built-in server '{}' in {}",
        server_name,
        tool_dir.display()
    );

    let npm_install = std::process::Command::new("npm")
        .arg("install")
        .current_dir(&tool_dir)
        .output()
        .map_err(|e| format!("Failed to run npm install: {}", e))?;

    if !npm_install.status.success() {
        let stderr = String::from_utf8_lossy(&npm_install.stderr);
        return Ok(McpSetupResult {
            success: false,
            message: format!("npm install failed: {}", stderr),
        });
    }

    let npm_build = std::process::Command::new("npm")
        .args(["run", "build"])
        .current_dir(&tool_dir)
        .output()
        .map_err(|e| format!("Failed to run npm run build: {}", e))?;

    if !npm_build.status.success() {
        let stderr = String::from_utf8_lossy(&npm_build.stderr);
        return Ok(McpSetupResult {
            success: false,
            message: format!("npm run build failed: {}", stderr),
        });
    }

    tracing::info!("[mcp] Built-in server '{}' setup complete", server_name);

    Ok(McpSetupResult {
        success: true,
        message: "Setup complete. Restart Golish to load this server.".to_string(),
    })
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct McpSetupResult {
    pub success: bool,
    pub message: String,
}

/// Refresh MCP tool definitions on all active agent bridges.
///
/// Called after connect/disconnect to keep all sessions in sync with the global manager.
async fn refresh_all_bridge_mcp_tools(state: &AppState) {
    let agent_state = state.extract_agent_state();
    let bridges = state.ai_state.bridges.read().await;
    for (session_id, bridge) in bridges.iter() {
        crate::ai::commands::setup_bridge_mcp_tools(bridge, &agent_state).await;
        tracing::debug!("[mcp] Refreshed MCP tools for session {}", session_id);
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::collections::HashSet;

    #[test]
    fn classify_mcp_server_source_uses_real_merge_precedence() {
        let project = HashSet::from(["shared".to_string()]);
        let user = HashSet::from(["shared".to_string(), "user-only".to_string()]);
        let builtin = HashSet::from([
            "shared".to_string(),
            "user-only".to_string(),
            "builtin-only".to_string(),
        ]);

        assert_eq!(
            classify_mcp_server_source("shared", &project, &user, &builtin),
            "project"
        );
        assert_eq!(
            classify_mcp_server_source("user-only", &project, &user, &builtin),
            "user"
        );
        assert_eq!(
            classify_mcp_server_source("builtin-only", &project, &user, &builtin),
            "builtin"
        );
    }
}
