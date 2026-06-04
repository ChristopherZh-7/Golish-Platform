use crate::Result;
use sqlx::postgres::PgRow;
use sqlx::{FromRow, PgPool};
use uuid::Uuid;

use crate::models::{ScopeType, Target, TargetType};

pub async fn create(
    pool: &PgPool,
    name: &str,
    target_type: TargetType,
    value: &str,
    tags: &serde_json::Value,
    scope: ScopeType,
    group: &str,
    project_path: Option<&str>,
) -> Result<Target> {
    let row = sqlx::query_as::<_, Target>(
        r#"INSERT INTO targets (name, target_type, value, tags, scope, grp, project_path)
           VALUES ($1, $2, $3, $4, $5, $6, $7)
           RETURNING *"#,
    )
    .bind(name)
    .bind(target_type)
    .bind(value)
    .bind(tags)
    .bind(scope)
    .bind(group)
    .bind(project_path)
    .fetch_one(pool)
    .await?;
    Ok(row)
}

pub async fn list(pool: &PgPool, project_path: Option<&str>) -> Result<Vec<Target>> {
    super::scoped::list_by_project(pool, "targets", "created_at DESC", project_path).await
}

pub async fn get(pool: &PgPool, id: Uuid) -> Result<Option<Target>> {
    super::scoped::get_by_id(pool, "targets", id).await
}

pub async fn update(
    pool: &PgPool,
    id: Uuid,
    name: &str,
    value: &str,
    tags: &serde_json::Value,
    scope: ScopeType,
    group: &str,
    notes: &str,
) -> Result<()> {
    sqlx::query(
        r#"UPDATE targets SET name = $1, value = $2, tags = $3, scope = $4, grp = $5, notes = $6, updated_at = NOW()
           WHERE id = $7"#,
    )
    .bind(name).bind(value).bind(tags).bind(scope).bind(group).bind(notes).bind(id)
    .execute(pool)
    .await?;
    Ok(())
}

pub async fn delete(pool: &PgPool, id: Uuid) -> Result<()> {
    super::scoped::delete_by_id(pool, "targets", id).await?;
    Ok(())
}

pub async fn list_groups(pool: &PgPool, project_path: Option<&str>) -> Result<Vec<String>> {
    let rows: Vec<(String,)> = sqlx::query_as(
        "SELECT name FROM target_groups WHERE project_path IS NOT DISTINCT FROM $1 ORDER BY name",
    )
    .bind(project_path)
    .fetch_all(pool)
    .await?;
    Ok(rows.into_iter().map(|r| r.0).collect())
}

// ── Legacy-visibility scoped helpers (P0-3b) ────────────────────────────────
//
// The `targets` domain uses a *legacy* visibility predicate
// `($n IS NULL OR project_path = $n OR project_path = '')` that also exposes
// historical global rows (`project_path = ''`). This differs from the generic
// `scoped::*` predicate (`IS NOT DISTINCT FROM`), so these helpers preserve the
// exact legacy SQL the `golish` command layer used before delegation, for zero
// query-semantics drift. See
// `docs/superpowers/plans/2026-05-30-p0-3b-idor-residual-sink-full.md`.
//
// Row-returning helpers are generic over `T: FromRow` so the command layer can
// pass its own `TargetRow` (which decodes the enum columns as text) without
// pulling that type into `golish-db`.

/// Column projection casting enum columns to `text`, matching the command-layer
/// `TargetRow` decode shape. Trusted compile-time literal — never interpolates
/// caller input.
const TARGET_ROW_COLS: &str = "id, name, target_type::text, value, tags, notes, scope::text, status::text, grp, owner, time_window_start, time_window_end, organization_id, source, parent_id, ports, real_ip, cdn_waf, http_title, http_status, webserver, os_info, content_type, created_at, updated_at";

fn build_get_id_scoped_legacy_sql() -> String {
    "SELECT id FROM targets WHERE id = $1 AND ($2 IS NULL OR project_path = $2 OR project_path = '')"
        .to_string()
}

fn build_delete_scoped_legacy_sql() -> String {
    "DELETE FROM targets WHERE id = $1 AND ($2 IS NULL OR project_path = $2 OR project_path = '')"
        .to_string()
}

fn build_update_status_scoped_legacy_sql() -> String {
    "UPDATE targets SET status = $1::target_status, updated_at = NOW() WHERE id = $2 AND ($3 IS NULL OR project_path = $3 OR project_path = '')".to_string()
}

