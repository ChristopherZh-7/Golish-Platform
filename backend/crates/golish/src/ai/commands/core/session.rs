//! AI session lifecycle and config commands.

use std::path::PathBuf;
use std::sync::Arc;

use tauri::{AppHandle, State};

use super::super::super::agent_bridge::AgentBridge;
use super::super::super::llm_client::{ProviderConfig, SharedComponentsConfig};
use super::super::configure_bridge;
use crate::runtime::TauriRuntime;
use crate::state::AppState;
use golish_ai::TranscriptWriter;
use golish_context::ContextManagerConfig;
use golish_core::runtime::GolishRuntime;


// ========== Session-specific commands ==========

/// Initialize AI agent for a specific session (tab).
///
/// Each session can have its own provider/model configuration, allowing
/// different tabs to use different AI providers simultaneously.
///
/// # Arguments
/// * `session_id` - The terminal session ID (tab) to initialize AI for
/// * `config` - Provider-specific configuration (VertexAi, Openrouter, Openai, etc.)
#[tauri::command]
pub async fn init_ai_session(
    state: State<'_, AppState>,
    app: AppHandle,
    session_id: String,
    config: ProviderConfig,
) -> Result<(), String> {
    // Clean up existing session bridge if present
    if state.ai_state.has_session_bridge(&session_id).await {
        tracing::debug!("Removing existing bridge for session {}", session_id);
        let _old_bridge = state.ai_state.remove_session_bridge(&session_id).await;
        // Explicitly drop outside the if to ensure it's fully dropped before continuing
        drop(_old_bridge);
    }

    // Create runtime for event emission
    let app_for_tools = app.clone();
    let runtime: Arc<dyn GolishRuntime> = Arc::new(TauriRuntime::new(app));

    // Load shared components config from application settings
    // This includes context management config and shell override
    let shared_config = {
        let settings = state.settings_manager.get().await;

        // Build context config if enabled
        let context_config = if settings.context.enabled {
            Some(ContextManagerConfig {
                enabled: settings.context.enabled,
                compaction_threshold: settings.context.compaction_threshold,
                protected_turns: settings.context.protected_turns,
                cooldown_seconds: settings.context.cooldown_seconds,
            })
        } else {
            None
        };

        // Shell override is now part of the settings instance passed to SharedComponentsConfig
        if settings.terminal.shell.is_some() {
            tracing::debug!(
                "Using shell override from settings for session {}: {:?}",
                session_id,
                settings.terminal.shell
            );
        }

        SharedComponentsConfig {
            context_config,
            settings,
        }
    };

    tracing::debug!(
        "Shared config for session {}: context={:?}",
        session_id,
        shared_config.context_config,
    );

    let workspace_path: std::path::PathBuf = config.workspace().into();
    let provider_name = config.provider_name().to_string();
    let model_name = config.model().to_string();

    // Dispatch to appropriate AgentBridge constructor based on provider
    // All constructors now use *_with_shared_config to pass both context and shell settings
    let mut bridge = match config {
        ProviderConfig::VertexAi {
            workspace: _,
            model,
            credentials_path,
            project_id,
            location,
        } => {
            AgentBridge::new_vertex_anthropic_with_shared_config(
                workspace_path.clone(),
                credentials_path.as_deref(),
                &project_id,
                &location,
                &model,
                shared_config,
                runtime,
                &session_id,
            )
            .await
        }
        ProviderConfig::Openrouter {
            workspace: _,
            model,
            api_key,
            provider_preferences,
        } => {
            AgentBridge::new_openrouter_with_shared_config(
                workspace_path.clone(),
                &model,
                &api_key,
                provider_preferences,
                shared_config,
                runtime,
                &session_id,
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
            AgentBridge::new_openai_with_shared_config(
                workspace_path.clone(),
                &model,
                &api_key,
                base_url.as_deref(),
                reasoning_effort.as_deref(),
                shared_config,
                runtime,
                &session_id,
            )
            .await
        }
        ProviderConfig::Anthropic {
            workspace: _,
            model,
            api_key,
        } => {
            AgentBridge::new_anthropic_with_shared_config(
                workspace_path.clone(),
                &model,
                &api_key,
                shared_config,
                runtime,
                &session_id,
            )
            .await
        }
        ProviderConfig::Ollama {
            workspace: _,
            model,
            base_url,
        } => {
            AgentBridge::new_ollama_with_shared_config(
                workspace_path.clone(),
                &model,
                base_url.as_deref(),
                shared_config,
                runtime,
                &session_id,
            )
            .await
        }
        ProviderConfig::Gemini {
            workspace: _,
            model,
            api_key,
            include_thoughts: _,
        } => {
            AgentBridge::new_gemini_with_shared_config(
                workspace_path.clone(),
                &model,
                &api_key,
                shared_config,
                runtime,
                &session_id,
            )
            .await
        }
        ProviderConfig::Groq {
            workspace: _,
            model,
            api_key,
        } => {
            AgentBridge::new_groq_with_shared_config(
                workspace_path.clone(),
                &model,
                &api_key,
                shared_config,
                runtime,
                &session_id,
            )
            .await
        }
        ProviderConfig::Xai {
            workspace: _,
            model,
            api_key,
        } => {
            AgentBridge::new_xai_with_shared_config(
                workspace_path.clone(),
                &model,
                &api_key,
                shared_config,
                runtime,
                &session_id,
            )
            .await
        }
        ProviderConfig::ZaiSdk {
            workspace: _,
            model,
            api_key,
            base_url,
            source_channel,
        } => {
            AgentBridge::new_zai_sdk_with_shared_config(
                workspace_path.clone(),
                &model,
                &api_key,
                base_url.as_deref(),
                source_channel.as_deref(),
                shared_config,
                runtime,
                &session_id,
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
                shared_config,
                runtime,
                &session_id,
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
            AgentBridge::new_vertex_gemini_with_shared_config(
                workspace_path.clone(),
                credentials_path.as_deref(),
                &project_id,
                &location,
                &model,
                include_thoughts,
                shared_config,
                runtime,
                &session_id,
            )
            .await
        }
    }
    .map_err(|e| e.to_string())?;

    configure_bridge(&mut bridge, &state, &session_id, Some(app_for_tools)).await;

    // Initialize transcript writer for persisting AI events to JSONL
    // Transcripts are stored in {workspace}/.golish/transcripts/{session_id}/transcript.jsonl
    // Falls back to ~/.golish/transcripts/ if workspace is "." or env override is set
    let transcripts_dir = std::env::var("VT_TRANSCRIPT_DIR")
        .map(PathBuf::from)
        .unwrap_or_else(|_| {
            if workspace_path.to_string_lossy() != "." {
                workspace_path.join(".golish/transcripts")
            } else {
                dirs::home_dir()
                    .unwrap_or_default()
                    .join(".golish/transcripts")
            }
        });

    match TranscriptWriter::new(&transcripts_dir, &session_id).await {
        Ok(writer) => {
            bridge.set_transcript_writer(writer, transcripts_dir.clone());
            tracing::debug!(
                "Transcript writer initialized for session {} at {:?}",
                session_id,
                transcripts_dir.join(&session_id)
            );
        }
        Err(e) => {
            tracing::warn!(
                "Failed to create transcript writer for session {}: {}",
                session_id,
                e
            );
        }
    }

    // Set the session_id on the bridge for terminal command execution
    bridge.set_session_id(Some(session_id.clone())).await;

    // Store the bridge in the session map
    state
        .ai_state
        .insert_session_bridge(session_id.clone(), bridge)
        .await;

    tracing::info!(
        "AI agent initialized for session {}: provider={}, model={}",
        session_id,
        provider_name,
        model_name
    );
    Ok(())
}

/// Shutdown AI agent for a specific session.
///
/// Removes the AI agent bridge for the specified session, freeing resources.
/// This should be called when a tab is closed or when the user clicks stop.
#[tauri::command]
pub async fn shutdown_ai_session(
    state: State<'_, AppState>,
    session_id: String,
) -> Result<(), String> {
    // Signal cancellation before removing the bridge so the running
    // agentic loop (which holds an Arc clone) sees the flag.
    {
        let bridges = state.ai_state.get_bridges().await;
        if let Some(bridge) = bridges.get(&session_id) {
            bridge.cancel();
            tracing::info!("Cancellation signalled for session {}", session_id);
        }
    }

    if state
        .ai_state
        .remove_session_bridge(&session_id)
        .await
        .is_some()
    {
        tracing::info!("AI agent shut down for session {}", session_id);
        Ok(())
    } else {
        tracing::debug!("No AI agent found for session {} to shut down", session_id);
        Ok(())
    }
}

/// Cancel the current AI generation for a session without tearing down the bridge.
///
/// Unlike `shutdown_ai_session`, this keeps the session alive so the user can
/// immediately send a new prompt without re-initialization (Cursor-like stop).
/// The cancelled flag is automatically cleared when the next execution starts.
#[tauri::command]
pub async fn cancel_ai_generation(
    state: State<'_, AppState>,
    session_id: String,
) -> Result<(), String> {
    if let Some(bridge) = state.ai_state.get_session_bridge(&session_id).await {
        bridge.cancel();
        tracing::info!("Generation cancelled (session kept alive) for {}", session_id);
        Ok(())
    } else {
        tracing::debug!("No AI agent found for session {} to cancel", session_id);
        Ok(())
    }
}

/// Check if AI agent is initialized for a specific session.
#[tauri::command]
pub async fn is_ai_session_initialized(
    state: State<'_, AppState>,
    session_id: String,
) -> Result<bool, String> {
    Ok(state.ai_state.has_session_bridge(&session_id).await)
}

/// Session AI configuration info.
#[derive(serde::Serialize)]
pub struct SessionAiConfig {
    pub provider: String,
    pub model: String,
}

/// Get the AI configuration for a specific session.
#[tauri::command]
pub async fn get_session_ai_config(
    state: State<'_, AppState>,
    session_id: String,
) -> Result<Option<SessionAiConfig>, String> {
    let bridges = state.ai_state.get_bridges().await;
    if let Some(bridge) = bridges.get(&session_id) {
        Ok(Some(SessionAiConfig {
            provider: bridge.provider_name().to_string(),
            model: bridge.model_name().to_string(),
        }))
    } else {
        Ok(None)
    }
}
