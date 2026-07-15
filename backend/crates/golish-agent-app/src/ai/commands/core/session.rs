//! AI session lifecycle and config commands.

use crate::error::GolishError;
use std::sync::Arc;

use tauri::{AppHandle, State};

use super::super::super::agent_bridge::AgentBridge;
use super::super::super::llm_client::ProviderConfig;
use super::super::{
    activate_bridge_background_listeners, configure_bridge, prepare_bridge_background_listeners,
};
use crate::ai::provider_bootstrap::normalize_agent_bootstrap;
use crate::runtime::TauriRuntime;
use crate::state::AgentState;
use golish_core::runtime::GolishRuntime;
use golish_events::TranscriptWriter;

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
    state: State<'_, AgentState>,
    app: AppHandle,
    session_id: String,
    config: ProviderConfig,
) -> Result<(), GolishError> {
    let install = state
        .ai_state
        .begin_session_bridge_install(&session_id)
        .await
        .map_err(GolishError::from)?;

    // Create runtime for event emission
    let app_for_tools = app.clone();
    let runtime: Arc<dyn GolishRuntime> =
        Arc::new(TauriRuntime::new(app, Some(state.pty_output_tap.clone())));

    // GUI and CLI normalize all settings-backed provider/runtime fields through
    // the same pure helper. Typed GUI fields (workspace/model/key/explicit
    // endpoint) remain authoritative; hidden settings fields are attached here.
    let settings = state.settings_manager.get().await;
    if settings.terminal.shell.is_some() {
        tracing::debug!(
            "Using shell override from settings for session {}: {:?}",
            session_id,
            settings.terminal.shell
        );
    }
    let bootstrap = normalize_agent_bootstrap(config, &settings);
    let config = bootstrap.provider_config;
    let shared_config = bootstrap.shared_components_config;

    tracing::debug!(
        "Shared config for session {}: context={:?}",
        session_id,
        shared_config.context_config,
    );

    let workspace_path: std::path::PathBuf = config.workspace().into();
    let provider_name = config.provider_name().to_string();
    let model_name = config.model().to_string();

    let mut bridge = match AgentBridge::from_provider_config(
        config,
        shared_config,
        runtime,
        &session_id,
    )
    .await
    {
        Ok(bridge) => bridge,
        Err(error) => {
            // A first init creates a stable slot before provider construction.
            // Failed construction has no bridge/late clone to protect, so drop
            // the transition and prune the otherwise permanent tombstone.
            drop(install);
            state
                .ai_state
                .prune_inactive_session_slot(&session_id)
                .await;
            return Err(GolishError::from(error));
        }
    };

    configure_bridge(&mut bridge, &state, &session_id, Some(app_for_tools)).await;

    // Initialize transcript writer for persisting AI events to JSONL.
    // Resolution is shared with the read side (the `harness_trace` tool and
    // `golish --replay`) via `resolve_transcript_base`, so writers and readers
    // never disagree about where a run's transcripts live (workspace-relative
    // for a real workspace, else `~/.golish/transcripts`).
    let transcripts_dir = golish_events::op_trace::resolve_transcript_base(Some(&workspace_path));
    // Let the per-run tracing log layer (golish::telemetry::session_log) co-locate
    // each run's `run.log` next to this `transcript.json` without re-resolving the
    // workspace.
    golish_events::op_trace::set_active_transcript_base(transcripts_dir.clone());

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
    // Subscribe before retiring the old generation, but do not start processing
    // until the map publish transition activates this candidate.
    let background_listeners = prepare_bridge_background_listeners(&bridge, state.inner()).await;

    // Atomically publish the next bridge generation. The stable per-session
    // request slot makes this fail fast while the old bridge is running and
    // permanently invalidates late clones after replacement.
    if let Err(error) = state
        .ai_state
        .finish_session_bridge_install(install, bridge, move |published| {
            if let Some(prepared) = background_listeners {
                activate_bridge_background_listeners(published, prepared);
            }
        })
        .await
    {
        state
            .ai_state
            .prune_inactive_session_slot(&session_id)
            .await;
        return Err(GolishError::from(error));
    }

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
    state: State<'_, AgentState>,
    session_id: String,
) -> Result<(), GolishError> {
    // Invalidate the stable session generation before cancellation. A sender
    // that cloned the old bridge but has not acquired yet must never be able to
    // reset cancellation and revive the removed session.
    let removed_bridge = state.ai_state.remove_session_bridge(&session_id).await;
    let had_bridge = removed_bridge.is_some();
    if let Some(bridge) = removed_bridge.as_ref() {
        bridge.cancel();
        tracing::info!("Cancellation signalled for session {}", session_id);
    }
    let killed_jobs =
        golish_app_core::background_jobs::manager().kill_running_for_session(&session_id);
    if killed_jobs > 0 {
        tracing::info!(
            killed_background_jobs = killed_jobs,
            "Killed background jobs while shutting down session {}",
            session_id
        );
    }

    // Drop the command's returned old-bridge Arc before GC. Any real late clone
    // or active request still retains the inner SessionRequestSlot and keeps the
    // tombstone, preserving cross-generation single-flight.
    drop(removed_bridge);
    state
        .ai_state
        .prune_inactive_session_slot(&session_id)
        .await;

    if had_bridge {
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
    state: State<'_, AgentState>,
    session_id: String,
) -> Result<(), GolishError> {
    if let Some(bridge) = state.ai_state.get_session_bridge(&session_id).await {
        bridge.cancel();
        let killed_jobs =
            golish_app_core::background_jobs::manager().kill_running_for_session(&session_id);
        tracing::info!(
            killed_background_jobs = killed_jobs,
            "Generation cancelled (session kept alive) for {}",
            session_id
        );
        Ok(())
    } else {
        let killed_jobs =
            golish_app_core::background_jobs::manager().kill_running_for_session(&session_id);
        tracing::debug!("No AI agent found for session {} to cancel", session_id);
        if killed_jobs > 0 {
            tracing::info!(
                killed_background_jobs = killed_jobs,
                "Cancelled orphaned background jobs for session {}",
                session_id
            );
        }
        Ok(())
    }
}

/// Cancel (kill) a background job started when a shell/pentest command exceeded
/// its soft timeout and was detached to the background.
///
/// The background-job manager is a process-wide singleton, so no session state
/// is needed. Killing moves the job to a terminal `Killed` state; the manager's
/// reaper then broadcasts a `JobCompletion`, which the per-session listener
/// (see `bridge_config::spawn_background_completion_listener`) turns into a
/// `ToolBackgroundCompleted` event — flipping the originating tool card out of
/// its "backgrounded" state. Returns `true` if the job existed.
#[tauri::command]
pub fn ai_cancel_background_job(job_id: String) -> bool {
    golish_app_core::background_jobs::manager().kill(&job_id)
}

/// Check if AI agent is initialized for a specific session.
#[tauri::command]
pub async fn is_ai_session_initialized(
    state: State<'_, AgentState>,
    session_id: String,
) -> Result<bool, GolishError> {
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
    state: State<'_, AgentState>,
    session_id: String,
) -> Result<Option<SessionAiConfig>, GolishError> {
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
