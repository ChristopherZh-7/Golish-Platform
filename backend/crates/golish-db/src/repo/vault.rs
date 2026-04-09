use anyhow::Result;
use sqlx::PgPool;
use uuid::Uuid;

use crate::models::{VaultEntry, VaultEntryType};

pub async fn create(
    pool: &PgPool,
    name: &str,
    entry_type: VaultEntryType,
    value: &str,
    username: &str,
    notes: &str,
    project: &str,
    tags: &serde_json::Value,
    project_path: Option<&str>,
) -> Result<VaultEntry> {
    let row = sqlx::query_as::<_, VaultEntry>(
        r#"INSERT INTO vault_entries (name, entry_type, value, username, notes, project, tags, project_path)
           VALUES ($1, $2, $3, $4, $5, $6, $7, $8)
           RETURNING *"#,
    )
    .bind(name)
    .bind(entry_type)
    .bind(value)
    .bind(username)
    .bind(notes)
    .bind(project)
    .bind(tags)
    .bind(project_path)
    .fetch_one(pool)
    .await?;
    Ok(row)
}

pub async fn list(pool: &PgPool, project_path: Option<&str>) -> Result<Vec<VaultEntry>> {
    let rows = sqlx::query_as::<_, VaultEntry>(
        "SELECT * FROM vault_entries WHERE project_path IS NOT DISTINCT FROM $1 ORDER BY created_at DESC",
    )
    .bind(project_path)
    .fetch_all(pool)
    .await?;
    Ok(rows)
}

pub async fn get(pool: &PgPool, id: Uuid) -> Result<Option<VaultEntry>> {
    let row = sqlx::query_as::<_, VaultEntry>("SELECT * FROM vault_entries WHERE id = $1")
        .bind(id)
        .fetch_optional(pool)
        .await?;
    Ok(row)
}

pub async fn update(pool: &PgPool, id: Uuid, name: &str, value: &str, username: &str, notes: &str, tags: &serde_json::Value) -> Result<()> {
    sqlx::query(
        "UPDATE vault_entries SET name=$1, value=$2, username=$3, notes=$4, tags=$5, updated_at=NOW() WHERE id=$6",
    )
    .bind(name).bind(value).bind(username).bind(notes).bind(tags).bind(id)
    .execute(pool)
    .await?;
    Ok(())
}

pub async fn delete(pool: &PgPool, id: Uuid) -> Result<()> {
    sqlx::query("DELETE FROM vault_entries WHERE id = $1")
        .bind(id)
        .execute(pool)
        .await?;
    Ok(())
}
