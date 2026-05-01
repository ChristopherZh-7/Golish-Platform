//! Sidecar configuration getter/setter commands.

use crate::error::GolishError;
use crate::state::AppState;
use tauri::State;

use super::super::config::SidecarConfig;

/// Get the sidecar configuration
#[tauri::command]
pub async fn sidecar_get_config(state: State<'_, AppState>) -> Result<SidecarConfig, GolishError> {
    Ok(state.sidecar_state.config())
}

/// Update the sidecar configuration
#[tauri::command]
pub async fn sidecar_set_config(
    state: State<'_, AppState>,
    config: SidecarConfig,
) -> Result<(), GolishError> {
    state.sidecar_state.set_config(config);
    Ok(())
}
