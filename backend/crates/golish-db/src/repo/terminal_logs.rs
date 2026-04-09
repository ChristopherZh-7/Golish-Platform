use anyhow::Result;
use sqlx::PgPool;
use uuid::Uuid;

use crate::models::{StreamType, TerminalLog};

pub async fn create(
    pool: &PgPool,
    session_id: Uuid,
    task_id: Option<Uuid>,
    subtask_id: Option<Uuid>,
    stream: StreamType,
    content: &str,
) -> Result<TerminalLog> {
    let row = sqlx::query_as::<_, TerminalLog>(
        r#"INSERT INTO terminal_logs (session_id, task_id, subtask_id, stream, content)
           VALUES ($1, $2, $3, $4, $5)
           RETURNING *"#,
    )
    .bind(session_id)
    .bind(task_id)
    .bind(subtask_id)
    .bind(stream)
    .bind(content)
    .fetch_one(pool)
    .await?;
    Ok(row)
}

pub async fn list_by_session(pool: &PgPool, session_id: Uuid) -> Result<Vec<TerminalLog>> {
    let rows = sqlx::query_as::<_, TerminalLog>(
        "SELECT * FROM terminal_logs WHERE session_id = $1 ORDER BY created_at ASC",
    )
    .bind(session_id)
    .fetch_all(pool)
    .await?;
    Ok(rows)
}
