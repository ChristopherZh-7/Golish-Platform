//! Generic, table-parameterised CRUD building blocks shared across `repo` modules.
//!
//! Consolidates the by-id / project-scoped SQL templates that were copy-pasted
//! across 10+ repo files. See `docs/design/2026-05-29-architecture-optimization.md`
//! §3.1 B-D1 / §5 P1-1.
//!
//! ## Safety (SQL injection)
//! [`get_by_id`], [`list_by_project`], etc. interpolate `table` / `order_by` into
//! the SQL string via `format!`. These MUST be **trusted compile-time string
//! literals** supplied by repo modules (table names, `"created_at DESC"`, …),
//! never user input. Every current caller passes a `&'static str`. A
//! [`is_safe_sql_fragment`] guard (via `debug_assert!`) catches accidental misuse
//! in debug/test builds.
//!
//! ## Design
//! Each public async helper is a thin wrapper over a pure `build_*_sql` function.
//! The pure builders are unit-tested to prove the generated SQL is byte-for-byte
//! the canonical form the per-table repo functions used before delegation, so the
//! refactor introduces **zero query semantics drift**.

use crate::Result;
use sqlx::postgres::PgRow;
use sqlx::{FromRow, PgPool};
use uuid::Uuid;

/// Conservative check that an interpolated SQL fragment (table name or `ORDER BY`
/// clause) only contains characters we expect from a trusted literal.
///
/// Allowed: ASCII alphanumerics plus `_`, space, and `,` (e.g. `vault_entries`,
/// `created_at DESC`, `updated_at DESC, name`). This is a defence-in-depth guard,
/// not the primary control — the primary control is that callers only pass
/// `&'static str` constants.
pub fn is_safe_sql_fragment(frag: &str) -> bool {
    !frag.is_empty()
        && frag
            .bytes()
            .all(|b| b.is_ascii_alphanumeric() || matches!(b, b'_' | b' ' | b','))
}

// ── Pure SQL builders (unit-testable, no DB) ────────────────────────────────

fn build_get_by_id_sql(table: &str) -> String {
    format!("SELECT * FROM {table} WHERE id = $1")
}

fn build_get_scoped_sql(table: &str) -> String {
    format!("SELECT * FROM {table} WHERE id = $1 AND project_path IS NOT DISTINCT FROM $2")
}

fn build_list_by_project_sql(table: &str, order_by: &str) -> String {
    format!("SELECT * FROM {table} WHERE project_path IS NOT DISTINCT FROM $1 ORDER BY {order_by}")
}

fn build_delete_by_id_sql(table: &str) -> String {
    format!("DELETE FROM {table} WHERE id = $1")
}

fn build_delete_scoped_sql(table: &str) -> String {
    format!("DELETE FROM {table} WHERE id = $1 AND project_path IS NOT DISTINCT FROM $2")
}

fn build_upsert_json_data_sql(table: &str) -> String {
    format!(
        "INSERT INTO {table} (id, data, project_path) VALUES ($1, $2, $3) \
         ON CONFLICT (id) DO UPDATE SET data = $2, updated_at = NOW()"
    )
}

fn build_get_json_data_scoped_sql(table: &str) -> String {
    format!("SELECT data FROM {table} WHERE id = $1 AND project_path IS NOT DISTINCT FROM $2")
}

fn build_list_json_data_by_project_sql(table: &str, order_by: &str) -> String {
    format!("SELECT data FROM {table} WHERE project_path = $1 ORDER BY {order_by}")
}

// ── Typed row helpers ───────────────────────────────────────────────────────

/// `SELECT * FROM <table> WHERE id = $1` (not project-scoped; caller scope-guards
/// before mutating). Returns `None` when the row does not exist.
pub async fn get_by_id<T>(pool: &PgPool, table: &str, id: Uuid) -> Result<Option<T>>
where
    T: for<'r> FromRow<'r, PgRow> + Send + Unpin,
{
    debug_assert!(
        is_safe_sql_fragment(table),
        "unsafe table identifier: {table:?}"
    );
    let row = sqlx::query_as::<_, T>(&build_get_by_id_sql(table))
        .bind(id)
        .fetch_optional(pool)
        .await?;
    Ok(row)
}

