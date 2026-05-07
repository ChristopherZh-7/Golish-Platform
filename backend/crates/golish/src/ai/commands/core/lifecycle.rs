//! AI agent lifecycle commands: unified provider-config init.
//!
//! Historically there were four init commands (`init_ai_agent`,
//! `init_ai_agent_openai`, `init_ai_agent_vertex`,
//! `init_ai_agent_unified`). They were collapsed into a single
//! `init_ai_agent(config: ProviderConfig)` in QW2 (2026-05) — the
//! `ProviderConfig` enum (defined in `golish-llm-providers`) carries
//! the provider-specific fields with serde tag dispatch, so the single
//! command handles every provider. The legacy bridge field
//! (`state.ai_state.bridge`) is still written here pending the
//! per-session bridge migration tracked separately.

use crate::error::GolishError;
use std::sync::Arc;

use tauri::{AppHandle, State};

use super::super::super::agent_bridge::AgentBridge;
use super::super::super::llm_client::{ProviderConfig, SharedComponentsConfig};
use super::super::configure_bridge;
use crate::runtime::TauriRuntime;
use crate::state::AppState;
use golish_core::runtime::GolishRuntime;

/// Initialize the AI agent using unified provider configuration.
///
/// `ProviderConfig` is a serde-tagged enum that carries provider-specific
/// fields (VertexAi / Openrouter / Openai / Anthropic / Ollama / Gemini /
/// Groq / Xai / ZaiSdk / Nvidia / VertexGemini). One command handles every
/// provider — the constructor on `AgentBridge::from_provider_config`
/// routes to the right backend.
///
/// If an existing AI agent is running, its sidecar session is ended and
/// its bridge is replaced. The previous bridge's `Drop` impl finalises
/// any in-flight session.
///
/// # Arguments
/// * `config` - Provider-specific configuration (snake_case fields per
///   `ProviderConfig` definition in `golish-llm-providers`)
#[tauri::command]
pub async fn init_ai_agent(
    state: State<'_, AppState>,
    app: AppHandle,
    config: ProviderConfig,
) -> Result<(), GolishError> {
    // Clean up existing session before replacing the bridge
    {
        let bridge_guard = state.ai_state.bridge.read().await;
        if bridge_guard.is_some() {
            if let Err(e) = state.sidecar_state.end_session() {
                tracing::warn!("Failed to end sidecar session during agent reinit: {}", e);
            } else {
                tracing::debug!("Sidecar session ended during agent reinit");
            }
        }
    }

    // Create runtime for event emission
    let app_for_tools = app.clone();
    let runtime: Arc<dyn GolishRuntime> = Arc::new(TauriRuntime::new(app));
    *state.ai_state.runtime.write().await = Some(runtime.clone());

    let workspace_path: std::path::PathBuf = config.workspace().into();
    let provider_name = config.provider_name().to_string();
    let model_name = config.model().to_string();

    let mut bridge =
        AgentBridge::from_provider_config(config, SharedComponentsConfig::default(), runtime, "")
            .await?;

    configure_bridge(&mut bridge, &state, "legacy", Some(app_for_tools)).await;

    // Replace the bridge
    *state.ai_state.bridge.write().await = Some(bridge);

    // Initialize sidecar with the workspace
    if let Err(e) = state.sidecar_state.initialize(workspace_path).await {
        tracing::warn!("Failed to initialize sidecar: {}", e);
    } else {
        tracing::info!("Sidecar initialized for workspace");
    }

    tracing::info!(
        "AI agent initialized with provider: {}, model: {}",
        provider_name,
        model_name
    );
    Ok(())
}
