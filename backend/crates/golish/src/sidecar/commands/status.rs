//! Status, initialization, and shutdown commands for the sidecar itself.

use crate::state::AppState;
use tauri::State;

use super::super::state::SidecarStatus;

/// Get the current sidecar status
#[tauri::command]
pub async fn sidecar_status(state: State<'_, AppState>) -> Result<SidecarStatus, String> {
    Ok(state.sidecar_state.status())
}

/// Initialize the sidecar for a workspace
#[tauri::command]
pub async fn sidecar_initialize(
    state: State<'_, AppState>,
    workspace_path: String,
) -> Result<(), String> {
    state
        .sidecar_state
        .initialize(workspace_path.into())
        .await
        .map_err(|e| e.to_string())
}

/// Shutdown the sidecar
#[tauri::command]
pub async fn sidecar_shutdown(state: State<'_, AppState>) -> Result<(), String> {
    state.sidecar_state.shutdown();
    Ok(())
}
