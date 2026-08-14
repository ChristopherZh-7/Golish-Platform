use crate::Result;
use sqlx::{Executor, PgPool, Postgres};
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
    create_with_executor(
        pool, session_id, task_id, subtask_id, agent, model, provider,
    )
    .await
}

pub async fn create_with_executor<'e, E>(
    executor: E,
    session_id: Uuid,
    task_id: Option<Uuid>,
    subtask_id: Option<Uuid>,
    agent: AgentType,
    model: Option<&str>,
    provider: Option<&str>,
) -> Result<MessageChain>
where
    E: Executor<'e, Database = Postgres>,
{
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
    .fetch_one(executor)
    .await?;
    Ok(row)
}

#[allow(clippy::too_many_arguments)]
pub async fn create_bound_with_executor<'e, E>(
    executor: E,
    id: Uuid,
    session_id: Uuid,
    operation_id: Uuid,
    subtask_id: Option<Uuid>,
    agent: AgentType,
    model: Option<&str>,
    provider: Option<&str>,
    chain: &serde_json::Value,
) -> Result<MessageChain>
where
    E: Executor<'e, Database = Postgres>,
{
    let row = sqlx::query_as::<_, MessageChain>(
        r#"INSERT INTO message_chains (
               id, session_id, task_id, subtask_id, agent, model, provider, chain
           ) VALUES ($1,$2,$3,$4,$5,$6,$7,$8)
           RETURNING *"#,
    )
    .bind(id)
    .bind(session_id)
    .bind(operation_id)
    .bind(subtask_id)
    .bind(agent)
    .bind(model)
    .bind(provider)
    .bind(chain)
    .fetch_one(executor)
    .await?;
    Ok(row)
}

pub async fn update_chain(pool: &PgPool, id: Uuid, chain: &serde_json::Value) -> Result<()> {
    sqlx::query("UPDATE message_chains SET chain = $1, updated_at = NOW() WHERE id = $2")
        .bind(chain)
        .bind(id)
        .execute(pool)
        .await?;
    Ok(())
}

