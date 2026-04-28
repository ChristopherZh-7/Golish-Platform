// Loop detection and protection commands.

use tauri::State;

use super::ai_session_not_initialized_error;
use crate::state::AppState;
use golish_ai::loop_detection::{LoopDetectorStats, LoopProtectionConfig};

/// Get the current loop protection configuration.
#[tauri::command]
pub async fn get_loop_protection_config(
    state: State<'_, AppState>,
    session_id: Option<String>,
) -> Result<LoopProtectionConfig, String> {
    if let Some(ref sid) = session_id {
        let bridge = state.ai_state.get_session_bridge(sid).await
            .ok_or_else(|| ai_session_not_initialized_error(sid))?;
        return Ok(bridge.get_loop_protection_config().await);
    }
    let guard = state.ai_state.get_bridge().await?;
    Ok(guard.as_ref().unwrap().get_loop_protection_config().await)
}

/// Set the loop protection configuration.
#[tauri::command]
pub async fn set_loop_protection_config(
    state: State<'_, AppState>,
    config: LoopProtectionConfig,
    session_id: Option<String>,
) -> Result<(), String> {
    if let Some(ref sid) = session_id {
        let bridge = state.ai_state.get_session_bridge(sid).await
            .ok_or_else(|| ai_session_not_initialized_error(sid))?;
        bridge.set_loop_protection_config(config).await;
        return Ok(());
    }
    let guard = state.ai_state.get_bridge().await?;
    guard.as_ref().unwrap().set_loop_protection_config(config).await;
    Ok(())
}

/// Get current loop detector statistics.
#[tauri::command]
pub async fn get_loop_detector_stats(
    state: State<'_, AppState>,
    session_id: Option<String>,
) -> Result<LoopDetectorStats, String> {
    if let Some(ref sid) = session_id {
        let bridge = state.ai_state.get_session_bridge(sid).await
            .ok_or_else(|| ai_session_not_initialized_error(sid))?;
        return Ok(bridge.get_loop_detector_stats().await);
    }
    let guard = state.ai_state.get_bridge().await?;
    Ok(guard.as_ref().unwrap().get_loop_detector_stats().await)
}

/// Check if loop detection is currently enabled.
#[tauri::command]
pub async fn is_loop_detection_enabled(
    state: State<'_, AppState>,
    session_id: Option<String>,
) -> Result<bool, String> {
    if let Some(ref sid) = session_id {
        let bridge = state.ai_state.get_session_bridge(sid).await
            .ok_or_else(|| ai_session_not_initialized_error(sid))?;
        return Ok(bridge.is_loop_detection_enabled().await);
    }
    let guard = state.ai_state.get_bridge().await?;
    Ok(guard.as_ref().unwrap().is_loop_detection_enabled().await)
}

/// Disable loop detection for the current session.
#[tauri::command]
pub async fn disable_loop_detection(
    state: State<'_, AppState>,
    session_id: Option<String>,
) -> Result<(), String> {
    if let Some(ref sid) = session_id {
        let bridge = state.ai_state.get_session_bridge(sid).await
            .ok_or_else(|| ai_session_not_initialized_error(sid))?;
        bridge.disable_loop_detection_for_session().await;
        return Ok(());
    }
    let guard = state.ai_state.get_bridge().await?;
    guard.as_ref().unwrap().disable_loop_detection_for_session().await;
    Ok(())
}

/// Re-enable loop detection.
#[tauri::command]
pub async fn enable_loop_detection(
    state: State<'_, AppState>,
    session_id: Option<String>,
) -> Result<(), String> {
    if let Some(ref sid) = session_id {
        let bridge = state.ai_state.get_session_bridge(sid).await
            .ok_or_else(|| ai_session_not_initialized_error(sid))?;
        bridge.enable_loop_detection().await;
        return Ok(());
    }
    let guard = state.ai_state.get_bridge().await?;
    guard.as_ref().unwrap().enable_loop_detection().await;
    Ok(())
}

/// Reset the loop detector (clears all tracking).
#[tauri::command]
pub async fn reset_loop_detector(
    state: State<'_, AppState>,
    session_id: Option<String>,
) -> Result<(), String> {
    if let Some(ref sid) = session_id {
        let bridge = state.ai_state.get_session_bridge(sid).await
            .ok_or_else(|| ai_session_not_initialized_error(sid))?;
        bridge.reset_loop_detector().await;
        return Ok(());
    }
    let guard = state.ai_state.get_bridge().await?;
    guard.as_ref().unwrap().reset_loop_detector().await;
    Ok(())
}
