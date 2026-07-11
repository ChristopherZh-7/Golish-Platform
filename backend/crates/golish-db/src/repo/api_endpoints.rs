use crate::Result;
use sqlx::PgPool;
use uuid::Uuid;

use crate::models::ApiEndpoint;
use crate::repo::scoped::{lock_target_write_guard, TargetWriteGuard};

/// Insert an endpoint, or — on `(target_id, url, method)` conflict — merge the
/// supplied `params` into the existing row's params as a deduped, sorted set
/// union. Backs the AI-assisted JS param recipe (设计 2026-06-26 §4.3): body /
/// form param names the regex pass cannot see are folded into the row WITHOUT
/// dropping the URL-query params already stored. `headers/auth_type/source/
/// risk_level` only seed a brand-new row; on conflict they are left untouched so
/// a second pass cannot downgrade earlier provenance.
const UPSERT_MERGE_PARAMS_SQL: &str = r#"INSERT INTO api_endpoints
        (target_id, project_path, url, method, path, params, headers, auth_type, source, risk_level)
    VALUES ($1, $2, $3, $4, $5, $6, $7, $8, $9, $10)
    ON CONFLICT (target_id, url, method) DO UPDATE SET
        params = (
            SELECT COALESCE(jsonb_agg(DISTINCT name ORDER BY name), '[]'::jsonb)
            FROM (
                SELECT jsonb_array_elements_text(
                    CASE WHEN jsonb_typeof(api_endpoints.params) = 'array'
                         THEN api_endpoints.params ELSE '[]'::jsonb END
                ) AS name
                UNION
                SELECT jsonb_array_elements_text(
                    CASE WHEN jsonb_typeof(EXCLUDED.params) = 'array'
                         THEN EXCLUDED.params ELSE '[]'::jsonb END
                ) AS name
            ) merged
        ),
        updated_at = NOW()
    RETURNING *"#;

/// Guarded writers must not update a stale row from an earlier project binding
/// when the `(target_id, url, method)` unique key conflicts. A false conflict
/// predicate returns no row, which makes `fetch_one` fail and rolls back the
/// short authorization transaction.
const GUARDED_UPSERT_MERGE_PARAMS_SQL: &str = r#"INSERT INTO api_endpoints
        (target_id, project_path, url, method, path, params, headers, auth_type, source, risk_level)
    VALUES ($1, $2, $3, $4, $5, $6, $7, $8, $9, $10)
    ON CONFLICT (target_id, url, method) DO UPDATE SET
        params = (
            SELECT COALESCE(jsonb_agg(DISTINCT name ORDER BY name), '[]'::jsonb)
            FROM (
                SELECT jsonb_array_elements_text(
                    CASE WHEN jsonb_typeof(api_endpoints.params) = 'array'
                         THEN api_endpoints.params ELSE '[]'::jsonb END
                ) AS name
                UNION
                SELECT jsonb_array_elements_text(
                    CASE WHEN jsonb_typeof(EXCLUDED.params) = 'array'
                         THEN EXCLUDED.params ELSE '[]'::jsonb END
                ) AS name
            ) merged
        ),
        updated_at = NOW()
    WHERE api_endpoints.project_path IS NOT DISTINCT FROM EXCLUDED.project_path
    RETURNING *"#;

const GUARDED_INSERT_OR_IGNORE_SQL: &str = r#"INSERT INTO api_endpoints
       (target_id, project_path, url, method, path, params, headers, auth_type, source, risk_level)
   VALUES ($1, $2, $3, $4, $5, $6, $7, $8, $9, $10)
   ON CONFLICT (target_id, url, method) DO NOTHING
   RETURNING *"#;

const GUARDED_SELECT_EXISTING_SQL: &str = r#"SELECT *
   FROM api_endpoints
   WHERE target_id = $1
     AND url = $2
     AND method = $3
     AND project_path IS NOT DISTINCT FROM $4"#;

