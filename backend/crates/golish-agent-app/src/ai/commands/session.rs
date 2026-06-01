// Session and conversation management commands.

use crate::error::GolishError;
use tauri::State;

use crate::ai::agent_mode::AgentMode;
use crate::state::AgentState;
use golish_session::{
    self as golish_sess, GolishMessageRole, GolishSessionSnapshot, SessionListingInfo,
    SessionPersistence,
};

/// Clear the AI agent's conversation history.
/// Call this when starting a new conversation or when the user wants to reset context.
///
/// This also ends the current sidecar session (if any) so that a new session
/// will be started with the next prompt.
#[tauri::command]
pub async fn clear_ai_conversation(
    state: State<'_, AgentState>,
    session_id: String,
) -> Result<(), GolishError> {
    let bridge = state
        .ai_state
        .get_session_bridge(&session_id)
        .await
        .ok_or_else(|| super::ai_session_not_initialized_error(&session_id))?;
    bridge.clear_conversation_history().await;
    tracing::info!("AI conversation history cleared for session {}", session_id);
    Ok(())
}

/// Restore conversation history for a specific AI session.
/// Called when reopening an existing conversation to give the AI context.
///
/// # Arguments
/// * `session_id` - The AI session ID to restore history for
/// * `messages` - List of [role, content] pairs to restore
#[tauri::command]
pub async fn restore_ai_conversation(
    state: State<'_, AgentState>,
    session_id: String,
    messages: Vec<(String, String)>,
) -> Result<(), GolishError> {
    let bridge = state
        .ai_state
        .get_session_bridge(&session_id)
        .await
        .ok_or_else(|| {
            format!(
                "AI agent not initialized for session '{}'. Call init_ai_session first.",
                session_id
            )
        })?;

    let count = messages.len();
    bridge.restore_conversation_history(messages).await;
    tracing::info!(
        "[restore] Restored {} messages for session '{}'",
        count,
        session_id
    );
    Ok(())
}

/// Get the current conversation history length.
/// Useful for debugging or showing context status in the UI.
#[tauri::command]
pub async fn get_ai_conversation_length(
    state: State<'_, AgentState>,
    session_id: String,
) -> Result<usize, GolishError> {
    let bridge = state
        .ai_state
        .get_session_bridge(&session_id)
        .await
        .ok_or_else(|| super::ai_session_not_initialized_error(&session_id))?;
    Ok(bridge.conversation_history_len().await)
}

/// List recent AI conversation sessions.
/// Uses PostgreSQL if available, falls back to file-based listing.
///
/// # Arguments
/// * `limit` - Maximum number of sessions to return (0 for all)
#[tauri::command]
pub async fn list_ai_sessions(
    state: State<'_, AgentState>,
    limit: Option<usize>,
) -> Result<Vec<SessionListingInfo>, GolishError> {
    let lim = limit.unwrap_or(20);
    let persistence = crate::ai::session_bridge::PgSessionPersistence::new(state.db_pool.clone());
    match persistence.list_sessions(lim).await {
        Ok(sessions) if !sessions.is_empty() => Ok(sessions),
        _ => golish_sess::list_recent_sessions(lim)
            .await
            .map_err(GolishError::from),
    }
}

/// Find a specific session by its identifier.
/// Uses PostgreSQL if available, falls back to file-based search.
///
/// # Arguments
/// * `identifier` - The session identifier (file stem)
#[tauri::command]
pub async fn find_ai_session(
    state: State<'_, AgentState>,
    identifier: String,
) -> Result<Option<SessionListingInfo>, GolishError> {
    let persistence = crate::ai::session_bridge::PgSessionPersistence::new(state.db_pool.clone());
    match persistence.find_session(&identifier).await {
        Ok(Some(session)) => Ok(Some(session)),
        _ => golish_sess::find_session(&identifier)
            .await
            .map_err(GolishError::from),
    }
}

/// Load a full session with all messages by its identifier.
/// Uses PostgreSQL if available, falls back to file-based loading.
///
/// # Arguments
/// * `identifier` - The session identifier (file stem)
#[tauri::command]
pub async fn load_ai_session(
    state: State<'_, AgentState>,
    identifier: String,
) -> Result<Option<GolishSessionSnapshot>, GolishError> {
    let persistence = crate::ai::session_bridge::PgSessionPersistence::new(state.db_pool.clone());
    match persistence.load_session(&identifier).await {
        Ok(Some(session)) => Ok(Some(session)),
        _ => golish_sess::load_session(&identifier)
            .await
            .map_err(GolishError::from),
    }
}

