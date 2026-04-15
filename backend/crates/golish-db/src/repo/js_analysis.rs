use anyhow::Result;
use sqlx::PgPool;
use uuid::Uuid;

use crate::models::JsAnalysisResult;

pub async fn insert(
    pool: &PgPool,
    target_id: Uuid,
    project_path: Option<&str>,
    url: &str,
    filename: &str,
    size_bytes: Option<i64>,
    hash_sha256: Option<&str>,
    frameworks: &serde_json::Value,
    libraries: &serde_json::Value,
    endpoints_found: &serde_json::Value,
    secrets_found: &serde_json::Value,
    comments: &serde_json::Value,
    source_maps: bool,
    risk_summary: &str,
    raw_analysis: &serde_json::Value,
) -> Result<JsAnalysisResult> {
    let row = sqlx::query_as::<_, JsAnalysisResult>(
        r#"INSERT INTO js_analysis_results
               (target_id, project_path, url, filename, size_bytes, hash_sha256,
                frameworks, libraries, endpoints_found, secrets_found, comments,
                source_maps, risk_summary, raw_analysis)
           VALUES ($1, $2, $3, $4, $5, $6, $7, $8, $9, $10, $11, $12, $13, $14)
           RETURNING *"#,
    )
    .bind(target_id)
    .bind(project_path)
    .bind(url)
    .bind(filename)
    .bind(size_bytes)
    .bind(hash_sha256)
    .bind(frameworks)
    .bind(libraries)
    .bind(endpoints_found)
    .bind(secrets_found)
    .bind(comments)
    .bind(source_maps)
    .bind(risk_summary)
    .bind(raw_analysis)
    .fetch_one(pool)
    .await?;
    Ok(row)
}

pub async fn list_by_target(pool: &PgPool, target_id: Uuid) -> Result<Vec<JsAnalysisResult>> {
    let rows = sqlx::query_as::<_, JsAnalysisResult>(
        "SELECT * FROM js_analysis_results WHERE target_id = $1 ORDER BY analyzed_at DESC",
    )
    .bind(target_id)
    .fetch_all(pool)
    .await?;
    Ok(rows)
}

pub async fn get(pool: &PgPool, id: Uuid) -> Result<Option<JsAnalysisResult>> {
    let row = sqlx::query_as::<_, JsAnalysisResult>(
        "SELECT * FROM js_analysis_results WHERE id = $1",
    )
    .bind(id)
    .fetch_optional(pool)
    .await?;
    Ok(row)
}

pub async fn delete(pool: &PgPool, id: Uuid) -> Result<()> {
    sqlx::query("DELETE FROM js_analysis_results WHERE id = $1")
        .bind(id)
        .execute(pool)
        .await?;
    Ok(())
}
