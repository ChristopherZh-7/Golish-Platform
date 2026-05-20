//! Error types for the intel-providers crate.

use thiserror::Error;

pub type IntelResult<T> = Result<T, IntelError>;

#[derive(Debug, Error)]
pub enum IntelError {
    /// API key is missing, empty, or invalid format.
    #[error("missing or invalid API key for provider '{provider}': {reason}")]
    InvalidKey { provider: String, reason: String },

    /// Provider rejected the credentials (HTTP 401 / 403 / quota exhausted).
    #[error("provider '{provider}' rejected credentials: {reason}")]
    AuthFailed { provider: String, reason: String },

    /// Rate-limit / quota exceeded.
    #[error("provider '{provider}' rate limit / quota exhausted: {reason}")]
    QuotaExceeded { provider: String, reason: String },

    /// Network or HTTP-level error.
    #[error("network error talking to '{provider}': {source}")]
    Network {
        provider: String,
        #[source]
        source: reqwest::Error,
    },

    /// Provider returned an unexpected response shape.
    #[error("provider '{provider}' returned malformed response: {reason}")]
    BadResponse { provider: String, reason: String },

    /// User asked for a query_type that this provider does not support.
    #[error("provider '{provider}' does not support query_type '{query_type}'")]
    UnsupportedQueryType {
        provider: String,
        query_type: String,
    },

    /// Catch-all for anything that does not fit the categories above.
    #[error("intel provider error: {0}")]
    Other(String),
}

impl IntelError {
    /// Construct a Network error from a reqwest error.
    pub fn network(provider: impl Into<String>, source: reqwest::Error) -> Self {
        Self::Network {
            provider: provider.into(),
            source,
        }
    }

    /// Construct a BadResponse error.
    pub fn bad_response(provider: impl Into<String>, reason: impl Into<String>) -> Self {
        Self::BadResponse {
            provider: provider.into(),
            reason: reason.into(),
        }
    }
}
