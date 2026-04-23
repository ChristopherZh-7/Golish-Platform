use anyhow::Result;
use sqlx::PgPool;
use uuid::Uuid;

use crate::models::{AgentType, MsgLog, MsgLogResultFormat, MsgLogType};

pub async fn create(
    pool: &PgPool,
    session_id: Uuid,
    task_id: Option<Uuid>,
    subtask_id: Option<Uuid>,
    agent: Option<AgentType>,
    msg_type: MsgLogType,
    message: &str,
    thinking: Option<&str>,
    project_path: Option<&str>,
) -> Result<MsgLog> {
    let row = sqlx::query_as::<_, MsgLog>(
        r#"INSERT INTO msg_logs (session_id, task_id, subtask_id, agent, msg_type, message, thinking, project_path)
           VALUES ($1, $2, $3, $4, $5, $6, $7, $8)
           RETURNING *"#,
    )
    .bind(session_id)
    .bind(task_id)
    .bind(subtask_id)
    .bind(agent)
    .bind(msg_type)
    .bind(message)
    .bind(thinking)
    .bind(project_path)
    .fetch_one(pool)
    .await?;
    Ok(row)
}

pub async fn update_result(
    pool: &PgPool,
    id: Uuid,
    result: &str,
    result_format: MsgLogResultFormat,
) -> Result<()> {
    sqlx::query("UPDATE msg_logs SET result = $1, result_format = $2 WHERE id = $3")
        .bind(result)
        .bind(result_format)
        .bind(id)
        .execute(pool)
        .await?;
    Ok(())
}

pub async fn list_by_session(pool: &PgPool, session_id: Uuid) -> Result<Vec<MsgLog>> {
    let rows = sqlx::query_as::<_, MsgLog>(
        "SELECT * FROM msg_logs WHERE session_id = $1 ORDER BY created_at ASC",
    )
    .bind(session_id)
    .fetch_all(pool)
    .await?;
    Ok(rows)
}

pub async fn list_by_task(pool: &PgPool, task_id: Uuid) -> Result<Vec<MsgLog>> {
    let rows = sqlx::query_as::<_, MsgLog>(
        "SELECT * FROM msg_logs WHERE task_id = $1 ORDER BY created_at ASC",
    )
    .bind(task_id)
    .fetch_all(pool)
    .await?;
    Ok(rows)
}

pub async fn list_by_subtask(pool: &PgPool, subtask_id: Uuid) -> Result<Vec<MsgLog>> {
    let rows = sqlx::query_as::<_, MsgLog>(
        "SELECT * FROM msg_logs WHERE subtask_id = $1 ORDER BY created_at ASC",
    )
    .bind(subtask_id)
    .fetch_all(pool)
    .await?;
    Ok(rows)
}
