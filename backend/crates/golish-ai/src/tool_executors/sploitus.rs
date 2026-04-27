//! Sploitus vulnerability database tool executor.
//!
//! Wraps [`golish_pentest::sploitus::SploitusClient`] so the LLM can call
//! `search_exploits` to look up exploits / tools / CVE entries directly,
//! avoiding ad-hoc `web_search` lookups.

use serde_json::json;

use golish_pentest::sploitus::SploitusClient;

use super::common::{error_result, extract_string_param, ToolResult};

/// Execute the `search_exploits` tool. Returns `None` for any other tool
/// name so the dispatcher can fall through to the next executor.
///
/// The Sploitus client is constructed per call (cheap — a `reqwest::Client`
/// behind an `Arc`); no shared state needs to be threaded into the bridge.
pub async fn execute_sploitus_tool(
    tool_name: &str,
    args: &serde_json::Value,
) -> Option<ToolResult> {
    if tool_name != "search_exploits" {
        return None;
    }

    let query = match extract_string_param(args, &["query", "q", "search_query"]) {
        Some(q) => q,
        None => {
            return Some(error_result(
                "search_exploits requires a non-empty 'query' parameter. \
                 Example: {\"query\": \"apache 2.4.49\"} or {\"query\": \"CVE-2021-41773\"}",
            ));
        }
    };
    let exploit_type = args
        .get("type")
        .and_then(|v| v.as_str())
        .map(String::from);
    let limit = args
        .get("limit")
        .and_then(|v| v.as_u64())
        .unwrap_or(10)
        .min(100) as usize;

    let client = SploitusClient::new();
    match client
        .search_exploits(&query, exploit_type.as_deref(), limit)
        .await
    {
        Ok(response) => {
            let entries: Vec<serde_json::Value> = response
                .exploits
                .iter()
                .map(|entry| {
                    json!({
                        "id": entry.id,
                        "title": entry.title,
                        "description": entry.description,
                        "source": entry.source,
                        "source_url": entry.source_url,
                        "published": entry.published,
                        "cve": entry.cve,
                        "score": entry.score,
                    })
                })
                .collect();
            let count = entries.len();
            Some((
                json!({
                    "query": query,
                    "type": exploit_type.as_deref().unwrap_or("exploits"),
                    "exploits": entries,
                    "count": count,
                    "total": response.total,
                }),
                true,
            ))
        }
        Err(e) => Some(error_result(format!(
            "Sploitus search failed: {e}. The exploit database may be temporarily unavailable; consider retrying or using web_search as a fallback."
        ))),
    }
}
