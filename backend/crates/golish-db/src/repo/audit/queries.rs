//! Read/clear queries over `audit_log`. Moved verbatim from the original
//! `audit.rs`; behaviour unchanged. Two predicate families coexist:
//! the `IS NULL`-aware `list`/`clear` (general callers) and the exact
//! `project_path = $n` variants used by the GUI `audit_*` Tauri commands.

use sqlx::postgres::PgRow;
use sqlx::{FromRow, PgPool};
use uuid::Uuid;

use crate::models::AuditEntry;
use crate::Result;

pub async fn list(
    pool: &PgPool,
    project_path: Option<&str>,
    limit: i64,
) -> Result<Vec<AuditEntry>> {
    let rows = sqlx::query_as::<_, AuditEntry>(
        r#"SELECT * FROM audit_log
           WHERE ($1 IS NULL OR project_path = $1 OR project_path IS NULL)
           ORDER BY created_at DESC LIMIT $2"#,
    )
    .bind(project_path)
    .bind(limit)
    .fetch_all(pool)
    .await?;
    Ok(rows)
}

pub async fn list_by_category(
    pool: &PgPool,
    category: &str,
    project_path: Option<&str>,
    limit: i64,
) -> Result<Vec<AuditEntry>> {
    let rows = sqlx::query_as::<_, AuditEntry>(
        r#"SELECT * FROM audit_log
           WHERE category = $1
             AND ($2 IS NULL OR project_path = $2 OR project_path IS NULL)
           ORDER BY created_at DESC LIMIT $3"#,
    )
    .bind(category)
    .bind(project_path)
    .bind(limit)
    .fetch_all(pool)
    .await?;
    Ok(rows)
}

pub(super) fn build_list_by_target_current_owner_sql() -> &'static str {
    r#"SELECT al.*
       FROM audit_log al
       JOIN targets t ON t.id = al.target_id
       WHERE al.target_id = $1
         AND t.scope::text = 'in'
         AND t.project_path IS NOT NULL
         AND al.project_path = t.project_path
       ORDER BY al.created_at DESC
       LIMIT $2"#
}

pub async fn list_by_target(pool: &PgPool, target_id: Uuid, limit: i64) -> Result<Vec<AuditEntry>> {
    let rows = sqlx::query_as::<_, AuditEntry>(build_list_by_target_current_owner_sql())
        .bind(target_id)
        .bind(limit)
        .fetch_all(pool)
        .await?;
    Ok(rows)
}

pub async fn list_by_session(
    pool: &PgPool,
    session_id: &str,
    limit: i64,
) -> Result<Vec<AuditEntry>> {
    let rows = sqlx::query_as::<_, AuditEntry>(
        r#"SELECT * FROM audit_log
           WHERE session_id = $1
           ORDER BY created_at DESC LIMIT $2"#,
    )
    .bind(session_id)
    .bind(limit)
    .fetch_all(pool)
    .await?;
    Ok(rows)
}

pub async fn search(
    pool: &PgPool,
    project_path: Option<&str>,
    query: &str,
    limit: i64,
) -> Result<Vec<AuditEntry>> {
    let pattern = format!("%{}%", query.to_lowercase());
    let rows = sqlx::query_as::<_, AuditEntry>(
        r#"SELECT * FROM audit_log
           WHERE ($1 IS NULL OR project_path = $1 OR project_path IS NULL)
             AND (LOWER(action) LIKE $2 OR LOWER(details) LIKE $2
                  OR LOWER(category) LIKE $2 OR LOWER(COALESCE(tool_name, '')) LIKE $2)
           ORDER BY created_at DESC LIMIT $3"#,
    )
    .bind(project_path)
    .bind(&pattern)
    .bind(limit)
    .fetch_all(pool)
    .await?;
    Ok(rows)
}

pub async fn count(pool: &PgPool, project_path: Option<&str>) -> Result<i64> {
    let (count,): (i64,) =
        sqlx::query_as("SELECT COUNT(*) FROM audit_log WHERE ($1 IS NULL OR project_path = $1 OR project_path IS NULL)")
            .bind(project_path)
            .fetch_one(pool)
            .await?;
    Ok(count)
}

pub async fn clear(pool: &PgPool, project_path: Option<&str>) -> Result<u64> {
    let result = sqlx::query(
        "DELETE FROM audit_log WHERE ($1 IS NULL OR project_path = $1 OR project_path IS NULL)",
    )
    .bind(project_path)
    .execute(pool)
    .await?;
    Ok(result.rows_affected())
}

// ── `audit_*` Tauri command helpers (exact `project_path = $n`) ─────────────
// The GUI `audit_list` / `audit_clear` commands use an exact `project_path = $n`
// match (project_path defaults to `''`), which differs from the `IS NULL`-aware
// predicate of `list` / `clear` above. Kept separate to preserve behaviour.

const AUDIT_ENTRY_LIST_COLS: &str =
    "created_at, action, category, details, entity_type, entity_id, source";

pub(super) fn build_list_by_project_exact_sql() -> String {
    format!(
        "SELECT {AUDIT_ENTRY_LIST_COLS} FROM audit_log WHERE ($1::text IS NULL OR category = $1) AND project_path = $2 ORDER BY created_at DESC LIMIT $3"
    )
}

pub(super) fn build_clear_by_project_exact_sql() -> String {
    "DELETE FROM audit_log WHERE project_path = $1".to_string()
}

/// List audit rows for an exact `project_path` match, optionally filtered by
/// `category`, newest first. Generic over the caller's row type (the command
/// layer's 7-column `AuditRow`).
pub async fn list_by_project_exact<T>(
    pool: &PgPool,
    category: Option<&str>,
    project_path: &str,
    limit: i64,
) -> Result<Vec<T>>
where
    T: for<'r> FromRow<'r, PgRow> + Send + Unpin,
{
    let rows = sqlx::query_as::<_, T>(&build_list_by_project_exact_sql())
        .bind(category)
        .bind(project_path)
        .bind(limit)
        .fetch_all(pool)
        .await?;
    Ok(rows)
}

/// Delete every audit row for an exact `project_path` match. Returns rows affected.
pub async fn clear_by_project_exact(pool: &PgPool, project_path: &str) -> Result<u64> {
    let res = sqlx::query(&build_clear_by_project_exact_sql())
        .bind(project_path)
        .execute(pool)
        .await?;
    Ok(res.rows_affected())
}
