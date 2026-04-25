//! Read-only listing / aggregate queries over the KB:
//!
//! - `list_cves_with_pocs`
//! - `list_unresearched_cves`
//! - `poc_stats`

use serde_json::json;

use crate::tool_executors::common::{error_result, ToolResult};

pub(super) async fn handle_list_cves_with_pocs(
    db_tracker: Option<&crate::db_tracking::DbTracker>,
) -> ToolResult {
    let Some(tracker) = db_tracker else {
        return error_result("Database not available");
    };

    match golish_db::repo::wiki_kb::list_cves_with_pocs(tracker.pool()).await {
        Ok(rows) => {
            let items: Vec<serde_json::Value> = rows
                .iter()
                .map(|r| {
                    json!({
                        "cve_id": r.cve_id,
                        "poc_count": r.poc_count,
                        "max_severity": r.max_severity,
                        "any_verified": r.any_verified,
                        "has_research": r.has_research,
                        "has_wiki": r.has_wiki,
                    })
                })
                .collect();
            (
                json!({
                    "total": items.len(),
                    "cves": items,
                }),
                true,
            )
        }
        Err(e) => error_result(format!("Failed to list CVEs: {}", e)),
    }
}

pub(super) async fn handle_list_unresearched(
    args: &serde_json::Value,
    db_tracker: Option<&crate::db_tracking::DbTracker>,
) -> ToolResult {
    let limit = args.get("limit").and_then(|v| v.as_i64()).unwrap_or(20);

    let Some(tracker) = db_tracker else {
        return error_result("Database not available");
    };

    match golish_db::repo::wiki_kb::list_unresearched_cves(tracker.pool(), limit).await {
        Ok(rows) => {
            let items: Vec<serde_json::Value> = rows
                .iter()
                .map(|r| {
                    json!({
                        "cve_id": r.cve_id,
                        "poc_count": r.poc_count,
                        "max_severity": r.max_severity,
                        "any_verified": r.any_verified,
                    })
                })
                .collect();
            (
                json!({
                    "total": items.len(),
                    "message": format!(
                        "{} CVEs have PoCs but no research yet — prioritize by severity",
                        items.len()
                    ),
                    "cves": items,
                }),
                true,
            )
        }
        Err(e) => error_result(format!("Failed to list unresearched CVEs: {}", e)),
    }
}

pub(super) async fn handle_poc_stats(
    db_tracker: Option<&crate::db_tracking::DbTracker>,
) -> ToolResult {
    let Some(tracker) = db_tracker else {
        return error_result("Database not available");
    };

    match golish_db::repo::wiki_kb::poc_stats(tracker.pool()).await {
        Ok(stats) => (stats, true),
        Err(e) => error_result(format!("Failed to get PoC stats: {}", e)),
    }
}
