use sqlx::PgPool;
use uuid::Uuid;

use crate::models::Note;
use crate::Result;

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

pub async fn list_for_entity(
    pool: &PgPool,
    entity_type: &str,
    entity_id: &str,
) -> Result<Vec<Note>> {
    let rows = sqlx::query_as::<_, Note>(
        "SELECT * FROM notes WHERE entity_type = $1 AND entity_id = $2 ORDER BY created_at DESC",
    )
    .bind(entity_type)
    .bind(entity_id)
    .fetch_all(pool)
    .await?;
    Ok(rows)
}

pub async fn list_filtered(
    pool: &PgPool,
    entity_type: Option<&str>,
    entity_id: Option<&str>,
    project_path: Option<&str>,
) -> Result<Vec<Note>> {
    let rows = sqlx::query_as::<_, Note>(
        r#"SELECT * FROM notes
           WHERE ($1::text IS NULL OR entity_type = $1)
             AND ($2::text IS NULL OR entity_id = $2)
             AND project_path IS NOT DISTINCT FROM $3
           ORDER BY created_at DESC"#,
    )
    .bind(entity_type)
    .bind(entity_id)
    .bind(project_path)
    .fetch_all(pool)
    .await?;
    Ok(rows)
}

/// Update a note, scoped to `project_path` (AGENTS.md I2). Returns the number of
/// affected rows so the caller can reject cross-project ids (zero rows == the
/// note is missing or belongs to another project).
pub async fn update(
    pool: &PgPool,
    id: Uuid,
    content: &str,
    color: &str,
    project_path: Option<&str>,
) -> Result<u64> {
    let res = sqlx::query(
        "UPDATE notes SET content = $1, color = $2, updated_at = NOW() \
         WHERE id = $3 AND project_path IS NOT DISTINCT FROM $4",
    )
    .bind(content)
    .bind(color)
    .bind(id)
    .bind(project_path)
    .execute(pool)
    .await?;
    Ok(res.rows_affected())
}

/// Delete a note, scoped to `project_path` (AGENTS.md I2). Returns the number of
/// affected rows so the caller can reject cross-project ids.
pub async fn delete(pool: &PgPool, id: Uuid, project_path: Option<&str>) -> Result<u64> {
    super::scoped::delete_scoped(pool, "notes", id, project_path).await
}
