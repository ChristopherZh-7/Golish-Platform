use anyhow::Result;
use sqlx::PgPool;
use uuid::Uuid;

use crate::models::{NewTask, Task, TaskStatus};

pub async fn create(pool: &PgPool, t: NewTask) -> Result<Task> {
    let row = sqlx::query_as::<_, Task>(
        r#"INSERT INTO tasks (session_id, title, input)
           VALUES ($1, $2, $3)
           RETURNING *"#,
    )
    .bind(t.session_id)
    .bind(&t.title)
    .bind(&t.input)
    .fetch_one(pool)
    .await?;
    Ok(row)
}

pub async fn get(pool: &PgPool, id: Uuid) -> Result<Option<Task>> {
    let row = sqlx::query_as::<_, Task>("SELECT * FROM tasks WHERE id = $1")
        .bind(id)
        .fetch_optional(pool)
        .await?;
    Ok(row)
}

pub async fn list_by_session(pool: &PgPool, session_id: Uuid) -> Result<Vec<Task>> {
    let rows = sqlx::query_as::<_, Task>(
        "SELECT * FROM tasks WHERE session_id = $1 ORDER BY created_at ASC",
    )
    .bind(session_id)
    .fetch_all(pool)
    .await?;
    Ok(rows)
}

pub async fn update_status(pool: &PgPool, id: Uuid, status: TaskStatus) -> Result<()> {
    sqlx::query("UPDATE tasks SET status = $1, updated_at = NOW() WHERE id = $2")
        .bind(status)
        .bind(id)
        .execute(pool)
        .await?;
    Ok(())
}

pub async fn set_result(pool: &PgPool, id: Uuid, result: &str, status: TaskStatus) -> Result<()> {
    sqlx::query(
        "UPDATE tasks SET result = $1, status = $2, updated_at = NOW() WHERE id = $3",
    )
    .bind(result)
    .bind(status)
    .bind(id)
    .execute(pool)
    .await?;
    Ok(())
}