fn build_list_rows_legacy_sql() -> String {
    format!(
        "SELECT {TARGET_ROW_COLS} FROM targets WHERE ($1 IS NULL OR project_path = $1 OR project_path = '') ORDER BY created_at"
    )
}

fn build_list_rows_by_project_exact_sql() -> String {
    format!("SELECT {TARGET_ROW_COLS} FROM targets WHERE project_path = $1 ORDER BY created_at")
}

fn build_find_row_by_value_legacy_sql() -> String {
    format!(
        "SELECT {TARGET_ROW_COLS} FROM targets WHERE value = $1 AND ($2 IS NULL OR project_path = $2 OR project_path = '') LIMIT 1"
    )
}

fn build_match_rows_legacy_sql() -> String {
    "SELECT name, tags FROM targets WHERE ($1 IS NULL OR project_path = $1 OR project_path = '')"
        .to_string()
}

fn build_find_id_by_value_or_name_sql() -> String {
    "SELECT id FROM targets WHERE (value = $1 OR name = $1) AND (project_path = $2 OR project_path IS NULL) LIMIT 1".to_string()
}

fn build_find_id_by_value_pair_sql() -> String {
    "SELECT id FROM targets WHERE (value = $1 OR value = $2) AND (project_path = $3 OR project_path = '') LIMIT 1".to_string()
}

fn build_list_values_by_project_exact_sql() -> String {
    "SELECT value FROM targets WHERE project_path = $1".to_string()
}

fn build_clear_by_project_exact_sql() -> String {
    "DELETE FROM targets WHERE project_path = $1".to_string()
}

fn build_exists_by_value_exact_sql() -> String {
    "SELECT EXISTS(SELECT 1 FROM targets WHERE value = $1 AND project_path = $2)".to_string()
}

/// Ownership guard (legacy visibility). `None` == missing or cross-project.
pub async fn get_id_scoped_legacy(
    pool: &PgPool,
    id: Uuid,
    project_path: Option<&str>,
) -> Result<Option<Uuid>> {
    let row = sqlx::query_scalar::<_, Uuid>(&build_get_id_scoped_legacy_sql())
        .bind(id)
        .bind(project_path)
        .fetch_optional(pool)
        .await?;
    Ok(row)
}

/// Delete by id (legacy visibility). Returns rows affected (0 == missing or
/// cross-project).
pub async fn delete_scoped_legacy(
    pool: &PgPool,
    id: Uuid,
    project_path: Option<&str>,
) -> Result<u64> {
    let res = sqlx::query(&build_delete_scoped_legacy_sql())
        .bind(id)
        .bind(project_path)
        .execute(pool)
        .await?;
    Ok(res.rows_affected())
}

/// Update status by id (legacy visibility). `status` is bound then cast to the
/// `target_status` enum. Returns rows affected.
pub async fn update_status_scoped_legacy(
    pool: &PgPool,
    id: Uuid,
    status: &str,
    project_path: Option<&str>,
) -> Result<u64> {
    let res = sqlx::query(&build_update_status_scoped_legacy_sql())
        .bind(status)
        .bind(id)
        .bind(project_path)
        .execute(pool)
        .await?;
    Ok(res.rows_affected())
}

/// List rows (legacy visibility), `ORDER BY created_at`. Generic over the
/// caller's row type.
pub async fn list_rows_legacy<T>(pool: &PgPool, project_path: Option<&str>) -> Result<Vec<T>>
where
    T: for<'r> FromRow<'r, PgRow> + Send + Unpin,
{
    let rows = sqlx::query_as::<_, T>(&build_list_rows_legacy_sql())
        .bind(project_path)
        .fetch_all(pool)
        .await?;
    Ok(rows)
}

/// List rows for an exact `project_path` match, `ORDER BY created_at`.
pub async fn list_rows_by_project_exact<T>(
    pool: &PgPool,
    project_path: Option<&str>,
) -> Result<Vec<T>>
where
    T: for<'r> FromRow<'r, PgRow> + Send + Unpin,
{
    let rows = sqlx::query_as::<_, T>(&build_list_rows_by_project_exact_sql())
        .bind(project_path)
        .fetch_all(pool)
        .await?;
    Ok(rows)
}

