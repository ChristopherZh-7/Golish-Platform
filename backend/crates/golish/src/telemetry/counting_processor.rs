//! Counting wrapper around an inner SpanProcessor; ties into TelemetryStats.

use std::sync::atomic::Ordering;
use std::sync::Arc;

use opentelemetry_sdk::trace::SpanProcessor;

use super::stats::TelemetryStats;


/// A span processor that counts spans and delegates to an inner processor.
///
/// This wraps the BatchSpanProcessor to track how many spans are being
/// created and processed, which helps diagnose tracing issues.
pub(super) struct CountingSpanProcessor<P> {
    inner: P,
    stats: Arc<TelemetryStats>,
}

impl<P> CountingSpanProcessor<P> {
    pub(super) fn new(inner: P, stats: Arc<TelemetryStats>) -> Self {
        Self { inner, stats }
    }
}

impl<P: std::fmt::Debug> std::fmt::Debug for CountingSpanProcessor<P> {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.debug_struct("CountingSpanProcessor")
            .field("inner", &self.inner)
            .field(
                "spans_started",
                &self.stats.spans_started.load(Ordering::Relaxed),
            )
            .field(
                "spans_ended",
                &self.stats.spans_ended.load(Ordering::Relaxed),
            )
            .finish()
    }
}

impl<P: SpanProcessor> SpanProcessor for CountingSpanProcessor<P> {
    fn on_start(&self, span: &mut opentelemetry_sdk::trace::Span, cx: &opentelemetry::Context) {
        self.stats.spans_started.fetch_add(1, Ordering::Relaxed);
        self.inner.on_start(span, cx);
    }

    fn on_end(&self, span: opentelemetry_sdk::trace::SpanData) {
        self.stats.spans_ended.fetch_add(1, Ordering::Relaxed);
        self.inner.on_end(span);
    }

    fn force_flush(&self) -> opentelemetry_sdk::error::OTelSdkResult {
        self.inner.force_flush()
    }

    fn shutdown(&self) -> opentelemetry_sdk::error::OTelSdkResult {
        self.inner.shutdown()
    }

    fn shutdown_with_timeout(
        &self,
        timeout: std::time::Duration,
    ) -> opentelemetry_sdk::error::OTelSdkResult {
        self.inner.shutdown_with_timeout(timeout)
    }
}
