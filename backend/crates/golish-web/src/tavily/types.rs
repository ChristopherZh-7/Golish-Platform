//! Tavily API request/response types and public result types.

use serde::{Deserialize, Serialize};

#[derive(Debug, Serialize)]
pub(super) struct TavilySearchRequest {
    pub(super) api_key: String,
    pub(super) query: String,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub(super) search_depth: Option<String>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub(super) chunks_per_source: Option<u32>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub(super) max_results: Option<u32>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub(super) topic: Option<String>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub(super) time_range: Option<String>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub(super) start_date: Option<String>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub(super) end_date: Option<String>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub(super) include_answer: Option<bool>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub(super) include_raw_content: Option<bool>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub(super) include_images: Option<bool>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub(super) include_image_descriptions: Option<bool>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub(super) include_favicon: Option<bool>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub(super) include_domains: Option<Vec<String>>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub(super) exclude_domains: Option<Vec<String>>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub(super) country: Option<String>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub(super) auto_parameters: Option<bool>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub(super) include_usage: Option<bool>,
}

#[derive(Debug, Serialize)]
#[serde(untagged)]
#[allow(dead_code)] // Single variant kept for API completeness
pub(super) enum TavilyUrls {
    Single(String),
    Array(Vec<String>),
}

#[derive(Debug, Serialize)]
pub(super) struct TavilyExtractRequest {
    pub(super) api_key: String,
    pub(super) urls: TavilyUrls,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub(super) query: Option<String>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub(super) chunks_per_source: Option<u32>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub(super) extract_depth: Option<String>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub(super) include_images: Option<bool>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub(super) include_favicon: Option<bool>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub(super) format: Option<String>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub(super) timeout: Option<u32>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub(super) include_usage: Option<bool>,
}

#[derive(Debug, Serialize)]
pub(super) struct TavilyCrawlRequest {
    pub(super) api_key: String,
    pub(super) url: String,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub(super) instructions: Option<String>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub(super) chunks_per_source: Option<u32>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub(super) max_depth: Option<u32>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub(super) max_breadth: Option<u32>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub(super) limit: Option<u32>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub(super) select_paths: Option<Vec<String>>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub(super) select_domains: Option<Vec<String>>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub(super) exclude_paths: Option<Vec<String>>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub(super) exclude_domains: Option<Vec<String>>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub(super) allow_external: Option<bool>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub(super) include_images: Option<bool>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub(super) extract_depth: Option<String>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub(super) format: Option<String>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub(super) include_favicon: Option<bool>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub(super) timeout: Option<u32>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub(super) include_usage: Option<bool>,
}

#[derive(Debug, Serialize)]
pub(super) struct TavilyMapRequest {
    pub(super) api_key: String,
    pub(super) url: String,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub(super) instructions: Option<String>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub(super) max_depth: Option<u32>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub(super) max_breadth: Option<u32>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub(super) limit: Option<u32>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub(super) select_paths: Option<Vec<String>>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub(super) select_domains: Option<Vec<String>>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub(super) exclude_paths: Option<Vec<String>>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub(super) exclude_domains: Option<Vec<String>>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub(super) allow_external: Option<bool>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub(super) timeout: Option<u32>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub(super) include_usage: Option<bool>,
}

// ============================================================================
// Response Types (Internal - from Tavily API)
// Fields marked dead_code are kept for API completeness and debugging
// ============================================================================

#[derive(Debug, Deserialize)]
#[allow(dead_code)]
pub(super) struct TavilySearchResponse {
    pub(super) query: String,
    #[serde(default)]
    pub(super) answer: Option<String>,
    pub(super) results: Vec<TavilySearchResult>,
    #[serde(default)]
    pub(super) images: Vec<String>,
    #[serde(default)]
    pub(super) usage: Option<serde_json::Value>,
}

#[derive(Debug, Deserialize)]
#[allow(dead_code)]
pub(super) struct TavilySearchResult {
    pub(super) title: String,
    pub(super) url: String,
    pub(super) content: String,
    pub(super) score: f64,
    #[serde(default)]
    pub(super) raw_content: Option<String>,
}

#[derive(Debug, Deserialize)]
pub(super) struct TavilyExtractResponse {
    pub(super) results: Vec<TavilyExtractResult>,
    #[serde(default)]
    pub(super) failed_results: Vec<TavilyFailedResult>,
}

#[derive(Debug, Deserialize)]
#[allow(dead_code)]
pub(super) struct TavilyExtractResult {
    pub(super) url: String,
    pub(super) raw_content: String,
    #[serde(default)]
    pub(super) images: Vec<String>,
}

#[derive(Debug, Deserialize)]
pub(super) struct TavilyCrawlResponse {
    pub(super) results: Vec<TavilyCrawlResult>,
    #[serde(default)]
    pub(super) failed_results: Vec<TavilyFailedResult>,
}

#[derive(Debug, Deserialize)]
#[allow(dead_code)]
pub(super) struct TavilyCrawlResult {
    pub(super) url: String,
    pub(super) raw_content: String,
    #[serde(default)]
    pub(super) images: Vec<String>,
}

#[derive(Debug, Deserialize)]
pub(super) struct TavilyMapResponse {
    pub(super) urls: Vec<String>,
    pub(super) base_url: String,
}

#[derive(Debug, Deserialize)]
#[allow(dead_code)]
pub(super) struct TavilyFailedResult {
    pub(super) url: String,
    #[serde(default)]
    pub(super) error: Option<String>,
}

// ============================================================================
// Public Result Types (for backward compatibility)
// ============================================================================

/// A single search result
#[derive(Debug, Clone)]
pub struct SearchResult {
    pub title: String,
    pub url: String,
    pub content: String,
    pub score: f64,
}

/// Search results container
#[derive(Debug)]
pub struct SearchResults {
    pub query: String,
    pub results: Vec<SearchResult>,
    pub answer: Option<String>,
}

/// Answer result with sources
#[derive(Debug)]
pub struct AnswerResult {
    pub query: String,
    pub answer: String,
    pub sources: Vec<SearchResult>,
}

/// A single extracted URL result
#[derive(Debug, Clone)]
pub struct ExtractResult {
    pub url: String,
    pub raw_content: String,
}

/// Extract results container
#[derive(Debug)]
pub struct ExtractResults {
    pub results: Vec<ExtractResult>,
    pub failed_urls: Vec<String>,
}

/// A single crawled URL result
#[derive(Debug, Clone)]
pub struct CrawlResult {
    pub url: String,
    pub raw_content: String,
}

/// Crawl results container
#[derive(Debug)]
pub struct CrawlResults {
    pub results: Vec<CrawlResult>,
    pub failed_urls: Vec<String>,
}

/// Map results container
#[derive(Debug)]
pub struct MapResults {
    pub urls: Vec<String>,
    pub base_url: String,
}
