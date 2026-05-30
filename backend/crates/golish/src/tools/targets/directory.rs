//! Directory-entry database helpers + the `directory_entry_list` Tauri
//! command.

use crate::error::GolishError;
use sqlx::PgPool;
use uuid::Uuid;

use crate::state::DbState;

use super::recon::{DirEntryRow, DirectoryEntry};

pub async fn db_directory_entry_add(
    pool: &PgPool,
    target_id: Option<Uuid>,
    url: &str,
    status_code: Option<i32>,
    content_length: Option<i32>,
    lines: Option<i32>,
    words: Option<i32>,
    tool: &str,
    project_path: Option<&str>,
) -> Result<DirectoryEntry, GolishError> {
    let row = sqlx::query_as::<_, DirEntryRow>(
        r#"INSERT INTO directory_entries (target_id, url, status_code, content_length, lines, words, tool, project_path)
           VALUES ($1, $2, $3, $4, $5, $6, $7, $8)
           ON CONFLICT (url, tool) WHERE target_id IS NOT NULL
           DO UPDATE SET status_code = EXCLUDED.status_code,
                         content_length = EXCLUDED.content_length,
                         lines = EXCLUDED.lines,
                         words = EXCLUDED.words
           RETURNING id, target_id, url, status_code, content_length, lines, words, content_type, tool, created_at"#,
    )
    .bind(target_id)
    .bind(url)
    .bind(status_code)
    .bind(content_length)
    .bind(lines)
    .bind(words)
    .bind(tool)
    .bind(project_path)
    .fetch_one(pool)
    .await
?;

    Ok(DirectoryEntry::from(row))
}

pub async fn db_directory_entries_list(
    pool: &PgPool,
    target_id: Option<Uuid>,
    project_path: Option<&str>,
) -> Result<Vec<DirectoryEntry>, GolishError> {
    let rows: Vec<DirEntryRow> = if let Some(tid) = target_id {
        golish_db::repo::directory_entries::list_by_target(pool, tid).await?
    } else {
        golish_db::repo::directory_entries::list_by_project(pool, project_path).await?
    };

    Ok(rows.into_iter().map(DirectoryEntry::from).collect())
}

// ============================================================================
// Tauri commands for directory entries
// ============================================================================

#[tauri::command]
pub async fn directory_entry_list(
    state: tauri::State<'_, DbState>,
    target_id: Option<String>,
    project_path: Option<String>,
) -> Result<Vec<DirectoryEntry>, GolishError> {
    let pool = state.pool_ready().await?;
    let tid: Option<Uuid> = target_id.and_then(|s| s.parse().ok());
    db_directory_entries_list(pool, tid, project_path.as_deref()).await
}