/// `SELECT * FROM <table> WHERE id = $1 AND project_path IS NOT DISTINCT FROM $2`
/// (AGENTS.md I2 — IDOR-safe single-row read). `None` == row missing or owned by
/// another project.
pub async fn get_scoped<T>(
    pool: &PgPool,
    table: &str,
    id: Uuid,
    project_path: Option<&str>,
) -> Result<Option<T>>
where
    T: for<'r> FromRow<'r, PgRow> + Send + Unpin,
{
    debug_assert!(
        is_safe_sql_fragment(table),
        "unsafe table identifier: {table:?}"
    );
    let row = sqlx::query_as::<_, T>(&build_get_scoped_sql(table))
        .bind(id)
        .bind(project_path)
        .fetch_optional(pool)
        .await?;
    Ok(row)
}

/// `SELECT * FROM <table> WHERE project_path IS NOT DISTINCT FROM $1 ORDER BY <order_by>`.
/// `order_by` must be a trusted literal (e.g. `"created_at DESC"`).
pub async fn list_by_project<T>(
    pool: &PgPool,
    table: &str,
    order_by: &str,
    project_path: Option<&str>,
) -> Result<Vec<T>>
where
    T: for<'r> FromRow<'r, PgRow> + Send + Unpin,
{
    debug_assert!(
        is_safe_sql_fragment(table),
        "unsafe table identifier: {table:?}"
    );
    debug_assert!(
        is_safe_sql_fragment(order_by),
        "unsafe order_by: {order_by:?}"
    );
    let rows = sqlx::query_as::<_, T>(&build_list_by_project_sql(table, order_by))
        .bind(project_path)
        .fetch_all(pool)
        .await?;
    Ok(rows)
}

// ── Delete helpers ──────────────────────────────────────────────────────────

/// `DELETE FROM <table> WHERE id = $1` (not project-scoped). Returns rows affected;
/// callers that don't need the count can ignore it.
pub async fn delete_by_id(pool: &PgPool, table: &str, id: Uuid) -> Result<u64> {
    debug_assert!(
        is_safe_sql_fragment(table),
        "unsafe table identifier: {table:?}"
    );
    let res = sqlx::query(&build_delete_by_id_sql(table))
        .bind(id)
        .execute(pool)
        .await?;
    Ok(res.rows_affected())
}

/// `DELETE FROM <table> WHERE id = $1 AND project_path IS NOT DISTINCT FROM $2`
/// (AGENTS.md I2 — IDOR-safe delete). Returns rows affected (0 == missing or
/// cross-project).
pub async fn delete_scoped(
    pool: &PgPool,
    table: &str,
    id: Uuid,
    project_path: Option<&str>,
) -> Result<u64> {
    debug_assert!(
        is_safe_sql_fragment(table),
        "unsafe table identifier: {table:?}"
    );
    let res = sqlx::query(&build_delete_scoped_sql(table))
        .bind(id)
        .bind(project_path)
        .execute(pool)
        .await?;
    Ok(res.rows_affected())
}

// ── JSON `data` blob helpers (methodology shape) ─────────────────

/// `INSERT INTO <table> (id, data, project_path) … ON CONFLICT (id) DO UPDATE SET
/// data = $2, updated_at = NOW()`. For tables shaped `(id, data JSONB, project_path)`.
pub async fn upsert_json_data(
    pool: &PgPool,
    table: &str,
    id: Uuid,
    data: &serde_json::Value,
    project_path: Option<&str>,
) -> Result<()> {
    debug_assert!(
        is_safe_sql_fragment(table),
        "unsafe table identifier: {table:?}"
    );
    sqlx::query(&build_upsert_json_data_sql(table))
        .bind(id)
        .bind(data)
        .bind(project_path)
        .execute(pool)
        .await?;
    Ok(())
}

/// `SELECT data FROM <table> WHERE id = $1 AND project_path IS NOT DISTINCT FROM $2`
/// (AGENTS.md I2). `None` == row missing or owned by another project.
pub async fn get_json_data_scoped(
    pool: &PgPool,
    table: &str,
    id: Uuid,
    project_path: Option<&str>,
) -> Result<Option<serde_json::Value>> {
    debug_assert!(
        is_safe_sql_fragment(table),
        "unsafe table identifier: {table:?}"
    );
    let row = sqlx::query_scalar::<_, serde_json::Value>(&build_get_json_data_scoped_sql(table))
        .bind(id)
        .bind(project_path)
        .fetch_optional(pool)
        .await?;
    Ok(row)
}

