//! Vulnerability knowledge-base tool executor.
//!
//! Handles every `*_knowledge*` / `*_cve*` / `*poc*` tool call by routing to
//! a per-verb submodule:
//!
//! - [`search`] — `search_knowledge_base`
//! - [`read`] — `read_knowledge`
//! - [`save`] — `write_knowledge`, `ingest_cve`, `save_poc`
//! - [`query`] — `list_cves_with_pocs`, `list_unresearched_cves`, `poc_stats`
//!
//! Shared filesystem helpers (frontmatter parsing, link extraction, index
//! rebuild, `wiki_base_dir`) live in [`wiki`].
//!
//! Storage strategy: the markdown wiki on disk under
//! `<app_data>/wiki/` is the source of truth; PostgreSQL provides full-text
//! search and cross-reference tables. When the DB tracker is unavailable,
//! search falls back to a filesystem scan via
//! [`wiki::kb_search_filesystem_fallback`].

use crate::tool_executors::common::ToolResult;

mod query;
mod read;
mod save;
mod search;
mod wiki;

/// Execute a vulnerability knowledge-base tool. Returns `None` when the
/// tool name isn't a KB tool (so the caller can dispatch elsewhere).
pub async fn execute_knowledge_base_tool(
    tool_name: &str,
    args: &serde_json::Value,
    db_tracker: Option<&crate::db_tracking::DbTracker>,
) -> Option<ToolResult> {
    match tool_name {
        "search_knowledge_base" => Some(search::handle_search(args, db_tracker).await),
        "read_knowledge" => Some(read::handle_read(args).await),
        "write_knowledge" => Some(save::handle_write(args, db_tracker).await),
        "ingest_cve" => Some(save::handle_ingest_cve(args, db_tracker).await),
        "save_poc" => Some(save::handle_save_poc(args, db_tracker).await),
        "list_cves_with_pocs" => Some(query::handle_list_cves_with_pocs(db_tracker).await),
        "list_unresearched_cves" => Some(query::handle_list_unresearched(args, db_tracker).await),
        "poc_stats" => Some(query::handle_poc_stats(db_tracker).await),
        _ => None,
    }
}
