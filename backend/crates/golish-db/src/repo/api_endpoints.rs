use anyhow::Result;
use sqlx::PgPool;
use uuid::Uuid;

use crate::models::ApiEndpoint;

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

pub async fn mark_tested(pool: &PgPool, id: Uuid, status_code: Option<i32>, notes: &str) -> Result<()> {
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
    sqlx::query("DELETE FROM api_endpoints WHERE id = $1")
        .bind(id)
        .execute(pool)
        .await?;
    Ok(())
}
