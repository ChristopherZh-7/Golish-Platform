//! Web crawl, map, and Brave search tool implementations.

use std::path::Path;
use std::sync::Arc;

use anyhow::Result;
use golish_core::Tool;
use serde_json::{json, Value};

use crate::brave::BraveSearchState;
use crate::tavily::TavilyState;

use golish_core::utils::{get_optional_u32, get_required_str};
use super::{WebSearchTool, WebSearchAnswerTool, WebExtractTool};

/// Web crawling tool using Tavily API.
pub struct WebCrawlTool {
    tavily: Arc<TavilyState>,
}

impl WebCrawlTool {
    /// Create a new WebCrawlTool with the given TavilyState.
    pub fn new(tavily: Arc<TavilyState>) -> Self {
        Self { tavily }
    }
}

#[async_trait::async_trait]
impl Tool for WebCrawlTool {
    fn name(&self) -> &'static str {
        "tavily_crawl"
    }

    fn description(&self) -> &'static str {
        "Crawl a website starting from a URL, following links to extract content from multiple pages. \
         Use for comprehensive site analysis."
    }

    fn parameters(&self) -> Value {
        json!({
            "type": "object",
            "properties": {
                "url": {
                    "type": "string",
                    "description": "Base URL to start crawling from"
                },
                "max_depth": {
                    "type": "integer",
                    "description": "Maximum crawl depth (levels of links to follow)"
                },
                "max_breadth": {
                    "type": "integer",
                    "description": "Maximum pages to crawl per level"
                },
                "limit": {
                    "type": "integer",
                    "description": "Maximum total pages to crawl"
                },
                "instructions": {
                    "type": "string",
                    "description": "Natural language instructions for what to focus on during crawling"
                },
                "allow_external": {
                    "type": "boolean",
                    "description": "Whether to follow external links outside the base domain"
                }
            },
            "required": ["url"]
        })
    }

    async fn execute(&self, args: Value, _workspace: &Path) -> Result<Value> {
        let url = match get_required_str(&args, "url") {
            Ok(u) => u.to_string(),
            Err(e) => return Ok(e),
        };

        let max_depth = get_optional_u32(&args, "max_depth");

        match self.tavily.crawl(url, max_depth).await {
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

// ============================================================================
// web_map
// ============================================================================

/// Website structure mapping tool using Tavily API.
pub struct WebMapTool {
    tavily: Arc<TavilyState>,
}

impl WebMapTool {
    /// Create a new WebMapTool with the given TavilyState.
    pub fn new(tavily: Arc<TavilyState>) -> Self {
        Self { tavily }
    }
}

#[async_trait::async_trait]
impl Tool for WebMapTool {
    fn name(&self) -> &'static str {
        "tavily_map"
    }

    fn description(&self) -> &'static str {
        "Map the structure of a website, returning a list of URLs found. \
         Use to discover site structure before crawling or extracting specific pages."
    }

    fn parameters(&self) -> Value {
        json!({
            "type": "object",
            "properties": {
                "url": {
                    "type": "string",
                    "description": "Base URL to map"
                },
                "max_depth": {
                    "type": "integer",
                    "description": "Maximum depth to explore"
                },
                "max_breadth": {
                    "type": "integer",
                    "description": "Maximum links to follow per level"
                },
                "limit": {
                    "type": "integer",
                    "description": "Maximum URLs to return"
                },
                "instructions": {
                    "type": "string",
                    "description": "Natural language instructions for what to focus on during mapping"
                }
            },
            "required": ["url"]
        })
    }

    async fn execute(&self, args: Value, _workspace: &Path) -> Result<Value> {
        let url = match get_required_str(&args, "url") {
            Ok(u) => u.to_string(),
            Err(e) => return Ok(e),
        };

        let max_depth = get_optional_u32(&args, "max_depth");

        match self.tavily.map(url, max_depth).await {
            Ok(results) => Ok(json!({
                "urls": results.urls,
                "base_url": results.base_url,
                "count": results.urls.len()
            })),
            Err(e) => Ok(json!({"error": e.to_string()})),
        }
    }
}

// ============================================================================
// brave_search
// ============================================================================

pub struct BraveSearchTool {
    brave: Arc<BraveSearchState>,
}

impl BraveSearchTool {
    pub fn new(brave: Arc<BraveSearchState>) -> Self {
        Self { brave }
    }
}

