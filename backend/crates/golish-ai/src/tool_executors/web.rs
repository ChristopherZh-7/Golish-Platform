use serde_json::json;
use golish_web::web_fetch::WebFetcher;
use super::common::{error_result, ToolResult};

/// Execute a web fetch tool using readability-based content extraction.
pub async fn execute_web_fetch_tool(tool_name: &str, args: &serde_json::Value) -> ToolResult {
    if tool_name != "web_fetch" {
        return error_result(format!("Unknown web fetch tool: {}", tool_name));
    }

    let url = match args.get("url").and_then(|v| v.as_str()) {
        Some(u) => u.to_string(),
        None => {
            return error_result(
                "web_fetch requires a 'url' parameter (string). Example: {\"url\": \"https://example.com\"}"
            )
        }
    };

    let fetcher = WebFetcher::new();

    match fetcher.fetch(&url).await {
        Ok(result) => (
            json!({
                "url": result.url,
                "content": result.content
            }),
            true,
        ),
        Err(e) => error_result(format!("Failed to fetch {}: {}", url, e)),
    }
}
