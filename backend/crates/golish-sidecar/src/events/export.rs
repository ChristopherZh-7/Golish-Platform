use chrono::{DateTime, Utc};
use serde::{Serialize, Deserialize};

use super::checkpoint::{Checkpoint, SidecarSession};
use super::session_event::SessionEvent;

/// Export format for session data
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct SessionExport {
    /// Export version for compatibility
    pub version: u32,
    /// When the export was created
    pub exported_at: DateTime<Utc>,
    /// Session metadata
    pub session: SidecarSession,
    /// All events in the session
    pub events: Vec<SessionEvent>,
    /// All checkpoints in the session
    pub checkpoints: Vec<Checkpoint>,
}

impl SessionExport {
    /// Current export version
    pub const VERSION: u32 = 1;

    /// Create a new export
    pub fn new(
        session: SidecarSession,
        events: Vec<SessionEvent>,
        checkpoints: Vec<Checkpoint>,
    ) -> Self {
        Self {
            version: Self::VERSION,
            exported_at: Utc::now(),
            session,
            events,
            checkpoints,
        }
    }

    /// Export to JSON string
    pub fn to_json(&self) -> Result<String, serde_json::Error> {
        serde_json::to_string_pretty(self)
    }

    /// Import from JSON string
    pub fn from_json(json: &str) -> Result<Self, serde_json::Error> {
        serde_json::from_str(json)
    }
}
