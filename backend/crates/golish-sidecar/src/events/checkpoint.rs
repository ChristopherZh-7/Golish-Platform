use std::path::PathBuf;
use chrono::{DateTime, Utc};
use serde::{Serialize, Deserialize};
use uuid::Uuid;

/// Periodic checkpoint summarizing a batch of events
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct Checkpoint {
    /// Unique identifier
    pub id: Uuid,
    /// Session this checkpoint belongs to
    pub session_id: Uuid,
    /// When this checkpoint was generated
    pub timestamp: DateTime<Utc>,
    /// LLM-generated summary of the events
    pub summary: String,
    /// Event IDs covered by this checkpoint
    pub event_ids: Vec<Uuid>,
    /// Files touched in these events
    pub files_touched: Vec<PathBuf>,
    /// Embedding of the summary
    #[serde(skip_serializing_if = "Option::is_none")]
    pub embedding: Option<Vec<f32>>,
}

impl Checkpoint {
    /// Create a new checkpoint
    pub fn new(
        session_id: Uuid,
        summary: String,
        event_ids: Vec<Uuid>,
        files_touched: Vec<PathBuf>,
    ) -> Self {
        Self {
            id: Uuid::new_v4(),
            session_id,
            timestamp: Utc::now(),
            summary,
            event_ids,
            files_touched,
            embedding: None,
        }
    }
}

/// Active session metadata
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct SidecarSession {
    /// Unique identifier
    pub id: Uuid,
    /// When the session started
    pub started_at: DateTime<Utc>,
    /// When the session ended (None if still active)
    #[serde(skip_serializing_if = "Option::is_none")]
    pub ended_at: Option<DateTime<Utc>>,
    /// The user's initial request
    pub initial_request: String,
    /// Workspace path for this session
    pub workspace_path: PathBuf,
    /// Number of events captured
    pub event_count: usize,
    /// Number of checkpoints generated
    pub checkpoint_count: usize,
    /// All files touched during the session
    pub files_touched: Vec<PathBuf>,
    /// Final summary (generated at session end)
    #[serde(skip_serializing_if = "Option::is_none")]
    pub final_summary: Option<String>,
}

impl SidecarSession {
    /// Create a new session
    pub fn new(workspace_path: PathBuf, initial_request: String) -> Self {
        Self {
            id: Uuid::new_v4(),
            started_at: Utc::now(),
            ended_at: None,
            initial_request,
            workspace_path,
            event_count: 0,
            checkpoint_count: 0,
            files_touched: vec![],
            final_summary: None,
        }
    }

    /// Check if the session is still active
    #[cfg(test)]
    pub fn is_active(&self) -> bool {
        self.ended_at.is_none()
    }

    /// End the session
    pub fn end(&mut self, summary: Option<String>) {
        self.ended_at = Some(Utc::now());
        self.final_summary = summary;
    }

    /// Record that a file was touched
    pub fn touch_file(&mut self, path: PathBuf) {
        if !self.files_touched.contains(&path) {
            self.files_touched.push(path);
        }
    }

    /// Increment event count
    pub fn increment_events(&mut self) {
        self.event_count += 1;
    }

    /// Increment checkpoint count
    pub fn increment_checkpoints(&mut self) {
        self.checkpoint_count += 1;
    }
}
