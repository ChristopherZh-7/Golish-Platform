//! Web fetch abstraction trait.
//!
//! Provides a provider-agnostic interface for fetching web content.
//! The concrete implementation lives in `golish-web`.

use async_trait::async_trait;

/// Result of a web fetch operation.
#[derive(Debug, Clone)]
pub struct WebFetchResult {
    pub url: String,
    pub content: String,
}

/// Provider trait for fetching web content.
///
/// `golish-ai` depends on this trait; the concrete implementation
/// (e.g. readability-based extraction) is injected by the application layer.
#[async_trait]
pub trait WebFetchProvider: Send + Sync {
    async fn fetch(&self, url: &str) -> anyhow::Result<WebFetchResult>;
}