/// Enable or disable session persistence.
///
/// When enabled, AI conversations are automatically saved to disk.
#[tauri::command]
pub async fn set_ai_session_persistence(
    state: State<'_, AgentState>,
    enabled: bool,
    session_id: String,
) -> Result<(), GolishError> {
    let bridge = state
        .ai_state
        .get_session_bridge(&session_id)
        .await
        .ok_or_else(|| super::ai_session_not_initialized_error(&session_id))?;
    bridge.set_session_persistence_enabled(enabled).await;
    Ok(())
}

/// Check if session persistence is enabled.
#[tauri::command]
pub async fn is_ai_session_persistence_enabled(
    state: State<'_, AgentState>,
    session_id: String,
) -> Result<bool, GolishError> {
    let bridge = state
        .ai_state
        .get_session_bridge(&session_id)
        .await
        .ok_or_else(|| super::ai_session_not_initialized_error(&session_id))?;
    Ok(bridge.is_session_persistence_enabled().await)
}

/// Manually finalize and save the current session.
#[tauri::command]
pub async fn finalize_ai_session(
    state: State<'_, AgentState>,
    session_id: String,
) -> Result<Option<String>, GolishError> {
    let bridge = state
        .ai_state
        .get_session_bridge(&session_id)
        .await
        .ok_or_else(|| super::ai_session_not_initialized_error(&session_id))?;
    let path = bridge.finalize_session().await;
    Ok(path.map(|p| p.display().to_string()))
}

/// Export a session transcript to a file.
///
/// # Arguments
/// * `identifier` - The session identifier (file stem)
/// * `output_path` - Path where the transcript should be saved
#[tauri::command]
pub async fn export_ai_session_transcript(
    state: State<'_, AgentState>,
    identifier: String,
    output_path: String,
) -> Result<(), GolishError> {
    let persistence = crate::ai::session_bridge::PgSessionPersistence::new(state.db_pool.clone());
    let session = match persistence.load_session(&identifier).await {
        Ok(Some(s)) => s,
        _ => golish_sess::load_session(&identifier)
            .await?
            .ok_or_else(|| format!("Session '{}' not found", identifier))?,
    };

    // Format as markdown transcript
    let mut transcript = format!(
        "# Session Transcript\n\n\
         - **Workspace**: {}\n\
         - **Model**: {}\n\
         - **Provider**: {}\n\
         - **Started**: {}\n\
         - **Ended**: {}\n\
         - **Messages**: {}\n\
         - **Tools Used**: {}\n\n\
         ---\n\n",
        session.workspace_label,
        session.model,
        session.provider,
        session.started_at.format("%Y-%m-%d %H:%M:%S UTC"),
        session.ended_at.format("%Y-%m-%d %H:%M:%S UTC"),
        session.total_messages,
        session.distinct_tools.join(", ")
    );

    for msg in &session.messages {
        let role_label = match msg.role {
            GolishMessageRole::User => "**User**",
            GolishMessageRole::Assistant => "**Assistant**",
            GolishMessageRole::System => "**System**",
            GolishMessageRole::Tool => "**Tool**",
        };
        transcript.push_str(&format!("{}\n\n{}\n\n---\n\n", role_label, msg.content));
    }

    std::fs::write(&output_path, transcript)
        .map_err(|e| format!("Failed to write transcript: {}", e))?;

    tracing::info!("Session transcript exported to {}", output_path);
    Ok(())
}