fn build_list_by_current_target_owner_sql() -> &'static str {
    r#"SELECT ae.*
       FROM api_endpoints ae
       JOIN targets t ON t.id = ae.target_id
       WHERE ae.target_id = $1
         AND t.scope::text = 'in'
         AND ae.project_path IS NOT DISTINCT FROM t.project_path
       ORDER BY ae.discovered_at DESC"#
}

fn build_list_untested_by_current_target_owner_sql() -> &'static str {
    r#"SELECT ae.*
       FROM api_endpoints ae
       JOIN targets t ON t.id = ae.target_id
       WHERE ae.target_id = $1
         AND t.scope::text = 'in'
         AND ae.project_path IS NOT DISTINCT FROM t.project_path
         AND ae.tested = false
       ORDER BY ae.risk_level DESC, ae.discovered_at DESC"#
}

fn build_count_by_current_target_owner_sql() -> &'static str {
    r#"SELECT COUNT(*), COUNT(*) FILTER (WHERE ae.tested = true)
       FROM api_endpoints ae
       JOIN targets t ON t.id = ae.target_id
       WHERE ae.target_id = $1
         AND t.scope::text = 'in'
         AND ae.project_path IS NOT DISTINCT FROM t.project_path"#
}

pub async fn insert(
    pool: &PgPool,
    target_id: Uuid,
    project_path: Option<&str>,
    url: &str,
    method: &str,
    path: &str,
    params: &serde_json::Value,
    headers: &serde_json::Value,
    auth_type: Option<&str>,
    source: &str,
    risk_level: &str,
) -> Result<ApiEndpoint> {
    let row = sqlx::query_as::<_, ApiEndpoint>(
        r#"INSERT INTO api_endpoints
               (target_id, project_path, url, method, path, params, headers, auth_type, source, risk_level)
           VALUES ($1, $2, $3, $4, $5, $6, $7, $8, $9, $10)
           RETURNING *"#,
    )
    .bind(target_id)
    .bind(project_path)
    .bind(url)
    .bind(method)
    .bind(path)
    .bind(params)
    .bind(headers)
    .bind(auth_type)
    .bind(source)
    .bind(risk_level)
    .fetch_one(pool)
    .await?;
    Ok(row)
}

/// Insert an endpoint only while the exact target authorization row still
/// matches `guard`. The target lock and child insert share one short
/// transaction, closing the producer's revalidate-to-write race.
#[allow(clippy::too_many_arguments)]
pub async fn insert_guarded(
    pool: &PgPool,
    guard: &TargetWriteGuard,
    url: &str,
    method: &str,
    path: &str,
    params: &serde_json::Value,
    headers: &serde_json::Value,
    auth_type: Option<&str>,
    source: &str,
    risk_level: &str,
) -> Result<ApiEndpoint> {
    let mut tx = pool.begin().await?;
    lock_target_write_guard(&mut tx, guard).await?;
    let row = sqlx::query_as::<_, ApiEndpoint>(
        r#"INSERT INTO api_endpoints
               (target_id, project_path, url, method, path, params, headers, auth_type, source, risk_level)
           VALUES ($1, $2, $3, $4, $5, $6, $7, $8, $9, $10)
           RETURNING *"#,
    )
    .bind(guard.target_id)
    .bind(&guard.project_path)
    .bind(url)
    .bind(method)
    .bind(path)
    .bind(params)
    .bind(headers)
    .bind(auth_type)
    .bind(source)
    .bind(risk_level)
    .fetch_one(&mut *tx)
    .await?;
    tx.commit().await?;
    Ok(row)
}