/// First row matching `value` within legacy visibility (the `db_target_add`
/// dedup probe). `None` == no such target visible.
pub async fn find_row_by_value_legacy<T>(
    pool: &PgPool,
    value: &str,
    project_path: Option<&str>,
) -> Result<Option<T>>
where
    T: for<'r> FromRow<'r, PgRow> + Send + Unpin,
{
    let row = sqlx::query_as::<_, T>(&build_find_row_by_value_legacy_sql())
        .bind(value)
        .bind(project_path)
        .fetch_optional(pool)
        .await?;
    Ok(row)
}

/// `(name, tags)` pairs for keyword matching (legacy visibility).
pub async fn match_rows_legacy(
    pool: &PgPool,
    project_path: Option<&str>,
) -> Result<Vec<(String, serde_json::Value)>> {
    let rows = sqlx::query_as::<_, (String, serde_json::Value)>(&build_match_rows_legacy_sql())
        .bind(project_path)
        .fetch_all(pool)
        .await?;
    Ok(rows)
}

/// All target `value`s for an exact `project_path` match (batch-add dedup set).
pub async fn list_values_by_project_exact(
    pool: &PgPool,
    project_path: Option<&str>,
) -> Result<Vec<String>> {
    let rows = sqlx::query_scalar::<_, String>(&build_list_values_by_project_exact_sql())
        .bind(project_path)
        .fetch_all(pool)
        .await?;
    Ok(rows)
}

/// Delete every target for an exact `project_path` match. Returns rows affected.
pub async fn clear_by_project_exact(pool: &PgPool, project_path: Option<&str>) -> Result<u64> {
    let res = sqlx::query(&build_clear_by_project_exact_sql())
        .bind(project_path)
        .execute(pool)
        .await?;
    Ok(res.rows_affected())
}

/// Whether a target with `value` exists for an exact `project_path` match.
/// Backs the pipeline storage dedup probe (`store_target_from_item`).
pub async fn exists_by_value_exact(
    pool: &PgPool,
    value: &str,
    project_path: Option<&str>,
) -> Result<bool> {
    let exists = sqlx::query_scalar::<_, bool>(&build_exists_by_value_exact_sql())
        .bind(value)
        .bind(project_path)
        .fetch_one(pool)
        .await?;
    Ok(exists)
}

/// Resolve a target id by matching `value` **or** `name` within a project,
/// where visibility is `project_path = $2 OR project_path IS NULL`. Used by the
/// finding recorder's target back-reference. `None` == no match.
pub async fn find_id_by_value_or_name(
    pool: &PgPool,
    value_or_name: &str,
    project_path: &str,
) -> Result<Option<Uuid>> {
    let id = sqlx::query_scalar::<_, Uuid>(&build_find_id_by_value_or_name_sql())
        .bind(value_or_name)
        .bind(project_path)
        .fetch_optional(pool)
        .await?;
    Ok(id)
}

/// Resolve a target id by matching either of two `value` candidates within a
/// project, where visibility is `project_path = $3 OR project_path = ''`. Used
/// by the JS/auth pentest-bridge tools (host vs URL). `None` == no match.
pub async fn find_id_by_value_pair(
    pool: &PgPool,
    value_a: &str,
    value_b: &str,
    project_path: &str,
) -> Result<Option<Uuid>> {
    let id = sqlx::query_scalar::<_, Uuid>(&build_find_id_by_value_pair_sql())
        .bind(value_a)
        .bind(value_b)
        .bind(project_path)
        .fetch_optional(pool)
        .await?;
    Ok(id)
}

// ── Domain-write helpers (servitization S1-3) ───────────────────────────────
//
// These mirror the writes previously living in `golish-recon-app`'s
// `targets::db_*` domain functions. They were sunk here (the data layer) so the
// recon service port adapter in `golish-app-core` can back the cross-service
// `ReconTargetsPort` writes without pentest/agent depending on `golish-recon-app`.
// Row-returning helpers stay generic over the caller's `T: FromRow` (the adapter
// passes its own row type), matching the read helpers above. SQL is preserved
// verbatim for zero behaviour drift.

fn build_insert_full_sql() -> String {
    format!(
        "INSERT INTO targets (name, target_type, value, tags, notes, scope, grp, owner, time_window_start, time_window_end, organization_id, project_path, source, parent_id)
           VALUES ($1, $2::target_type, $3, '[]', '', 'in'::scope_type, $4, $5, $6, $7, $8, $9, $10, $11)
           RETURNING {TARGET_ROW_COLS}"
    )
}

