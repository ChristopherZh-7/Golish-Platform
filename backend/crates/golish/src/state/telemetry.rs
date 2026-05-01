use std::sync::Arc;

use crate::telemetry::TelemetryStats;

/// Telemetry / observability managed state.
pub struct TelemetryState {
    pub langfuse_active: bool,
    pub stats: Option<Arc<TelemetryStats>>,
}

impl TelemetryState {
    pub fn new(langfuse_active: bool, stats: Option<Arc<TelemetryStats>>) -> Self {
        Self {
            langfuse_active,
            stats,
        }
    }
}
