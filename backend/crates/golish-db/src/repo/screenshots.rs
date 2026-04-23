use anyhow::Result;
use sqlx::PgPool;
use uuid::Uuid;

use crate::models::Screenshot;

pub async fn create(
    pool: &PgPool,
    session_id: Uuid,
    task_id: Option<Uuid>,
    subtask_id: Option<Uuid>,
    name: &str,
    url: &str,
    file_path: Option<&str>,
    content_type: &str,
    size_bytes: Option<i32>,
    project_path: Option<&str>,
) -> Result<Screenshot> {
    let row = sqlx::query_as::<_, Screenshot>(
        r#"INSERT INTO screenshots (session_id, task_id, subtask_id, name, url, file_path, content_type, size_bytes, project_path)
           VALUES ($1, $2, $3, $4, $5, $6, $7, $8, $9)
           RETURNING *"#,
    )
    .bind(session_id)
    .bind(task_id)
    .bind(subtask_id)
    .bind(name)
    .bind(url)
    .bind(file_path)
    .bind(content_type)
    .bind(size_bytes)
    .bind(project_path)
    .fetch_one(pool)
    .await?;
    Ok(row)
}

pub async fn list_by_session(pool: &PgPool, session_id: Uuid) -> Result<Vec<Screenshot>> {
    let rows = sqlx::query_as::<_, Screenshot>(
        "SELECT * FROM screenshots WHERE session_id = $1 ORDER BY created_at DESC",
    )
    .bind(session_id)
    .fetch_all(pool)
    .await?;
    Ok(rows)
}