/// Idempotent guarded insert for crawler-style producers.
///
/// The target authorization lock and insert share one short transaction. A
/// duplicate `(target_id, url, method)` is accepted only when the existing row
/// has the same project binding; a stale/foreign-project conflict fails closed
/// instead of being mistaken for a successful no-op.
#[allow(clippy::too_many_arguments)]
pub async fn insert_or_ignore_guarded(
    pool: &PgPool,
    guard: &TargetWriteGuard,
    url: &str,
    method: &str,
    path: &str,
    params: &serde_json::Value,
    headers: &serde_json::Value,
    auth_type: Option<&str>,
    source: &str,
    risk_level: &str,
) -> Result<ApiEndpoint> {
    let mut tx = pool.begin().await?;
    lock_target_write_guard(&mut tx, guard).await?;
    let inserted = sqlx::query_as::<_, ApiEndpoint>(GUARDED_INSERT_OR_IGNORE_SQL)
        .bind(guard.target_id)
        .bind(&guard.project_path)
        .bind(url)
        .bind(method)
        .bind(path)
        .bind(params)
        .bind(headers)
        .bind(auth_type)
        .bind(source)
        .bind(risk_level)
        .fetch_optional(&mut *tx)
        .await?;

    let row = match inserted {
        Some(row) => row,
        None => sqlx::query_as::<_, ApiEndpoint>(GUARDED_SELECT_EXISTING_SQL)
        .bind(guard.target_id)
        .bind(url)
        .bind(method)
        .bind(&guard.project_path)
        .fetch_optional(&mut *tx)
        .await?
        .ok_or_else(|| {
            crate::DbError::NotFound(format!(
                "guarded api endpoint conflict has a different project binding for {} {method} {url}",
                guard.target_id
            ))
        })?,
    };

    tx.commit().await?;
    Ok(row)
}

/// See [`UPSERT_MERGE_PARAMS_SQL`]. Returns the inserted-or-merged row.
#[allow(clippy::too_many_arguments)]
pub async fn upsert_merge_params(
    pool: &PgPool,
    target_id: Uuid,
    project_path: Option<&str>,
    url: &str,
    method: &str,
    path: &str,
    params: &serde_json::Value,
    headers: &serde_json::Value,
    auth_type: Option<&str>,
    source: &str,
    risk_level: &str,
) -> Result<ApiEndpoint> {
    let row = sqlx::query_as::<_, ApiEndpoint>(UPSERT_MERGE_PARAMS_SQL)
        .bind(target_id)
        .bind(project_path)
        .bind(url)
        .bind(method)
        .bind(path)
        .bind(params)
        .bind(headers)
        .bind(auth_type)
        .bind(source)
        .bind(risk_level)
        .fetch_one(pool)
        .await?;
    Ok(row)
}

/// Guarded counterpart of [`upsert_merge_params`]. The authorization row lock,
/// raw snapshot comparison, and endpoint upsert are committed atomically.
#[allow(clippy::too_many_arguments)]
pub async fn upsert_merge_params_guarded(
    pool: &PgPool,
    guard: &TargetWriteGuard,
    url: &str,
    method: &str,
    path: &str,
    params: &serde_json::Value,
    headers: &serde_json::Value,
    auth_type: Option<&str>,
    source: &str,
    risk_level: &str,
) -> Result<ApiEndpoint> {
    let mut tx = pool.begin().await?;
    lock_target_write_guard(&mut tx, guard).await?;
    let row = sqlx::query_as::<_, ApiEndpoint>(GUARDED_UPSERT_MERGE_PARAMS_SQL)
        .bind(guard.target_id)
        .bind(&guard.project_path)
        .bind(url)
        .bind(method)
        .bind(path)
        .bind(params)
        .bind(headers)
        .bind(auth_type)
        .bind(source)
        .bind(risk_level)
        .fetch_one(&mut *tx)
        .await?;
    tx.commit().await?;
    Ok(row)
}

pub async fn list_by_target(pool: &PgPool, target_id: Uuid) -> Result<Vec<ApiEndpoint>> {
    let rows = sqlx::query_as::<_, ApiEndpoint>(
        "SELECT * FROM api_endpoints WHERE target_id = $1 ORDER BY discovered_at DESC",
    )
    .bind(target_id)
    .fetch_all(pool)
    .await?;
    Ok(rows)
}

