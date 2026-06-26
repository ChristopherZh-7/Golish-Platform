use crate::Result;
use sqlx::PgPool;
use uuid::Uuid;

use crate::models::ApiEndpoint;

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

pub async fn list_by_target(pool: &PgPool, target_id: Uuid) -> Result<Vec<ApiEndpoint>> {
    let rows = sqlx::query_as::<_, ApiEndpoint>(
        "SELECT * FROM api_endpoints WHERE target_id = $1 ORDER BY discovered_at DESC",
    )
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

pub async fn update_capture_path(pool: &PgPool, id: Uuid, capture_path: &str) -> Result<()> {
    sqlx::query("UPDATE api_endpoints SET capture_path = $2, updated_at = NOW() WHERE id = $1")
        .bind(id)
        .bind(capture_path)
        .execute(pool)
        .await?;
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
    }
}
