use anyhow::Result;
use sqlx::PgPool;

use crate::models::TopologyScan;

pub async fn upsert(pool: &PgPool, name: &str, data: &serde_json::Value, project_path: Option<&str>) -> Result<()> {
    sqlx::query(
        r#"INSERT INTO topology_scans (name, data, project_path)
           VALUES ($1, $2, $3)
           ON CONFLICT (name, project_path) DO UPDATE SET data = $2, created_at = NOW()"#,
    )
    .bind(name)
    .bind(data)
    .bind(project_path)
    .execute(pool)
    .await?;
    Ok(())
}

pub async fn get(pool: &PgPool, name: &str, project_path: Option<&str>) -> Result<Option<TopologyScan>> {
    let row = sqlx::query_as::<_, TopologyScan>(
        "SELECT * FROM topology_scans WHERE name = $1 AND project_path IS NOT DISTINCT FROM $2",
    )
    .bind(name)
    .bind(project_path)
    .fetch_optional(pool)
    .await?;
    Ok(row)
}

pub async fn list(pool: &PgPool, project_path: Option<&str>) -> Result<Vec<TopologyScan>> {
    let rows = sqlx::query_as::<_, TopologyScan>(
        "SELECT * FROM topology_scans WHERE project_path IS NOT DISTINCT FROM $1 ORDER BY created_at DESC",
    )
    .bind(project_path)
    .fetch_all(pool)
    .await?;
    Ok(rows)
}

pub async fn delete(pool: &PgPool, name: &str, project_path: Option<&str>) -> Result<()> {
    sqlx::query("DELETE FROM topology_scans WHERE name = $1 AND project_path IS NOT DISTINCT FROM $2")
        .bind(name)
        .bind(project_path)
        .execute(pool)
        .await?;
    Ok(())
}
