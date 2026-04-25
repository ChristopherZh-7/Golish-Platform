//! Claude-native server tools (web_search, web_fetch) and their result
//! content union types.

use serde::{Deserialize, Serialize};

use super::request::ToolDefinition;

/// Configuration for citations in web fetch results.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct CitationsConfig {
    pub enabled: bool,
}

/// Server tool definitions for Claude's native tools. These use a
/// type-based format instead of the function-based format.
#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(tag = "type")]
pub enum ServerTool {
    /// Native web search tool (`web_search_20250305`).
    #[serde(rename = "web_search_20250305")]
    WebSearch {
        name: String,
        #[serde(skip_serializing_if = "Option::is_none")]
        max_uses: Option<u32>,
        #[serde(skip_serializing_if = "Option::is_none")]
        allowed_domains: Option<Vec<String>>,
        #[serde(skip_serializing_if = "Option::is_none")]
        blocked_domains: Option<Vec<String>>,
    },
    /// Native web fetch tool (`web_fetch_20250910`).
    #[serde(rename = "web_fetch_20250910")]
    WebFetch {
        name: String,
        #[serde(skip_serializing_if = "Option::is_none")]
        max_uses: Option<u32>,
        #[serde(skip_serializing_if = "Option::is_none")]
        citations: Option<CitationsConfig>,
        #[serde(skip_serializing_if = "Option::is_none")]
        max_content_tokens: Option<u32>,
    },
}

/// Union type for the tools array in API requests. Can contain both
/// function tools and server tools.
#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(untagged)]
pub enum ToolEntry {
    /// Traditional function-based tool definition.
    Function(ToolDefinition),
    /// Server-side tool (web_search, web_fetch).
    Server(ServerTool),
}

// ============================================================================
// Server Tool Result Types
// ============================================================================

/// Web search result from Claude's native web search.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct WebSearchResult {
    /// URL of the search result.
    pub url: String,
    /// Title of the page.
    pub title: String,
    /// Encrypted content (must be passed back for citations).
    #[serde(skip_serializing_if = "Option::is_none")]
    pub encrypted_content: Option<String>,
    /// When the page was last updated.
    #[serde(skip_serializing_if = "Option::is_none")]
    pub page_age: Option<String>,
}

/// Document source in web fetch result.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct WebFetchDocumentSource {
    #[serde(rename = "type")]
    pub source_type: String,
    pub media_type: String,
    pub data: String,
}

/// Document content in web fetch result.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct WebFetchDocument {
    #[serde(rename = "type")]
    pub doc_type: String,
    pub source: WebFetchDocumentSource,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub title: Option<String>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub citations: Option<CitationsConfig>,
}

/// Web fetch result content.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct WebFetchResultContent {
    #[serde(rename = "type")]
    pub content_type: String,
    pub url: String,
    pub content: WebFetchDocument,
    pub retrieved_at: String,
}

/// Web search tool result content (can be results or error).
#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(untagged)]
pub enum WebSearchToolResultContent {
    /// Successful search results.
    Results(Vec<WebSearchResult>),
    /// Error response.
    Error(WebToolError),
}

/// Web fetch tool result content (can be result or error).
#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(untagged)]
pub enum WebFetchToolResultContent {
    /// Successful fetch result.
    Result(WebFetchResultContent),
    /// Error response.
    Error(WebToolError),
}

/// Error from web tools.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct WebToolError {
    #[serde(rename = "type")]
    pub error_type: String,
    pub error_code: String,
}

/// Citation from web search or fetch.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct WebCitation {
    #[serde(rename = "type")]
    pub citation_type: String,
    pub url: String,
    pub title: String,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub encrypted_index: Option<String>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub cited_text: Option<String>,
}
