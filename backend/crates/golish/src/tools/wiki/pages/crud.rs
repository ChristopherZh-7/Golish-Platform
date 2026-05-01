//! Page-level CRUD: list / read / write / delete / rename / create_dir.
//!
//! Writes mirror the page into the `wiki_pages` Postgres table best-effort
//! so search and dashboards stay current. Failures are logged but never
//! abort the user-facing operation.

use golish_db::models::NewWikiPage;
use tokio::fs;

use crate::state::DbState;

use super::super::{is_wiki_file, wiki_base_dir};
use super::frontmatter::{build_tree, extract_frontmatter, infer_category_from_path, WikiEntry};

#[tauri::command]
pub async fn wiki_list() -> Result<Vec<WikiEntry>, String> {
    let base = wiki_base_dir();
    if !base.exists() {
        fs::create_dir_all(&base)
            .await
            .map_err(|e| format!("cannot create wiki dir: {e}"))?;
        return Ok(Vec::new());
    }
    build_tree(&base, "").await.map_err(|e| e.to_string())
}

#[tauri::command]
pub async fn wiki_read(path: String) -> Result<String, String> {
    let full = wiki_base_dir().join(&path);
    if !full.exists() {
        return Err(format!("file not found: {path}"));
    }
    fs::read_to_string(&full)
        .await
        .map_err(|e| format!("read failed: {e}"))
}

#[tauri::command]
pub async fn wiki_write(
    state: tauri::State<'_, DbState>,
    path: String,
    content: String,
) -> Result<(), String> {
    let full = wiki_base_dir().join(&path);
    if let Some(parent) = full.parent() {
        fs::create_dir_all(parent)
            .await
            .map_err(|e| format!("mkdir failed: {e}"))?;
    }
    fs::write(&full, &content)
        .await
        .map_err(|e| format!("write failed: {e}"))?;

    if is_wiki_file(&path) {
        if let Ok(pool) = state.pool_ready().await {
            let (title, fm_category, tags, status) = extract_frontmatter(&content);
            let category = if fm_category == "uncategorized" {
                infer_category_from_path(&path)
            } else {
                fm_category
            };
            let page = NewWikiPage {
                path: path.clone(),
                title,
                category,
                tags,
                status,
                content: content.clone(),
            };
            if let Err(e) = golish_db::repo::wiki_kb::upsert_page(pool, &page).await {
                tracing::warn!("[wiki] DB sync failed for {path}: {e}");
            }
        }
    }

    Ok(())
}

#[tauri::command]
pub async fn wiki_delete(
    state: tauri::State<'_, DbState>,
    path: String,
) -> Result<(), String> {
    let full = wiki_base_dir().join(&path);
    if !full.exists() {
        return Ok(());
    }
    let meta = fs::metadata(&full)
        .await
        .map_err(|e| format!("stat failed: {e}"))?;
    if meta.is_dir() {
        fs::remove_dir_all(&full)
            .await
            .map_err(|e| format!("rmdir failed: {e}"))?;
        if let Ok(pool) = state.pool_ready().await {
            let prefix = if path.ends_with('/') {
                path.clone()
            } else {
                format!("{}/", path)
            };
            if let Err(e) = golish_db::repo::wiki_kb::delete_pages_by_prefix(pool, &prefix).await {
                tracing::warn!("[wiki] DB delete_pages_by_prefix failed for {path}: {e}");
            }
        }
    } else {
        fs::remove_file(&full)
            .await
            .map_err(|e| format!("rm failed: {e}"))?;
        if let Ok(pool) = state.pool_ready().await {
            if let Err(e) = golish_db::repo::wiki_kb::delete_page(pool, &path).await {
                tracing::warn!("[wiki] DB delete_page failed for {path}: {e}");
            }
        }
    }
    Ok(())
}

#[tauri::command]
pub async fn wiki_rename(old_path: String, new_path: String) -> Result<(), String> {
    let base = wiki_base_dir();
    let from = base.join(&old_path);
    let to = base.join(&new_path);
    if !from.exists() {
        return Err(format!("source not found: {old_path}"));
    }
    if let Some(parent) = to.parent() {
        fs::create_dir_all(parent)
            .await
            .map_err(|e| format!("mkdir failed: {e}"))?;
    }
    fs::rename(&from, &to)
        .await
        .map_err(|e| format!("rename failed: {e}"))
}

#[tauri::command]
pub async fn wiki_create_dir(path: String) -> Result<(), String> {
    let full = wiki_base_dir().join(&path);
    fs::create_dir_all(&full)
        .await
        .map_err(|e| format!("mkdir failed: {e}"))
}
