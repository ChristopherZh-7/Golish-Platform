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
    let request = bridge
        .begin_top_level_request()
        .await
        .map_err(|error| GolishError::Internal(error.to_string()))?;
    bridge.clear_conversation_history().await;
    bridge
        .clear_top_level_request_state(&request)
        .await
        .map_err(|error| GolishError::Internal(error.to_string()))?;
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

    let request = bridge
        .begin_top_level_request()
        .await
        .map_err(|error| GolishError::Internal(error.to_string()))?;
    let count = messages.len();
    bridge.restore_conversation_history(messages).await;
    bridge
        .clear_top_level_request_state(&request)
        .await
        .map_err(|error| GolishError::Internal(error.to_string()))?;
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
    let request = bridge
        .begin_top_level_request()
        .await
        .map_err(|error| GolishError::Internal(error.to_string()))?;

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

    if let Some(sidecar) = bridge.session_capture_backend() {
        restore_bridge_sidecar_session(sidecar.as_ref(), &session, &initial_request).await;
    } else {
        tracing::debug!("No per-bridge sidecar backend configured for restored session");
    }

    bridge
        .clear_top_level_request_state(&request)
        .await
        .map_err(|error| GolishError::Internal(error.to_string()))?;

    // Return the session so the frontend can display the restored messages
    Ok(session)
}

async fn restore_bridge_sidecar_session(
    sidecar: &dyn golish_agent_kit::sidecar_trait::SessionCaptureBackend,
    session: &GolishSessionSnapshot,
    initial_request: &str,
) {
    // The backend belongs to the exact bridge generation guarded by the caller's
    // top-level lease. Never mutate AgentState's legacy global SidecarState here.
    if let Err(error) = sidecar.end_session() {
        tracing::debug!("No existing per-bridge sidecar session to end: {}", error);
    }

    let sidecar_session_id = if let Some(ref id) = session.sidecar_session_id {
        Some(id.clone())
    } else {
        tracing::debug!(
            "No sidecar session ID in restored session, searching for matching session by workspace and timestamp"
        );
        let workspace_path = std::path::Path::new(&session.workspace_path);
        match sidecar
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
            Err(error) => {
                tracing::warn!("Error searching for matching sidecar session: {}", error);
                None
            }
        }
    };

    if let Some(ref sidecar_session_id) = sidecar_session_id {
        match sidecar.resume_session(sidecar_session_id) {
            Ok(()) => {
                tracing::info!(
                    "Resumed original sidecar session {} for restored AI session",
                    sidecar_session_id
                );
                return;
            }
            Err(error) => {
                tracing::warn!(
                    "Could not resume original sidecar session {}: {}. Starting new session.",
                    sidecar_session_id,
                    error
                );
            }
        }
    } else {
        tracing::debug!("No sidecar session to resume, starting new session");
    }

    // Sidecar can be intentionally disabled; degrade gracefully.
    match sidecar.start_session(initial_request) {
        Ok(session_id) => {
            tracing::info!(
                "Started new sidecar session {} for restored session",
                session_id
            );
        }
        Err(error) => {
            tracing::info!(
                "Sidecar session not started for restored session: {}",
                error
            );
        }
    }
}

#[cfg(test)]
mod sidecar_restore_tests {
    use std::sync::Mutex;

    use golish_agent_kit::sidecar_trait::{
        AiEventProcessor, EndedSessionInfo, SessionCaptureBackend,
    };
    use golish_core::events::AiEvent;

    use super::{restore_bridge_sidecar_session, GolishSessionSnapshot};

    struct NoopProcessor;

    impl AiEventProcessor for NoopProcessor {
        fn process(&mut self, _event: &AiEvent) {}
    }

    struct RecordingSidecar {
        calls: Mutex<Vec<String>>,
        resume_succeeds: bool,
    }

    impl RecordingSidecar {
        fn calls(&self) -> Vec<String> {
            self.calls.lock().unwrap().clone()
        }
    }

    #[async_trait::async_trait]
    impl SessionCaptureBackend for RecordingSidecar {
        fn current_session_id(&self) -> Option<String> {
            None
        }

        fn start_session(&self, initial_request: &str) -> anyhow::Result<String> {
            self.calls
                .lock()
                .unwrap()
                .push(format!("start:{initial_request}"));
            Ok("new-sidecar".to_string())
        }

        fn end_session(&self) -> anyhow::Result<Option<EndedSessionInfo>> {
            self.calls.lock().unwrap().push("end".to_string());
            Ok(None)
        }

        fn resume_session(&self, session_id: &str) -> anyhow::Result<()> {
            self.calls
                .lock()
                .unwrap()
                .push(format!("resume:{session_id}"));
            if self.resume_succeeds {
                Ok(())
            } else {
                anyhow::bail!("resume failed")
            }
        }

        async fn find_matching_session(
            &self,
            _workspace_path: &std::path::Path,
            _started_at: chrono::DateTime<chrono::Utc>,
            _tolerance_secs: Option<i64>,
        ) -> anyhow::Result<Option<String>> {
            self.calls.lock().unwrap().push("find".to_string());
            Ok(Some("legacy-sidecar".to_string()))
        }

        fn capture_user_prompt(&self, _session_id: &str, _text: &str) {}
        fn capture_ai_response(&self, _session_id: &str, _text: &str) {}
        fn capture_event(&self, _event: &AiEvent) {}

        fn create_event_processor(&self) -> Box<dyn AiEventProcessor> {
            Box::new(NoopProcessor)
        }

        async fn get_injectable_context(&self) -> anyhow::Result<Option<String>> {
            Ok(None)
        }
    }

    fn snapshot(sidecar_session_id: Option<&str>) -> GolishSessionSnapshot {
        let now = chrono::Utc::now();
        GolishSessionSnapshot {
            workspace_label: "workspace".to_string(),
            workspace_path: "/tmp/workspace".to_string(),
            model: "model".to_string(),
            provider: "provider".to_string(),
            started_at: now,
            ended_at: now,
            total_messages: 0,
            distinct_tools: Vec::new(),
            transcript: Vec::new(),
            messages: Vec::new(),
            sidecar_session_id: sidecar_session_id.map(str::to_string),
            total_tokens: None,
            agent_mode: None,
        }
    }

    #[tokio::test]
    async fn restore_uses_supplied_bridge_backend_and_falls_back_locally() {
        let selected = RecordingSidecar {
            calls: Mutex::new(Vec::new()),
            resume_succeeds: false,
        };
        let unrelated = RecordingSidecar {
            calls: Mutex::new(Vec::new()),
            resume_succeeds: true,
        };

        restore_bridge_sidecar_session(&selected, &snapshot(Some("saved")), "initial").await;

        assert_eq!(
            selected.calls(),
            vec!["end", "resume:saved", "start:initial"]
        );
        assert!(
            unrelated.calls().is_empty(),
            "another session's backend must remain untouched"
        );
    }

    #[tokio::test]
    async fn legacy_restore_finds_then_resumes_on_same_backend() {
        let selected = RecordingSidecar {
            calls: Mutex::new(Vec::new()),
            resume_succeeds: true,
        };

        restore_bridge_sidecar_session(&selected, &snapshot(None), "initial").await;

        assert_eq!(
            selected.calls(),
            vec!["end", "find", "resume:legacy-sidecar"]
        );
    }
}
