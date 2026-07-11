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

use crate::{DbError, Result};
use serde::{Deserialize, Serialize};
use sqlx::postgres::PgRow;
use sqlx::{FromRow, PgConnection, PgPool};
use uuid::Uuid;

/// Immutable raw target row captured when an active producer is authorized.
///
/// Target-bound business writers lock the current `targets` row and compare
/// every field before inserting/updating child rows. Keeping the raw
/// `name`/`value`/`ports` witness avoids reimplementing Web Origin
/// normalization in SQL: any mutation that could revoke the exact-origin
/// authorization changes the witness and makes the write fail closed.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct TargetWriteGuard {
    pub target_id: Uuid,
    pub organization_id: Option<Uuid>,
    pub project_path: String,
    pub scope: String,
    pub name: String,
    pub value: String,
    pub ports: serde_json::Value,
}

#[derive(Debug, FromRow)]
struct TargetWriteGuardRow {
    id: Uuid,
    organization_id: Option<Uuid>,
    project_path: Option<String>,
    scope: String,
    name: String,
    value: String,
    ports: serde_json::Value,
}

const LOCK_TARGET_WRITE_GUARD_SQL: &str = r#"SELECT id,
       organization_id,
       project_path,
       scope::text AS scope,
       name,
       value,
       ports
FROM targets
WHERE id = $1
FOR UPDATE"#;

fn build_load_target_write_guard_sql() -> &'static str {
    r#"SELECT id,
              organization_id,
              project_path,
              scope::text AS scope,
              name,
              value,
              ports
       FROM targets
       WHERE id = $1
         AND scope::text = 'in'
         AND project_path IS NOT NULL"#
}

impl From<TargetWriteGuardRow> for TargetWriteGuard {
    fn from(row: TargetWriteGuardRow) -> Self {
        Self {
            target_id: row.id,
            organization_id: row.organization_id,
            project_path: row
                .project_path
                .expect("guard loader requires a non-null project_path"),
            scope: row.scope,
            name: row.name,
            value: row.value,
            ports: row.ports,
        }
    }
}

/// Capture the immutable raw owner/scope/origin witness for a current in-scope
/// target. Network-capable callers keep this snapshot across preparation and
/// revalidate it immediately before returning a launch decision.
pub async fn load_target_write_guard(
    pool: &PgPool,
    target_id: Uuid,
) -> Result<Option<TargetWriteGuard>> {
    let row = sqlx::query_as::<_, TargetWriteGuardRow>(build_load_target_write_guard_sql())
        .bind(target_id)
        .fetch_optional(pool)
        .await?;
    Ok(row.map(TargetWriteGuard::from))
}

fn target_matches_write_guard(current: &TargetWriteGuardRow, guard: &TargetWriteGuard) -> bool {
    current.id == guard.target_id
        && current.organization_id == guard.organization_id
        && current.project_path.as_deref() == Some(guard.project_path.as_str())
        && current.scope == "in"
        && current.scope == guard.scope
        && current.name == guard.name
        && current.value == guard.value
        && current.ports == guard.ports
}

/// Lock and validate the exact target authorization snapshot for a short DB
/// transaction. Callers must perform the child-row write on the same
/// connection before committing; no network or other long-running work belongs
/// between this guard and the write.
pub async fn lock_target_write_guard(
    connection: &mut PgConnection,
    guard: &TargetWriteGuard,
) -> Result<()> {
    let current = sqlx::query_as::<_, TargetWriteGuardRow>(LOCK_TARGET_WRITE_GUARD_SQL)
        .bind(guard.target_id)
        .fetch_optional(&mut *connection)
        .await?
        .ok_or_else(|| {
            DbError::NotFound(format!(
                "target write guard rejected missing target {}",
                guard.target_id
            ))
        })?;

    if !target_matches_write_guard(&current, guard) {
        return Err(DbError::Other(anyhow::anyhow!(
            "target write guard rejected authorization drift for {}",
            guard.target_id
        )));
    }

    Ok(())
}

/// Revalidate a previously captured guard under the same row lock used by
/// guarded writers. The transaction contains DB work only and releases the
/// lock immediately after the comparison.
pub async fn validate_target_write_guard(pool: &PgPool, guard: &TargetWriteGuard) -> Result<()> {
    let mut tx = pool.begin().await?;
    lock_target_write_guard(&mut tx, guard).await?;
    tx.commit().await?;
    Ok(())
}

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

    fn write_guard() -> TargetWriteGuard {
        TargetWriteGuard {
            target_id: Uuid::new_v4(),
            organization_id: Some(Uuid::new_v4()),
            project_path: "/workspace/a".to_string(),
            scope: "in".to_string(),
            name: "app.example".to_string(),
            value: "https://app.example/".to_string(),
            ports: serde_json::json!([{
                "port": 443,
                "state": "open",
                "url": "https://app.example/"
            }]),
        }
    }

    fn guard_row(guard: &TargetWriteGuard) -> TargetWriteGuardRow {
        TargetWriteGuardRow {
            id: guard.target_id,
            organization_id: guard.organization_id,
            project_path: Some(guard.project_path.clone()),
            scope: guard.scope.clone(),
            name: guard.name.clone(),
            value: guard.value.clone(),
            ports: guard.ports.clone(),
        }
    }

    #[test]
    fn target_write_guard_locks_one_target_row() {
        assert!(LOCK_TARGET_WRITE_GUARD_SQL.contains("FROM targets"));
        assert!(LOCK_TARGET_WRITE_GUARD_SQL.contains("WHERE id = $1"));
        assert!(LOCK_TARGET_WRITE_GUARD_SQL.contains("scope::text AS scope"));
        assert!(LOCK_TARGET_WRITE_GUARD_SQL.contains("FOR UPDATE"));
    }

    #[test]
    fn target_write_guard_loader_is_current_in_scope_and_project_bound() {
        let sql = build_load_target_write_guard_sql();
        assert!(sql.contains("WHERE id = $1"));
        assert!(sql.contains("scope::text = 'in'"));
        assert!(sql.contains("project_path IS NOT NULL"));
    }

    #[test]
    fn target_write_guard_requires_every_raw_snapshot_field() {
        let guard = write_guard();
        let current = guard_row(&guard);
        assert!(target_matches_write_guard(&current, &guard));

        let mut drifted = guard_row(&guard);
        drifted.organization_id = Some(Uuid::new_v4());
        assert!(!target_matches_write_guard(&drifted, &guard));

        let mut drifted = guard_row(&guard);
        drifted.project_path = Some("/workspace/b".to_string());
        assert!(!target_matches_write_guard(&drifted, &guard));

        let mut drifted = guard_row(&guard);
        drifted.scope = "out".to_string();
        assert!(!target_matches_write_guard(&drifted, &guard));

        let mut drifted = guard_row(&guard);
        drifted.name = "other.example".to_string();
        assert!(!target_matches_write_guard(&drifted, &guard));

        let mut drifted = guard_row(&guard);
        drifted.value = "https://other.example/".to_string();
        assert!(!target_matches_write_guard(&drifted, &guard));

        let mut drifted = guard_row(&guard);
        drifted.ports = serde_json::json!([]);
        assert!(!target_matches_write_guard(&drifted, &guard));
    }
}
