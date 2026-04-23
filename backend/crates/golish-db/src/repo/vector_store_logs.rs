use anyhow::Result;
use sqlx::PgPool;
use uuid::Uuid;

use crate::models::{AgentType, VecStoreAction, VectorStoreLog};

pub async fn create(
    pool: &PgPool,
    session_id: Uuid,
    task_id: Option<Uuid>,
    subtask_id: Option<Uuid>,
    initiator: Option<AgentType>,
    executor: Option<AgentType>,
    action: VecStoreAction,
    query: &str,
    filter: &serde_json::Value,
    result: &str,
    result_count: i32,
    project_path: Option<&str>,
) -> Result<VectorStoreLog> {
    let row = sqlx::query_as::<_, VectorStoreLog>(
        r#"INSERT INTO vector_store_logs
           (session_id, task_id, subtask_id, initiator, executor, action, query, filter, result, result_count, project_path)
           VALUES ($1, $2, $3, $4, $5, $6, $7, $8, $9, $10, $11)
           RETURNING *"#,
    )
    .bind(session_id)
    .bind(task_id)
    .bind(subtask_id)
    .bind(initiator)
    .bind(executor)
    .bind(action)
    .bind(query)
    .bind(filter)
    .bind(result)
    .bind(result_count)
    .bind(project_path)
    .fetch_one(pool)
    .await?;
    Ok(row)
}

pub async fn list_by_session(pool: &PgPool, session_id: Uuid) -> Result<Vec<VectorStoreLog>> {
    let rows = sqlx::query_as::<_, VectorStoreLog>(
        "SELECT * FROM vector_store_logs WHERE session_id = $1 ORDER BY created_at DESC",
    )
    .bind(session_id)
    .fetch_all(pool)
    .await?;
    Ok(rows)
}

pub async fn list_by_action(
    pool: &PgPool,
    session_id: Uuid,
    action: VecStoreAction,
) -> Result<Vec<VectorStoreLog>> {
    let rows = sqlx::query_as::<_, VectorStoreLog>(
        "SELECT * FROM vector_store_logs WHERE session_id = $1 AND action = $2 ORDER BY created_at DESC",
    )
    .bind(session_id)
    .bind(action)
    .fetch_all(pool)
    .await?;
    Ok(rows)
}
