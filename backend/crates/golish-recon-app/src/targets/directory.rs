//! Directory-entry database helpers + the `directory_entry_list` Tauri
//! command.

use golish_app_core::GolishError;
use sqlx::PgPool;
use uuid::Uuid;

use golish_app_core::DbState;

use super::recon::{DirEntryRow, DirectoryEntry};

#[allow(clippy::too_many_arguments)]
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
    let row: DirEntryRow = golish_db::repo::directory_entries::insert_entry(
        pool,
        target_id,
        url,
        status_code,
        content_length,
        lines,
        words,
        tool,
        project_path,
    )
    .await?;
    Ok(DirectoryEntry::from(row))
}

#[allow(clippy::too_many_arguments)]
pub async fn db_directory_entry_add_guarded(
    pool: &PgPool,
    guard: &golish_db::repo::scoped::TargetWriteGuard,
    url: &str,
    status_code: Option<i32>,
    content_length: Option<i32>,
    lines: Option<i32>,
    words: Option<i32>,
    tool: &str,
) -> Result<DirectoryEntry, GolishError> {
    let row: DirEntryRow = golish_db::repo::directory_entries::insert_entry_guarded(
        pool,
        guard,
        url,
        status_code,
        content_length,
        lines,
        words,
        tool,
    )
    .await?;
    Ok(DirectoryEntry::from(row))
}

pub async fn db_directory_entries_list(
    pool: &PgPool,
    target_id: Option<Uuid>,
    project_path: Option<&str>,
) -> Result<Vec<DirectoryEntry>, GolishError> {
    let rows: Vec<DirEntryRow> = if let Some(tid) = target_id {
        golish_db::repo::directory_entries::list_by_current_target_owner(pool, tid).await?
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
