// Loop detection and protection commands.

use crate::error::GolishError;
use tauri::State;

use super::ai_session_not_initialized_error;
use crate::state::AgentState;
use golish_agent_kit::loop_detection::{LoopDetectorStats, LoopProtectionConfig};

/// Get the current loop protection configuration.
#[tauri::command]
pub async fn get_loop_protection_config(
    state: State<'_, AgentState>,
    session_id: String,
) -> Result<LoopProtectionConfig, GolishError> {
    let bridge = state
        .ai_state
        .get_session_bridge(&session_id)
        .await
        .ok_or_else(|| ai_session_not_initialized_error(&session_id))?;
    Ok(bridge.get_loop_protection_config().await)
}

/// Set the loop protection configuration.
#[tauri::command]
pub async fn set_loop_protection_config(
    state: State<'_, AgentState>,
    config: LoopProtectionConfig,
    session_id: String,
) -> Result<(), GolishError> {
    let bridge = state
        .ai_state
        .get_session_bridge(&session_id)
        .await
        .ok_or_else(|| ai_session_not_initialized_error(&session_id))?;
    bridge.set_loop_protection_config(config).await;
    Ok(())
}

/// Get current loop detector statistics.
#[tauri::command]
pub async fn get_loop_detector_stats(
    state: State<'_, AgentState>,
    session_id: String,
) -> Result<LoopDetectorStats, GolishError> {
    let bridge = state
        .ai_state
        .get_session_bridge(&session_id)
        .await
        .ok_or_else(|| ai_session_not_initialized_error(&session_id))?;
    Ok(bridge.get_loop_detector_stats().await)
}

/// Check if loop detection is currently enabled.
#[tauri::command]
pub async fn is_loop_detection_enabled(
    state: State<'_, AgentState>,
    session_id: String,
) -> Result<bool, GolishError> {
    let bridge = state
        .ai_state
        .get_session_bridge(&session_id)
        .await
        .ok_or_else(|| ai_session_not_initialized_error(&session_id))?;
    Ok(bridge.is_loop_detection_enabled().await)
}

/// Disable loop detection for the current session.
#[tauri::command]
pub async fn disable_loop_detection(
    state: State<'_, AgentState>,
    session_id: String,
) -> Result<(), GolishError> {
    let bridge = state
        .ai_state
        .get_session_bridge(&session_id)
        .await
        .ok_or_else(|| ai_session_not_initialized_error(&session_id))?;
    bridge.disable_loop_detection_for_session().await;
    Ok(())
}

/// Re-enable loop detection.
#[tauri::command]
pub async fn enable_loop_detection(
    state: State<'_, AgentState>,
    session_id: String,
) -> Result<(), GolishError> {
    let bridge = state
        .ai_state
        .get_session_bridge(&session_id)
        .await
        .ok_or_else(|| ai_session_not_initialized_error(&session_id))?;
    bridge.enable_loop_detection().await;
    Ok(())
}

/// Reset the loop detector (clears all tracking).
#[tauri::command]
pub async fn reset_loop_detector(
    state: State<'_, AgentState>,
    session_id: String,
) -> Result<(), GolishError> {
    let bridge = state
        .ai_state
        .get_session_bridge(&session_id)
        .await
        .ok_or_else(|| ai_session_not_initialized_error(&session_id))?;
    bridge.reset_loop_detector().await;
    Ok(())
}
