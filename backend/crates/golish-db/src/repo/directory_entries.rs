//! `directory_entries` project-scoped repo helpers (AGENTS.md I2).
//!
//! Sinks the scoped `list` (by target / by project) and `EXISTS` probe used by
//! the command layer (`golish::tools::targets::directory`) and the pipeline
//! storage adapter. Inserts stay in the command layer.

use crate::Result;
use sqlx::postgres::PgRow;
use sqlx::{FromRow, PgPool};
use uuid::Uuid;

use super::scoped::{lock_target_write_guard, TargetWriteGuard};
use super::technique_outcomes::{lock_attempt_generation_current, TechniqueOutcomeAttemptGuard};

#[derive(Debug)]
pub enum ConditionalDirectoryEntryWrite<T> {
    Applied(T),
    Superseded,
}

const DIR_ENTRY_COLS: &str =
    "id, target_id, url, status_code, content_length, lines, words, content_type, tool, created_at";
const CURRENT_OWNER_DIR_ENTRY_COLS: &str = "de.id, de.target_id, de.url, de.status_code, de.content_length, de.lines, de.words, de.content_type, de.tool, de.created_at";

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

fn build_list_by_target_project_sql() -> String {
    format!(
        "SELECT {DIR_ENTRY_COLS} FROM directory_entries WHERE target_id = $1 AND project_path IS NOT DISTINCT FROM $2 ORDER BY created_at"
    )
}

