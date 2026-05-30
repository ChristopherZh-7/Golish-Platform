//! P0-4: sub-agent dispatch lifecycle persistence.
//!
//! Used by the agent runtime to record every `execute_sub_agent_with_client`
//! call so the app can list mid-flight invocations after a restart.

use crate::Result;
use sqlx::PgPool;
use uuid::Uuid;

use crate::models::{SubAgentDispatch, SubAgentDispatchStatus};

pub async fn record_start(
    pool: &PgPool,
    session_id: Uuid,
    parent_dispatch_id: Option<Uuid>,
    agent_id: &str,
    tool_call_id: Option<&str>,
    depth: i32,
    args: &serde_json::Value,
) -> Result<Uuid> {
    let row: (Uuid,) = sqlx::query_as(
        r#"INSERT INTO sub_agent_dispatches
           (session_id, parent_dispatch_id, agent_id, tool_call_id, depth, args)
           VALUES ($1, $2, $3, $4, $5, $6)
           RETURNING id"#,
    )
    .bind(session_id)
    .bind(parent_dispatch_id)
    .bind(agent_id)
    .bind(tool_call_id)
    .bind(depth)
    .bind(args)
    .fetch_one(pool)
    .await?;
    Ok(row.0)
}

pub async fn record_finish(
    pool: &PgPool,
    id: Uuid,
    status: SubAgentDispatchStatus,
    result: Option<&serde_json::Value>,
    error_message: Option<&str>,
) -> Result<()> {
    sqlx::query(
        r#"UPDATE sub_agent_dispatches
           SET status = $2, result = $3, error_message = $4, finished_at = NOW()
           WHERE id = $1"#,
    )
    .bind(id)
    .bind(status)
    .bind(result)
    .bind(error_message)
    .execute(pool)
    .await?;
    Ok(())
}

pub async fn list_running(pool: &PgPool, session_id: Uuid) -> Result<Vec<SubAgentDispatch>> {
    let rows = sqlx::query_as::<_, SubAgentDispatch>(
        r#"SELECT id, session_id, parent_dispatch_id, agent_id, tool_call_id,
                  depth, status, args, result, error_message, started_at, finished_at
           FROM sub_agent_dispatches
           WHERE session_id = $1 AND status = 'running'
           ORDER BY started_at ASC"#,
    )
    .bind(session_id)
    .fetch_all(pool)
    .await?;
    Ok(rows)
}

pub async fn mark_session_running_as_cancelled(
    pool: &PgPool,
    session_id: Uuid,
    older_than_secs: i64,
) -> Result<u64> {
    let res = sqlx::query(
        r#"UPDATE sub_agent_dispatches
           SET status = 'cancelled', finished_at = NOW(),
               error_message = 'app restart / stale running'
           WHERE session_id = $1
             AND status = 'running'
             AND started_at < NOW() - ($2 || ' seconds')::interval"#,
    )
    .bind(session_id)
    .bind(older_than_secs.to_string())
    .execute(pool)
    .await?;
    Ok(res.rows_affected())
}
