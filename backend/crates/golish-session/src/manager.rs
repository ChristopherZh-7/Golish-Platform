//! [`GolishSessionManager`]: in-memory active session + dual-write to disk
//! and (optionally) PostgreSQL.

use std::path::{Path, PathBuf};
use std::sync::Arc;

use anyhow::{Context, Result};
use chrono::Utc;

use golish_core::session::{
    MessageRole, SessionArchive, SessionArchiveMetadata, SessionMessage,
};

use crate::db;
use crate::types::{GolishMessageRole, GolishSessionMessage, GolishSessionSnapshot};

fn truncate(s: &str, max_len: usize) -> String {
    if s.chars().count() <= max_len {
        s.to_string()
    } else {
        let truncated: String = s.chars().take(max_len).collect();
        format!("{}...", truncated)
    }
}


/// Active session manager for creating and finalizing session archives.
pub struct GolishSessionManager {
    archive: Option<SessionArchive>,
    #[allow(dead_code)] // Metadata stored in archive; kept for debugging
    workspace_label: String,
    #[allow(dead_code)] // Metadata stored in archive; kept for debugging
    workspace_path: PathBuf,
    #[allow(dead_code)] // Metadata stored in archive; kept for debugging
    model: String,
    #[allow(dead_code)] // Metadata stored in archive; kept for debugging
    provider: String,
    messages: Vec<GolishSessionMessage>,
    tools_used: std::collections::HashSet<String>,
    transcript: Vec<String>,
    /// Associated sidecar session ID (for context restoration)
    sidecar_session_id: Option<String>,
    /// Agent mode used in this session ("default", "auto-approve", "planning")
    agent_mode: Option<String>,
    /// Optional PostgreSQL persistence handle for dual-write
    db_handle: Option<db::DbSessionHandle>,
}

impl GolishSessionManager {
    /// Create a new session manager.
    pub async fn new(
        workspace_path: PathBuf,
        model: impl Into<String>,
        provider: impl Into<String>,
    ) -> Result<Self> {
        let workspace_label = workspace_path
            .file_name()
            .and_then(|n| n.to_str())
            .unwrap_or("workspace")
            .to_string();

        let model = model.into();
        let provider = provider.into();

        let metadata = SessionArchiveMetadata::new(
            &workspace_label,
            workspace_path.display().to_string(),
            &model,
            &provider,
            "default",  // theme
            "standard", // reasoning_effort
        );

        let archive = SessionArchive::new(metadata)
            .await
            .context("Failed to create session archive")?;

        Ok(Self {
            archive: Some(archive),
            workspace_label,
            workspace_path,
            model,
            provider,
            messages: Vec::new(),
            tools_used: std::collections::HashSet::new(),
            transcript: Vec::new(),
            sidecar_session_id: None,
            agent_mode: None,
            db_handle: None,
        })
    }

    /// Set the database pool for dual-write persistence.
    pub fn set_db_pool(&mut self, pool: Arc<sqlx::PgPool>) {
        self.db_handle = Some(db::DbSessionHandle {
            pool,
            session_uuid: uuid::Uuid::new_v4(),
        });
    }

    /// Update the workspace path and label.
    ///
    /// This recreates the underlying archive with updated metadata to ensure
    /// the session is saved with the correct workspace path.
    pub async fn update_workspace(&mut self, new_path: PathBuf) {
        let new_label = new_path
            .file_name()
            .and_then(|n| n.to_str())
            .unwrap_or("workspace")
            .to_string();

        // Only update if workspace actually changed
        if self.workspace_path == new_path {
            return;
        }

        self.workspace_path = new_path.clone();
        self.workspace_label = new_label.clone();

        // Recreate the archive with updated metadata if it hasn't been finalized yet
        if self.archive.is_some() {
            let metadata = SessionArchiveMetadata::new(
                &new_label,
                new_path.display().to_string(),
                &self.model,
                &self.provider,
                "default",
                "standard",
            );
            match SessionArchive::new(metadata).await {
                Ok(new_archive) => {
                    self.archive = Some(new_archive);
                    tracing::debug!(
                        "[session] Recreated archive with updated workspace: {}",
                        new_path.display()
                    );
                }
                Err(e) => {
                    tracing::warn!(
                        "[session] Failed to recreate archive with new workspace: {}",
                        e
                    );
                }
            }
        }
    }

