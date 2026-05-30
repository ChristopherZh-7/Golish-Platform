//! `scan_queue` project-scoped repo helpers (AGENTS.md I2).
//!
//! The command layer (`golish::tools::scan_queue`) must route its scoped
//! `list` / `clear` / `delete-by-url` / `clear-completed` operations through
//! these helpers instead of inlining the SQL. Inserts/upserts stay in the
//! command layer (they are not scope guards).

use anyhow::Result;
use sqlx::postgres::PgRow;
use sqlx::{FromRow, PgPool};

fn build_list_by_project_sql() -> String {
    "SELECT id::text, url, scan_id, progress, status, alerts, added_at FROM scan_queue WHERE project_path = $1 ORDER BY added_at ASC".to_string()
}

fn build_clear_by_project_sql() -> String {
    "DELETE FROM scan_queue WHERE project_path = $1".to_string()
}

fn build_delete_by_url_sql() -> String {
    "DELETE FROM scan_queue WHERE url = $1 AND project_path = $2".to_string()
}

fn build_clear_completed_sql() -> String {
    "DELETE FROM scan_queue WHERE status = 'complete' AND project_path = $1".to_string()
}

/// List queued scan endpoints for a project, oldest first. Generic over the
/// caller's row type (the command layer's `ScanQueueRow` tuple).
pub async fn list_by_project<T>(pool: &PgPool, project_path: Option<&str>) -> Result<Vec<T>>
where
    T: for<'r> FromRow<'r, PgRow> + Send + Unpin,
{
    let rows = sqlx::query_as::<_, T>(&build_list_by_project_sql())
        .bind(project_path)
        .fetch_all(pool)
        .await?;
    Ok(rows)
}

/// Delete every scan-queue row for a project. Returns rows affected.
pub async fn clear_by_project(pool: &PgPool, project_path: Option<&str>) -> Result<u64> {
    let res = sqlx::query(&build_clear_by_project_sql())
        .bind(project_path)
        .execute(pool)
        .await?;
    Ok(res.rows_affected())
}

/// Delete a scan-queue row by `url` within a project. Returns rows affected.
pub async fn delete_by_url(pool: &PgPool, url: &str, project_path: Option<&str>) -> Result<u64> {
    let res = sqlx::query(&build_delete_by_url_sql())
        .bind(url)
        .bind(project_path)
        .execute(pool)
        .await?;
    Ok(res.rows_affected())
}

/// Delete completed scan-queue rows for a project. Returns rows affected.
pub async fn clear_completed(pool: &PgPool, project_path: Option<&str>) -> Result<u64> {
    let res = sqlx::query(&build_clear_completed_sql())
        .bind(project_path)
        .execute(pool)
        .await?;
    Ok(res.rows_affected())
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn scan_queue_sql_matches_command_layer() {
        assert_eq!(
            build_list_by_project_sql(),
            "SELECT id::text, url, scan_id, progress, status, alerts, added_at FROM scan_queue WHERE project_path = $1 ORDER BY added_at ASC"
        );
        assert_eq!(
            build_clear_by_project_sql(),
            "DELETE FROM scan_queue WHERE project_path = $1"
        );
        assert_eq!(
            build_delete_by_url_sql(),
            "DELETE FROM scan_queue WHERE url = $1 AND project_path = $2"
        );
        assert_eq!(
            build_clear_completed_sql(),
            "DELETE FROM scan_queue WHERE status = 'complete' AND project_path = $1"
        );
    }
}