/// List only endpoint rows that still match the target's current in-scope
/// project binding. Historical rows remain preserved but do not follow a
/// target into a different workspace.
pub async fn list_by_current_target_owner(
    pool: &PgPool,
    target_id: Uuid,
) -> Result<Vec<ApiEndpoint>> {
    let rows = sqlx::query_as::<_, ApiEndpoint>(build_list_by_current_target_owner_sql())
        .bind(target_id)
        .fetch_all(pool)
        .await?;
    Ok(rows)
}

pub async fn list_untested(pool: &PgPool, target_id: Uuid) -> Result<Vec<ApiEndpoint>> {
    let rows = sqlx::query_as::<_, ApiEndpoint>(
        "SELECT * FROM api_endpoints WHERE target_id = $1 AND tested = false ORDER BY risk_level DESC, discovered_at DESC",
    )
    .bind(target_id)
    .fetch_all(pool)
    .await?;
    Ok(rows)
}

pub async fn list_untested_by_current_target_owner(
    pool: &PgPool,
    target_id: Uuid,
) -> Result<Vec<ApiEndpoint>> {
    let rows = sqlx::query_as::<_, ApiEndpoint>(build_list_untested_by_current_target_owner_sql())
        .bind(target_id)
        .fetch_all(pool)
        .await?;
    Ok(rows)
}

pub async fn mark_tested(
    pool: &PgPool,
    id: Uuid,
    status_code: Option<i32>,
    notes: &str,
) -> Result<()> {
    sqlx::query(
        "UPDATE api_endpoints SET tested = true, status_code = $2, notes = $3, updated_at = NOW() WHERE id = $1",
    )
    .bind(id)
    .bind(status_code)
    .bind(notes)
    .execute(pool)
    .await?;
    Ok(())
}

pub async fn count_by_target(pool: &PgPool, target_id: Uuid) -> Result<(i64, i64)> {
    let (total,): (i64,) =
        sqlx::query_as("SELECT COUNT(*) FROM api_endpoints WHERE target_id = $1")
            .bind(target_id)
            .fetch_one(pool)
            .await?;
    let (tested,): (i64,) =
        sqlx::query_as("SELECT COUNT(*) FROM api_endpoints WHERE target_id = $1 AND tested = true")
            .bind(target_id)
            .fetch_one(pool)
            .await?;
    Ok((total, tested))
}

pub async fn count_by_current_target_owner(pool: &PgPool, target_id: Uuid) -> Result<(i64, i64)> {
    let counts = sqlx::query_as::<_, (i64, i64)>(build_count_by_current_target_owner_sql())
        .bind(target_id)
        .fetch_one(pool)
        .await?;
    Ok(counts)
}

pub async fn update_capture_path(pool: &PgPool, id: Uuid, capture_path: &str) -> Result<()> {
    sqlx::query("UPDATE api_endpoints SET capture_path = $2, updated_at = NOW() WHERE id = $1")
        .bind(id)
        .bind(capture_path)
        .execute(pool)
        .await?;
    Ok(())
}

pub async fn update_response_evidence(
    pool: &PgPool,
    id: Uuid,
    headers: &serde_json::Value,
    response_type: Option<&str>,
    status_code: Option<i32>,
    capture_path: Option<&str>,
) -> Result<()> {
    sqlx::query(
        r#"UPDATE api_endpoints
           SET headers = $2,
               response_type = COALESCE($3, response_type),
               status_code = COALESCE($4, status_code),
               capture_path = COALESCE($5, capture_path),
               updated_at = NOW()
           WHERE id = $1"#,
    )
    .bind(id)
    .bind(headers)
    .bind(response_type)
    .bind(status_code)
    .bind(capture_path)
    .execute(pool)
    .await?;
    Ok(())
}

