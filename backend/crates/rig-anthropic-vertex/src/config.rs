//! Configuration types and beta-feature flags for the Anthropic Vertex provider.
//!
//! Kept as small POD structs so they can be mutated by the builder methods on
//! `CompletionModel` (`with_thinking`, `with_web_search_config`, …) without
//! touching the request/response code paths.  The beta-header constants live
//! here too because they're consumed only by `CompletionModel::beta_header_value`.

/// Beta header for web fetch feature
pub(crate) const WEB_FETCH_BETA: &str = "web-fetch-2025-09-10";

/// Beta header for 1M token context window
pub(crate) const CONTEXT_1M_BETA: &str = "context-1m-2025-08-07";

/// Configuration for native web search
#[derive(Debug, Clone)]
pub struct WebSearchConfig {
    /// Maximum number of searches per request
    pub max_uses: Option<u32>,
    /// Only include results from these domains
    pub allowed_domains: Option<Vec<String>>,
    /// Never include results from these domains
    pub blocked_domains: Option<Vec<String>>,
}

impl Default for WebSearchConfig {
    fn default() -> Self {
        Self {
            max_uses: Some(5),
            allowed_domains: None,
            blocked_domains: None,
        }
    }
}

/// Configuration for native web fetch
#[derive(Debug, Clone)]
pub struct WebFetchConfig {
    /// Maximum number of fetches per request
    pub max_uses: Option<u32>,
    /// Enable citations for fetched content
    pub citations_enabled: bool,
    /// Maximum content length in tokens
    pub max_content_tokens: Option<u32>,
}

impl Default for WebFetchConfig {
    fn default() -> Self {
        Self {
            max_uses: Some(10),
            citations_enabled: true,
            max_content_tokens: Some(100000),
        }
    }
}

/// Server tools configuration for Claude's native tools
#[derive(Debug, Clone, Default)]
pub struct ServerToolsConfig {
    /// Native web search configuration
    pub web_search: Option<WebSearchConfig>,
    /// Native web fetch configuration
    pub web_fetch: Option<WebFetchConfig>,
}

/// Default max tokens for different Claude models.
///
/// Opus models get a much larger budget than Sonnet/Haiku because they're
/// expected to produce longer reasoning traces; everything else falls back
/// to `types::DEFAULT_MAX_TOKENS`.
pub(crate) fn default_max_tokens_for_model(model: &str) -> u32 {
    if model.contains("opus") {
        32000
    } else if model.contains("sonnet") || model.contains("haiku") {
        8192
    } else {
        crate::types::DEFAULT_MAX_TOKENS
    }
}
