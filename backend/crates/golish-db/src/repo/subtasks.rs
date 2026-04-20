use anyhow::Result;
use sqlx::PgPool;
use uuid::Uuid;

use crate::models::{AgentType, Subtask, SubtaskStatus};

pub struct NewSubtask {
    pub task_id: Uuid,
    pub session_id: Uuid,
    pub title: Option<String>,
    pub description: Option<String>,
    pub agent: Option<AgentType>,
}

pub async fn create(pool: &PgPool, s: NewSubtask) -> Result<Subtask> {
    let row = sqlx::query_as::<_, Subtask>(
        r#"INSERT INTO subtasks (task_id, session_id, title, description, agent)
           VALUES ($1, $2, $3, $4, $5)
           RETURNING *"#,
    )
    .bind(s.task_id)
    .bind(s.session_id)
    .bind(&s.title)
    .bind(&s.description)
    .bind(s.agent)
    .fetch_one(pool)
    .await?;
    Ok(row)
}

pub async fn get(pool: &PgPool, id: Uuid) -> Result<Option<Subtask>> {
    let row = sqlx::query_as::<_, Subtask>("SELECT * FROM subtasks WHERE id = $1")
        .bind(id)
        .fetch_optional(pool)
        .await?;
    Ok(row)
}

pub async fn list_by_task(pool: &PgPool, task_id: Uuid) -> Result<Vec<Subtask>> {
    let rows = sqlx::query_as::<_, Subtask>(
        "SELECT * FROM subtasks WHERE task_id = $1 ORDER BY created_at ASC",
    )
    .bind(task_id)
    .fetch_all(pool)
    .await?;
    Ok(rows)
}

pub async fn update_status(pool: &PgPool, id: Uuid, status: SubtaskStatus) -> Result<()> {
    sqlx::query("UPDATE subtasks SET status = $1, updated_at = NOW() WHERE id = $2")
        .bind(status)
        .bind(id)
        .execute(pool)
        .await?;
    Ok(())
}

pub async fn set_result(
    pool: &PgPool,
    id: Uuid,
    result: &str,
    status: SubtaskStatus,
) -> Result<()> {
    sqlx::query(
        "UPDATE subtasks SET result = $1, status = $2, updated_at = NOW() WHERE id = $3",
    )
    .bind(result)
    .bind(status)
    .bind(id)
    .execute(pool)
    .await?;
    Ok(())
}

pub async fn set_context(pool: &PgPool, id: Uuid, context: &str) -> Result<()> {
    sqlx::query("UPDATE subtasks SET context = $1, updated_at = NOW() WHERE id = $2")
        .bind(context)
        .bind(id)
        .execute(pool)
        .await?;
    Ok(())
}

pub async fn next_pending(pool: &PgPool, task_id: Uuid) -> Result<Option<Subtask>> {
    let row = sqlx::query_as::<_, Subtask>(
        r#"SELECT * FROM subtasks
           WHERE task_id = $1 AND status = 'created'
           ORDER BY created_at ASC
           LIMIT 1"#,
    )
    .bind(task_id)
    .fetch_optional(pool)
    .await?;
    Ok(row)
}

pub async fn delete_pending(pool: &PgPool, task_id: Uuid) -> Result<u64> {
    let result = sqlx::query(
        "DELETE FROM subtasks WHERE task_id = $1 AND status = 'created'",
    )
    .bind(task_id)
    .execute(pool)
    .await?;
    Ok(result.rows_affected())
}