    /// Record a user message.
    pub fn add_user_message(&mut self, content: &str) {
        self.messages.push(GolishSessionMessage::user(content));
        self.transcript
            .push(format!("User: {}", truncate(content, 200)));
    }

    /// Record an assistant message.
    pub fn add_assistant_message(&mut self, content: &str) {
        self.messages.push(GolishSessionMessage::assistant(content));
        self.transcript
            .push(format!("Assistant: {}", truncate(content, 200)));
    }

    /// Record a tool use.
    #[allow(dead_code)] // Public API for session recording
    pub fn add_tool_use(&mut self, tool_name: &str, result: &str) {
        self.tools_used.insert(tool_name.to_string());
        self.messages
            .push(GolishSessionMessage::tool_use(tool_name, result));
        self.transcript
            .push(format!("Tool[{}]: {}", tool_name, truncate(result, 100)));
    }

    /// Save the current session state to disk without finalizing.
    /// This allows incremental saves after each message.
    ///
    /// Returns the path to the saved session file.
    pub fn save(&self) -> Result<PathBuf> {
        let archive = self.archive.as_ref().context("Session already finalized")?;

        // Convert GolishSessionMessages to SessionMessages
        let session_messages: Vec<SessionMessage> = self
            .messages
            .iter()
            .map(|m| {
                let role = match m.role {
                    GolishMessageRole::User => MessageRole::User,
                    GolishMessageRole::Assistant => MessageRole::Assistant,
                    GolishMessageRole::System => MessageRole::System,
                    GolishMessageRole::Tool => MessageRole::Tool,
                };
                SessionMessage::with_tool_call_id(role, &m.content, m.tool_call_id.clone())
            })
            .collect();

        let distinct_tools: Vec<String> = self.tools_used.iter().cloned().collect();

        let path = archive
            .finalize(
                self.transcript.clone(),
                self.messages.len(),
                distinct_tools,
                session_messages,
            )
            .context("Failed to save session archive")?;

        // Save sidecar session ID to companion file if available
        if let Some(ref sidecar_id) = self.sidecar_session_id {
            if let Err(e) = Self::write_sidecar_session_id(&path, sidecar_id) {
                tracing::warn!("Failed to save sidecar session ID: {}", e);
            }
        }

        // Save agent mode to companion file if available
        if let Some(ref mode) = self.agent_mode {
            if let Err(e) = Self::write_agent_mode(&path, mode) {
                tracing::warn!("Failed to save agent mode: {}", e);
            }
        }

        // Dual-write to PostgreSQL if configured
        if let Some(ref handle) = self.db_handle {
            let snapshot = self.build_snapshot();
            let pool = handle.pool.clone();
            let uuid = handle.session_uuid;
            tokio::spawn(async move {
                if let Err(e) = db::save_session_to_db(&pool, &snapshot, &uuid).await {
                    tracing::warn!("Failed to save session to DB: {}", e);
                }
            });
        }

        Ok(path)
    }