/// `SELECT data FROM <table> WHERE project_path = $1 ORDER BY <order_by>` (exact
/// `project_path` match). `order_by` must be a trusted literal.
pub async fn list_json_data_by_project(
    pool: &PgPool,
    table: &str,
    order_by: &str,
    project_path: Option<&str>,
) -> Result<Vec<serde_json::Value>> {
    debug_assert!(
        is_safe_sql_fragment(table),
        "unsafe table identifier: {table:?}"
    );
    debug_assert!(
        is_safe_sql_fragment(order_by),
        "unsafe order_by: {order_by:?}"
    );
    let rows = sqlx::query_scalar::<_, serde_json::Value>(&build_list_json_data_by_project_sql(
        table, order_by,
    ))
    .bind(project_path)
    .fetch_all(pool)
    .await?;
    Ok(rows)
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn get_by_id_sql_matches_canonical() {
        assert_eq!(
            build_get_by_id_sql("findings"),
            "SELECT * FROM findings WHERE id = $1"
        );
        assert_eq!(
            build_get_by_id_sql("vault_entries"),
            "SELECT * FROM vault_entries WHERE id = $1"
        );
    }

    #[test]
    fn get_scoped_sql_matches_unified_predicate() {
        assert_eq!(
            build_get_scoped_sql("targets"),
            "SELECT * FROM targets WHERE id = $1 AND project_path IS NOT DISTINCT FROM $2"
        );
    }

    #[test]
    fn list_by_project_sql_matches_canonical() {
        // findings / targets / vault used created_at DESC ...
        assert_eq!(
            build_list_by_project_sql("findings", "created_at DESC"),
            "SELECT * FROM findings WHERE project_path IS NOT DISTINCT FROM $1 ORDER BY created_at DESC"
        );
        // ... methodology_projects used updated_at DESC.
        assert_eq!(
            build_list_by_project_sql("methodology_projects", "updated_at DESC"),
            "SELECT * FROM methodology_projects WHERE project_path IS NOT DISTINCT FROM $1 ORDER BY updated_at DESC"
        );
    }

    #[test]
    fn delete_sql_matches_canonical() {
        assert_eq!(
            build_delete_by_id_sql("sessions"),
            "DELETE FROM sessions WHERE id = $1"
        );
        assert_eq!(
            build_delete_scoped_sql("vault_entries"),
            "DELETE FROM vault_entries WHERE id = $1 AND project_path IS NOT DISTINCT FROM $2"
        );
    }

    #[test]
    fn json_data_sql_matches_canonical() {
        assert_eq!(
            build_upsert_json_data_sql("methodology_projects"),
            "INSERT INTO methodology_projects (id, data, project_path) VALUES ($1, $2, $3) ON CONFLICT (id) DO UPDATE SET data = $2, updated_at = NOW()"
        );
        assert_eq!(
            build_get_json_data_scoped_sql("methodology_projects"),
            "SELECT data FROM methodology_projects WHERE id = $1 AND project_path IS NOT DISTINCT FROM $2"
        );
        assert_eq!(
            build_list_json_data_by_project_sql("methodology_projects", "updated_at DESC"),
            "SELECT data FROM methodology_projects WHERE project_path = $1 ORDER BY updated_at DESC"
        );
    }

    #[test]
    fn safe_fragment_accepts_trusted_literals() {
        assert!(is_safe_sql_fragment("findings"));
        assert!(is_safe_sql_fragment("vault_entries"));
        assert!(is_safe_sql_fragment("created_at DESC"));
        assert!(is_safe_sql_fragment("updated_at DESC, name"));
    }

    #[test]
    fn safe_fragment_rejects_injection_shapes() {
        assert!(!is_safe_sql_fragment(""));
        assert!(!is_safe_sql_fragment("findings; DROP TABLE findings"));
        assert!(!is_safe_sql_fragment("findings WHERE 1=1 --"));
        assert!(!is_safe_sql_fragment("a)('b"));
    }
}
