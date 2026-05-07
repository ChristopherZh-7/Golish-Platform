//! Session archive creation and persistence.
//!
//! This module provides the `SessionArchive` struct for creating and finalizing
//! AI conversation sessions.

#[cfg(test)]
mod tests;

use std::path::PathBuf;

use anyhow::{Context, Result};
use chrono::{DateTime, Utc};
use serde::{Deserialize, Serialize};
use uuid::Uuid;

use super::listing::SessionSnapshot;
use super::message::SessionMessage;
use super::storage;

/// Session archive metadata.
///
/// Contains information about the session that is persisted to disk.
/// This is a drop-in replacement for vtcode-core's SessionArchiveMetadata.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct SessionArchiveMetadata {
    /// Unique session identifier (UUID)
    #[serde(default = "generate_session_id")]
    pub session_id: String,
    /// Human-readable workspace label (typically the directory name)
    pub workspace_label: String,
    /// Full path to the workspace
    pub workspace_path: String,
    /// Model name/identifier
    pub model: String,
    /// Provider name (e.g., "anthropic_vertex", "openrouter")
    pub provider: String,
    /// Theme name (currently unused, kept for compatibility)
    #[serde(default = "default_theme")]
    pub theme: String,
    /// Reasoning effort level (currently unused, kept for compatibility)
    #[serde(default = "default_reasoning_effort")]
    pub reasoning_effort: String,
}

fn generate_session_id() -> String {
    Uuid::new_v4().to_string()
}

fn default_theme() -> String {
    "default".to_string()
}

fn default_reasoning_effort() -> String {
    "standard".to_string()
}

impl SessionArchiveMetadata {
    /// Create new session metadata.
    ///
    /// This matches the vtcode-core interface:
    /// ```rust,ignore
    /// SessionArchiveMetadata::new(
    ///     &workspace_label,      // &str
    ///     workspace_path,        // String
    ///     &model,               // &str
    ///     &provider,            // &str
    ///     "default",            // theme: &str
    ///     "standard",           // reasoning_effort: &str
    /// )
    /// ```
    pub fn new(
        workspace_label: &str,
        workspace_path: String,
        model: &str,
        provider: &str,
        theme: &str,
        reasoning_effort: &str,
    ) -> Self {
        Self {
            session_id: Uuid::new_v4().to_string(),
            workspace_label: workspace_label.to_string(),
            workspace_path,
            model: model.to_string(),
            provider: provider.to_string(),
            theme: theme.to_string(),
            reasoning_effort: reasoning_effort.to_string(),
        }
    }
}

/// Session archive for creating and persisting AI conversations.
///
/// This is a drop-in replacement for `vtcode_core::utils::session_archive::SessionArchive`.
///
/// ## Interface Contract
///
/// The following interface MUST be preserved for compatibility:
///
/// ```rust,ignore
/// // Creation (session.rs:218-220)
/// let archive = SessionArchive::new(metadata).await?;
///
/// // Finalization (session.rs:305-312)
/// let path = archive.finalize(
///     transcript,        // Vec<String>
///     message_count,     // usize
///     distinct_tools,    // Vec<String>
///     messages,          // Vec<SessionMessage>
/// )?;
/// ```
pub struct SessionArchive {
    /// Session metadata
    metadata: SessionArchiveMetadata,
    /// When the session was started
    started_at: DateTime<Utc>,
    /// Sessions directory path
    sessions_dir: PathBuf,
}

impl SessionArchive {
    /// Create a new session archive.
    ///
    /// Uses the workspace_path from metadata to derive a per-project sessions dir
    /// (`{workspace}/.golish/sessions/`). Falls back to `~/.golish/sessions/` when
    /// workspace is "." or empty.
    pub async fn new(metadata: SessionArchiveMetadata) -> Result<Self> {
        let sessions_dir =
            storage::get_sessions_dir_for(PathBuf::from(&metadata.workspace_path).as_path())
                .context("Failed to get sessions directory")?;

        Ok(Self {
            metadata,
            started_at: Utc::now(),
            sessions_dir,
        })
    }

    /// Finalize the session and save to disk.
    ///
    /// This method saves the session to disk. Takes `&self` for compatibility
    /// with vtcode-core's interface which allows multiple saves.
    /// Returns the path to the saved session file.
    ///
    /// ## Arguments
    /// * `transcript` - Human-readable transcript lines
    /// * `message_count` - Total number of messages (used for validation/metadata)
    /// * `distinct_tools` - List of unique tool names used in the session
    /// * `messages` - Full message history
    pub fn finalize(
        &self,
        transcript: Vec<String>,
        message_count: usize,
        distinct_tools: Vec<String>,
        messages: Vec<SessionMessage>,
    ) -> Result<PathBuf> {
        let ended_at = Utc::now();

        // Create the snapshot
        let snapshot = SessionSnapshot {
            metadata: self.metadata.clone(),
            started_at: self.started_at,
            ended_at,
            total_messages: message_count,
            distinct_tools,
            transcript,
            messages,
        };

        // Save to disk
        storage::save_session(&self.sessions_dir, &snapshot)
    }

    /// Get the session ID.
    pub fn session_id(&self) -> &str {
        &self.metadata.session_id
    }

    /// Get the started_at timestamp.
    pub fn started_at(&self) -> DateTime<Utc> {
        self.started_at
    }
}