/// Update response evidence only when both the target authorization snapshot
/// and the endpoint's target/project ownership still match.
pub async fn update_response_evidence_guarded(
    pool: &PgPool,
    guard: &TargetWriteGuard,
    id: Uuid,
    headers: &serde_json::Value,
    response_type: Option<&str>,
    status_code: Option<i32>,
    capture_path: Option<&str>,
) -> Result<()> {
    let mut tx = pool.begin().await?;
    lock_target_write_guard(&mut tx, guard).await?;
    let updated = sqlx::query_scalar::<_, Uuid>(
        r#"UPDATE api_endpoints
           SET headers = $2,
               response_type = COALESCE($3, response_type),
               status_code = COALESCE($4, status_code),
               capture_path = COALESCE($5, capture_path),
               updated_at = NOW()
           WHERE id = $1
             AND target_id = $6
             AND project_path IS NOT DISTINCT FROM $7
           RETURNING id"#,
    )
    .bind(id)
    .bind(headers)
    .bind(response_type)
    .bind(status_code)
    .bind(capture_path)
    .bind(guard.target_id)
    .bind(&guard.project_path)
    .fetch_optional(&mut *tx)
    .await?;
    if updated.is_none() {
        return Err(crate::DbError::NotFound(format!(
            "guarded api endpoint response update rejected for {id}"
        )));
    }
    tx.commit().await?;
    Ok(())
}

pub async fn delete(pool: &PgPool, id: Uuid) -> Result<()> {
    super::scoped::delete_by_id(pool, "api_endpoints", id).await?;
    Ok(())
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn upsert_merge_params_sql_conflicts_on_target_url_method_and_unions_params() {
        // Conflict target must match uq_api_endpoint_target_url_method so the
        // merge fires; on conflict only params (+ updated_at) change.
        assert!(UPSERT_MERGE_PARAMS_SQL.contains("ON CONFLICT (target_id, url, method) DO UPDATE"));
        assert!(UPSERT_MERGE_PARAMS_SQL.contains("jsonb_agg(DISTINCT name ORDER BY name)"));
        assert!(UPSERT_MERGE_PARAMS_SQL.contains("api_endpoints.params"));
        assert!(UPSERT_MERGE_PARAMS_SQL.contains("EXCLUDED.params"));
        assert!(UPSERT_MERGE_PARAMS_SQL.contains("updated_at = NOW()"));
        // Must NOT clobber provenance columns on conflict (only params change).
        assert!(!UPSERT_MERGE_PARAMS_SQL.contains("source = EXCLUDED.source"));
        assert!(!UPSERT_MERGE_PARAMS_SQL.contains("auth_type = EXCLUDED.auth_type"));

        assert!(GUARDED_UPSERT_MERGE_PARAMS_SQL
            .contains("ON CONFLICT (target_id, url, method) DO UPDATE"));
        assert!(GUARDED_UPSERT_MERGE_PARAMS_SQL.contains(
            "WHERE api_endpoints.project_path IS NOT DISTINCT FROM EXCLUDED.project_path"
        ));
    }

    #[test]
    fn guarded_idempotent_insert_checks_project_on_conflict() {
        assert!(GUARDED_INSERT_OR_IGNORE_SQL
            .contains("ON CONFLICT (target_id, url, method) DO NOTHING"));
        assert!(GUARDED_INSERT_OR_IGNORE_SQL.contains("RETURNING *"));
        assert!(GUARDED_SELECT_EXISTING_SQL.contains("project_path IS NOT DISTINCT FROM $4"));
    }

    #[test]
    fn current_owner_reads_join_target_scope_and_project() {
        for sql in [
            build_list_by_current_target_owner_sql(),
            build_list_untested_by_current_target_owner_sql(),
            build_count_by_current_target_owner_sql(),
        ] {
            assert!(sql.contains("JOIN targets t ON t.id = ae.target_id"));
            assert!(sql.contains("t.scope::text = 'in'"));
            assert!(sql.contains("ae.project_path IS NOT DISTINCT FROM t.project_path"));
        }
    }
}
