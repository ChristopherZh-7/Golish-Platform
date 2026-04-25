//! Wiki search — both filesystem-grep and Postgres FTS.
//!
//! Two search surfaces with different trade-offs are kept here side by side:
//! - [`wiki_search`]    — synchronous-ish recursive walk over `<wiki>/`,
//!   matches filename + line content, returns positional hits.  Used as a
//!   fallback when the DB isn't available or hasn't been indexed yet.
//! - [`wiki_search_db`] — full-text-search over the `wiki_pages` table
//!   built from on-write upserts in [`super::pages`].  Supports
//!   category / tag filters and is the primary search path in the UI.
//! - [`wiki_stats`]     — small count + recent-pages summary used by the
//!   home dashboard.

use serde::{Deserialize, Serialize};
use tokio::fs;

use crate::state::AppState;

use super::{is_text_searchable, wiki_base_dir};

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct WikiSearchResult {
    pub path: String,
    pub name: String,
    pub line: usize,
    pub content: String,
}

#[tauri::command]
pub async fn wiki_search(query: String) -> Result<Vec<WikiSearchResult>, String> {
    let base = wiki_base_dir();
    if !base.exists() {
        return Ok(Vec::new());
    }
    let query_lower = query.to_lowercase();
    let mut results = Vec::new();
    let mut stack = vec![base.clone()];

    while let Some(dir) = stack.pop() {
        let Ok(mut rd) = fs::read_dir(&dir).await else {
            continue;
        };
        while let Ok(Some(entry)) = rd.next_entry().await {
            let name = entry.file_name().to_string_lossy().to_string();
            if name.starts_with('.') {
                continue;
            }
            let path = entry.path();
            if path.is_dir() {
                stack.push(path);
                continue;
            }
            if !is_text_searchable(&name) {
                continue;
            }
            let rel = path
                .strip_prefix(&base)
                .unwrap_or(&path)
                .to_string_lossy()
                .to_string();

            if name.to_lowercase().contains(&query_lower) {
                results.push(WikiSearchResult {
                    path: rel.clone(),
                    name: name.clone(),
                    line: 0,
                    content: name.clone(),
                });
            }

            if let Ok(content) = fs::read_to_string(&path).await {
                for (i, line) in content.lines().enumerate() {
                    if line.to_lowercase().contains(&query_lower) {
                        results.push(WikiSearchResult {
                            path: rel.clone(),
                            name: name.clone(),
                            line: i + 1,
                            content: line.chars().take(200).collect(),
                        });
                        if results.len() >= 100 {
                            return Ok(results);
                        }
                    }
                }
            }
        }
    }

    Ok(results)
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct WikiSearchResultDb {
    pub path: String,
    pub title: String,
    pub category: String,
    pub tags: Vec<String>,
    pub snippet: String,
    pub word_count: i32,
}

#[tauri::command]
pub async fn wiki_search_db(
    state: tauri::State<'_, AppState>,
    query: String,
    category: Option<String>,
    tag: Option<String>,
    limit: Option<i64>,
) -> Result<Vec<WikiSearchResultDb>, String> {
    let pool = state.db_pool_ready().await?;
    let max = limit.unwrap_or(20);

    let pages = if let Some(cat) = category {
        golish_db::repo::wiki_kb::search_by_category(pool, &cat, max)
            .await
            .map_err(|e| e.to_string())?
    } else if let Some(t) = tag {
        golish_db::repo::wiki_kb::search_by_tag(pool, &t, max)
            .await
            .map_err(|e| e.to_string())?
    } else {
        golish_db::repo::wiki_kb::search_fts(pool, &query, max)
            .await
            .map_err(|e| e.to_string())?
    };

    Ok(pages
        .into_iter()
        .map(|p| {
            let snippet = p.content.chars().take(300).collect();
            WikiSearchResultDb {
                path: p.path,
                title: p.title,
                category: p.category,
                tags: p.tags,
                snippet,
                word_count: p.word_count,
            }
        })
        .collect())
}

#[tauri::command]
pub async fn wiki_stats(
    state: tauri::State<'_, AppState>,
) -> Result<serde_json::Value, String> {
    let pool = state.db_pool_ready().await?;
    let count = golish_db::repo::wiki_kb::count_pages(pool)
        .await
        .map_err(|e| e.to_string())?;
    let recent = golish_db::repo::wiki_kb::list_recent(pool, 5)
        .await
        .map_err(|e| e.to_string())?;
    let recent_paths: Vec<String> = recent.into_iter().map(|p| p.path).collect();
    Ok(serde_json::json!({
        "total_pages": count,
        "recent_updated": recent_paths,
    }))
}
