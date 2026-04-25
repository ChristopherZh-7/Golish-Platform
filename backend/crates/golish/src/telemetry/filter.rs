//! Field-filter visitor + filtered-fields formatter for tracing output.

use tracing_subscriber::fmt::FormatFields;


// =============================================================================
// Filtered Field Formatting
// =============================================================================
//
// These types filter out telemetry-specific fields (langfuse.*, gen_ai.*) from
// console/file log output while preserving them for OpenTelemetry export.
// This keeps logs readable without losing observability data.

/// Field name prefixes to filter from console/file output.
/// These fields are still captured by the OpenTelemetry layer for Langfuse.
const FILTERED_FIELD_PREFIXES: &[&str] = &[
    "langfuse.", // Langfuse-specific: session.id, observation.input/output/type
    "gen_ai.",   // GenAI semantic conventions: request.model, usage.*, prompt, completion
];

/// Check if a field name should be filtered from log output.
#[inline]
fn should_filter_field(name: &str) -> bool {
    FILTERED_FIELD_PREFIXES
        .iter()
        .any(|prefix| name.starts_with(prefix))
}

/// A visitor wrapper that filters out telemetry-specific fields.
///
/// This wraps another `Visit` implementation and skips recording any fields
/// whose names start with filtered prefixes (e.g., `langfuse.`, `gen_ai.`).
#[allow(dead_code)]
pub struct FilteredVisitor<V> {
    inner: V,
}

#[allow(dead_code)]
impl<V> FilteredVisitor<V> {
    /// Create a new filtered visitor wrapping the given visitor.
    pub fn new(inner: V) -> Self {
        Self { inner }
    }
}

impl<V: tracing::field::Visit> tracing::field::Visit for FilteredVisitor<V> {
    fn record_debug(&mut self, field: &tracing::field::Field, value: &dyn std::fmt::Debug) {
        if !should_filter_field(field.name()) {
            self.inner.record_debug(field, value);
        }
    }

    fn record_f64(&mut self, field: &tracing::field::Field, value: f64) {
        if !should_filter_field(field.name()) {
            self.inner.record_f64(field, value);
        }
    }

    fn record_i64(&mut self, field: &tracing::field::Field, value: i64) {
        if !should_filter_field(field.name()) {
            self.inner.record_i64(field, value);
        }
    }

    fn record_u64(&mut self, field: &tracing::field::Field, value: u64) {
        if !should_filter_field(field.name()) {
            self.inner.record_u64(field, value);
        }
    }

    fn record_i128(&mut self, field: &tracing::field::Field, value: i128) {
        if !should_filter_field(field.name()) {
            self.inner.record_i128(field, value);
        }
    }

    fn record_u128(&mut self, field: &tracing::field::Field, value: u128) {
        if !should_filter_field(field.name()) {
            self.inner.record_u128(field, value);
        }
    }

    fn record_bool(&mut self, field: &tracing::field::Field, value: bool) {
        if !should_filter_field(field.name()) {
            self.inner.record_bool(field, value);
        }
    }

    fn record_str(&mut self, field: &tracing::field::Field, value: &str) {
        if !should_filter_field(field.name()) {
            self.inner.record_str(field, value);
        }
    }

    fn record_error(
        &mut self,
        field: &tracing::field::Field,
        value: &(dyn std::error::Error + 'static),
    ) {
        if !should_filter_field(field.name()) {
            self.inner.record_error(field, value);
        }
    }
}

/// A field formatter that filters out telemetry-specific fields.
///
/// This formatter skips fields with filtered prefixes (langfuse.*, gen_ai.*)
/// to keep console/file logs clean while still sending full telemetry data
/// to OpenTelemetry.
///
/// # Example
///
/// ```ignore
/// let layer = tracing_subscriber::fmt::layer()
///     .fmt_fields(FilteredFields::default())
///     .compact();
/// ```
#[derive(Default)]
pub struct FilteredFields;

impl FilteredFields {
    /// Create a new filtered field formatter.
    pub fn new() -> Self {
        Self
    }
}

impl<'writer> FormatFields<'writer> for FilteredFields {
    fn format_fields<R: tracing_subscriber::field::RecordFields>(
        &self,
        writer: tracing_subscriber::fmt::format::Writer<'writer>,
        fields: R,
    ) -> std::fmt::Result {
        // Format fields directly, skipping those with filtered prefixes.
        use tracing_subscriber::field::VisitOutput;

        struct FilteredWriter<'a, 'w> {
            writer: &'a mut tracing_subscriber::fmt::format::Writer<'w>,
            first: bool,
        }

        impl<'a, 'w> FilteredWriter<'a, 'w> {
            fn new(writer: &'a mut tracing_subscriber::fmt::format::Writer<'w>) -> Self {
                Self {
                    writer,
                    first: true,
                }
            }

            fn write_field(&mut self, name: &str, value: &dyn std::fmt::Debug) -> std::fmt::Result {
                if !self.first {
                    self.writer.write_char(' ')?;
                }
                self.first = false;
                write!(self.writer, "{}={:?}", name, value)
            }
        }

        struct DirectVisitor<'a, 'w> {
            writer: FilteredWriter<'a, 'w>,
            result: std::fmt::Result,
        }

        impl<'a, 'w> DirectVisitor<'a, 'w> {
            fn new(writer: &'a mut tracing_subscriber::fmt::format::Writer<'w>) -> Self {
                Self {
                    writer: FilteredWriter::new(writer),
                    result: Ok(()),
                }
            }
        }

        impl<'a, 'w> tracing::field::Visit for DirectVisitor<'a, 'w> {
            fn record_debug(&mut self, field: &tracing::field::Field, value: &dyn std::fmt::Debug) {
                if self.result.is_ok() && !should_filter_field(field.name()) {
                    self.result = self.writer.write_field(field.name(), value);
                }
            }

            fn record_str(&mut self, field: &tracing::field::Field, value: &str) {
                if self.result.is_ok() && !should_filter_field(field.name()) {
                    self.result = self.writer.write_field(field.name(), &value);
                }
            }

            fn record_i64(&mut self, field: &tracing::field::Field, value: i64) {
                if self.result.is_ok() && !should_filter_field(field.name()) {
                    self.result = self.writer.write_field(field.name(), &value);
                }
            }

            fn record_u64(&mut self, field: &tracing::field::Field, value: u64) {
                if self.result.is_ok() && !should_filter_field(field.name()) {
                    self.result = self.writer.write_field(field.name(), &value);
                }
            }

            fn record_bool(&mut self, field: &tracing::field::Field, value: bool) {
                if self.result.is_ok() && !should_filter_field(field.name()) {
                    self.result = self.writer.write_field(field.name(), &value);
                }
            }

            fn record_f64(&mut self, field: &tracing::field::Field, value: f64) {
                if self.result.is_ok() && !should_filter_field(field.name()) {
                    self.result = self.writer.write_field(field.name(), &value);
                }
            }

            fn record_error(
                &mut self,
                field: &tracing::field::Field,
                value: &(dyn std::error::Error + 'static),
            ) {
                if self.result.is_ok() && !should_filter_field(field.name()) {
                    self.result = self.writer.write_field(field.name(), &value);
                }
            }
        }

        impl<'a, 'w> VisitOutput<std::fmt::Result> for DirectVisitor<'a, 'w> {
            fn finish(self) -> std::fmt::Result {
                self.result
            }
        }

        let mut writer = writer;
        let mut visitor = DirectVisitor::new(&mut writer);
        fields.record(&mut visitor);
        visitor.finish()
    }
}
