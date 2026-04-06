//! Web search and content fetching for Qbit AI.
//!
//! This crate provides:
//! - Tavily web search integration
//! - Web content fetching and extraction

pub mod brave;
pub mod tavily;
pub mod tool;
pub mod web_fetch;

pub use brave::BraveSearchState;
pub use tavily::TavilyState;
pub use tool::{
    create_brave_tools, create_tavily_tools, BraveSearchTool, WebCrawlTool, WebExtractTool,
    WebMapTool, WebSearchAnswerTool, WebSearchTool,
};
pub use web_fetch::{FetchResult, WebFetcher};