fn build_list_by_current_target_owner_sql() -> String {
    format!(
        "SELECT {CURRENT_OWNER_DIR_ENTRY_COLS} FROM directory_entries de JOIN targets t ON t.id = de.target_id WHERE de.target_id = $1 AND t.scope::text = 'in' AND de.project_path IS NOT DISTINCT FROM t.project_path ORDER BY de.created_at"
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

/// List only entries whose stored project still matches the target's current
/// in-scope owner binding.
pub async fn list_by_current_target_owner<T>(pool: &PgPool, target_id: Uuid) -> Result<Vec<T>>
where
    T: for<'r> FromRow<'r, PgRow> + Send + Unpin,
{
    let rows = sqlx::query_as::<_, T>(&build_list_by_current_target_owner_sql())
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

/// List one target's directory discoveries without carrying rows from an old
/// workspace binding after the target itself moves.
pub async fn list_by_target_project<T>(
    pool: &PgPool,
    target_id: Uuid,
    project_path: Option<&str>,
) -> Result<Vec<T>>
where
    T: for<'r> FromRow<'r, PgRow> + Send + Unpin,
{
    let rows = sqlx::query_as::<_, T>(&build_list_by_target_project_sql())
        .bind(target_id)
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
           ON CONFLICT (target_id, url, tool) WHERE target_id IS NOT NULL
           DO UPDATE SET status_code = EXCLUDED.status_code,
                         content_length = EXCLUDED.content_length,
                         lines = EXCLUDED.lines,
                         words = EXCLUDED.words
           WHERE directory_entries.project_path IS NOT DISTINCT FROM EXCLUDED.project_path
           RETURNING {DIR_ENTRY_COLS}"
    )
}

/// Insert (or upsert by target + `url` + `tool` when `target_id` is set) a
/// directory entry, returning the row. Mirrors the legacy
/// `db_directory_entry_add`. Generic over the caller's row type.
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

/// Insert/upsert one target-owned directory discovery while the raw target
/// authorization witness still matches. The target row lock and child write
/// share one short transaction, so an organization/scope/project/origin move
/// cannot land the result under a different owner after producer revalidation.
#[allow(clippy::too_many_arguments)]
pub async fn insert_entry_guarded<T>(
    pool: &PgPool,
    guard: &TargetWriteGuard,
    url: &str,
    status_code: Option<i32>,
    content_length: Option<i32>,
    lines: Option<i32>,
    words: Option<i32>,
    tool: &str,
) -> Result<T>
where
    T: for<'r> FromRow<'r, PgRow> + Send + Unpin,
{
    let mut tx = pool.begin().await?;
    lock_target_write_guard(&mut tx, guard).await?;
    let row = sqlx::query_as::<_, T>(&build_insert_entry_sql())
        .bind(Some(guard.target_id))
        .bind(url)
        .bind(status_code)
        .bind(content_length)
        .bind(lines)
        .bind(words)
        .bind(tool)
        .bind(Some(guard.project_path.as_str()))
        .fetch_one(&mut *tx)
        .await?;
    tx.commit().await?;
    Ok(row)
}

/// Insert/upsert a route-probe discovery only while the exact producer
/// generation is still current. Target, operation epoch, and DIR outcome are
/// locked in one short transaction; a newer attempt therefore prevents a late
/// HTTP response from contradicting its authoritative terminal outcome.
#[allow(clippy::too_many_arguments)]
pub async fn insert_entry_guarded_if_attempt_current<T>(
    pool: &PgPool,
    guard: &TargetWriteGuard,
    attempt_guard: &TechniqueOutcomeAttemptGuard,
    run_id: &str,
    asset: &str,
    technique: &str,
    url: &str,
    status_code: Option<i32>,
    content_length: Option<i32>,
    lines: Option<i32>,
    words: Option<i32>,
    tool: &str,
) -> Result<ConditionalDirectoryEntryWrite<T>>
where
    T: for<'r> FromRow<'r, PgRow> + Send + Unpin,
{
    if guard.organization_id != Some(attempt_guard.organization_id)
        || run_id.trim().is_empty()
        || asset.trim().is_empty()
        || technique.trim().is_empty()
    {
        return Err(crate::DbError::Other(anyhow::anyhow!(
            "conditional directory write does not match its attempt witness"
        )));
    }
    let mut tx = pool.begin().await?;
    lock_target_write_guard(&mut tx, guard).await?;
    let techniques = vec![technique.to_string()];
    if !lock_attempt_generation_current(&mut tx, attempt_guard, run_id, asset, &techniques).await? {
        tx.rollback().await?;
        return Ok(ConditionalDirectoryEntryWrite::Superseded);
    }
    let row = sqlx::query_as::<_, T>(&build_insert_entry_sql())
        .bind(Some(guard.target_id))
        .bind(url)
        .bind(status_code)
        .bind(content_length)
        .bind(lines)
        .bind(words)
        .bind(tool)
        .bind(Some(guard.project_path.as_str()))
        .fetch_one(&mut *tx)
        .await?;
    tx.commit().await?;
    Ok(ConditionalDirectoryEntryWrite::Applied(row))
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
            build_list_by_target_project_sql(),
            format!(
                "SELECT {cols} FROM directory_entries WHERE target_id = $1 AND project_path IS NOT DISTINCT FROM $2 ORDER BY created_at"
            )
        );
        assert_eq!(
            build_exists_by_url_project_sql(),
            "SELECT EXISTS(SELECT 1 FROM directory_entries WHERE url = $1 AND project_path = $2)"
        );
        let insert = build_insert_entry_sql();
        assert!(insert.contains("ON CONFLICT (target_id, url, tool) WHERE target_id IS NOT NULL"));
        assert!(insert.contains(
            "WHERE directory_entries.project_path IS NOT DISTINCT FROM EXCLUDED.project_path"
        ));
        assert!(!insert.contains("ON CONFLICT (url, tool)"));
    }

    #[test]
    fn current_owner_list_checks_scope_and_project() {
        let sql = build_list_by_current_target_owner_sql();
        assert!(sql.starts_with(
            "SELECT de.id, de.target_id, de.url, de.status_code, de.content_length, de.lines, de.words, de.content_type, de.tool, de.created_at FROM"
        ));
        assert!(sql.contains("JOIN targets t ON t.id = de.target_id"));
        assert!(sql.contains("t.scope::text = 'in'"));
        assert!(sql.contains("de.project_path IS NOT DISTINCT FROM t.project_path"));
    }
}
