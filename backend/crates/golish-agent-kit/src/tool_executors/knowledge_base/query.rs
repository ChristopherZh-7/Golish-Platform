//! Read-only listing / aggregate queries over the KB:
//!
//! - `list_cves_with_pocs`
//! - `list_unresearched_cves`
//! - `poc_stats`

use crate::tool_executors::common::{error_result, ToolResult};

pub(super) async fn handle_list_cves_with_pocs(
    db_tracker: Option<&crate::db_tracking::DbTracker>,
) -> ToolResult {
    let Some(tracker) = db_tracker else {
        return error_result("Database not available");
    };
    let Some(repo) = tracker.repo() else {
        return error_result("Database repository not available");
    };

    match crate::db_shim::wiki_kb::list_cves_with_pocs(repo).await {
        Ok(result) => (result, true),
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
    let Some(repo) = tracker.repo() else {
        return error_result("Database repository not available");
    };

    match crate::db_shim::wiki_kb::list_unresearched_cves(repo, limit).await {
        Ok(result) => (result, true),
        Err(e) => error_result(format!("Failed to list unresearched CVEs: {}", e)),
    }
}

pub(super) async fn handle_poc_stats(
    db_tracker: Option<&crate::db_tracking::DbTracker>,
) -> ToolResult {
    let Some(tracker) = db_tracker else {
        return error_result("Database not available");
    };
    let Some(repo) = tracker.repo() else {
        return error_result("Database repository not available");
    };

    match crate::db_shim::wiki_kb::poc_stats(repo).await {
        Ok(stats) => (stats, true),
        Err(e) => error_result(format!("Failed to get PoC stats: {}", e)),
    }
}
