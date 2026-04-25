//! AI agent lifecycle commands: init / unified-init.

use std::sync::Arc;

use tauri::{AppHandle, State};

use super::super::super::agent_bridge::AgentBridge;
use super::super::super::llm_client::{ProviderConfig, SharedComponentsConfig};
use super::super::configure_bridge;
use crate::runtime::TauriRuntime;
use crate::state::AppState;
use golish_core::runtime::GolishRuntime;


/// Initialize the AI agent with the specified configuration.
///
/// If an existing AI agent is running, its session will be finalized and the
/// sidecar session will be ended before the new agent is initialized.
///
/// # Arguments
/// * `workspace` - Path to the workspace directory
/// * `provider` - LLM provider name (e.g., "openrouter", "anthropic")
/// * `model` - Model identifier (e.g., "anthropic/claude-3.5-sonnet")
/// * `api_key` - API key for the provider
#[tauri::command]
pub async fn init_ai_agent(
    state: State<'_, AppState>,
    app: AppHandle,
    workspace: String,
    provider: String,
    model: String,
    api_key: String,
) -> Result<(), String> {
    // Clean up existing session before replacing the bridge
    // This ensures sessions are properly finalized when switching models/providers
    {
        let bridge_guard = state.ai_state.bridge.read().await;
        if bridge_guard.is_some() {
            // End the sidecar session (the bridge's Drop impl will finalize its session)
            if let Err(e) = state.sidecar_state.end_session() {
                tracing::warn!("Failed to end sidecar session during agent reinit: {}", e);
            } else {
                tracing::debug!("Sidecar session ended during agent reinit");
            }
        }
    }

    // Phase 5: Use runtime-based constructor
    // TauriRuntime handles event emission via Tauri's event system
    let app_for_tools = app.clone();
    let runtime: Arc<dyn GolishRuntime> = Arc::new(TauriRuntime::new(app));

    // Store runtime in AiState (for potential future use by other components)
    *state.ai_state.runtime.write().await = Some(runtime.clone());

    // Create bridge with runtime (Phase 5 - new path)
    let mut bridge =
        AgentBridge::new_with_runtime(workspace.into(), &provider, &model, &api_key, runtime)
            .await
            .map_err(|e| e.to_string())?;

    configure_bridge(&mut bridge, &state, "legacy", Some(app_for_tools)).await;

    // Replace the bridge (old bridge's Drop impl will finalize its session)
    *state.ai_state.bridge.write().await = Some(bridge);

    tracing::info!(
        "AI agent initialized with provider: {}, model: {}",
        provider,
        model
    );
    Ok(())
}

/// Initialize the AI agent using unified provider configuration.
///
/// This is the unified initialization command that can handle any provider
/// using the ProviderConfig enum. It routes to the appropriate AgentBridge
/// constructor based on the provider variant.
///
/// If an existing AI agent is running, its session will be finalized and the
/// sidecar session will be ended before the new agent is initialized.
///
/// # Arguments
/// * `config` - Provider-specific configuration (VertexAi, Openrouter, Openai, etc.)
#[tauri::command]
pub async fn init_ai_agent_unified(
    state: State<'_, AppState>,
    app: AppHandle,
    config: ProviderConfig,
) -> Result<(), String> {
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

    // Dispatch to appropriate AgentBridge constructor based on provider
    let mut bridge = match config {
        ProviderConfig::VertexAi {
            workspace: _,
            model,
            credentials_path,
            project_id,
            location,
        } => {
            AgentBridge::new_vertex_anthropic_with_runtime(
                workspace_path.clone(),
                credentials_path.as_deref(),
                &project_id,
                &location,
                &model,
                runtime,
            )
            .await
        }
        ProviderConfig::Openrouter {
            workspace: _,
            model,
            api_key,
            ..
        } => {
            AgentBridge::new_with_runtime(
                workspace_path.clone(),
                "openrouter",
                &model,
                &api_key,
                runtime,
            )
            .await
        }
        ProviderConfig::Openai {
            workspace: _,
            model,
            api_key,
            base_url,
            reasoning_effort,
            ..
        } => {
            // Note: Web search settings are passed via settings.toml and handled in AgentBridge
            AgentBridge::new_openai_with_runtime(
                workspace_path.clone(),
                &model,
                &api_key,
                base_url.as_deref(),
                reasoning_effort.as_deref(),
                runtime,
            )
            .await
        }
        ProviderConfig::Anthropic {
            workspace: _,
            model,
            api_key,
        } => {
            AgentBridge::new_anthropic_with_runtime(
                workspace_path.clone(),
                &model,
                &api_key,
                runtime,
            )
            .await
        }
        ProviderConfig::Ollama {
            workspace: _,
            model,
            base_url,
        } => {
            AgentBridge::new_ollama_with_runtime(
                workspace_path.clone(),
                &model,
                base_url.as_deref(),
                runtime,
            )
            .await
        }
        ProviderConfig::Gemini {
            workspace: _,
            model,
            api_key,
            include_thoughts: _,
        } => {
            AgentBridge::new_gemini_with_runtime(workspace_path.clone(), &model, &api_key, runtime)
                .await
        }
        ProviderConfig::Groq {
            workspace: _,
            model,
            api_key,
        } => {
            AgentBridge::new_groq_with_runtime(workspace_path.clone(), &model, &api_key, runtime)
                .await
        }
        ProviderConfig::Xai {
            workspace: _,
            model,
            api_key,
        } => {
            AgentBridge::new_xai_with_runtime(workspace_path.clone(), &model, &api_key, runtime)
                .await
        }
        ProviderConfig::ZaiSdk {
            workspace: _,
            model,
            api_key,
            base_url,
            source_channel,
        } => {
            AgentBridge::new_zai_sdk_with_runtime(
                workspace_path.clone(),
                &model,
                &api_key,
                base_url.as_deref(),
                source_channel.as_deref(),
                runtime,
            )
            .await
        }
        ProviderConfig::Nvidia {
            workspace: _,
            model,
            api_key,
            base_url,
        } => {
            AgentBridge::new_nvidia_with_shared_config(
                workspace_path.clone(),
                &model,
                &api_key,
                base_url.as_deref(),
                SharedComponentsConfig::default(),
                runtime,
                "",
            )
            .await
        }
        ProviderConfig::VertexGemini {
            workspace: _,
            model,
            credentials_path,
            project_id,
            location,
            include_thoughts,
        } => {
            AgentBridge::new_vertex_gemini_with_runtime(
                workspace_path.clone(),
                credentials_path.as_deref(),
                &project_id,
                &location,
                &model,
                include_thoughts,
                runtime,
            )
            .await
        }
    }
    .map_err(|e| e.to_string())?;

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
