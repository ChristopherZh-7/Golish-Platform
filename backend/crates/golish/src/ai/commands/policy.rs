// Tool policy management commands.

use tauri::State;

use super::ai_session_not_initialized_error;
use crate::state::AppState;
use golish_ai::tool_policy::{ToolPolicy, ToolPolicyConfig};

/// Get the current tool policy configuration.
#[tauri::command]
pub async fn get_tool_policy_config(
    state: State<'_, AppState>,
    session_id: Option<String>,
) -> Result<ToolPolicyConfig, String> {
    if let Some(ref sid) = session_id {
        let bridge = state.ai_state.get_session_bridge(sid).await
            .ok_or_else(|| ai_session_not_initialized_error(sid))?;
        return Ok(bridge.get_tool_policy_config().await);
    }
    let guard = state.ai_state.get_bridge().await?;
    Ok(guard.as_ref().unwrap().get_tool_policy_config().await)
}

/// Update the tool policy configuration.
#[tauri::command]
pub async fn set_tool_policy_config(
    state: State<'_, AppState>,
    config: ToolPolicyConfig,
    session_id: Option<String>,
) -> Result<(), String> {
    if let Some(ref sid) = session_id {
        let bridge = state.ai_state.get_session_bridge(sid).await
            .ok_or_else(|| ai_session_not_initialized_error(sid))?;
        return bridge.set_tool_policy_config(config).await.map_err(|e| e.to_string());
    }
    let guard = state.ai_state.get_bridge().await?;
    guard.as_ref().unwrap().set_tool_policy_config(config).await.map_err(|e| e.to_string())
}

/// Get the policy for a specific tool.
#[tauri::command]
pub async fn get_tool_policy(
    state: State<'_, AppState>,
    tool_name: String,
    session_id: Option<String>,
) -> Result<ToolPolicy, String> {
    if let Some(ref sid) = session_id {
        let bridge = state.ai_state.get_session_bridge(sid).await
            .ok_or_else(|| ai_session_not_initialized_error(sid))?;
        return Ok(bridge.get_tool_policy(&tool_name).await);
    }
    let guard = state.ai_state.get_bridge().await?;
    Ok(guard.as_ref().unwrap().get_tool_policy(&tool_name).await)
}

/// Set the policy for a specific tool.
#[tauri::command]
pub async fn set_tool_policy(
    state: State<'_, AppState>,
    tool_name: String,
    policy: ToolPolicy,
    session_id: Option<String>,
) -> Result<(), String> {
    if let Some(ref sid) = session_id {
        let bridge = state.ai_state.get_session_bridge(sid).await
            .ok_or_else(|| ai_session_not_initialized_error(sid))?;
        return bridge.set_tool_policy(&tool_name, policy).await.map_err(|e| e.to_string());
    }
    let guard = state.ai_state.get_bridge().await?;
    guard.as_ref().unwrap().set_tool_policy(&tool_name, policy).await.map_err(|e| e.to_string())
}

/// Reset tool policies to defaults.
#[tauri::command]
pub async fn reset_tool_policies(
    state: State<'_, AppState>,
    session_id: Option<String>,
) -> Result<(), String> {
    if let Some(ref sid) = session_id {
        let bridge = state.ai_state.get_session_bridge(sid).await
            .ok_or_else(|| ai_session_not_initialized_error(sid))?;
        return bridge.reset_tool_policies().await.map_err(|e| e.to_string());
    }
    let guard = state.ai_state.get_bridge().await?;
    guard.as_ref().unwrap().reset_tool_policies().await.map_err(|e| e.to_string())
}

/// Enable full-auto mode for tool execution.
#[tauri::command]
pub async fn enable_full_auto_mode(
    state: State<'_, AppState>,
    allowed_tools: Vec<String>,
    session_id: Option<String>,
) -> Result<(), String> {
    if let Some(ref sid) = session_id {
        let bridge = state.ai_state.get_session_bridge(sid).await
            .ok_or_else(|| ai_session_not_initialized_error(sid))?;
        bridge.enable_full_auto_mode(allowed_tools).await;
        return Ok(());
    }
    let guard = state.ai_state.get_bridge().await?;
    guard.as_ref().unwrap().enable_full_auto_mode(allowed_tools).await;
    Ok(())
}

/// Disable full-auto mode for tool execution.
#[tauri::command]
pub async fn disable_full_auto_mode(
    state: State<'_, AppState>,
    session_id: Option<String>,
) -> Result<(), String> {
    if let Some(ref sid) = session_id {
        let bridge = state.ai_state.get_session_bridge(sid).await
            .ok_or_else(|| ai_session_not_initialized_error(sid))?;
        bridge.disable_full_auto_mode().await;
        return Ok(());
    }
    let guard = state.ai_state.get_bridge().await?;
    guard.as_ref().unwrap().disable_full_auto_mode().await;
    Ok(())
}

/// Check if full-auto mode is enabled.
#[tauri::command]
pub async fn is_full_auto_mode_enabled(
    state: State<'_, AppState>,
    session_id: Option<String>,
) -> Result<bool, String> {
    if let Some(ref sid) = session_id {
        let bridge = state.ai_state.get_session_bridge(sid).await
            .ok_or_else(|| ai_session_not_initialized_error(sid))?;
        return Ok(bridge.is_full_auto_mode_enabled().await);
    }
    let guard = state.ai_state.get_bridge().await?;
    Ok(guard.as_ref().unwrap().is_full_auto_mode_enabled().await)
}
