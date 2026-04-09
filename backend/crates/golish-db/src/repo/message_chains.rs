use anyhow::Result;
use sqlx::PgPool;
use uuid::Uuid;

use crate::models::{AgentType, MessageChain};

pub async fn create(
    pool: &PgPool,
    session_id: Uuid,
    task_id: Option<Uuid>,
    subtask_id: Option<Uuid>,
    agent: AgentType,
    model: Option<&str>,
    provider: Option<&str>,
) -> Result<MessageChain> {
    let row = sqlx::query_as::<_, MessageChain>(
        r#"INSERT INTO message_chains (session_id, task_id, subtask_id, agent, model, provider)
           VALUES ($1, $2, $3, $4, $5, $6)
           RETURNING *"#,
    )
    .bind(session_id)
    .bind(task_id)
    .bind(subtask_id)
    .bind(agent)
    .bind(model)
    .bind(provider)
    .fetch_one(pool)
    .await?;
    Ok(row)
}

pub async fn update_chain(
    pool: &PgPool,
    id: Uuid,
    chain: &serde_json::Value,
) -> Result<()> {
    sqlx::query("UPDATE message_chains SET chain = $1, updated_at = NOW() WHERE id = $2")
        .bind(chain)
        .bind(id)
        .execute(pool)
        .await?;
    Ok(())
}

pub async fn update_usage(
    pool: &PgPool,
    id: Uuid,
    tokens_in: i32,
    tokens_out: i32,
    tokens_cache_in: i32,
    cost_in_usd: f64,
    cost_out_usd: f64,
    duration_ms: i32,
) -> Result<()> {
    sqlx::query(
        r#"UPDATE message_chains
           SET tokens_in = tokens_in + $1,
               tokens_out = tokens_out + $2,
               tokens_cache_in = tokens_cache_in + $3,
               cost_in_usd = cost_in_usd + $4,
               cost_out_usd = cost_out_usd + $5,
               duration_ms = duration_ms + $6,
               updated_at = NOW()
           WHERE id = $7"#,
    )
    .bind(tokens_in)
    .bind(tokens_out)
    .bind(tokens_cache_in)
    .bind(cost_in_usd)
    .bind(cost_out_usd)
    .bind(duration_ms)
    .bind(id)
    .execute(pool)
    .await?;
    Ok(())
}

pub async fn list_by_session(pool: &PgPool, session_id: Uuid) -> Result<Vec<MessageChain>> {
    let rows = sqlx::query_as::<_, MessageChain>(
        "SELECT * FROM message_chains WHERE session_id = $1 ORDER BY created_at ASC",
    )
    .bind(session_id)
    .fetch_all(pool)
    .await?;
    Ok(rows)
}

/// Aggregate usage stats across all sessions
pub async fn usage_stats_total(pool: &PgPool) -> Result<UsageStats> {
    let row = sqlx::query_as::<_, UsageStats>(
        r#"SELECT COALESCE(SUM(tokens_in), 0) as total_tokens_in,
                  COALESCE(SUM(tokens_out), 0) as total_tokens_out,
                  COALESCE(SUM(cost_in_usd), 0) as total_cost_in,
                  COALESCE(SUM(cost_out_usd), 0) as total_cost_out
           FROM message_chains"#,
    )
    .fetch_one(pool)
    .await?;
    Ok(row)
}

/// Usage stats grouped by agent type
pub async fn usage_by_agent(pool: &PgPool) -> Result<Vec<AgentUsageStats>> {
    let rows = sqlx::query_as::<_, AgentUsageStats>(
        r#"SELECT agent,
                  COALESCE(SUM(tokens_in), 0) as total_tokens_in,
                  COALESCE(SUM(tokens_out), 0) as total_tokens_out,
                  COALESCE(SUM(cost_in_usd + cost_out_usd), 0) as total_cost
           FROM message_chains
           GROUP BY agent ORDER BY total_cost DESC"#,
    )
    .fetch_all(pool)
    .await?;
    Ok(rows)
}

#[derive(Debug, Clone, serde::Serialize, sqlx::FromRow)]
pub struct UsageStats {
    pub total_tokens_in: i64,
    pub total_tokens_out: i64,
    pub total_cost_in: f64,
    pub total_cost_out: f64,
}

#[derive(Debug, Clone, serde::Serialize, sqlx::FromRow)]
pub struct AgentUsageStats {
    pub agent: AgentType,
    pub total_tokens_in: i64,
    pub total_tokens_out: i64,
    pub total_cost: f64,
}