#[async_trait::async_trait]
impl Tool for BraveSearchTool {
    fn name(&self) -> &'static str {
        "brave_search"
    }

    fn description(&self) -> &'static str {
        "Search the web using Brave Search API. Returns relevant results with titles, URLs, and descriptions. \
         Use for privacy-focused web search with no tracking."
    }

    fn parameters(&self) -> Value {
        json!({
            "type": "object",
            "properties": {
                "query": {
                    "type": "string",
                    "description": "The search query"
                },
                "count": {
                    "type": "integer",
                    "description": "Number of results to return (default: 10, max: 20)",
                    "default": 10
                },
                "freshness": {
                    "type": "string",
                    "description": "Freshness filter: 'pd' (past day), 'pw' (past week), 'pm' (past month)",
                    "enum": ["pd", "pw", "pm"]
                }
            },
            "required": ["query"]
        })
    }

    async fn execute(&self, args: Value, _workspace_path: &Path) -> Result<Value> {
        let query = args
            .get("query")
            .and_then(|v| v.as_str())
            .ok_or_else(|| anyhow::anyhow!("Missing required argument: query"))?;
        let count = get_optional_u32(&args, "count");
        let freshness = args.get("freshness").and_then(|v| v.as_str());

        match self.brave.web_search(query, count, freshness).await {
            Ok(results) => Ok(json!({
                "query": results.query,
                "results": results.results,
                "infobox": results.infobox,
                "result_count": results.results.len()
            })),
            Err(e) => Ok(json!({"error": e.to_string()})),
        }
    }
}

// ============================================================================
// Helper functions for tool registration
// ============================================================================

/// Create all Tavily tools with shared state.
/// Tools are registered even if API key is missing; errors occur at execution time.
pub fn create_tavily_tools(tavily: Arc<TavilyState>) -> Vec<std::sync::Arc<dyn Tool>> {
    vec![
        std::sync::Arc::new(WebSearchTool::new(tavily.clone())),
        std::sync::Arc::new(WebSearchAnswerTool::new(tavily.clone())),
        std::sync::Arc::new(WebExtractTool::new(tavily.clone())),
        std::sync::Arc::new(WebCrawlTool::new(tavily.clone())),
        std::sync::Arc::new(WebMapTool::new(tavily)),
    ]
}

/// Create Brave Search tool with shared state.
pub fn create_brave_tools(brave: Arc<BraveSearchState>) -> Vec<std::sync::Arc<dyn Tool>> {
    vec![std::sync::Arc::new(BraveSearchTool::new(brave))]
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn test_web_search_tool_metadata() {
        let tavily = Arc::new(TavilyState::from_api_key(None));
        let tool = WebSearchTool::new(tavily);

        assert_eq!(tool.name(), "tavily_search");
        assert!(!tool.description().is_empty());

        let params = tool.parameters();
        assert_eq!(params["type"], "object");
        assert!(params["properties"]["query"].is_object());
        assert!(params["required"]
            .as_array()
            .unwrap()
            .contains(&json!("query")));
    }

    #[test]
    fn test_web_search_answer_tool_metadata() {
        let tavily = Arc::new(TavilyState::from_api_key(None));
        let tool = WebSearchAnswerTool::new(tavily);

        assert_eq!(tool.name(), "tavily_search_answer");
        assert!(!tool.description().is_empty());
    }

    #[test]
    fn test_web_extract_tool_metadata() {
        let tavily = Arc::new(TavilyState::from_api_key(None));
        let tool = WebExtractTool::new(tavily);

        assert_eq!(tool.name(), "tavily_extract");
        assert!(!tool.description().is_empty());

        let params = tool.parameters();
        assert!(params["properties"]["urls"].is_object());
    }

    #[test]
    fn test_web_crawl_tool_metadata() {
        let tavily = Arc::new(TavilyState::from_api_key(None));
        let tool = WebCrawlTool::new(tavily);

        assert_eq!(tool.name(), "tavily_crawl");
        assert!(!tool.description().is_empty());

        let params = tool.parameters();
        assert!(params["properties"]["url"].is_object());
    }

    #[test]
    fn test_web_map_tool_metadata() {
        let tavily = Arc::new(TavilyState::from_api_key(None));
        let tool = WebMapTool::new(tavily);

        assert_eq!(tool.name(), "tavily_map");
        assert!(!tool.description().is_empty());

        let params = tool.parameters();
        assert!(params["properties"]["url"].is_object());
    }

    #[test]
    fn test_create_tavily_tools_always_returns_all_tools() {
        let tavily = Arc::new(TavilyState::from_api_key(None));
        let tools = create_tavily_tools(tavily);

        let names: Vec<String> = tools.iter().map(|t| t.name().to_string()).collect();
        assert!(names.contains(&"tavily_search".to_string()));
        assert!(names.contains(&"tavily_search_answer".to_string()));
        assert!(names.contains(&"tavily_extract".to_string()));
        assert!(names.contains(&"tavily_crawl".to_string()));
        assert!(names.contains(&"tavily_map".to_string()));
    }
}