    /// Finalize the session and save to disk.
    /// After this, the session cannot be updated further.
    ///
    /// Returns the path to the saved session file.
    pub fn finalize(&mut self) -> Result<PathBuf> {
        let archive = self.archive.take().context("Session already finalized")?;

        // Convert GolishSessionMessages to SessionMessages
        let session_messages: Vec<SessionMessage> = self
            .messages
            .iter()
            .map(|m| {
                let role = match m.role {
                    GolishMessageRole::User => MessageRole::User,
                    GolishMessageRole::Assistant => MessageRole::Assistant,
                    GolishMessageRole::System => MessageRole::System,
                    GolishMessageRole::Tool => MessageRole::Tool,
                };
                SessionMessage::with_tool_call_id(role, &m.content, m.tool_call_id.clone())
            })
            .collect();

        let distinct_tools: Vec<String> = self.tools_used.iter().cloned().collect();

        let path = archive
            .finalize(
                self.transcript.clone(),
                self.messages.len(),
                distinct_tools,
                session_messages,
            )
            .context("Failed to finalize session archive")?;

        // Save sidecar session ID to companion file if available
        if let Some(ref sidecar_id) = self.sidecar_session_id {
            if let Err(e) = Self::write_sidecar_session_id(&path, sidecar_id) {
                tracing::warn!("Failed to save sidecar session ID: {}", e);
            }
        }

        // Save agent mode to companion file if available
        if let Some(ref mode) = self.agent_mode {
            if let Err(e) = Self::write_agent_mode(&path, mode) {
                tracing::warn!("Failed to save agent mode: {}", e);
            }
        }

        // Dual-write to PostgreSQL if configured (finalize = mark completed)
        if let Some(ref handle) = self.db_handle {
            let snapshot = self.build_snapshot();
            let pool = handle.pool.clone();
            let uuid = handle.session_uuid;
            tokio::spawn(async move {
                if let Err(e) = db::finalize_session_in_db(&pool, &snapshot, &uuid).await {
                    tracing::warn!("Failed to finalize session in DB: {}", e);
                }
            });
        }

        Ok(path)
    }

    /// Build a GolishSessionSnapshot from current state (for DB persistence).
    fn build_snapshot(&self) -> GolishSessionSnapshot {
        GolishSessionSnapshot {
            workspace_label: self.workspace_label.clone(),
            workspace_path: self.workspace_path.display().to_string(),
            model: self.model.clone(),
            provider: self.provider.clone(),
            started_at: Utc::now(),
            ended_at: Utc::now(),
            total_messages: self.messages.len(),
            distinct_tools: self.tools_used.iter().cloned().collect(),
            transcript: self.transcript.clone(),
            messages: self.messages.clone(),
            sidecar_session_id: self.sidecar_session_id.clone(),
            total_tokens: None,
            agent_mode: self.agent_mode.clone(),
        }
    }

    /// Get the current message count.
    #[allow(dead_code)] // Public API for session inspection
    pub fn message_count(&self) -> usize {
        self.messages.len()
    }

    /// Get the tools used in this session.
    #[cfg(test)]
    pub fn tools_used(&self) -> Vec<String> {
        self.tools_used.iter().cloned().collect()
    }

    /// Set the sidecar session ID for this AI session
    pub fn set_sidecar_session_id(&mut self, sidecar_session_id: String) {
        self.sidecar_session_id = Some(sidecar_session_id);
    }

    /// Set the agent mode for this session
    pub fn set_agent_mode(&mut self, agent_mode: String) {
        self.agent_mode = Some(agent_mode);
    }

    /// Write sidecar session ID to a companion file
    fn write_sidecar_session_id(session_path: &Path, sidecar_session_id: &str) -> Result<()> {
        // Create companion file with .sidecar extension
        let sidecar_meta_path = session_path.with_extension("sidecar");
        std::fs::write(&sidecar_meta_path, sidecar_session_id)
            .context("Failed to write sidecar session ID")?;
        Ok(())
    }

    /// Read sidecar session ID from a companion file
    #[cfg_attr(not(feature = "tauri"), allow(dead_code))]
    pub(crate) fn read_sidecar_session_id(session_path: &Path) -> Option<String> {
        let sidecar_meta_path = session_path.with_extension("sidecar");
        if sidecar_meta_path.exists() {
            std::fs::read_to_string(&sidecar_meta_path).ok()
        } else {
            None
        }
    }

    /// Write agent mode to a companion file
    fn write_agent_mode(session_path: &Path, agent_mode: &str) -> Result<()> {
        // Create companion file with .mode extension
        let mode_path = session_path.with_extension("mode");
        std::fs::write(&mode_path, agent_mode).context("Failed to write agent mode")?;
        Ok(())
    }

    /// Read agent mode from a companion file
    #[cfg_attr(not(feature = "tauri"), allow(dead_code))]
    pub(crate) fn read_agent_mode(session_path: &Path) -> Option<String> {
        let mode_path = session_path.with_extension("mode");
        if mode_path.exists() {
            std::fs::read_to_string(&mode_path)
                .ok()
                .map(|s| s.trim().to_string())
        } else {
            None
        }
    }
}
