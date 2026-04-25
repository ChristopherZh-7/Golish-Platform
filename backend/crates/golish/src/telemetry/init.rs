//! init_tracing entrypoint and Langfuse tracer initialization.

use std::sync::Arc;

use opentelemetry::trace::TracerProvider as _;
use opentelemetry::KeyValue;
use opentelemetry_langfuse::ExporterBuilder;
use opentelemetry_sdk::runtime::Tokio as TokioRuntime;
use opentelemetry_sdk::trace::span_processor_with_async_runtime::BatchSpanProcessor;
use opentelemetry_sdk::trace::{Sampler, SdkTracerProvider};
use opentelemetry_sdk::Resource;
use tracing_opentelemetry::OpenTelemetryLayer;
use tracing_subscriber::fmt::format::FmtSpan;
use tracing_subscriber::{layer::SubscriberExt, util::SubscriberInitExt, EnvFilter, Registry};

use super::counting_processor::CountingSpanProcessor;
use super::filter::FilteredFields;
use super::guard::TelemetryGuard;
use super::langfuse::LangfuseConfig;
use super::stats::TelemetryStats;


/// Initialize tracing with optional Langfuse/OpenTelemetry export.
///
/// This function sets up:
/// 1. Standard `tracing_subscriber` with console output
/// 2. OpenTelemetry layer exporting to Langfuse (if configured)
///
/// # Arguments
///
/// * `langfuse_config` - Optional Langfuse configuration. If None, only console tracing is enabled.
/// * `log_level` - Log level for console output (e.g., "debug", "info", "warn")
/// * `extra_directives` - Additional tracing directives (e.g., "golish=debug")
///
/// # Returns
///
/// A `TelemetryGuard` that should be held for the lifetime of the application.
/// When dropped, it will flush pending traces.
pub fn init_tracing(
    langfuse_config: Option<LangfuseConfig>,
    log_level: &str,
    extra_directives: &[&str],
) -> Result<TelemetryGuard, Box<dyn std::error::Error + Send + Sync>> {
    // Build the base env filter for console/file output
    // This filter is intentionally more restrictive to reduce log verbosity
    let mut filter = EnvFilter::from_default_env();

    // Add log level directive
    if let Ok(directive) = format!("golish={}", log_level).parse() {
        filter = filter.add_directive(directive);
    }

    // Reduce verbosity of deeply nested agent spans for console/file output
    // These modules produce very verbose DEBUG logs that clutter the output
    // OpenTelemetry/Langfuse still captures everything via its own layer
    if log_level == "debug" || log_level == "trace" {
        // Limit sub-agent executor to info (it creates nested llm_completion spans)
        if let Ok(directive) = "golish_sub_agents::executor=info".parse() {
            filter = filter.add_directive(directive);
        }
        // Limit agentic loop streaming details to info
        if let Ok(directive) = "golish_ai::agentic_loop=info".parse() {
            filter = filter.add_directive(directive);
        }
    }

    // Add extra directives
    for directive_str in extra_directives {
        if let Ok(directive) = directive_str.parse() {
            filter = filter.add_directive(directive);
        }
    }

    // Set up file logging to ~/.golish/backend.log
    // Using compact format with span events disabled to reduce verbosity
    // FilteredFields removes langfuse.* and gen_ai.* fields from output
    let (file_layer, file_guard) = if let Some(home) = dirs::home_dir() {
        let golish_dir = home.join(".golish");
        // Create ~/.golish directory if it doesn't exist
        if let Err(e) = std::fs::create_dir_all(&golish_dir) {
            eprintln!("Warning: Failed to create ~/.golish directory: {}", e);
            (None, None)
        } else {
            let file_appender = tracing_appender::rolling::never(&golish_dir, "backend.log");
            let (non_blocking, guard) = tracing_appender::non_blocking(file_appender);
            let filtered_fields = FilteredFields::new();
            let file_layer = tracing_subscriber::fmt::layer()
                .fmt_fields(filtered_fields)
                .with_writer(non_blocking)
                .with_ansi(false)
                .with_span_events(FmtSpan::NONE) // Don't log span enter/exit events
                .compact();
            (Some(file_layer), Some(guard))
        }
    } else {
        (None, None)
    };

    // Detect CI environment to disable ANSI colors
    // Most CI systems set CI=true (GitHub Actions, GitLab CI, CircleCI, Travis, etc.)
    let is_ci = std::env::var("CI").map(|v| v == "true").unwrap_or(false);

    // Create the base subscriber with fmt layer
    // Using compact format with minimal span context for cleaner console output
    // Span events are disabled to reduce noise - OpenTelemetry layer captures full spans
    // FilteredFields removes langfuse.* and gen_ai.* fields from output (still sent to OTel)
    let fmt_layer = tracing_subscriber::fmt::layer()
        .fmt_fields(FilteredFields::new())
        .with_target(true)
        .with_thread_ids(false)
        .with_file(false)
        .with_line_number(false)
        .with_span_events(FmtSpan::NONE) // Don't log span enter/exit events
        .with_ansi(!is_ci) // Disable ANSI colors in CI for cleaner logs
        .compact();

    if let Some(config) = langfuse_config {
        // Set up OpenTelemetry with Langfuse exporter
        let (tracer_provider, stats) = init_langfuse_tracer(&config)?;
        let tracer = tracer_provider.tracer("golish");

        // Create the OpenTelemetry layer
        let otel_layer = OpenTelemetryLayer::new(tracer);

        // Build the subscriber with both layers
        Registry::default()
            .with(filter)
            .with(file_layer)
            .with(fmt_layer)
            .with(otel_layer)
            .try_init()
            .map_err(|e| format!("Failed to initialize tracing: {}", e))?;

        tracing::info!(
            langfuse_host = %config.host,
            "Langfuse tracing enabled"
        );

        Ok(TelemetryGuard {
            langfuse_active: true,
            file_guard,
            tracer_provider: Some(tracer_provider),
            stats: Some(stats),
        })
    } else {
        // No Langfuse, just use fmt layer
        Registry::default()
            .with(filter)
            .with(file_layer)
            .with(fmt_layer)
            .try_init()
            .map_err(|e| format!("Failed to initialize tracing: {}", e))?;

        Ok(TelemetryGuard {
            langfuse_active: false,
            file_guard,
            tracer_provider: None,
            stats: None,
        })
    }
}

