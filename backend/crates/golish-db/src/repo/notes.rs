use anyhow::Result;
use sqlx::PgPool;
use uuid::Uuid;

use crate::models::Note;

pub async fn create(
    pool: &PgPool,
    entity_type: &str,
    entity_id: &str,
    content: &str,
    color: &str,
    project_path: Option<&str>,
) -> Result<Note> {
    let row = sqlx::query_as::<_, Note>(
        r#"INSERT INTO notes (entity_type, entity_id, content, color, project_path)
           VALUES ($1, $2, $3, $4, $5)
           RETURNING *"#,
    )
    .bind(entity_type)
    .bind(entity_id)
    .bind(content)
    .bind(color)
    .bind(project_path)
    .fetch_one(pool)
    .await?;
    Ok(row)
}

pub async fn list_for_entity(pool: &PgPool, entity_type: &str, entity_id: &str) -> Result<Vec<Note>> {
    let rows = sqlx::query_as::<_, Note>(
        "SELECT * FROM notes WHERE entity_type = $1 AND entity_id = $2 ORDER BY created_at DESC",
    )
    .bind(entity_type)
    .bind(entity_id)
    .fetch_all(pool)
    .await?;
    Ok(rows)
}

pub async fn update(pool: &PgPool, id: Uuid, content: &str, color: &str) -> Result<()> {
    sqlx::query("UPDATE notes SET content = $1, color = $2, updated_at = NOW() WHERE id = $3")
        .bind(content)
        .bind(color)
        .bind(id)
        .execute(pool)
        .await?;
    Ok(())
}

pub async fn delete(pool: &PgPool, id: Uuid) -> Result<()> {
    sqlx::query("DELETE FROM notes WHERE id = $1")
        .bind(id)
        .execute(pool)
        .await?;
    Ok(())
}
