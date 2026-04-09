use anyhow::Result;
use sqlx::PgPool;
use uuid::Uuid;

use crate::models::{AgentType, SearchLog};

pub async fn create(
    pool: &PgPool,
    session_id: Uuid,
    task_id: Option<Uuid>,
    subtask_id: Option<Uuid>,
    initiator: Option<AgentType>,
    engine: &str,
    query: &str,
    result: Option<&str>,
) -> Result<SearchLog> {
    let row = sqlx::query_as::<_, SearchLog>(
        r#"INSERT INTO search_logs (session_id, task_id, subtask_id, initiator, engine, query, result)
           VALUES ($1, $2, $3, $4, $5, $6, $7)
           RETURNING *"#,
    )
    .bind(session_id)
    .bind(task_id)
    .bind(subtask_id)
    .bind(initiator)
    .bind(engine)
    .bind(query)
    .bind(result)
    .fetch_one(pool)
    .await?;
    Ok(row)
}

pub async fn list_by_session(pool: &PgPool, session_id: Uuid) -> Result<Vec<SearchLog>> {
    let rows = sqlx::query_as::<_, SearchLog>(
        "SELECT * FROM search_logs WHERE session_id = $1 ORDER BY created_at ASC",
    )
    .bind(session_id)
    .fetch_all(pool)
    .await?;
    Ok(rows)
}