/// Initialize the OpenTelemetry tracer provider for Langfuse.
///
/// Returns the tracer provider and telemetry stats for monitoring.
fn init_langfuse_tracer(
    config: &LangfuseConfig,
) -> Result<(SdkTracerProvider, Arc<TelemetryStats>), Box<dyn std::error::Error + Send + Sync>> {
    // Create the Langfuse exporter with direct configuration
    let exporter = ExporterBuilder::new()
        .with_host(&config.host)
        .with_basic_auth(&config.public_key, &config.secret_key)
        .build()?;

    // Build resource with service info
    let resource = Resource::builder()
        .with_service_name(config.service_name.clone())
        .with_attributes([KeyValue::new(
            "service.version",
            config.service_version.clone(),
        )])
        .build();

    // Configure sampler based on sampling ratio
    let sampler = if (config.sampling_ratio - 1.0).abs() < f64::EPSILON {
        Sampler::AlwaysOn
    } else if config.sampling_ratio <= 0.0 {
        Sampler::AlwaysOff
    } else {
        Sampler::TraceIdRatioBased(config.sampling_ratio)
    };

    // Build batch span processor with Tokio async runtime
    // This uses the experimental async runtime feature that properly handles async exporters
    let batch_processor = BatchSpanProcessor::builder(exporter, TokioRuntime).build();

    // Create telemetry stats and wrap batch processor with counting processor
    let stats = Arc::new(TelemetryStats::new());
    let counting_processor = CountingSpanProcessor::new(batch_processor, Arc::clone(&stats));

    // Build the tracer provider with the counting processor
    let provider = SdkTracerProvider::builder()
        .with_span_processor(counting_processor)
        .with_sampler(sampler)
        .with_resource(resource)
        .build();

    tracing::info!(
        host = %config.host,
        public_key_prefix = %&config.public_key[..20],
        "Langfuse exporter initialized"
    );

    // Set as global tracer provider
    opentelemetry::global::set_tracer_provider(provider.clone());

    Ok((provider, stats))
}

/// Helper macro for creating spans with GenAI semantic conventions for Langfuse.
///
/// This creates spans that Langfuse will recognize as "generation" observations
/// when they include model information.
///
/// ## Langfuse Property Mapping
///
/// | Attribute | Langfuse Mapping |
/// |-----------|------------------|
/// | `gen_ai.request.model` | Model name |
/// | `gen_ai.system` | Provider/system |
/// | `gen_ai.prompt` | Input (prompt) |
/// | `gen_ai.completion` | Output (completion) |
/// | `gen_ai.usage.prompt_tokens` | Input token count |
/// | `gen_ai.usage.completion_tokens` | Output token count |
/// | `langfuse.session.id` | Session grouping |
/// | `langfuse.observation.type` | "generation" for LLM calls |
///
/// Usage:
/// ```ignore
/// let _span = gen_ai_span!(
///     "chat_completion",
///     model = "claude-3-opus",
///     provider = "anthropic",
///     session_id = "sess_123"
/// );
/// ```
#[macro_export]
macro_rules! gen_ai_span {
    ($operation:expr, model = $model:expr, provider = $provider:expr $(, session_id = $session:expr)? $(,)?) => {
        tracing::info_span!(
            $operation,
            "gen_ai.operation.name" = $operation,
            "gen_ai.request.model" = $model,
            "gen_ai.system" = $provider,
            "langfuse.observation.type" = "generation",
            $("langfuse.session.id" = $session,)?
        )
    };
    ($operation:expr $(, $key:ident = $value:expr)*) => {
        tracing::info_span!(
            $operation,
            "gen_ai.operation.name" = $operation,
            $($key = $value,)*
        )
    };
}

