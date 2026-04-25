//! `search_knowledge_base` — full-text / category / tag search across the
//! markdown wiki. Falls back to a filesystem scan when the database is
//! unavailable.

use serde_json::json;

use crate::tool_executors::common::{error_result, extract_string_param, ToolResult};

use super::wiki::kb_search_filesystem_fallback;

pub(super) async fn handle_search(
    args: &serde_json::Value,
    db_tracker: Option<&crate::db_tracking::DbTracker>,
) -> ToolResult {
    let query = match extract_string_param(args, &["query", "q", "search"]) {
        Some(q) if !q.is_empty() => q,
        _ => return error_result(
            "search_knowledge_base requires a non-empty 'query' parameter",
        ),
    };
    let category = args
        .get("category")
        .and_then(|v| v.as_str())
        .map(String::from);
    let tag = args.get("tag").and_then(|v| v.as_str()).map(String::from);
    let limit = args
        .get("limit")
        .and_then(|v| v.as_i64())
        .unwrap_or(10)
        .min(50);

    let tracker = match db_tracker {
        Some(t) => t,
        None => {
            return kb_search_filesystem_fallback(&query, limit as usize).await;
        }
    };

    let pages = if let Some(cat) = category {
        golish_db::repo::wiki_kb::search_by_category(tracker.pool(), &cat, limit).await
    } else if let Some(t) = tag {
        golish_db::repo::wiki_kb::search_by_tag(tracker.pool(), &t, limit).await
    } else {
        golish_db::repo::wiki_kb::search_fts(tracker.pool(), &query, limit).await
    };

    match pages {
        Ok(results) => {
            let items: Vec<serde_json::Value> = results
                .iter()
                .map(|p| {
                    json!({
                        "path": p.path,
                        "title": p.title,
                        "category": p.category,
                        "tags": p.tags,
                        "status": p.status,
                        "snippet": p.content.chars().take(500).collect::<String>(),
                        "word_count": p.word_count,
                    })
                })
                .collect();
            let count = items.len();
            (json!({ "results": items, "count": count, "query": query }), true)
        }
        Err(e) => error_result(format!("KB search failed: {}", e)),
    }
}
