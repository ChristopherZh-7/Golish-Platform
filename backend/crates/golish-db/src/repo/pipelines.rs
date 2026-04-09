use anyhow::Result;
use sqlx::PgPool;
use uuid::Uuid;

use crate::models::Pipeline;

pub async fn upsert(pool: &PgPool, id: Uuid, data: &serde_json::Value, project_path: Option<&str>) -> Result<()> {
    sqlx::query(
        r#"INSERT INTO pipelines (id, data, project_path)
           VALUES ($1, $2, $3)
           ON CONFLICT (id) DO UPDATE SET data = $2, updated_at = NOW()"#,
    )
    .bind(id)
    .bind(data)
    .bind(project_path)
    .execute(pool)
    .await?;
    Ok(())
}

pub async fn get(pool: &PgPool, id: Uuid) -> Result<Option<Pipeline>> {
    let row = sqlx::query_as::<_, Pipeline>("SELECT * FROM pipelines WHERE id = $1")
        .bind(id)
        .fetch_optional(pool)
        .await?;
    Ok(row)
}

pub async fn list(pool: &PgPool, project_path: Option<&str>) -> Result<Vec<Pipeline>> {
    let rows = sqlx::query_as::<_, Pipeline>(
        "SELECT * FROM pipelines WHERE project_path IS NOT DISTINCT FROM $1 ORDER BY updated_at DESC",
    )
    .bind(project_path)
    .fetch_all(pool)
    .await?;
    Ok(rows)
}

pub async fn delete(pool: &PgPool, id: Uuid) -> Result<()> {
    sqlx::query("DELETE FROM pipelines WHERE id = $1")
        .bind(id)
        .execute(pool)
        .await?;
    Ok(())
}
