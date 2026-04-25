//! AI tool / sub-agent commands.


use tauri::State;

use crate::state::AppState;


/// Send a prompt to the AI agent and receive streaming response via events.
/// This is the legacy command - prefer send_ai_prompt_session for new code.
///
/// # Arguments
/// * `prompt` - The user's message
#[tauri::command]
pub async fn send_ai_prompt(state: State<'_, AppState>, prompt: String) -> Result<String, String> {
    let bridge_guard = state.ai_state.get_bridge().await?;
    let bridge = bridge_guard.as_ref().unwrap();

    bridge.execute(&prompt).await.map_err(|e| e.to_string())
}

/// Execute a specific tool with the given arguments.
#[tauri::command]
pub async fn execute_ai_tool(
    state: State<'_, AppState>,
    tool_name: String,
    args: serde_json::Value,
) -> Result<serde_json::Value, String> {
    let bridge_guard = state.ai_state.get_bridge().await?;
    let bridge = bridge_guard.as_ref().unwrap();

    bridge
        .execute_tool(&tool_name, args)
        .await
        .map_err(|e| e.to_string())
}

/// Get the list of available tools.
#[tauri::command]
pub async fn get_available_tools(
    state: State<'_, AppState>,
) -> Result<Vec<serde_json::Value>, String> {
    let bridge_guard = state.ai_state.get_bridge().await?;
    let bridge = bridge_guard.as_ref().unwrap();
    Ok(bridge.available_tools().await)
}

/// Sub-agent information for the frontend.
#[derive(serde::Serialize)]
pub struct SubAgentInfo {
    pub id: String,
    pub name: String,
    pub description: String,
    /// Model override if set: (provider, model)
    pub model_override: Option<(String, String)>,
}

/// Get the list of available sub-agents.
#[tauri::command]
pub async fn list_sub_agents(state: State<'_, AppState>) -> Result<Vec<SubAgentInfo>, String> {
    let bridge_guard = state.ai_state.get_bridge().await?;
    let bridge = bridge_guard.as_ref().unwrap();
    let registry = bridge.sub_agent_registry().read().await;

    Ok(registry
        .all()
        .map(|agent| SubAgentInfo {
            id: agent.id.clone(),
            name: agent.name.clone(),
            description: agent.description.clone(),
            model_override: agent.model_override.clone(),
        })
        .collect())
}

/// Shutdown the AI agent and cleanup resources.
#[tauri::command]
pub async fn shutdown_ai_agent(state: State<'_, AppState>) -> Result<(), String> {
    let mut bridge_guard = state.ai_state.bridge.write().await;
    *bridge_guard = None;
    tracing::info!("AI agent shut down");
    Ok(())
}

/// Check if the AI agent is initialized.
#[tauri::command]
pub async fn is_ai_initialized(state: State<'_, AppState>) -> Result<bool, String> {
    Ok(state.ai_state.bridge.read().await.is_some())
}
