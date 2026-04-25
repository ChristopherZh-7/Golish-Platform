//! Langfuse exporter configuration loader.

/// Langfuse configuration for OpenTelemetry tracing.
#[derive(Debug, Clone)]
pub struct LangfuseConfig {
    /// Langfuse public key
    pub public_key: String,
    /// Langfuse secret key
    pub secret_key: String,
    /// Langfuse host URL
    pub host: String,
    /// Service name for this application
    pub service_name: String,
    /// Service version
    pub service_version: String,
    /// Sampling ratio (0.0 to 1.0, default 1.0 = sample everything)
    pub sampling_ratio: f64,
}

impl Default for LangfuseConfig {
    fn default() -> Self {
        Self {
            public_key: String::new(),
            secret_key: String::new(),
            host: "https://cloud.langfuse.com".to_string(),
            service_name: "golish".to_string(),
            service_version: env!("CARGO_PKG_VERSION").to_string(),
            sampling_ratio: 1.0,
        }
    }
}

impl LangfuseConfig {
    /// Create config from environment variables.
    ///
    /// Reads from:
    /// - `LANGFUSE_PUBLIC_KEY` (required)
    /// - `LANGFUSE_SECRET_KEY` (required)
    /// - `LANGFUSE_HOST` (optional, defaults to https://cloud.langfuse.com)
    pub fn from_env() -> Option<Self> {
        let public_key = std::env::var("LANGFUSE_PUBLIC_KEY").ok()?;
        let secret_key = std::env::var("LANGFUSE_SECRET_KEY").ok()?;

        if public_key.is_empty() || secret_key.is_empty() {
            return None;
        }

        Some(Self {
            public_key,
            secret_key,
            host: std::env::var("LANGFUSE_HOST")
                .unwrap_or_else(|_| "https://cloud.langfuse.com".to_string()),
            ..Default::default()
        })
    }

    /// Create config from settings.
    pub fn from_settings(settings: &crate::settings::LangfuseSettings) -> Option<Self> {
        if !settings.enabled {
            return None;
        }

        // Resolve public key from settings or environment
        let public_key = crate::settings::get_with_env_fallback(
            &settings.public_key,
            &["LANGFUSE_PUBLIC_KEY"],
            None,
        )?;

        // Resolve secret key from settings or environment
        let secret_key = crate::settings::get_with_env_fallback(
            &settings.secret_key,
            &["LANGFUSE_SECRET_KEY"],
            None,
        )?;

        if public_key.is_empty() || secret_key.is_empty() {
            return None;
        }

        Some(Self {
            public_key,
            secret_key,
            host: settings
                .host
                .clone()
                .unwrap_or_else(|| "https://cloud.langfuse.com".to_string()),
            service_name: "golish".to_string(),
            service_version: env!("CARGO_PKG_VERSION").to_string(),
            sampling_ratio: settings.sampling_ratio.unwrap_or(1.0),
        })
    }
}
