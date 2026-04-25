//! TelemetryGuard: holds tracer and worker guard for Drop-based shutdown.

use std::sync::Arc;

use opentelemetry_sdk::trace::SdkTracerProvider;
use tracing_appender::non_blocking::WorkerGuard;

use super::stats::TelemetryStats;

/// Result of telemetry initialization.
pub struct TelemetryGuard {
    /// Whether Langfuse tracing is active
    pub langfuse_active: bool,
    /// Guard for the file appender (keeps the background writer thread alive)
    pub file_guard: Option<WorkerGuard>,
    /// Tracer provider (kept to ensure proper shutdown/flush)
    pub(super) tracer_provider: Option<SdkTracerProvider>,
    /// Telemetry statistics (only populated when Langfuse is active)
    pub stats: Option<Arc<TelemetryStats>>,
}

impl Drop for TelemetryGuard {
    fn drop(&mut self) {
        // Shutdown OpenTelemetry first to flush pending spans
        if let Some(provider) = self.tracer_provider.take() {
            tracing::debug!("Flushing OpenTelemetry spans...");
            if let Err(e) = provider.shutdown() {
                eprintln!(
                    "Warning: Failed to shutdown OpenTelemetry provider: {:?}",
                    e
                );
            }
        }

        // Drop the file guard to flush any pending logs
        if self.file_guard.is_some() {
            tracing::debug!("Shutting down file logging...");
        }
        let _ = self.file_guard.take();
    }
}