/// Insert a new target with full recon columns defaulted (`tags='[]'`,
/// `notes=''`, `scope='in'`), returning the created row. Mirrors the legacy
/// `db_target_add` INSERT. The caller owns the dedup probe (see
/// [`find_row_by_value_legacy`]). Generic over the caller's row type.
#[allow(clippy::too_many_arguments)]
pub async fn insert_full<T>(
    pool: &PgPool,
    name: &str,
    target_type: &str,
    value: &str,
    grp: &str,
    owner: &str,
    time_window_start: Option<chrono::DateTime<chrono::Utc>>,
    time_window_end: Option<chrono::DateTime<chrono::Utc>>,
    organization_id: Option<Uuid>,
    project_path: Option<&str>,
    source: &str,
    parent_id: Option<Uuid>,
) -> Result<T>
where
    T: for<'r> FromRow<'r, PgRow> + Send + Unpin,
{
    let row = sqlx::query_as::<_, T>(&build_insert_full_sql())
        .bind(name)
        .bind(target_type)
        .bind(value)
        .bind(grp)
        .bind(owner)
        .bind(time_window_start)
        .bind(time_window_end)
        .bind(organization_id)
        .bind(project_path)
        .bind(source)
        .bind(parent_id)
        .fetch_one(pool)
        .await?;
    Ok(row)
}

/// Update a target's `status` by id (no project scope — the caller already owns
/// the id). Mirrors legacy `db_target_update_status`.
pub async fn update_status_by_id(pool: &PgPool, id: Uuid, status: &str) -> Result<()> {
    sqlx::query("UPDATE targets SET status=$1::target_status, updated_at=NOW() WHERE id=$2")
        .bind(status)
        .bind(id)
        .execute(pool)
        .await?;
    Ok(())
}

/// Overwrite a target's `ports` JSON by id. Mirrors legacy
/// `db_target_update_recon`.
pub async fn update_ports_by_id(pool: &PgPool, id: Uuid, ports: &serde_json::Value) -> Result<()> {
    sqlx::query("UPDATE targets SET ports=$1, updated_at=NOW() WHERE id=$2")
        .bind(ports)
        .bind(id)
        .execute(pool)
        .await?;
    Ok(())
}

