use crate::Result;
use sqlx::postgres::PgRow;
use sqlx::{FromRow, PgPool};
use uuid::Uuid;

use crate::models::{ScopeType, Target, TargetType};

const REAL_IP_TARGET_TYPE_GUARD_SQL: &str =
    "target_type::text NOT IN ('ip', 'ipv4', 'ip_address', 'cidr')";

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
const TARGET_ROW_COLS: &str = "id, name, target_type::text, value, tags, notes, scope::text, status::text, grp, owner, time_window_start, time_window_end, organization_id, source, parent_id, ports, real_ip, cdn_waf, http_title, http_status, webserver, os_info, content_type, liveness_state, liveness_reason, created_at, updated_at";

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

fn build_update_scope_scoped_legacy_sql() -> String {
    "UPDATE targets SET scope = $1::scope_type, updated_at = NOW() WHERE id = $2 AND ($3 IS NULL OR project_path = $3 OR project_path = '')".to_string()
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

fn build_list_in_scope_values_legacy_sql() -> String {
    "SELECT DISTINCT value FROM targets \
       WHERE scope::text = 'in' \
         AND ($1 IS NULL OR project_path = $1 OR project_path = '') \
         AND ($2 IS NULL OR organization_id = $2) \
       ORDER BY value"
        .to_string()
}

fn build_list_in_scope_values_before_legacy_sql() -> String {
    "SELECT DISTINCT value FROM targets \
       WHERE scope::text = 'in' \
         AND ($1 IS NULL OR project_path = $1 OR project_path = '') \
         AND ($2 IS NULL OR organization_id = $2) \
         AND created_at <= $3 \
       ORDER BY value"
        .to_string()
}

fn build_clear_by_project_exact_sql() -> String {
    "DELETE FROM targets WHERE project_path = $1".to_string()
}

fn build_exists_by_value_exact_sql() -> String {
    "SELECT EXISTS(SELECT 1 FROM targets WHERE value = $1 AND project_path = $2)".to_string()
}

fn build_artifact_reference_values_by_org_subtree_sql() -> String {
    "WITH RECURSIVE subtree AS ( \
         SELECT id FROM organizations WHERE id = $1 \
         UNION ALL \
         SELECT o.id FROM organizations o JOIN subtree s ON o.parent_id = s.id \
       ), org_targets AS ( \
         SELECT id, value, real_ip FROM targets WHERE organization_id IN (SELECT id FROM subtree) \
       ), refs AS ( \
         SELECT value AS ref FROM org_targets \
         UNION ALL SELECT real_ip AS ref FROM org_targets WHERE real_ip <> '' \
         UNION ALL SELECT ta.value AS ref FROM target_assets ta JOIN org_targets t ON t.id = ta.target_id \
         UNION ALL SELECT ae.url AS ref FROM api_endpoints ae JOIN org_targets t ON t.id = ae.target_id \
         UNION ALL SELECT de.url AS ref FROM directory_entries de JOIN org_targets t ON t.id = de.target_id \
         UNION ALL SELECT ja.url AS ref FROM js_analysis_results ja JOIN org_targets t ON t.id = ja.target_id \
         UNION ALL SELECT ps.url AS ref FROM passive_scan_logs ps JOIN org_targets t ON t.id = ps.target_id \
       ) \
       SELECT DISTINCT ref FROM refs WHERE ref IS NOT NULL AND btrim(ref) <> ''"
        .to_string()
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

/// Update `scope` by id (legacy visibility). `scope` ("in"/"out") is bound then
/// cast to the `scope_type` enum. The `project_path` predicate is the IDOR guard
/// (AGENTS.md I2) — a cross-project id matches no row. Returns rows affected
/// (0 == missing or cross-project). Mirrors the `target_update` command's scoped
/// scope write.
pub async fn update_scope_scoped_legacy(
    pool: &PgPool,
    id: Uuid,
    scope: &str,
    project_path: Option<&str>,
) -> Result<u64> {
    let res = sqlx::query(&build_update_scope_scoped_legacy_sql())
        .bind(scope)
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

/// Distinct in-scope (`scope='in'`) target `value`s within legacy visibility.
/// This is the authoritative in-scope asset set the harness coverage gate uses
/// (populated by organization recon, manual target-add, etc.). `None`
/// project_path = all visible targets (single-workspace default). `org_id`
/// narrows the set to one organization's in-scope targets (coverage asset-axis
/// isolation, design 2026-06-09); `None` keeps the legacy whole-DB behaviour.
pub async fn list_in_scope_values(
    pool: &PgPool,
    project_path: Option<&str>,
    org_id: Option<Uuid>,
) -> Result<Vec<String>> {
    let rows = sqlx::query_scalar::<_, String>(&build_list_in_scope_values_legacy_sql())
        .bind(project_path)
        .bind(org_id)
        .fetch_all(pool)
        .await?;
    Ok(rows)
}

/// Distinct in-scope target values that existed at or before `cutoff`.
///
/// Used by stage expansion wave barrier Phase 1 to freeze the current wave's
/// coverage denominator without adding a schema table. Same project/org/scope
/// visibility as [`list_in_scope_values`], plus `targets.created_at <= cutoff`.
pub async fn list_in_scope_values_created_before(
    pool: &PgPool,
    project_path: Option<&str>,
    org_id: Option<Uuid>,
    cutoff: chrono::DateTime<chrono::Utc>,
) -> Result<Vec<String>> {
    let rows = sqlx::query_scalar::<_, String>(&build_list_in_scope_values_before_legacy_sql())
        .bind(project_path)
        .bind(org_id)
        .bind(cutoff)
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

/// Collect target-bound values/URLs for an organization subtree before cascade
/// delete removes the rows. Used by callers that must clean local artifacts
/// keyed by host (captures/js/api output, sitemap entries, etc.).
pub async fn artifact_reference_values_by_org_subtree(
    pool: &PgPool,
    org_id: Uuid,
) -> Result<Vec<String>> {
    let rows =
        sqlx::query_scalar::<_, String>(&build_artifact_reference_values_by_org_subtree_sql())
            .bind(org_id)
            .fetch_all(pool)
            .await?;
    Ok(rows)
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
    // Phase D (design 2026-06-22 §3.3): port scan is a collection site, so stamp
    // `ports_scanned_at = NOW()` for the gate's time-windowed PORT truth read.
    sqlx::query(
        "UPDATE targets SET ports=$1, ports_scanned_at=NOW(), updated_at=NOW() WHERE id=$2",
    )
    .bind(ports)
    .bind(id)
    .execute(pool)
    .await?;
    Ok(())
}

/// EAS-hit alive predicate (design 2026-07-02-dead-asset-liveness-state §1.2):
/// an HTTP answer (`http_status`) or a merged-in open port. Passive `real_ip` is
/// only a primary-address cache and cannot prove reachability. Mirrors
/// `coverage_truth::build_liveness_values_sql`'s
/// alive form so the stamped `liveness_state` and the coverage-gate truth never
/// drift. `$1`=real_ip, `$4`=http_status, `$8`=incoming ports (pre-merge). Only
/// stamps `alive`; a signal-less call keeps the prior `liveness_state` (never
/// downgrades to dead — confirmed-dead marking is a separate probed-but-empty
/// sweep, not this per-hit landing write). See the caller for `$` bindings.
fn eas_hit_alive_predicate_sql() -> String {
    "($4 IS NOT NULL \
      OR ($8::jsonb <> '[]'::jsonb AND EXISTS ( \
           SELECT 1 FROM jsonb_array_elements($8::jsonb) p \
           WHERE COALESCE(p->>'state','open') = 'open')))"
        .to_string()
}

fn build_update_recon_extended_sql() -> String {
    format!(
        r#"UPDATE targets SET
            real_ip       = CASE WHEN $1 != '' AND {real_ip_guard} THEN $1 ELSE real_ip END,
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
            -- Phase D (design 2026-06-22 §3.3): stamp per-dim freshness at this
            -- collection site. PORT only when ports were actually provided
            -- ($8 != '[]'); LIVENESS only when an active signal (http_status /
            -- confirmed-open port) was provided — so passive real_ip alone does not
            -- falsely mark the dim collected this run.
            ports_scanned_at    = CASE WHEN $8::jsonb = '[]'::jsonb THEN ports_scanned_at ELSE NOW() END,
            liveness_checked_at = CASE WHEN {alive} THEN NOW() ELSE liveness_checked_at END,
            -- Dead-asset marking P2 (design 2026-07-02-dead-asset-liveness-state
            -- §4.1): stamp liveness_state='alive' when this hit proves the asset
            -- is up. Only ever sets 'alive' + clears reason — a signal-less call
            -- keeps the prior state so a landing write never mislabels an asset
            -- dead (I8: confirmed-dead is a separate probed-but-empty sweep).
            liveness_state  = CASE WHEN {alive} THEN 'alive' ELSE liveness_state END,
            liveness_reason = CASE WHEN {alive} THEN NULL ELSE liveness_reason END,
            updated_at    = NOW()
           WHERE id = $9"#,
        real_ip_guard = REAL_IP_TARGET_TYPE_GUARD_SQL,
        alive = eas_hit_alive_predicate_sql(),
    )
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
    let sql = build_update_recon_extended_sql();
    sqlx::query(&sql)
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

// ── IP-centric host tree: primary resolved IP (design 2026-06-15 Phase 0) ────

fn build_backfill_real_ip_sql() -> String {
    // Passive DNS derives a deterministic primary cache only. IPv4 is preferred;
    // within a family canonical string ordering is stable. It never stamps active
    // liveness or reachability.
    "UPDATE targets t SET real_ip = sub.ip, updated_at = NOW() \
       FROM (SELECT DISTINCT ON (target_id) target_id, value AS ip \
               FROM dns_records WHERE record_type IN ('A', 'AAAA') \
               ORDER BY target_id, CASE WHEN record_type = 'A' THEN 0 ELSE 1 END, value) sub \
      WHERE t.id = sub.target_id AND t.real_ip = '' \
        AND t.target_type::text NOT IN ('ip', 'ipv4', 'ip_address', 'cidr') \
        AND ($1 IS NULL OR t.project_path = $1)"
        .to_string()
}

fn build_set_real_ip_by_id_sql() -> &'static str {
    // Passive DNS observation: update the primary-address cache only. Reachability
    // is owned by active EAS evidence (`http_status` or confirmed-open ports).
    "UPDATE targets \
        SET real_ip = $1, updated_at = NOW() \
      WHERE id = $2 AND target_type::text NOT IN ('ip', 'ipv4', 'ip_address', 'cidr')"
}

/// Set a target's primary resolved IP (`real_ip`) by id. No project scope — the
/// caller owns the id (recon DNS landing). Idempotent: re-running overwrites.
/// Unlike [`update_recon_extended_by_id`] this is an unconditional single-column
/// write used by the host-tree resolution path.
pub async fn set_real_ip_by_id(pool: &PgPool, id: Uuid, real_ip: &str) -> Result<()> {
    sqlx::query(build_set_real_ip_by_id_sql())
        .bind(real_ip)
        .bind(id)
        .execute(pool)
        .await?;
    Ok(())
}

/// Backfill `real_ip` from the first (earliest) A record already in
/// `dns_records`, for targets that have none yet (`real_ip = ''`). Requires **no
/// new DNS resolution** — derives purely from stored answers. Returns the number
/// of target rows updated. `project_path = None` = all visible targets.
pub async fn backfill_real_ip_from_dns(pool: &PgPool, project_path: Option<&str>) -> Result<u64> {
    let res = sqlx::query(&build_backfill_real_ip_sql())
        .bind(project_path)
        .execute(pool)
        .await?;
    Ok(res.rows_affected())
}

// Shared "still has no alive signal" guard for the ongoing dead/unreachable
// marking (design 2026-07-02-dead-asset-liveness-state §4). Keeps the two setters
// byte-identical on the guard: only stamp a non-alive verdict while the row is
// not already 'alive' and carries no http_status / open port. Passive `real_ip`
// cache is deliberately absent: DNS resolution is not reachability. Makes the
// write idempotent + order-independent w.r.t. the P2 alive stamps (a host naabu
// proves has open ports stays/gets 'alive'; a later hit re-stamps 'alive',
// self-correcting). Trusted compile-time literal — no caller input interpolated.
const NO_ALIVE_SIGNAL_GUARD_SQL: &str = "liveness_state IS DISTINCT FROM 'alive' \
        AND http_status IS NULL \
        AND NOT EXISTS ( \
            SELECT 1 FROM jsonb_array_elements(ports) p \
            WHERE COALESCE(p->>'state','open') = 'open')";

fn build_mark_no_signal_liveness_by_id_sql(state: &str, reason: &str) -> String {
    // `state`/`reason` are fixed caller-chosen literals (never user input), so the
    // interpolation is injection-safe; kept as params only to share one builder
    // between the dead + unreachable setters.
    format!(
        "UPDATE targets SET \
            liveness_state = '{state}', \
            liveness_reason = '{reason}', \
            liveness_checked_at = NOW(), \
            updated_at = NOW() \
          WHERE id = $1 AND {NO_ALIVE_SIGNAL_GUARD_SQL}"
    )
}

/// EAS ongoing dead-marking (design 2026-07-02-dead-asset-liveness-state §4): the
/// counterpart to the P2 alive stamps. When an EAS liveness probe covered an
/// asset but found no signal (checked-empty), mark the matching target
/// `liveness_state='dead'`, **guarded** ([`NO_ALIVE_SIGNAL_GUARD_SQL`]) so it
/// only fires while the row genuinely has no alive signal and is not already
/// `alive`. Caller owns the id. Returns rows updated (0 when the guard held).
pub async fn mark_dead_if_no_signal_by_id(pool: &PgPool, id: Uuid) -> Result<u64> {
    let res = sqlx::query(&build_mark_no_signal_liveness_by_id_sql(
        "dead",
        "no_service",
    ))
    .bind(id)
    .execute(pool)
    .await?;
    Ok(res.rows_affected())
}

/// EAS ongoing unreachable-marking (design 2026-07-02-dead-asset-liveness-state
/// §4): like [`mark_dead_if_no_signal_by_id`] but for an asset the probe could
/// not reach (DNS resolution failure / connection refused), stamped
/// `liveness_state='unreachable'` (P3 does NOT exclude unreachable, since it may
/// be a transient network / WAF condition). Same no-alive-signal guard. Caller
/// owns the id. Returns rows updated (0 when the guard held).
pub async fn mark_unreachable_if_no_signal_by_id(pool: &PgPool, id: Uuid) -> Result<u64> {
    let res = sqlx::query(&build_mark_no_signal_liveness_by_id_sql(
        "unreachable",
        "probe_error",
    ))
    .bind(id)
    .execute(pool)
    .await?;
    Ok(res.rows_affected())
}

fn build_set_ip_whois_sql() -> String {
    // Phase D (design 2026-06-22 §3.3): RIR RDAP fetch is a collection site, so
    // stamp `ip_whois_collected_at = NOW()` for the gate's windowed IPWHOIS read.
    "UPDATE targets SET ip_whois = $1, ip_whois_collected_at = NOW(), updated_at = NOW() WHERE id = $2"
        .to_string()
}

/// Host-aware coverage 2c-3 (design 2026-06-15-host-aware-coverage-2c3-ip-native):
/// set a target's RIR/netblock IP-WHOIS (RDAP `/ip/`) JSON by id. No project
/// scope — the caller owns the id (recon IP-WHOIS landing). Idempotent: re-running
/// overwrites. Distinct from `organizations.whois` (domain RDAP, org-level).
pub async fn set_ip_whois_by_id(
    pool: &PgPool,
    id: Uuid,
    ip_whois: &serde_json::Value,
) -> Result<()> {
    sqlx::query(&build_set_ip_whois_sql())
        .bind(ip_whois)
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
        assert_eq!(
            build_update_scope_scoped_legacy_sql(),
            "UPDATE targets SET scope = $1::scope_type, updated_at = NOW() WHERE id = $2 AND ($3 IS NULL OR project_path = $3 OR project_path = '')"
        );
    }

    #[test]
    fn legacy_list_and_lookup_sql_preserve_projection_and_predicate() {
        let cols = "id, name, target_type::text, value, tags, notes, scope::text, status::text, grp, owner, time_window_start, time_window_end, organization_id, source, parent_id, ports, real_ip, cdn_waf, http_title, http_status, webserver, os_info, content_type, liveness_state, liveness_reason, created_at, updated_at";
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
    fn list_in_scope_values_sql_filters_scope_project_and_org() {
        // Coverage asset-axis isolation (design 2026-06-09): the in-scope value
        // set must be narrowable to one organization so a persistent DB with
        // residue from other orgs/runs cannot explode the coverage denominator.
        assert_eq!(
            build_list_in_scope_values_legacy_sql(),
            "SELECT DISTINCT value FROM targets \
               WHERE scope::text = 'in' \
                 AND ($1 IS NULL OR project_path = $1 OR project_path = '') \
                 AND ($2 IS NULL OR organization_id = $2) \
               ORDER BY value"
        );
    }

    #[test]
    fn list_in_scope_values_before_sql_adds_wave_cutoff() {
        assert_eq!(
            build_list_in_scope_values_before_legacy_sql(),
            "SELECT DISTINCT value FROM targets \
               WHERE scope::text = 'in' \
                 AND ($1 IS NULL OR project_path = $1 OR project_path = '') \
                 AND ($2 IS NULL OR organization_id = $2) \
                 AND created_at <= $3 \
               ORDER BY value"
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
    fn artifact_reference_values_by_org_subtree_sql_collects_target_bound_refs() {
        let sql = build_artifact_reference_values_by_org_subtree_sql();
        assert!(sql.contains("WITH RECURSIVE subtree"));
        assert!(sql.contains("JOIN subtree s ON o.parent_id = s.id"));
        assert!(sql.contains("SELECT id, value, real_ip FROM targets"));
        assert!(sql.contains("UNION ALL SELECT ta.value AS ref FROM target_assets"));
        assert!(sql.contains("UNION ALL SELECT ae.url AS ref FROM api_endpoints"));
        assert!(sql.contains("UNION ALL SELECT de.url AS ref FROM directory_entries"));
        assert!(sql.contains("UNION ALL SELECT ja.url AS ref FROM js_analysis_results"));
        assert!(sql.contains("UNION ALL SELECT ps.url AS ref FROM passive_scan_logs"));
        assert!(sql.contains("SELECT DISTINCT ref FROM refs"));
    }

    #[test]
    fn backfill_real_ip_sql_picks_first_a_record_for_unset_targets() {
        // Host-tree primary IP (design 2026-06-15 Phase 0): pick the earliest A
        // record per target, only fill targets that have no real_ip yet, and
        // honour the project_path filter (NULL = all).
        let sql = build_backfill_real_ip_sql();
        assert!(sql.contains("DISTINCT ON (target_id)"));
        assert!(sql.contains("record_type IN ('A', 'AAAA')"));
        assert!(sql.contains("CASE WHEN record_type = 'A' THEN 0 ELSE 1 END, value"));
        assert!(sql.contains("t.real_ip = ''"));
        assert!(sql.contains("t.target_type::text NOT IN ('ip', 'ipv4', 'ip_address', 'cidr')"));
        assert!(sql.contains("($1 IS NULL OR t.project_path = $1)"));
        assert!(
            !sql.contains("liveness_checked_at"),
            "passive DNS backfill must not become active liveness: {sql}"
        );
    }

    #[test]
    fn set_real_ip_by_id_sql_does_not_write_ip_targets() {
        let sql = build_set_real_ip_by_id_sql();
        assert!(sql.contains("SET real_ip = $1"));
        assert!(sql.contains("WHERE id = $2"));
        assert!(sql.contains("target_type::text NOT IN ('ip', 'ipv4', 'ip_address', 'cidr')"));
    }

    #[test]
    fn set_real_ip_by_id_sql_does_not_stamp_active_liveness() {
        // Passive DNS only records an address relation/cache. It does not prove
        // that the target is reachable in the active EAS sense.
        let sql = build_set_real_ip_by_id_sql();
        assert!(!sql.contains("liveness_state"));
        assert!(!sql.contains("liveness_reason"));
        assert!(!sql.contains("liveness_checked_at"));
    }

    #[test]
    fn update_recon_extended_sql_stamps_alive_only_on_signal() {
        // Dead-asset P2: an EAS hit landing stamps liveness_state='alive' when it
        // carries http_status / an open port, and never downgrades to
        // dead here (ELSE keeps the prior state).
        let sql = build_update_recon_extended_sql();
        assert!(sql.contains("liveness_state  = CASE WHEN"));
        assert!(sql.contains("THEN 'alive' ELSE liveness_state END"));
        assert!(sql.contains("liveness_reason = CASE WHEN"));
        assert!(sql.contains("ELSE liveness_reason END"));
        // Must not stamp dead/unreachable from this per-hit landing write.
        assert!(!sql.contains("'dead'"));
        assert!(!sql.contains("'unreachable'"));
        let alive = eas_hit_alive_predicate_sql();
        assert!(
            !alive.contains("$1"),
            "real_ip must not prove alive: {alive}"
        );
        assert!(alive.contains("$4 IS NOT NULL"));
        assert!(alive.contains("$8::jsonb"));
    }

    #[test]
    fn mark_no_signal_liveness_sql_is_guarded_for_both_verdicts() {
        // Dead-asset ongoing marking: only stamps a non-alive verdict when the row
        // still has no alive signal and is not already 'alive' (idempotent,
        // order-independent w.r.t. P2 alive stamps). dead vs unreachable share the
        // exact guard, differing only in the stamped state/reason.
        let dead = build_mark_no_signal_liveness_by_id_sql("dead", "no_service");
        assert!(dead.contains("liveness_state = 'dead'"));
        assert!(dead.contains("liveness_reason = 'no_service'"));
        let unreachable = build_mark_no_signal_liveness_by_id_sql("unreachable", "probe_error");
        assert!(unreachable.contains("liveness_state = 'unreachable'"));
        assert!(unreachable.contains("liveness_reason = 'probe_error'"));
        for sql in [&dead, &unreachable] {
            assert!(sql.contains("liveness_state IS DISTINCT FROM 'alive'"));
            assert!(sql.contains("http_status IS NULL"));
            assert!(!sql.contains("real_ip"));
            assert!(sql.contains("COALESCE(p->>'state','open') = 'open'"));
            assert!(sql.contains("WHERE id = $1"));
        }
    }

    #[test]
    fn set_ip_whois_sql_targets_ip_whois_column_by_id() {
        // Host-aware coverage 2c-3: per-IP RIR WHOIS setter writes the ip_whois
        // JSONB column, keyed by target id (caller owns the id).
        let sql = build_set_ip_whois_sql();
        assert!(sql.contains("UPDATE targets SET ip_whois = $1"));
        assert!(sql.contains("WHERE id = $2"));
        // Phase D: RDAP collection site stamps per-dim freshness.
        assert!(sql.contains("ip_whois_collected_at = NOW()"));
    }

    #[test]
    fn insert_full_sql_projects_full_row() {
        let cols = "id, name, target_type::text, value, tags, notes, scope::text, status::text, grp, owner, time_window_start, time_window_end, organization_id, source, parent_id, ports, real_ip, cdn_waf, http_title, http_status, webserver, os_info, content_type, liveness_state, liveness_reason, created_at, updated_at";
        let sql = build_insert_full_sql();
        assert!(sql.starts_with("INSERT INTO targets (name, target_type, value, tags, notes, scope, grp, owner, time_window_start, time_window_end, organization_id, project_path, source, parent_id)"));
        assert!(sql.contains("'[]', '', 'in'::scope_type"));
        assert!(sql.trim_end().ends_with(&format!("RETURNING {cols}")));
    }
}