/// Update the exact chain already bound to one runtime operation. The caller
/// owns the surrounding worker-fenced transaction; a zero-row CAS is never a
/// durable checkpoint success.
pub async fn update_bound_chain_cas_with_executor<'e, E>(
    executor: E,
    id: Uuid,
    operation_id: Uuid,
    chain: &serde_json::Value,
) -> Result<u64>
where
    E: Executor<'e, Database = Postgres>,
{
    let rows = sqlx::query(
        "UPDATE message_chains SET chain=$3, updated_at=NOW() WHERE id=$1 AND task_id=$2",
    )
    .bind(id)
    .bind(operation_id)
    .bind(chain)
    .execute(executor)
    .await?
    .rows_affected();
    Ok(rows)
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

pub async fn exists_by_id(pool: &PgPool, id: Uuid) -> Result<bool> {
    Ok(
        sqlx::query_scalar("SELECT EXISTS(SELECT 1 FROM message_chains WHERE id=$1)")
            .bind(id)
            .fetch_one(pool)
            .await?,
    )
}

#[derive(Debug, Clone, sqlx::FromRow)]
pub struct ResumeBoundChainRow {
    pub worker_run_id: Uuid,
    pub worker_status: String,
    pub message_chain_id: Option<Uuid>,
    pub exact_chain_id: Option<Uuid>,
    pub chain: Option<serde_json::Value>,
}

const LIST_EXACT_RESUME_BOUND_CHAINS_SQL: &str = r#"SELECT worker.id AS worker_run_id,
              worker.status AS worker_status,
              worker.message_chain_id,
              chain.id AS exact_chain_id,
              chain.chain
         FROM operation_state operation
         JOIN stage_runs execution
           ON execution.operation_id=operation.operation_id
          AND execution.stage_kind=operation.current_stage
          AND execution.status='started'
         JOIN stage_worker_runs worker
           ON worker.operation_id=operation.operation_id
          AND worker.stage_execution_id=execution.id
    LEFT JOIN message_chains chain
           ON chain.id=worker.message_chain_id
          AND chain.session_id=$2
          AND chain.task_id=operation.operation_id
          AND chain.agent=(CASE
                WHEN EXISTS (
                    SELECT 1
                      FROM investigation_asset_primary_current_authorities asset_schedule
                     WHERE asset_schedule.operation_id=worker.operation_id
                       AND asset_schedule.stage_execution_id=worker.stage_execution_id
                       AND asset_schedule.stage_run_unit_id=worker.stage_run_unit_id
                       AND (
                            (asset_schedule.primary_work_item_id=worker.work_item_id
                             AND asset_schedule.primary_worker_run_id=worker.id)
                            OR (
                                asset_schedule.execution_rearm_receipt_id IS NOT NULL
                                AND asset_schedule.authority_primary_work_item_id=worker.work_item_id
                                AND asset_schedule.authority_primary_worker_run_id=worker.id
                            )
                       )
                ) THEN 'primary'::agent_type
                WHEN worker.specialist='reporter' THEN 'reporter'::agent_type
                WHEN worker.specialist IN (
                    'company_stage_controller','generic_intel_worker','intel_goal_reviewer',
                    'content_mapper','browser_runtime','js_api_analyzer','parameter_analyzer',
                    'resolution_analyst','coverage_reviewer','recon','prober','enumerator',
                    'vuln_scanner','application_understanding','attack_analyst',
                    'candidate_verifier','pentester','investigation','researcher','browser',
                    'coder','installer','enricher','memorist','adviser'
                ) THEN 'pentester'::agent_type
                ELSE NULL
              END)
        WHERE operation.operation_id=$1
          AND NOT (
                worker.message_chain_id IS NULL
                AND worker.status IN ('passed','failed','superseded','exhausted')
                AND worker.terminal_at IS NOT NULL
                AND worker.lease_token IS NULL
                AND worker.active_tool_call_id IS NULL
                AND (
                    EXISTS (
                        SELECT 1
                          FROM investigation_asset_primary_schedules fixed
                          JOIN investigation_asset_primary_current_authorities current_primary
                            ON current_primary.asset_lane_id=fixed.asset_lane_id
                           AND current_primary.operation_id=fixed.operation_id
                           AND current_primary.stage_execution_id=fixed.stage_execution_id
                           AND current_primary.stage_run_unit_id=fixed.stage_run_unit_id
                           AND current_primary.primary_message_chain_id=fixed.primary_message_chain_id
                         WHERE fixed.schedule_contract='fixed_roster_v1'
                           AND fixed.status='applied'
                           AND (
                                (fixed.primary_work_item_id=worker.work_item_id
                                 AND fixed.primary_worker_run_id=worker.id)
                                OR worker.work_item_id IN (
                                    fixed.browser_work_item_id,
                                    fixed.researcher_work_item_id,
                                    fixed.pentester_work_item_id,
                                    fixed.adviser_work_item_id
                                )
                           )
                    )
                    OR EXISTS (
                        SELECT 1
                          FROM investigation_asset_primary_current_authorities current_primary
                         WHERE current_primary.operation_id=worker.operation_id
                           AND current_primary.stage_execution_id=worker.stage_execution_id
                           AND current_primary.stage_run_unit_id=worker.stage_run_unit_id
                           AND current_primary.execution_rearm_receipt_id IS NOT NULL
                           AND current_primary.authority_primary_work_item_id=worker.work_item_id
                           AND current_primary.authority_primary_worker_run_id=worker.id
                    )
                )
          )
     ORDER BY worker.id"#;

/// Load every current-stage worker's exact session/task/coarse-agent chain
/// binding. The left join intentionally retains missing/cross-owned chains so
/// the caller can fail closed instead of silently dropping an invalid worker.
pub async fn list_exact_resume_bound_chains(
    pool: &PgPool,
    operation_id: Uuid,
    session_id: Uuid,
) -> Result<Vec<ResumeBoundChainRow>> {
    let rows = sqlx::query_as::<_, ResumeBoundChainRow>(LIST_EXACT_RESUME_BOUND_CHAINS_SQL)
        .bind(operation_id)
        .bind(session_id)
        .fetch_all(pool)
        .await?;
    Ok(rows)
}

#[cfg(test)]
mod exact_resume_tests {
    use super::LIST_EXACT_RESUME_BOUND_CHAINS_SQL;

    #[test]
    fn exact_resume_chain_query_uses_asset_schedule_to_identify_primary() {
        for required in [
            "investigation_asset_primary_current_authorities asset_schedule",
            "asset_schedule.primary_work_item_id=worker.work_item_id",
            "asset_schedule.primary_worker_run_id=worker.id",
            "asset_schedule.execution_rearm_receipt_id IS NOT NULL",
            "asset_schedule.authority_primary_work_item_id=worker.work_item_id",
            "asset_schedule.authority_primary_worker_run_id=worker.id",
            "THEN 'primary'::agent_type",
        ] {
            assert!(
                LIST_EXACT_RESUME_BOUND_CHAINS_SQL.contains(required),
                "missing {required}: {LIST_EXACT_RESUME_BOUND_CHAINS_SQL}"
            );
        }
    }

    #[test]
    fn exact_resume_chain_query_admits_every_server_enumeration_role() {
        for role in [
            "content_mapper",
            "browser_runtime",
            "js_api_analyzer",
            "parameter_analyzer",
            "resolution_analyst",
            "coverage_reviewer",
        ] {
            assert!(
                LIST_EXACT_RESUME_BOUND_CHAINS_SQL.contains(&format!("'{role}'")),
                "missing coarse-agent mapping for {role}"
            );
        }
    }

    #[test]
    fn exact_resume_chain_query_excludes_only_receipt_backed_asset_predecessors() {
        for required in [
            "worker.message_chain_id IS NULL",
            "worker.status IN ('passed','failed','superseded','exhausted')",
            "worker.terminal_at IS NOT NULL",
            "worker.lease_token IS NULL",
            "worker.active_tool_call_id IS NULL",
            "fixed.schedule_contract='fixed_roster_v1'",
            "JOIN investigation_asset_primary_current_authorities current_primary",
            "fixed.primary_worker_run_id=worker.id",
            "fixed.browser_work_item_id",
            "current_primary.execution_rearm_receipt_id IS NOT NULL",
            "current_primary.authority_primary_worker_run_id=worker.id",
        ] {
            assert!(
                LIST_EXACT_RESUME_BOUND_CHAINS_SQL.contains(required),
                "missing exact audit-predecessor fence {required}: {LIST_EXACT_RESUME_BOUND_CHAINS_SQL}"
            );
        }
    }
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
