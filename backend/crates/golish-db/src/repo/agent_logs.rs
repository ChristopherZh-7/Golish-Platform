use anyhow::Result;
use sqlx::PgPool;
use uuid::Uuid;

use crate::models::{AgentLog, AgentType, NewAgentLog};

pub async fn create(pool: &PgPool, log: NewAgentLog) -> Result<AgentLog> {
    let row = sqlx::query_as::<_, AgentLog>(
        r#"INSERT INTO agent_logs (session_id, task_id, subtask_id, initiator, executor, task)
           VALUES ($1, $2, $3, $4, $5, $6)
           RETURNING *"#,
    )
    .bind(log.session_id)
    .bind(log.task_id)
    .bind(log.subtask_id)
    .bind(log.initiator)
    .bind(log.executor)
    .bind(&log.task)
    .fetch_one(pool)
    .await?;
    Ok(row)
}

pub async fn complete(
    pool: &PgPool,
    id: Uuid,
    result: &str,
    duration_ms: i32,
) -> Result<()> {
    sqlx::query(
        "UPDATE agent_logs SET result = $1, duration_ms = $2 WHERE id = $3",
    )
    .bind(result)
    .bind(duration_ms)
    .bind(id)
    .execute(pool)
    .await?;
    Ok(())
}

pub async fn list_by_session(pool: &PgPool, session_id: Uuid) -> Result<Vec<AgentLog>> {
    let rows = sqlx::query_as::<_, AgentLog>(
        "SELECT * FROM agent_logs WHERE session_id = $1 ORDER BY created_at ASC",
    )
    .bind(session_id)
    .fetch_all(pool)
    .await?;
    Ok(rows)
}

pub async fn stats_by_executor(pool: &PgPool, session_id: Option<Uuid>) -> Result<Vec<AgentCallStats>> {
    let rows = if let Some(sid) = session_id {
        sqlx::query_as::<_, AgentCallStats>(
            r#"SELECT executor, COUNT(*) as call_count,
                      COALESCE(SUM(duration_ms), 0) as total_duration_ms
               FROM agent_logs WHERE session_id = $1
               GROUP BY executor ORDER BY call_count DESC"#,
        )
        .bind(sid)
        .fetch_all(pool)
        .await?
    } else {
        sqlx::query_as::<_, AgentCallStats>(
            r#"SELECT executor, COUNT(*) as call_count,
                      COALESCE(SUM(duration_ms), 0) as total_duration_ms
               FROM agent_logs
               GROUP BY executor ORDER BY call_count DESC"#,
        )
        .fetch_all(pool)
        .await?
    };
    Ok(rows)
}

#[derive(Debug, Clone, serde::Serialize, sqlx::FromRow)]
pub struct AgentCallStats {
    pub executor: AgentType,
    pub call_count: i64,
    pub total_duration_ms: i64,
}
