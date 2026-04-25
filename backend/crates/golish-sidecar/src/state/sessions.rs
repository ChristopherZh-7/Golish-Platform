//! Session-management methods on [`SidecarState`]: start / resume / end /
//! list / find / capture / context retrieval.

use std::path::PathBuf;

use anyhow::Result;

use super::super::events::{SessionEvent, SidecarEvent};
use super::super::session::{Session, SessionMeta};

use super::SidecarState;

impl SidecarState {

    /// Start a new session
    ///
    /// This method is thread-safe and atomic - if called concurrently, only one
    /// session will be created and subsequent calls will return the existing session ID.
    pub fn start_session(&self, initial_request: &str) -> Result<String> {
        let config = self.config.read().unwrap();
        if !config.enabled {
            anyhow::bail!("Sidecar is disabled");
        }
        let sessions_dir = config.sessions_dir();
        drop(config);

        // Use write lock throughout to make check-and-set atomic
        // This prevents race conditions where two threads could both pass the
        // "session exists" check before either sets the session ID
        let mut state = self.state.write().unwrap();

        if !state.initialized {
            anyhow::bail!("Sidecar not initialized");
        }

        // Check if session already exists (atomic with the set below)
        if let Some(ref existing_id) = state.current_session_id {
            tracing::debug!(
                existing_session = %existing_id,
                "Session already exists, returning existing ID"
            );
            return Ok(existing_id.clone());
        }

        let cwd = state
            .workspace_path
            .clone()
            .unwrap_or_else(|| PathBuf::from("."));

        // Generate session ID and set it atomically (while still holding the lock)
        let session_id = uuid::Uuid::new_v4().to_string();
        state.current_session_id = Some(session_id.clone());

        // Release the lock before creating the session
        drop(state);

        // Create session directory and files SYNCHRONOUSLY to avoid race conditions
        // where events arrive before state.md exists
        let sid = session_id.clone();
        let req = initial_request.to_string();
        let cwd_clone = cwd.clone();

        // Use spawn_blocking + block_on to safely run async code synchronously
        // This ensures state.md exists before we return
        std::thread::spawn(move || {
            let rt = tokio::runtime::Builder::new_current_thread()
                .enable_all()
                .build()
                .unwrap();
            rt.block_on(async {
                if let Err(e) = Session::create(&sessions_dir, sid, cwd_clone, req).await {
                    tracing::error!("Failed to create session: {}", e);
                }
            });
        })
        .join()
        .expect("Session creation thread panicked");

        // Emit session started event
        self.emit_event(SidecarEvent::SessionStarted {
            session_id: session_id.clone(),
        });

        tracing::info!("Started new session: {}", session_id);
        Ok(session_id)
    }

    /// Resume an existing session by session ID
    ///
    /// This reactivates a previously created session, updating its status to Active
    /// and setting it as the current session. This is useful when restoring a
    /// previous AI conversation session to preserve the sidecar context.
    pub fn resume_session(&self, session_id: &str) -> Result<SessionMeta> {
        let config = self.config.read().unwrap();
        if !config.enabled {
            anyhow::bail!("Sidecar is disabled");
        }
        let sessions_dir = config.sessions_dir();
        drop(config);

        let mut state = self.state.write().unwrap();

        if !state.initialized {
            anyhow::bail!("Sidecar not initialized");
        }

        // Validate session exists
        let session_dir = sessions_dir.join(session_id);
        if !session_dir.exists() {
            anyhow::bail!("Session {} not found", session_id);
        }

        // Set as current session
        state.current_session_id = Some(session_id.to_string());
        drop(state);

        // Load and update session metadata (in a blocking manner to update the file)
        let sid = session_id.to_string();
        let meta = std::thread::spawn(move || {
            let rt = tokio::runtime::Builder::new_current_thread()
                .enable_all()
                .build()
                .unwrap();
            rt.block_on(async {
                // Load the session
                let mut session = Session::load(&sessions_dir, &sid).await?;

                // Update status and timestamp
                let mut meta = session.meta().clone();
                meta.status = super::super::session::SessionStatus::Active;
                meta.updated_at = chrono::Utc::now();

                // Write updated metadata
                session.update_meta(&meta).await?;

                Ok::<_, anyhow::Error>(meta)
            })
        })
        .join()
        .map_err(|_| anyhow::anyhow!("Failed to join thread"))?
        .map_err(|e| anyhow::anyhow!("Failed to update session metadata: {}", e))?;

        // Emit session started event
        self.emit_event(SidecarEvent::SessionStarted {
            session_id: session_id.to_string(),
        });

        tracing::info!("Resumed sidecar session: {}", session_id);
        Ok(meta)
    }

