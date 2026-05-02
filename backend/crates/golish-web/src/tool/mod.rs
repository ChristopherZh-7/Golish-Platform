//! Tool implementations for Tavily web search.
//!
//! These tools implement the `golish_core::Tool` trait for integration
//! with the Golish tool registry.

mod crawl;
pub use crawl::{BraveSearchTool, WebCrawlTool, WebMapTool, create_brave_tools, create_tavily_tools};

use std::path::Path;
use std::sync::Arc;

use anyhow::Result;
use golish_core::Tool;
use serde_json::{json, Value};

use crate::tavily::TavilyState;

use golish_core::utils::{get_optional_usize, get_required_str};

// ============================================================================
// web_search
// ============================================================================

/// Web search tool using Tavily API.
pub struct WebSearchTool {
    tavily: Arc<TavilyState>,
}

impl WebSearchTool {
    /// Create a new WebSearchTool with the given TavilyState.
    pub fn new(tavily: Arc<TavilyState>) -> Self {
        Self { tavily }
    }
}

#[async_trait::async_trait]
impl Tool for WebSearchTool {
    fn name(&self) -> &'static str {
        "tavily_search"
    }

    fn description(&self) -> &'static str {
        "Search the web for information. Returns relevant results with titles, URLs, and content snippets. \
         Use this when you need current information, news, documentation, or facts beyond your training data."
    }

    fn parameters(&self) -> Value {
        json!({
            "type": "object",
            "properties": {
                "query": {
                    "type": "string",
                    "description": "The search query"
                },
                "max_results": {
                    "type": "integer",
                    "description": "Maximum number of results to return (default: 5)"
                },
                "search_depth": {
                    "type": "string",
                    "enum": ["basic", "advanced"],
                    "description": "Search depth: 'basic' for quick results, 'advanced' for comprehensive search (default: basic)"
                },
                "topic": {
                    "type": "string",
                    "description": "Search topic category like 'general', 'news', etc."
                },
                "include_domains": {
                    "type": "array",
                    "items": { "type": "string" },
                    "description": "List of domains to include in search results"
                },
                "exclude_domains": {
                    "type": "array",
                    "items": { "type": "string" },
                    "description": "List of domains to exclude from search results"
                }
            },
            "required": ["query"]
        })
    }

    async fn execute(&self, args: Value, _workspace: &Path) -> Result<Value> {
        let query = match get_required_str(&args, "query") {
            Ok(q) => q,
            Err(e) => return Ok(e),
        };

        let max_results = get_optional_usize(&args, "max_results");

        match self.tavily.search(query, max_results).await {
            Ok(results) => Ok(json!({
                "query": results.query,
                "results": results.results.iter().map(|r| json!({
                    "title": r.title,
                    "url": r.url,
                    "content": r.content,
                    "score": r.score
                })).collect::<Vec<_>>(),
                "answer": results.answer,
                "count": results.results.len()
            })),
            Err(e) => Ok(json!({"error": e.to_string()})),
        }
    }
}

// ============================================================================
// web_search_answer
// ============================================================================

/// Web search tool that returns an AI-generated answer using Tavily API.
pub struct WebSearchAnswerTool {
    tavily: Arc<TavilyState>,
}

impl WebSearchAnswerTool {
    /// Create a new WebSearchAnswerTool with the given TavilyState.
    pub fn new(tavily: Arc<TavilyState>) -> Self {
        Self { tavily }
    }
}

#[async_trait::async_trait]
impl Tool for WebSearchAnswerTool {
    fn name(&self) -> &'static str {
        "tavily_search_answer"
    }

    fn description(&self) -> &'static str {
        "Get an AI-generated answer from web search results. \
         Best for direct questions that need a synthesized answer from multiple sources."
    }

    fn parameters(&self) -> Value {
        json!({
            "type": "object",
            "properties": {
                "query": {
                    "type": "string",
                    "description": "The question to answer"
                }
            },
            "required": ["query"]
        })
    }

    async fn execute(&self, args: Value, _workspace: &Path) -> Result<Value> {
        let query = match get_required_str(&args, "query") {
            Ok(q) => q,
            Err(e) => return Ok(e),
        };

        match self.tavily.answer(query).await {
            Ok(result) => Ok(json!({
                "query": result.query,
                "answer": result.answer,
                "sources": result.sources.iter().map(|r| json!({
                    "title": r.title,
                    "url": r.url,
                    "content": r.content,
                    "score": r.score
                })).collect::<Vec<_>>()
            })),
            Err(e) => Ok(json!({"error": e.to_string()})),
        }
    }
}

// ============================================================================
// web_extract
// ============================================================================

/// Web content extraction tool using Tavily API.
pub struct WebExtractTool {
    tavily: Arc<TavilyState>,
}

impl WebExtractTool {
    /// Create a new WebExtractTool with the given TavilyState.
    pub fn new(tavily: Arc<TavilyState>) -> Self {
        Self { tavily }
    }
}

#[async_trait::async_trait]
impl Tool for WebExtractTool {
    fn name(&self) -> &'static str {
        "tavily_extract"
    }

    fn description(&self) -> &'static str {
        "Extract and parse content from specific URLs. \
         Use this to get the full content of web pages for deeper analysis."
    }

    fn parameters(&self) -> Value {
        json!({
            "type": "object",
            "properties": {
                "urls": {
                    "type": "array",
                    "items": { "type": "string" },
                    "description": "List of URLs to extract content from"
                },
                "query": {
                    "type": "string",
                    "description": "Optional query to focus extraction on specific information"
                },
                "extract_depth": {
                    "type": "string",
                    "enum": ["basic", "advanced"],
                    "description": "Extraction depth: 'basic' for quick extraction, 'advanced' for comprehensive extraction"
                },
                "format": {
                    "type": "string",
                    "enum": ["markdown", "text"],
                    "description": "Output format for extracted content (default: markdown)"
                }
            },
            "required": ["urls"]
        })
    }

    async fn execute(&self, args: Value, _workspace: &Path) -> Result<Value> {
        let urls: Vec<String> = args
            .get("urls")
            .and_then(|v| v.as_array())
            .map(|arr| {
                arr.iter()
                    .filter_map(|v| v.as_str().map(|s| s.to_string()))
                    .collect()
            })
            .unwrap_or_default();

        if urls.is_empty() {
            return Ok(json!({"error": "Missing required argument: urls"}));
        }

        match self.tavily.extract(urls).await {
            Ok(results) => Ok(json!({
                "results": results.results.iter().map(|r| json!({
                    "url": r.url,
                    "content": r.raw_content
                })).collect::<Vec<_>>(),
                "failed_urls": results.failed_urls,
                "count": results.results.len()
            })),
            Err(e) => Ok(json!({"error": e.to_string()})),
        }
    }
}

