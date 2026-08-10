//! Structured, append-only short-term work memory for the Target Intel Main AI.

use anyhow::{bail, Result};
use serde_json::Value;
use sqlx::PgPool;
use uuid::Uuid;

#[derive(Debug, Clone, PartialEq, sqlx::FromRow)]
pub struct TargetIntelGoalWorkJournalEntryRow {
    pub id: Uuid,
    pub stable_request_id: Uuid,
    pub operation_id: Uuid,
    pub organization_id: Uuid,
    pub team_plan_id: Uuid,
    pub goal_epoch_id: Uuid,
    pub goal_epoch: i64,
    pub controller_worker_run_id: Uuid,
    pub controller_message_chain_id: Uuid,
    pub ordinal: i64,
    pub entry_kind: String,
    pub payload: Value,
    pub related_frontier_refs: Value,
    pub evidence_refs: Value,
    pub tool_call_refs: Value,
    pub observation_refs: Value,
    pub entry_sha256: String,
}

pub async fn append(
    pool: &PgPool,
    row: &TargetIntelGoalWorkJournalEntryRow,
) -> Result<TargetIntelGoalWorkJournalEntryRow> {
    if row.id.is_nil()
        || row.stable_request_id.is_nil()
        || row.operation_id.is_nil()
        || row.organization_id.is_nil()
        || row.team_plan_id.is_nil()
        || row.goal_epoch_id.is_nil()
        || row.controller_worker_run_id.is_nil()
        || row.controller_message_chain_id.is_nil()
        || row.ordinal < 0
        || !row.payload.is_object()
        || !row.entry_sha256.starts_with("sha256:")
    {
        bail!("TARGET_INTEL_WORK_JOURNAL_INPUT_INVALID");
    }
    sqlx::query(
        r#"INSERT INTO target_intel_goal_work_journal_entries(
               id,stable_request_id,operation_id,organization_id,team_plan_id,
               goal_epoch_id,goal_epoch,controller_worker_run_id,controller_message_chain_id,
               ordinal,entry_kind,payload,related_frontier_refs,evidence_refs,
               tool_call_refs,observation_refs,entry_sha256
           ) VALUES($1,$2,$3,$4,$5,$6,$7,$8,$9,$10,$11,$12,$13,$14,$15,$16,$17)
           ON CONFLICT(stable_request_id) DO NOTHING"#,
    )
    .bind(row.id)
    .bind(row.stable_request_id)
    .bind(row.operation_id)
    .bind(row.organization_id)
    .bind(row.team_plan_id)
    .bind(row.goal_epoch_id)
    .bind(row.goal_epoch)
    .bind(row.controller_worker_run_id)
    .bind(row.controller_message_chain_id)
    .bind(row.ordinal)
    .bind(&row.entry_kind)
    .bind(&row.payload)
    .bind(&row.related_frontier_refs)
    .bind(&row.evidence_refs)
    .bind(&row.tool_call_refs)
    .bind(&row.observation_refs)
    .bind(&row.entry_sha256)
    .execute(pool)
    .await?;
    let persisted = sqlx::query_as::<_, TargetIntelGoalWorkJournalEntryRow>(
        r#"SELECT id,stable_request_id,operation_id,organization_id,team_plan_id,
                  goal_epoch_id,goal_epoch,controller_worker_run_id,controller_message_chain_id,
                  ordinal,entry_kind,payload,related_frontier_refs,evidence_refs,
                  tool_call_refs,observation_refs,entry_sha256
             FROM target_intel_goal_work_journal_entries WHERE stable_request_id=$1"#,
    )
    .bind(row.stable_request_id)
    .fetch_one(pool)
    .await?;
    if &persisted != row {
        bail!("TARGET_INTEL_WORK_JOURNAL_REPLAY_MISMATCH");
    }
    Ok(persisted)
}
