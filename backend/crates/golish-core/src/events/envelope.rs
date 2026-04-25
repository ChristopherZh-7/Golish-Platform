//! [`AiEventEnvelope`] — sequence/timestamp wrapper for reliable event delivery.

use serde::{Deserialize, Serialize};

use super::event::AiEvent;

/// Envelope wrapping an AiEvent with reliability metadata.
///
/// This struct is used to ensure reliable event delivery by adding:
/// - A monotonically increasing sequence number for ordering and gap detection
/// - A timestamp for debugging and replay
///
/// The event is flattened during serialization, so the JSON looks like:
/// `{"seq": 42, "ts": "2024-01-15T10:30:00Z", "type": "started", "turn_id": "..."}`
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct AiEventEnvelope {
    /// Monotonically increasing sequence number (per-session)
    pub seq: u64,
    /// RFC 3339 timestamp when the event was created
    pub ts: String,
    /// The wrapped event (flattened during serialization)
    #[serde(flatten)]
    pub event: AiEvent,
}