/// Restore a previous session by loading its conversation history.
///
/// This loads the session's messages into the AI agent's conversation history,
/// allowing the user to continue from where they left off.
///
/// # Arguments
/// * `session_id` - The terminal session ID (tab) to restore into
/// * `identifier` - The session identifier (file stem)
#[tauri::command]
pub async fn restore_ai_session(
    state: State<'_, AgentState>,
    session_id: String,
    identifier: String,
) -> Result<GolishSessionSnapshot, GolishError> {
    let persistence = crate::ai::session_bridge::PgSessionPersistence::new(state.db_pool.clone());
    let session = match persistence.load_session(&identifier).await {
        Ok(Some(s)) => s,
        _ => golish_sess::load_session(&identifier)
            .await?
            .ok_or_else(|| format!("Session '{}' not found", identifier))?,
    };

    // Get the per-session bridge and restore the conversation history
    let bridge = state
        .ai_state
        .get_session_bridge(&session_id)
        .await
        .ok_or_else(|| {
            format!(
                "AI agent not initialized for session '{}'. Call init_ai_session first.",
                session_id
            )
        })?;

    // Convert session messages to rig messages and restore
    let rig_messages: Vec<rig::completion::Message> = session
        .messages
        .iter()
        .filter_map(|m| m.to_rig_message())
        .collect();
    bridge.restore_session_from_messages(rig_messages).await;

    // Restore the agent mode if it was saved with the session
    if let Some(ref mode_str) = session.agent_mode {
        if let Ok(mode) = mode_str.parse::<AgentMode>() {
            bridge.set_agent_mode(mode).await;
            tracing::info!("Restored agent mode: {}", mode);
        } else {
            tracing::warn!("Invalid agent mode in session: {}", mode_str);
        }
    }

    tracing::info!(
        "Restored session '{}' with {} messages",
        identifier,
        session.messages.len()
    );

    // Start a sidecar session for context capture
    // Extract the first user message as the initial request
    let initial_request = session
        .messages
        .iter()
        .find(|m| m.role == GolishMessageRole::User)
        .map(|m| m.content.clone())
        .unwrap_or_else(|| format!("Restored session: {}", identifier));

    // End any existing sidecar session first
    if let Err(e) = state.sidecar_state.end_session() {
        tracing::debug!("No existing sidecar session to end: {}", e);
    }

    // Try to resume the original sidecar session if it exists, otherwise start a new one
    let sidecar_session_id = if let Some(ref id) = session.sidecar_session_id {
        Some(id.clone())
    } else {
        // Legacy session without explicit sidecar ID - try to find a matching session
        tracing::debug!(
            "No sidecar session ID in restored session, searching for matching session by workspace and timestamp"
        );
        let workspace_path = std::path::Path::new(&session.workspace_path);
        match state
            .sidecar_state
            .find_matching_session(workspace_path, session.started_at, Some(120))
            .await
        {
            Ok(Some(id)) => {
                tracing::info!(
                    "Found matching sidecar session {} by workspace/timestamp heuristic",
                    id
                );
                Some(id)
            }
            Ok(None) => {
                tracing::debug!("No matching sidecar session found for legacy session");
                None
            }
            Err(e) => {
                tracing::warn!("Error searching for matching sidecar session: {}", e);
                None
            }
        }
    };

    if let Some(ref sidecar_session_id) = sidecar_session_id {
        // Attempt to resume the original sidecar session
        match state.sidecar_state.resume_session(sidecar_session_id) {
            Ok(_) => {
                tracing::info!(
                    "Resumed original sidecar session {} for restored AI session",
                    sidecar_session_id
                );
            }
            Err(e) => {
                tracing::warn!(
                    "Could not resume original sidecar session {}: {}. Starting new session.",
                    sidecar_session_id,
                    e
                );
                // Fall back to starting a new sidecar session. The sidecar
                // can be intentionally disabled — degrade gracefully and
                // log at info level so it doesn't pollute startup with
                // WARNs that look like real failures.
                match state.sidecar_state.start_session(&initial_request) {
                    Ok(sid) => {
                        tracing::info!("Started new sidecar session {} for restored session", sid);
                    }
                    Err(e) => {
                        tracing::info!("Sidecar session not started for restored session: {}", e);
                    }
                }
            }
        }
    } else {
        // No sidecar session found - start a new one
        tracing::debug!("No sidecar session to resume, starting new session");
        match state.sidecar_state.start_session(&initial_request) {
            Ok(sid) => {
                tracing::info!("Started new sidecar session {} for restored session", sid);
            }
            Err(e) => {
                tracing::info!("Sidecar session not started for restored session: {}", e);
            }
        }
    }

    // Return the session so the frontend can display the restored messages
    Ok(session)
}
