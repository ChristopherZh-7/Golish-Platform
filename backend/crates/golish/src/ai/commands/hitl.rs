// HITL (Human-in-the-Loop) approval commands.

use tauri::State;

use super::ai_session_not_initialized_error;
use crate::state::AppState;
use golish_ai::hitl::{ApprovalPattern, ToolApprovalConfig};
use golish_core::hitl::ApprovalDecision;

/// Get approval patterns for all tools.
#[tauri::command]
pub async fn get_approval_patterns(
    state: State<'_, AppState>,
    session_id: Option<String>,
) -> Result<Vec<ApprovalPattern>, String> {
    if let Some(ref sid) = session_id {
        let bridge = state.ai_state.get_session_bridge(sid).await
            .ok_or_else(|| ai_session_not_initialized_error(sid))?;
        return Ok(bridge.get_approval_patterns().await);
    }
    let bridge_guard = state.ai_state.get_bridge().await?;
    let bridge = bridge_guard.as_ref().unwrap();
    Ok(bridge.get_approval_patterns().await)
}

/// Get the approval pattern for a specific tool.
#[tauri::command]
pub async fn get_tool_approval_pattern(
    state: State<'_, AppState>,
    tool_name: String,
    session_id: Option<String>,
) -> Result<Option<ApprovalPattern>, String> {
    if let Some(ref sid) = session_id {
        let bridge = state.ai_state.get_session_bridge(sid).await
            .ok_or_else(|| ai_session_not_initialized_error(sid))?;
        return Ok(bridge.get_tool_approval_pattern(&tool_name).await);
    }
    let bridge_guard = state.ai_state.get_bridge().await?;
    let bridge = bridge_guard.as_ref().unwrap();
    Ok(bridge.get_tool_approval_pattern(&tool_name).await)
}

/// Get the HITL configuration.
#[tauri::command]
pub async fn get_hitl_config(
    state: State<'_, AppState>,
    session_id: Option<String>,
) -> Result<ToolApprovalConfig, String> {
    if let Some(ref sid) = session_id {
        let bridge = state.ai_state.get_session_bridge(sid).await
            .ok_or_else(|| ai_session_not_initialized_error(sid))?;
        return Ok(bridge.get_hitl_config().await);
    }
    let bridge_guard = state.ai_state.get_bridge().await?;
    let bridge = bridge_guard.as_ref().unwrap();
    Ok(bridge.get_hitl_config().await)
}

/// Update the HITL configuration.
#[tauri::command]
pub async fn set_hitl_config(
    state: State<'_, AppState>,
    config: ToolApprovalConfig,
    session_id: Option<String>,
) -> Result<(), String> {
    if let Some(ref sid) = session_id {
        let bridge = state.ai_state.get_session_bridge(sid).await
            .ok_or_else(|| ai_session_not_initialized_error(sid))?;
        return bridge.set_hitl_config(config).await.map_err(|e| e.to_string());
    }
    let bridge_guard = state.ai_state.get_bridge().await?;
    let bridge = bridge_guard.as_ref().unwrap();
    bridge.set_hitl_config(config).await.map_err(|e| e.to_string())
}

/// Add a tool to the always-allow list.
#[tauri::command]
pub async fn add_tool_always_allow(
    state: State<'_, AppState>,
    tool_name: String,
    session_id: Option<String>,
) -> Result<(), String> {
    if let Some(ref sid) = session_id {
        let bridge = state.ai_state.get_session_bridge(sid).await
            .ok_or_else(|| ai_session_not_initialized_error(sid))?;
        return bridge.add_tool_always_allow(&tool_name).await.map_err(|e| e.to_string());
    }
    let bridge_guard = state.ai_state.get_bridge().await?;
    let bridge = bridge_guard.as_ref().unwrap();
    bridge.add_tool_always_allow(&tool_name).await.map_err(|e| e.to_string())
}

/// Remove a tool from the always-allow list.
#[tauri::command]
pub async fn remove_tool_always_allow(
    state: State<'_, AppState>,
    tool_name: String,
    session_id: Option<String>,
) -> Result<(), String> {
    if let Some(ref sid) = session_id {
        let bridge = state.ai_state.get_session_bridge(sid).await
            .ok_or_else(|| ai_session_not_initialized_error(sid))?;
        return bridge.remove_tool_always_allow(&tool_name).await.map_err(|e| e.to_string());
    }
    let bridge_guard = state.ai_state.get_bridge().await?;
    let bridge = bridge_guard.as_ref().unwrap();
    bridge.remove_tool_always_allow(&tool_name).await.map_err(|e| e.to_string())
}

/// Reset all approval patterns (does not reset configuration).
#[tauri::command]
pub async fn reset_approval_patterns(
    state: State<'_, AppState>,
    session_id: Option<String>,
) -> Result<(), String> {
    if let Some(ref sid) = session_id {
        let bridge = state.ai_state.get_session_bridge(sid).await
            .ok_or_else(|| ai_session_not_initialized_error(sid))?;
        return bridge.reset_approval_patterns().await.map_err(|e| e.to_string());
    }
    let bridge_guard = state.ai_state.get_bridge().await?;
    let bridge = bridge_guard.as_ref().unwrap();
    bridge.reset_approval_patterns().await.map_err(|e| e.to_string())
}

/// Respond to a tool approval request.
#[tauri::command]
pub async fn respond_to_tool_approval(
    state: State<'_, AppState>,
    session_id: String,
    decision: ApprovalDecision,
) -> Result<(), String> {
    let bridge = state
        .ai_state
        .get_session_bridge(&session_id)
        .await
        .ok_or_else(|| ai_session_not_initialized_error(&session_id))?;

    bridge
        .respond_to_approval(decision)
        .await
        .map_err(|e| e.to_string())
}