/// Record LLM usage metrics on the current span.
///
/// Call this after an LLM completion to record token usage.
/// Uses GenAI semantic conventions: prompt_tokens and completion_tokens.
///
/// Usage:
/// ```ignore
/// record_llm_usage!(prompt_tokens = 100, completion_tokens = 50);
/// ```
#[macro_export]
macro_rules! record_llm_usage {
    (prompt_tokens = $input:expr, completion_tokens = $output:expr $(, total_tokens = $total:expr)?) => {
        tracing::Span::current().record("gen_ai.usage.prompt_tokens", $input);
        tracing::Span::current().record("gen_ai.usage.completion_tokens", $output);
        $(tracing::Span::current().record("gen_ai.usage.total_tokens", $total);)?
    };
}

/// Record the prompt/input for an LLM call on the current span.
///
/// Usage:
/// ```ignore
/// record_llm_input!("What is the capital of France?");
/// ```
#[macro_export]
macro_rules! record_llm_input {
    ($input:expr) => {
        tracing::Span::current().record("gen_ai.prompt", $input);
    };
}

/// Record the completion/output for an LLM call on the current span.
///
/// Usage:
/// ```ignore
/// record_llm_output!("The capital of France is Paris.");
/// ```
#[macro_export]
macro_rules! record_llm_output {
    ($output:expr) => {
        tracing::Span::current().record("gen_ai.completion", $output);
    };
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn test_langfuse_config_default() {
        let config = LangfuseConfig::default();
        assert_eq!(config.host, "https://cloud.langfuse.com");
        assert_eq!(config.service_name, "golish");
        assert!((config.sampling_ratio - 1.0).abs() < f64::EPSILON);
    }

    #[test]
    fn test_from_env_missing_keys() {
        // Ensure the env vars are not set
        std::env::remove_var("LANGFUSE_PUBLIC_KEY");
        std::env::remove_var("LANGFUSE_SECRET_KEY");
        assert!(LangfuseConfig::from_env().is_none());
    }

    // Tests for field filtering
    #[test]
    fn test_should_filter_langfuse_fields() {
        // Langfuse fields should be filtered
        assert!(should_filter_field("langfuse.session.id"));
        assert!(should_filter_field("langfuse.observation.input"));
        assert!(should_filter_field("langfuse.observation.output"));
        assert!(should_filter_field("langfuse.observation.type"));
    }

    #[test]
    fn test_should_filter_gen_ai_fields() {
        // GenAI semantic convention fields should be filtered
        assert!(should_filter_field("gen_ai.request.model"));
        assert!(should_filter_field("gen_ai.system"));
        assert!(should_filter_field("gen_ai.prompt"));
        assert!(should_filter_field("gen_ai.completion"));
        assert!(should_filter_field("gen_ai.usage.prompt_tokens"));
        assert!(should_filter_field("gen_ai.usage.completion_tokens"));
        assert!(should_filter_field("gen_ai.operation.name"));
    }

    #[test]
    fn test_should_not_filter_regular_fields() {
        // Regular application fields should NOT be filtered
        assert!(!should_filter_field("model"));
        assert!(!should_filter_field("provider"));
        assert!(!should_filter_field("agent_type"));
        assert!(!should_filter_field("tool_name"));
        assert!(!should_filter_field("session_id")); // Without langfuse. prefix
        assert!(!should_filter_field("message"));
        assert!(!should_filter_field("error"));
        assert!(!should_filter_field("duration_ms"));
    }

    #[test]
    fn test_should_not_filter_similar_prefixes() {
        // Fields with similar but not matching prefixes should NOT be filtered
        assert!(!should_filter_field("langfuse_host")); // underscore, not dot
        assert!(!should_filter_field("gen_ai_model")); // underscore, not dot
        assert!(!should_filter_field("my_langfuse.field")); // prefix doesn't match
        assert!(!should_filter_field("the_gen_ai.field")); // prefix doesn't match
    }
}