/// Apply an extended recon update (httpx/nmap-derived fields) by id: only
/// non-empty scalar fields overwrite, and `ports` are merged by `(port, proto)`.
/// Mirrors legacy `db_target_update_recon_extended` (SQL verbatim).
#[allow(clippy::too_many_arguments)]
pub async fn update_recon_extended_by_id(
    pool: &PgPool,
    id: Uuid,
    real_ip: &str,
    cdn_waf: &str,
    http_title: &str,
    http_status: Option<i32>,
    webserver: &str,
    os_info: &str,
    content_type: &str,
    ports: &serde_json::Value,
) -> Result<()> {
    sqlx::query(
        r#"UPDATE targets SET
            real_ip       = CASE WHEN $1 != '' THEN $1 ELSE real_ip END,
            cdn_waf       = CASE WHEN $2 != '' THEN $2 ELSE cdn_waf END,
            http_title    = CASE WHEN $3 != '' THEN $3 ELSE http_title END,
            http_status   = COALESCE($4, http_status),
            webserver     = CASE WHEN $5 != '' THEN $5 ELSE webserver END,
            os_info       = CASE WHEN $6 != '' THEN $6 ELSE os_info END,
            content_type  = CASE WHEN $7 != '' THEN $7 ELSE content_type END,
            ports         = CASE WHEN $8::jsonb = '[]'::jsonb THEN ports
                            ELSE (
                                SELECT COALESCE(jsonb_agg(merged), '[]'::jsonb) FROM (
                                    -- Existing ports that are NOT in the new data (keep as-is)
                                    SELECT ep AS merged
                                    FROM jsonb_array_elements(ports) ep
                                    WHERE NOT EXISTS (
                                        SELECT 1 FROM jsonb_array_elements($8::jsonb) np
                                        WHERE (ep->>'port') = (np->>'port')
                                          AND COALESCE(ep->>'proto','tcp') = COALESCE(np->>'proto','tcp')
                                    )
                                    UNION ALL
                                    -- New/updated ports: merge with existing entry if present
                                    SELECT CASE
                                        WHEN ep IS NOT NULL THEN ep || np
                                        ELSE np
                                    END AS merged
                                    FROM jsonb_array_elements($8::jsonb) np
                                    LEFT JOIN LATERAL (
                                        SELECT ep FROM jsonb_array_elements(ports) ep
                                        WHERE (ep->>'port') = (np->>'port')
                                          AND COALESCE(ep->>'proto','tcp') = COALESCE(np->>'proto','tcp')
                                        LIMIT 1
                                    ) existing(ep) ON true
                                ) sub
                            ) END,
            updated_at    = NOW()
           WHERE id = $9"#,
    )
    .bind(real_ip)
    .bind(cdn_waf)
    .bind(http_title)
    .bind(http_status)
    .bind(webserver)
    .bind(os_info)
    .bind(content_type)
    .bind(ports)
    .bind(id)
    .execute(pool)
    .await?;
    Ok(())
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn reverse_lookup_sql_matches_pentest_bridge() {
        assert_eq!(
            build_find_id_by_value_or_name_sql(),
            "SELECT id FROM targets WHERE (value = $1 OR name = $1) AND (project_path = $2 OR project_path IS NULL) LIMIT 1"
        );
        assert_eq!(
            build_find_id_by_value_pair_sql(),
            "SELECT id FROM targets WHERE (value = $1 OR value = $2) AND (project_path = $3 OR project_path = '') LIMIT 1"
        );
    }

    #[test]
    fn legacy_scoped_sql_matches_command_layer() {
        assert_eq!(
            build_get_id_scoped_legacy_sql(),
            "SELECT id FROM targets WHERE id = $1 AND ($2 IS NULL OR project_path = $2 OR project_path = '')"
        );
        assert_eq!(
            build_delete_scoped_legacy_sql(),
            "DELETE FROM targets WHERE id = $1 AND ($2 IS NULL OR project_path = $2 OR project_path = '')"
        );
        assert_eq!(
            build_update_status_scoped_legacy_sql(),
            "UPDATE targets SET status = $1::target_status, updated_at = NOW() WHERE id = $2 AND ($3 IS NULL OR project_path = $3 OR project_path = '')"
        );
    }

    #[test]
    fn legacy_list_and_lookup_sql_preserve_projection_and_predicate() {
        let cols = "id, name, target_type::text, value, tags, notes, scope::text, status::text, grp, owner, time_window_start, time_window_end, organization_id, source, parent_id, ports, real_ip, cdn_waf, http_title, http_status, webserver, os_info, content_type, created_at, updated_at";
        assert_eq!(
            build_list_rows_legacy_sql(),
            format!("SELECT {cols} FROM targets WHERE ($1 IS NULL OR project_path = $1 OR project_path = '') ORDER BY created_at")
        );
        assert_eq!(
            build_list_rows_by_project_exact_sql(),
            format!("SELECT {cols} FROM targets WHERE project_path = $1 ORDER BY created_at")
        );
        assert_eq!(
            build_find_row_by_value_legacy_sql(),
            format!("SELECT {cols} FROM targets WHERE value = $1 AND ($2 IS NULL OR project_path = $2 OR project_path = '') LIMIT 1")
        );
    }

    #[test]
    fn match_and_exact_sql_match_command_layer() {
        assert_eq!(
            build_match_rows_legacy_sql(),
            "SELECT name, tags FROM targets WHERE ($1 IS NULL OR project_path = $1 OR project_path = '')"
        );
        assert_eq!(
            build_list_values_by_project_exact_sql(),
            "SELECT value FROM targets WHERE project_path = $1"
        );
        assert_eq!(
            build_clear_by_project_exact_sql(),
            "DELETE FROM targets WHERE project_path = $1"
        );
        assert_eq!(
            build_exists_by_value_exact_sql(),
            "SELECT EXISTS(SELECT 1 FROM targets WHERE value = $1 AND project_path = $2)"
        );
    }

    #[test]
    fn insert_full_sql_projects_full_row() {
        let cols = "id, name, target_type::text, value, tags, notes, scope::text, status::text, grp, owner, time_window_start, time_window_end, organization_id, source, parent_id, ports, real_ip, cdn_waf, http_title, http_status, webserver, os_info, content_type, created_at, updated_at";
        let sql = build_insert_full_sql();
        assert!(sql.starts_with("INSERT INTO targets (name, target_type, value, tags, notes, scope, grp, owner, time_window_start, time_window_end, organization_id, project_path, source, parent_id)"));
        assert!(sql.contains("'[]', '', 'in'::scope_type"));
        assert!(sql.trim_end().ends_with(&format!("RETURNING {cols}")));
    }
}
