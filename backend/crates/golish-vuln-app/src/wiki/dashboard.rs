//! Karpathy-style wiki dashboard commands.
//!
//! These read-only endpoints power the at-a-glance wiki overview UI:
//! - [`wiki_pages_grouped`]   — every page bucketed by category.
//! - [`wiki_pages_for_paths`] — bulk lookup for a set of relative paths.
//! - [`wiki_suggest_for_cve`] — page suggestions seeded from a CVE id.
//! - [`wiki_changelog_list`]  — recent edits across the wiki.
//! - [`wiki_backlinks`]       — pages that link to a given page.
//! - [`wiki_stats_full`]      — aggregate count + size + status mix.
//! - [`wiki_orphan_pages`]    — pages with no inbound links, sorted by
//!   age so they bubble up for cleanup.

use golish_app_core::GolishError;
use serde::{Deserialize, Serialize};

use golish_app_core::DbState;

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct WikiPageInfo {
    pub path: String,
    pub title: String,
    pub category: String,
    pub tags: Vec<String>,
    pub status: String,
    pub word_count: i32,
    pub updated_at: String,
}

/// DB summary → API DTO.  Centralised because every dashboard endpoint
/// returns the same shape and would otherwise duplicate this conversion.
fn summary_to_info(s: golish_db::models::WikiPageSummary) -> WikiPageInfo {
    WikiPageInfo {
        path: s.path,
        title: s.title,
        category: s.category,
        tags: s.tags,
        status: s.status,
        word_count: s.word_count,
        updated_at: s.updated_at.to_rfc3339(),
    }
}

#[tauri::command]
pub async fn wiki_pages_grouped(
    state: tauri::State<'_, DbState>,
) -> Result<Vec<WikiPageInfo>, GolishError> {
    let pool = state.pool_ready().await?;
    let pages = golish_db::repo::wiki_kb::list_pages_grouped_by_category(pool).await?;
    Ok(pages.into_iter().map(summary_to_info).collect())
}

#[tauri::command]
pub async fn wiki_pages_for_paths(
    state: tauri::State<'_, DbState>,
    paths: Vec<String>,
) -> Result<Vec<WikiPageInfo>, GolishError> {
    let pool = state.pool_ready().await?;
    let pages = golish_db::repo::wiki_kb::list_pages_for_paths(pool, &paths).await?;
    Ok(pages.into_iter().map(summary_to_info).collect())
}

#[tauri::command]
pub async fn wiki_suggest_for_cve(
    state: tauri::State<'_, DbState>,
    cve_id: String,
    limit: Option<i64>,
) -> Result<Vec<WikiPageInfo>, GolishError> {
    let pool = state.pool_ready().await?;
    let pages =
        golish_db::repo::wiki_kb::suggest_pages_for_cve(pool, &cve_id, limit.unwrap_or(10)).await?;
    Ok(pages.into_iter().map(summary_to_info).collect())
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct WikiChangelogEntry {
    pub id: i64,
    pub page_path: String,
    pub action: String,
    pub title: String,
    pub category: String,
    pub actor: String,
    pub summary: String,
    pub created_at: String,
}

#[tauri::command]
pub async fn wiki_changelog_list(
    state: tauri::State<'_, DbState>,
    limit: Option<i64>,
) -> Result<Vec<WikiChangelogEntry>, GolishError> {
    let pool = state.pool_ready().await?;
    let entries = golish_db::repo::wiki_kb::list_changelog(pool, limit.unwrap_or(50)).await?;
    Ok(entries
        .into_iter()
        .map(|e| WikiChangelogEntry {
            id: e.id,
            page_path: e.page_path,
            action: e.action,
            title: e.title,
            category: e.category,
            actor: e.actor,
            summary: e.summary,
            created_at: e.created_at.to_rfc3339(),
        })
        .collect())
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct WikiBacklink {
    pub source_path: String,
    pub context: String,
}

#[tauri::command]
pub async fn wiki_backlinks(
    state: tauri::State<'_, DbState>,
    path: String,
) -> Result<Vec<WikiBacklink>, GolishError> {
    let pool = state.pool_ready().await?;
    let refs = golish_db::repo::wiki_kb::get_backlinks(pool, &path).await?;
    Ok(refs
        .into_iter()
        .map(|r| WikiBacklink {
            source_path: r.source_path,
            context: r.context,
        })
        .collect())
}

#[tauri::command]
pub async fn wiki_stats_full(
    state: tauri::State<'_, DbState>,
) -> Result<serde_json::Value, GolishError> {
    let pool = state.pool_ready().await?;
    golish_db::repo::wiki_kb::wiki_stats_full(pool)
        .await
        .map_err(GolishError::from)
}

#[tauri::command]
pub async fn wiki_orphan_pages(
    state: tauri::State<'_, DbState>,
    limit: Option<i64>,
) -> Result<Vec<WikiPageInfo>, GolishError> {
    let pool = state.pool_ready().await?;
    let pages = golish_db::repo::wiki_kb::list_orphan_pages(pool, limit.unwrap_or(20)).await?;
    Ok(pages.into_iter().map(summary_to_info).collect())
}
