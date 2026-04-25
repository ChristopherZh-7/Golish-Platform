//! Background initialisation of the sidecar subsystem.

use std::sync::Arc;

use tauri::{async_runtime, AppHandle};

use crate::app::workspace::resolve_workspace_path;
use crate::settings::SettingsManager;
use crate::sidecar::SidecarState;

/// Attach the Tauri app handle to the shared [`SidecarState`] and spawn a
/// background task that initialises the sidecar for the active workspace
/// (unless disabled in settings).
pub(crate) fn spawn_sidecar_initialization(
    sidecar_state: Arc<SidecarState>,
    settings_manager: Arc<SettingsManager>,
    app_handle: AppHandle,
) {
    sidecar_state.set_app_handle(app_handle);

    async_runtime::spawn(async move {
        let settings = settings_manager.get().await;

        if !settings.sidecar.enabled {
            tracing::debug!(
                "[tauri-setup] Sidecar disabled in settings, skipping initialization"
            );
            return;
        }

        let workspace = resolve_workspace_path();

        tracing::info!(
            "[tauri-setup] Initializing sidecar for workspace: {:?}",
            workspace
        );

        if let Err(e) = sidecar_state.initialize(workspace).await {
            tracing::warn!("[tauri-setup] Failed to initialize sidecar: {}", e);
        } else {
            tracing::info!("[tauri-setup] Sidecar initialized successfully");
        }
    });
}
