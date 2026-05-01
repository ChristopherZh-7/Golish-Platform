//! Session listing and lookup functionality.
//!
//! This module provides types and functions for listing and finding sessions.
//!
//! Tests live in the sibling [`tests`] file because they cover storage I/O
//! (with `tempfile` + `serial_test`) and would otherwise dominate the file.

use std::path::PathBuf;

use anyhow::Result;
use chrono::{DateTime, Utc};
use serde::{Deserialize, Serialize};

use super::archive::SessionArchiveMetadata;
use super::message::{MessageRole, SessionMessage};
use super::storage;

#[cfg(test)]
mod tests;

/// Full session snapshot that is serialized to disk.
///
/// This structure matches the JSON format of existing session files
/// for backwards compatibility.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct SessionSnapshot {
    /// Session metadata.
    pub metadata: SessionArchiveMetadata,
    /// When the session started.
    pub started_at: DateTime<Utc>,
    /// When the session ended.
    pub ended_at: DateTime<Utc>,
    /// Total number of messages in the session.
    pub total_messages: usize,
    /// List of unique tool names used.
    pub distinct_tools: Vec<String>,
    /// Human-readable transcript lines.
    pub transcript: Vec<String>,
    /// Full message history.
    pub messages: Vec<SessionMessage>,
}

/// Session listing entry for display and lookup.
///
/// This provides metadata about a session without necessarily
/// loading the full message history.
#[derive(Debug, Clone)]
pub struct SessionListing {
    /// Path to the session file.
    pub path: PathBuf,
    /// When the session started.
    pub started_at: DateTime<Utc>,
    /// When the session ended.
    pub ended_at: DateTime<Utc>,
    /// Full session snapshot (for accessing messages and metadata).
    pub snapshot: SessionSnapshot,
}

impl SessionListing {
    /// Create a listing from a snapshot and path.
    pub fn from_snapshot(snapshot: SessionSnapshot, path: PathBuf) -> Self {
        Self {
            started_at: snapshot.started_at,
            ended_at: snapshot.ended_at,
            path,
            snapshot,
        }
    }

    /// Get the session identifier (session_id from metadata).
    ///
    /// This matches the vtcode-core interface: `listing.identifier()`.
    pub fn identifier(&self) -> String {
        self.snapshot.metadata.session_id.clone()
    }

    /// Get a preview of the first user prompt.
    ///
    /// Returns the content of the first User message, truncated if necessary.
    pub fn first_prompt_preview(&self) -> Option<String> {
        self.snapshot
            .messages
            .iter()
            .find(|m| m.role == MessageRole::User)
            .map(|m| {
                let text = m.content.as_text();
                if text.len() > 200 {
                    let end = text.floor_char_boundary(200);
                    format!("{}...", &text[..end])
                } else {
                    text
                }
            })
    }

    /// Get the first assistant reply content.
    ///
    /// Returns the full content of the first Assistant message.
    pub fn first_reply_preview(&self) -> Option<String> {
        self.snapshot
            .messages
            .iter()
            .find(|m| m.role == MessageRole::Assistant)
            .map(|m| m.content.as_text())
    }
}

/// Find a session by its identifier.
///
/// Drop-in replacement for
/// `vtcode_core::utils::session_archive::find_session_by_identifier()`.
///
/// # Arguments
/// * `identifier` - Session ID or prefix to search for.
///
/// # Returns
/// * `Ok(Some(listing))` if a matching session is found.
/// * `Ok(None)` if no matching session exists.
/// * `Err(_)` if there was an error reading the sessions directory.
///
/// # Note
/// The vtcode-core version is async, but our implementation is synchronous.
/// We provide an async wrapper for interface compatibility.
pub async fn find_session_by_identifier(identifier: &str) -> Result<Option<SessionListing>> {
    storage::find_session(identifier)
}

/// List recent sessions.
///
/// Drop-in replacement for
/// `vtcode_core::utils::session_archive::list_recent_sessions()`.
///
/// # Arguments
/// * `limit` - Maximum number of sessions to return (0 for unlimited).
///
/// # Returns
/// Sessions sorted by start time, most recent first.
///
/// # Note
/// The vtcode-core version is async, but our implementation is synchronous.
/// We provide an async wrapper for interface compatibility.
pub async fn list_recent_sessions(limit: usize) -> Result<Vec<SessionListing>> {
    storage::list_sessions(limit)
}
