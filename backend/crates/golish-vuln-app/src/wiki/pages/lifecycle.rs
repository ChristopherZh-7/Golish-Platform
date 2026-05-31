//! Lifecycle commands: `wiki_init` (first-run scaffolding) and
//! `wiki_reindex` (filesystem → Postgres re-sync).

use golish_app_core::GolishError;
use golish_db::models::NewWikiPage;
use tokio::fs;

use golish_app_core::DbState;

use super::super::{is_wiki_file, wiki_base_dir};
use super::frontmatter::{extract_frontmatter, infer_category_from_path};
use super::templates::{INDEX_MD_HEADER, LOG_MD_HEADER, SCHEMA_MD, WIKI_CATEGORIES};

#[tauri::command]
pub async fn wiki_init() -> Result<(), GolishError> {
    let base = wiki_base_dir();
    fs::create_dir_all(&base)
        .await
        .map_err(|e| format!("mkdir failed: {e}"))?;

    for cat in WIKI_CATEGORIES {
        fs::create_dir_all(base.join(cat))
            .await
            .map_err(|e| format!("mkdir {cat} failed: {e}"))?;
    }

    let schema_path = base.join("SCHEMA.md");
    if !schema_path.exists() {
        fs::write(&schema_path, SCHEMA_MD)
            .await
            .map_err(|e| format!("write SCHEMA.md failed: {e}"))?;
    }

    let index_path = base.join("index.md");
    if !index_path.exists() {
        fs::write(&index_path, INDEX_MD_HEADER)
            .await
            .map_err(|e| format!("write index.md failed: {e}"))?;
    }

    let log_path = base.join("log.md");
    if !log_path.exists() {
        fs::write(&log_path, LOG_MD_HEADER)
            .await
            .map_err(|e| format!("write log.md failed: {e}"))?;
    }

    Ok(())
}

/// Re-index all wiki pages: scan filesystem, re-extract frontmatter,
/// infer category from path, and upsert into PostgreSQL.
///
/// Fixes "uncategorized" pages from before the category system.
#[tauri::command]
pub async fn wiki_reindex(
    state: tauri::State<'_, DbState>,
) -> Result<serde_json::Value, GolishError> {
    let pool = state.pool_ready().await?;
    let base = wiki_base_dir();
    if !base.exists() {
        return Ok(serde_json::json!({ "reindexed": 0 }));
    }

    let mut count = 0u32;
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
            if !is_wiki_file(&name) {
                continue;
            }
            let rel = path
                .strip_prefix(&base)
                .unwrap_or(&path)
                .to_string_lossy()
                .to_string();
            if rel == "index.md" || rel == "log.md" || rel == "SCHEMA.md" {
                continue;
            }

            if let Ok(content) = fs::read_to_string(&path).await {
                let (title, fm_category, tags, status) = extract_frontmatter(&content);
                let category = if fm_category == "uncategorized" {
                    infer_category_from_path(&rel)
                } else {
                    fm_category
                };
                let page = NewWikiPage {
                    path: rel.clone(),
                    title,
                    category,
                    tags,
                    status,
                    content,
                };
                if let Err(e) = golish_db::repo::wiki_kb::upsert_page(pool, &page).await {
                    tracing::warn!("[wiki] reindex failed for {}: {}", rel, e);
                } else {
                    count += 1;
                }
            }
        }
    }

    Ok(serde_json::json!({ "reindexed": count }))
}
