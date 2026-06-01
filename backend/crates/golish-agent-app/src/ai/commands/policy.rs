// Tool policy management commands.

use crate::error::GolishError;
use tauri::State;

use super::ai_session_not_initialized_error;
use crate::state::AgentState;
use golish_agent_kit::tool_policy::{ToolPolicy, ToolPolicyConfig};

/// Get the current tool policy configuration.
#[tauri::command]
pub async fn get_tool_policy_config(
    state: State<'_, AgentState>,
    session_id: String,
) -> Result<ToolPolicyConfig, GolishError> {
    let bridge = state
        .ai_state
        .get_session_bridge(&session_id)
        .await
        .ok_or_else(|| ai_session_not_initialized_error(&session_id))?;
    Ok(bridge.get_tool_policy_config().await)
}

/// Update the tool policy configuration.
#[tauri::command]
pub async fn set_tool_policy_config(
    state: State<'_, AgentState>,
    config: ToolPolicyConfig,
    session_id: String,
) -> Result<(), GolishError> {
    let bridge = state
        .ai_state
        .get_session_bridge(&session_id)
        .await
        .ok_or_else(|| ai_session_not_initialized_error(&session_id))?;
    bridge
        .set_tool_policy_config(config)
        .await
        .map_err(GolishError::from)
}

/// Get the policy for a specific tool.
#[tauri::command]
pub async fn get_tool_policy(
    state: State<'_, AgentState>,
    tool_name: String,
    session_id: String,
) -> Result<ToolPolicy, GolishError> {
    let bridge = state
        .ai_state
        .get_session_bridge(&session_id)
        .await
        .ok_or_else(|| ai_session_not_initialized_error(&session_id))?;
    Ok(bridge.get_tool_policy(&tool_name).await)
}

/// Set the policy for a specific tool.
#[tauri::command]
pub async fn set_tool_policy(
    state: State<'_, AgentState>,
    tool_name: String,
    policy: ToolPolicy,
    session_id: String,
) -> Result<(), GolishError> {
    let bridge = state
        .ai_state
        .get_session_bridge(&session_id)
        .await
        .ok_or_else(|| ai_session_not_initialized_error(&session_id))?;
    bridge
        .set_tool_policy(&tool_name, policy)
        .await
        .map_err(GolishError::from)
}

/// Reset tool policies to defaults.
#[tauri::command]
pub async fn reset_tool_policies(
    state: State<'_, AgentState>,
    session_id: String,
) -> Result<(), GolishError> {
    let bridge = state
        .ai_state
        .get_session_bridge(&session_id)
        .await
        .ok_or_else(|| ai_session_not_initialized_error(&session_id))?;
    bridge
        .reset_tool_policies()
        .await
        .map_err(GolishError::from)
}

/// Enable full-auto mode for tool execution.
#[tauri::command]
pub async fn enable_full_auto_mode(
    state: State<'_, AgentState>,
    allowed_tools: Vec<String>,
    session_id: String,
) -> Result<(), GolishError> {
    let bridge = state
        .ai_state
        .get_session_bridge(&session_id)
        .await
        .ok_or_else(|| ai_session_not_initialized_error(&session_id))?;
    bridge.enable_full_auto_mode(allowed_tools).await;
    Ok(())
}

/// Disable full-auto mode for tool execution.
#[tauri::command]
pub async fn disable_full_auto_mode(
    state: State<'_, AgentState>,
    session_id: String,
) -> Result<(), GolishError> {
    let bridge = state
        .ai_state
        .get_session_bridge(&session_id)
        .await
        .ok_or_else(|| ai_session_not_initialized_error(&session_id))?;
    bridge.disable_full_auto_mode().await;
    Ok(())
}

/// Check if full-auto mode is enabled.
#[tauri::command]
pub async fn is_full_auto_mode_enabled(
    state: State<'_, AgentState>,
    session_id: String,
) -> Result<bool, GolishError> {
    let bridge = state
        .ai_state
        .get_session_bridge(&session_id)
        .await
        .ok_or_else(|| ai_session_not_initialized_error(&session_id))?;
    Ok(bridge.is_full_auto_mode_enabled().await)
}
