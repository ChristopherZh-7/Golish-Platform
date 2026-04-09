use anyhow::Result;
use sqlx::PgPool;
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
    let rows = sqlx::query_as::<_, Target>(
        "SELECT * FROM targets WHERE project_path IS NOT DISTINCT FROM $1 ORDER BY created_at DESC",
    )
    .bind(project_path)
    .fetch_all(pool)
    .await?;
    Ok(rows)
}

pub async fn get(pool: &PgPool, id: Uuid) -> Result<Option<Target>> {
    let row = sqlx::query_as::<_, Target>("SELECT * FROM targets WHERE id = $1")
        .bind(id)
        .fetch_optional(pool)
        .await?;
    Ok(row)
}

pub async fn update(pool: &PgPool, id: Uuid, name: &str, value: &str, tags: &serde_json::Value, scope: ScopeType, group: &str, notes: &str) -> Result<()> {
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
    sqlx::query("DELETE FROM targets WHERE id = $1")
        .bind(id)
        .execute(pool)
        .await?;
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
