//! `directory_entries` project-scoped repo helpers (AGENTS.md I2).
//!
//! Sinks the scoped `list` (by target / by project) and `EXISTS` probe used by
//! the command layer (`golish::tools::targets::directory`) and the pipeline
//! storage adapter. Inserts stay in the command layer.

use crate::Result;
use sqlx::postgres::PgRow;
use sqlx::{FromRow, PgPool};
use uuid::Uuid;

const DIR_ENTRY_COLS: &str =
    "id, target_id, url, status_code, content_length, lines, words, content_type, tool, created_at";

fn build_list_by_target_sql() -> String {
    format!(
        "SELECT {DIR_ENTRY_COLS} FROM directory_entries WHERE target_id = $1 ORDER BY created_at"
    )
}

fn build_list_by_project_sql() -> String {
    format!(
        "SELECT {DIR_ENTRY_COLS} FROM directory_entries WHERE project_path = $1 ORDER BY created_at"
    )
}

fn build_exists_by_url_project_sql() -> String {
    "SELECT EXISTS(SELECT 1 FROM directory_entries WHERE url = $1 AND project_path = $2)"
        .to_string()
}

/// List directory entries for a `target_id`, `ORDER BY created_at`. Generic over
/// the caller's row type (the command layer's `DirEntryRow`).
pub async fn list_by_target<T>(pool: &PgPool, target_id: Uuid) -> Result<Vec<T>>
where
    T: for<'r> FromRow<'r, PgRow> + Send + Unpin,
{
    let rows = sqlx::query_as::<_, T>(&build_list_by_target_sql())
        .bind(target_id)
        .fetch_all(pool)
        .await?;
    Ok(rows)
}

/// List directory entries for a project, `ORDER BY created_at`. Generic over the
/// caller's row type.
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

/// Whether a directory entry with `url` exists within a project. Backs the
/// pipeline storage dedup probe (`store_dirent_from_item`).
pub async fn exists_by_url_project(
    pool: &PgPool,
    url: &str,
    project_path: Option<&str>,
) -> Result<bool> {
    let exists = sqlx::query_scalar::<_, bool>(&build_exists_by_url_project_sql())
        .bind(url)
        .bind(project_path)
        .fetch_one(pool)
        .await?;
    Ok(exists)
}

fn build_insert_entry_sql() -> String {
    format!(
        "INSERT INTO directory_entries (target_id, url, status_code, content_length, lines, words, tool, project_path)
           VALUES ($1, $2, $3, $4, $5, $6, $7, $8)
           ON CONFLICT (url, tool) WHERE target_id IS NOT NULL
           DO UPDATE SET status_code = EXCLUDED.status_code,
                         content_length = EXCLUDED.content_length,
                         lines = EXCLUDED.lines,
                         words = EXCLUDED.words
           RETURNING {DIR_ENTRY_COLS}"
    )
}

/// Insert (or upsert by `url`+`tool` when `target_id` is set) a directory entry,
/// returning the row. Mirrors the legacy `db_directory_entry_add`. Generic over
/// the caller's row type.
#[allow(clippy::too_many_arguments)]
pub async fn insert_entry<T>(
    pool: &PgPool,
    target_id: Option<Uuid>,
    url: &str,
    status_code: Option<i32>,
    content_length: Option<i32>,
    lines: Option<i32>,
    words: Option<i32>,
    tool: &str,
    project_path: Option<&str>,
) -> Result<T>
where
    T: for<'r> FromRow<'r, PgRow> + Send + Unpin,
{
    let row = sqlx::query_as::<_, T>(&build_insert_entry_sql())
        .bind(target_id)
        .bind(url)
        .bind(status_code)
        .bind(content_length)
        .bind(lines)
        .bind(words)
        .bind(tool)
        .bind(project_path)
        .fetch_one(pool)
        .await?;
    Ok(row)
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn directory_entries_sql_matches_command_layer() {
        let cols = "id, target_id, url, status_code, content_length, lines, words, content_type, tool, created_at";
        assert_eq!(
            build_list_by_target_sql(),
            format!(
                "SELECT {cols} FROM directory_entries WHERE target_id = $1 ORDER BY created_at"
            )
        );
        assert_eq!(
            build_list_by_project_sql(),
            format!(
                "SELECT {cols} FROM directory_entries WHERE project_path = $1 ORDER BY created_at"
            )
        );
        assert_eq!(
            build_exists_by_url_project_sql(),
            "SELECT EXISTS(SELECT 1 FROM directory_entries WHERE url = $1 AND project_path = $2)"
        );
    }
}