    /// End the current session
    pub fn end_session(&self) -> Result<Option<SessionMeta>> {
        let session_id = {
            let mut state = self.state.write().unwrap();
            state.current_session_id.take()
        };

        let Some(session_id) = session_id else {
            tracing::trace!("No active session to end");
            return Ok(None);
        };

        tracing::info!(session_id = %session_id, "Ending sidecar session");

        // Emit session ended event
        self.emit_event(SidecarEvent::SessionEnded {
            session_id: session_id.clone(),
        });

        // Signal processor to end session
        if let Some(processor) = self.processor.read().unwrap().as_ref() {
            processor.end_session(session_id.clone());
        }

        // Load session metadata
        let config = self.config.read().unwrap();
        let sessions_dir = config.sessions_dir();

        let meta = std::thread::spawn(move || {
            let rt = tokio::runtime::Builder::new_current_thread()
                .enable_all()
                .build()
                .unwrap();
            rt.block_on(async {
                match Session::load(&sessions_dir, &session_id).await {
                    Ok(session) => Some(session.meta().clone()),
                    Err(e) => {
                        tracing::error!("Failed to load session metadata: {}", e);
                        None
                    }
                }
            })
        })
        .join()
        .unwrap_or(None);

        tracing::info!("Ended session: {:?}", meta.as_ref().map(|m| &m.session_id));
        Ok(meta)
    }

    /// Get current session ID
    pub fn current_session_id(&self) -> Option<String> {
        self.state.read().unwrap().current_session_id.clone()
    }

    /// Capture an event
    pub fn capture(&self, event: SessionEvent) {
        let config = self.config.read().unwrap();
        if !config.enabled {
            tracing::trace!("[sidecar-state] Sidecar disabled, skipping event capture");
            return;
        }

        // Filter based on config
        if !config.capture_tool_calls
            && matches!(event.event_type, super::super::events::EventType::ToolCall { .. })
        {
            tracing::trace!("[sidecar-state] Tool call capture disabled, skipping");
            return;
        }
        if !config.capture_reasoning
            && matches!(
                event.event_type,
                super::super::events::EventType::AgentReasoning { .. }
            )
        {
            tracing::trace!("[sidecar-state] Reasoning capture disabled, skipping");
            return;
        }

        // Log event being captured (trace level for high-frequency events like reasoning)
        tracing::trace!(
            "[sidecar-state] Capturing event: {} for session {} (files_modified: {})",
            event.event_type.name(),
            event.session_id,
            event.files_modified.len()
        );

        // Forward to processor
        if let Some(processor) = self.processor.read().unwrap().as_ref() {
            processor.process_event(event.session_id.clone(), event);
        } else {
            tracing::warn!("[sidecar-state] No processor available, event not forwarded");
        }
    }

    /// Get injectable context (state.md body) for current session
    pub async fn get_injectable_context(&self) -> Result<Option<String>> {
        let session_id = match self.current_session_id() {
            Some(id) => id,
            None => return Ok(None),
        };

        let sessions_dir = self.config.read().unwrap().sessions_dir();
        let session = Session::load(&sessions_dir, &session_id).await?;
        let state = session.read_state().await?;
        Ok(Some(state))
    }

    /// Get session state.md content (body only)
    pub async fn get_session_state(&self, session_id: &str) -> Result<String> {
        let sessions_dir = self.config.read().unwrap().sessions_dir();
        let session = Session::load(&sessions_dir, session_id).await?;
        session.read_state().await
    }

    /// Get session metadata
    pub async fn get_session_meta(&self, session_id: &str) -> Result<SessionMeta> {
        let sessions_dir = self.config.read().unwrap().sessions_dir();
        let session = Session::load(&sessions_dir, session_id).await?;
        Ok(session.meta().clone())
    }

    /// List all sessions
    pub async fn list_sessions(&self) -> Result<Vec<SessionMeta>> {
        let sessions_dir = self.config.read().unwrap().sessions_dir();
        super::super::session::list_sessions(&sessions_dir).await
    }

    /// Find a matching sidecar session by workspace path and timestamp.
    ///
    /// This is a fallback for legacy sessions that don't have an explicit sidecar_session_id.
    /// It searches for sessions with matching workspace path created within a time window.
    ///
    /// # Arguments
    /// * `workspace_path` - The workspace path to match
    /// * `started_at` - The AI session start time to match
    /// * `tolerance_secs` - Time tolerance in seconds (default: 60)
    pub async fn find_matching_session(
        &self,
        workspace_path: &std::path::Path,
        started_at: chrono::DateTime<chrono::Utc>,
        tolerance_secs: Option<i64>,
    ) -> Result<Option<String>> {
        let tolerance = tolerance_secs.unwrap_or(60);
        let sessions = self.list_sessions().await?;

        // Find sessions that match workspace path and are within the time tolerance
        let matching = sessions.into_iter().find(|meta| {
            // Check workspace path matches
            let path_matches = meta.cwd == workspace_path;

            // Check timestamp is within tolerance
            let time_diff = (meta.created_at - started_at).num_seconds().abs();
            let time_matches = time_diff <= tolerance;

            if path_matches && time_matches {
                tracing::debug!(
                    "Found matching sidecar session {} (path match: {}, time diff: {}s)",
                    meta.session_id,
                    path_matches,
                    time_diff
                );
            }

            path_matches && time_matches
        });

        Ok(matching.map(|m| m.session_id))
    }
}
