//! Telemetry statistics types: counters and snapshot DTOs.

use std::sync::atomic::{AtomicU64, Ordering};

use serde::Serialize;


// =============================================================================
// Telemetry Statistics
// =============================================================================
//
// These types track span processing statistics for debugging and monitoring.
// Stats are exposed via Tauri commands for the frontend to display.

/// Statistics about telemetry/tracing activity.
///
/// These counters help diagnose issues with Langfuse tracing by tracking
/// how many spans are being created and processed.
#[derive(Debug, Default)]
pub struct TelemetryStats {
    /// Total spans that have started (entered on_start)
    pub spans_started: AtomicU64,
    /// Total spans that have ended (entered on_end, queued for export)
    pub spans_ended: AtomicU64,
    /// Timestamp (Unix millis) when stats were created
    pub started_at: AtomicU64,
}

impl TelemetryStats {
    /// Create new telemetry stats, recording the current time.
    pub fn new() -> Self {
        let now = std::time::SystemTime::now()
            .duration_since(std::time::UNIX_EPOCH)
            .map(|d| d.as_millis() as u64)
            .unwrap_or(0);

        Self {
            spans_started: AtomicU64::new(0),
            spans_ended: AtomicU64::new(0),
            started_at: AtomicU64::new(now),
        }
    }

    /// Get a snapshot of current stats.
    pub fn snapshot(&self) -> TelemetryStatsSnapshot {
        TelemetryStatsSnapshot {
            spans_started: self.spans_started.load(Ordering::Relaxed),
            spans_ended: self.spans_ended.load(Ordering::Relaxed),
            started_at: self.started_at.load(Ordering::Relaxed),
        }
    }
}

/// Serializable snapshot of telemetry stats for frontend consumption.
#[derive(Debug, Clone, Serialize)]
pub struct TelemetryStatsSnapshot {
    /// Total spans that have started
    pub spans_started: u64,
    /// Total spans that have ended (queued for export)
    pub spans_ended: u64,
    /// Timestamp (Unix millis) when tracking started
    pub started_at: u64,
}
