use std::sync::Arc;

use crate::telemetry::TelemetryStats;

/// Telemetry / observability managed state.
///
/// Reserved for Tauri `manage<TelemetryState>()` migration (P2-2).
#[allow(dead_code)]
pub struct TelemetryState {
    pub langfuse_active: bool,
    pub stats: Option<Arc<TelemetryStats>>,
}

#[allow(dead_code)]
impl TelemetryState {
    pub fn new(langfuse_active: bool, stats: Option<Arc<TelemetryStats>>) -> Self {
        Self {
            langfuse_active,
            stats,
        }
    }
}
