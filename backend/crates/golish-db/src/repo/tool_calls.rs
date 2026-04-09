use anyhow::Result;
use sqlx::PgPool;
use uuid::Uuid;

use crate::models::{NewToolCall, ToolCall, ToolcallStatus};

pub async fn create(pool: &PgPool, tc: NewToolCall) -> Result<ToolCall> {
    let row = sqlx::query_as::<_, ToolCall>(
        r#"INSERT INTO tool_calls (call_id, session_id, task_id, subtask_id, agent, name, args, source)
           VALUES ($1, $2, $3, $4, $5, $6, $7, $8)
           RETURNING *"#,
    )
    .bind(&tc.call_id)
    .bind(tc.session_id)
    .bind(tc.task_id)
    .bind(tc.subtask_id)
    .bind(tc.agent)
    .bind(&tc.name)
    .bind(&tc.args)
    .bind(&tc.source)
    .fetch_one(pool)
    .await?;
    Ok(row)
}

pub async fn get(pool: &PgPool, id: Uuid) -> Result<Option<ToolCall>> {
    let row = sqlx::query_as::<_, ToolCall>("SELECT * FROM tool_calls WHERE id = $1")
        .bind(id)
        .fetch_optional(pool)
        .await?;
    Ok(row)
}

pub async fn list_by_session(pool: &PgPool, session_id: Uuid) -> Result<Vec<ToolCall>> {
    let rows = sqlx::query_as::<_, ToolCall>(
        "SELECT * FROM tool_calls WHERE session_id = $1 ORDER BY created_at ASC",
    )
    .bind(session_id)
    .fetch_all(pool)
    .await?;
    Ok(rows)
}

pub async fn list_by_name(pool: &PgPool, name: &str, limit: i64) -> Result<Vec<ToolCall>> {
    let rows = sqlx::query_as::<_, ToolCall>(
        "SELECT * FROM tool_calls WHERE name = $1 ORDER BY created_at DESC LIMIT $2",
    )
    .bind(name)
    .bind(limit)
    .fetch_all(pool)
    .await?;
    Ok(rows)
}

pub async fn update_status(
    pool: &PgPool,
    id: Uuid,
    status: ToolcallStatus,
    result: Option<&str>,
    duration_ms: Option<i32>,
) -> Result<()> {
    sqlx::query(
        r#"UPDATE tool_calls
           SET status = $1, result = COALESCE($2, result),
               duration_ms = COALESCE($3, duration_ms), updated_at = NOW()
           WHERE id = $4"#,
    )
    .bind(status)
    .bind(result)
    .bind(duration_ms)
    .bind(id)
    .execute(pool)
    .await?;
    Ok(())
}

/// Aggregate stats for analytics
pub async fn stats_by_name(pool: &PgPool, session_id: Option<Uuid>) -> Result<Vec<ToolCallStats>> {
    let rows = if let Some(sid) = session_id {
        sqlx::query_as::<_, ToolCallStats>(
            r#"SELECT name, COUNT(*) as total_count,
                      COALESCE(SUM(duration_ms), 0) as total_duration_ms,
                      COALESCE(AVG(duration_ms), 0) as avg_duration_ms
               FROM tool_calls WHERE session_id = $1
               GROUP BY name ORDER BY total_count DESC"#,
        )
        .bind(sid)
        .fetch_all(pool)
        .await?
    } else {
        sqlx::query_as::<_, ToolCallStats>(
            r#"SELECT name, COUNT(*) as total_count,
                      COALESCE(SUM(duration_ms), 0) as total_duration_ms,
                      COALESCE(AVG(duration_ms), 0) as avg_duration_ms
               FROM tool_calls
               GROUP BY name ORDER BY total_count DESC"#,
        )
        .fetch_all(pool)
        .await?
    };
    Ok(rows)
}

#[derive(Debug, Clone, serde::Serialize, sqlx::FromRow)]
pub struct ToolCallStats {
    pub name: String,
    pub total_count: i64,
    pub total_duration_ms: i64,
    pub avg_duration_ms: f64,
}
