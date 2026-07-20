//! Durable Stage Team Scheduler rows and compound producer-completion seams.
//!
//! `StageRunUnit` remains the Gate boundary. These rows make TeamPlan and
//! WorkItem the durable scheduling authority without allowing a producer to
//! close its Unit. Only the runtime-memory team finalizer may do that.

use chrono::{DateTime, Utc};
use serde_json::Value;
use sqlx::{Executor, PgConnection, PgPool, Postgres};
use uuid::Uuid;

use super::runtime_memory_tx::{
    RuntimeMemoryStoreError, RuntimeMemoryStoreResult, RuntimeMemoryTxFence,
};
use super::{canonical_fact_refs, operation_scope_decisions, stage_run_units, stage_worker_runs};

const PLAN_COLUMNS: &str = r#"id,operation_id,stage_execution_id,stage_run_unit_id,
    scope_snapshot_id,organization_id,stage_kind,unit_generation,schema_version,plan_version,
    plan_hash,leader_role,aggregator_kind,aggregator_role,allowed_worker_roles,
    max_workers_total,max_workers_active,dynamic_requests_allowed,dynamic_request_policy,
    dispatch_epoch,requests_closed_at,final_submitter_kind,final_submitter_worker_run_id,
    created_from_stage_spec_hash,row_version,created_at,updated_at"#;

const ITEM_COLUMNS: &str = r#"id,team_plan_id,operation_id,stage_execution_id,stage_run_unit_id,
    scope_snapshot_id,organization_id,dispatch_epoch,kind,stable_key,role,input_manifest_hash,
    input_refs,required_for_barrier,conflict_key,priority,status,attempt_policy,budget,
    output_schema,created_by,row_version,created_at,updated_at,started_at,terminal_at"#;

const OUTPUT_COLUMNS: &str = r#"id,team_plan_id,work_item_id,worker_run_id,operation_id,
    stage_execution_id,stage_run_unit_id,scope_snapshot_id,organization_id,output_schema,
    output_version,business_disposition,canonical_output,canonical_fact_refs,evidence_ids,
    checked_empty_cells,blocker_codes,output_hash,created_at"#;

const REQUEST_COLUMNS: &str = r#"id,team_plan_id,operation_id,stage_execution_id,
    stage_run_unit_id,scope_snapshot_id,organization_id,parent_work_item_id,parent_worker_run_id,
    dispatch_epoch,requested_role,request_kind,bounded_subject_refs,reason_code,
    expected_output_schema,budget_hint,dedupe_key,request_payload_hash,status,
    decision_reason_code,accepted_work_item_id,created_at"#;

/// Company Controller plans are bounded by exact scope/epoch admission, live
/// concurrency and per-WorkItem attempts. Their historical lifetime totals
/// remain frozen only so an already-running TeamPlan can replay byte-for-byte.
pub(crate) fn enforces_lifetime_worker_total(dynamic_request_policy: &Value) -> bool {
    dynamic_request_policy
        .get("coordination_mode")
        .and_then(Value::as_str)
        != Some("company_controller")
}

const RECOVERY_DECISION_COLUMNS: &str = r#"id,request_id,team_plan_id,work_item_id,
    worker_run_id,tool_call_record_id,operation_id,stage_execution_id,stage_run_unit_id,
    scope_snapshot_id,organization_id,expected_work_item_row_version,expected_checkpoint_version,
    expected_attempt_epoch,resolution_kind,resolution_payload,resolution_hash,resolved_by,created_at"#;

pub(crate) const UNIT_GAP_COLUMNS: &str = r#"id,request_id,team_plan_id,operation_id,
    stage_execution_id,stage_run_unit_id,scope_snapshot_id,organization_id,source_dispatch_epoch,source_manifest_hash,
    source_attempt_epoch,source_checkpoint_version,source_lease_token,
    source_aggregator_work_item_id,source_aggregator_worker_run_id,deliverable_submission_id,
    gate_decision_hash,gap_manifest,gap_manifest_hash,repair_generation,disposition,created_at"#;

pub(crate) const REPAIR_GENERATION_COLUMNS: &str = r#"id,team_plan_id,source_gap_id,
    operation_id,stage_execution_id,stage_run_unit_id,scope_snapshot_id,organization_id,
    dispatch_epoch,repair_work_item_id,aggregator_work_item_id,manifest,manifest_hash,
    status,created_at,sealed_at"#;

#[derive(Debug, Clone, sqlx::FromRow, PartialEq)]
pub struct StageTeamPlanRow {
    pub id: Uuid,
    pub operation_id: Uuid,
    pub stage_execution_id: Uuid,
    pub stage_run_unit_id: Uuid,
    pub scope_snapshot_id: Uuid,
    pub organization_id: Uuid,
    pub stage_kind: String,
    pub unit_generation: i32,
    pub schema_version: i32,
    pub plan_version: i32,
    pub plan_hash: String,
    pub leader_role: String,
    pub aggregator_kind: String,
    pub aggregator_role: Option<String>,
    pub allowed_worker_roles: Value,
    pub max_workers_total: i32,
    pub max_workers_active: i32,
    pub dynamic_requests_allowed: bool,
    pub dynamic_request_policy: Value,
    pub dispatch_epoch: i64,
    pub requests_closed_at: Option<DateTime<Utc>>,
    pub final_submitter_kind: String,
    pub final_submitter_worker_run_id: Option<Uuid>,
    pub created_from_stage_spec_hash: String,
    pub row_version: i64,
    pub created_at: DateTime<Utc>,
    pub updated_at: DateTime<Utc>,
}

#[derive(Debug, Clone, sqlx::FromRow, PartialEq)]
pub struct StageWorkItemRow {
    pub id: Uuid,
    pub team_plan_id: Uuid,
    pub operation_id: Uuid,
    pub stage_execution_id: Uuid,
    pub stage_run_unit_id: Uuid,
    pub scope_snapshot_id: Uuid,
    pub organization_id: Uuid,
    pub dispatch_epoch: i64,
    pub kind: String,
    pub stable_key: String,
    pub role: String,
    pub input_manifest_hash: String,
    pub input_refs: Value,
    pub required_for_barrier: bool,
    pub conflict_key: Option<String>,
    pub priority: i32,
    pub status: String,
    pub attempt_policy: Value,
    pub budget: Value,
    pub output_schema: String,
    pub created_by: String,
    pub row_version: i64,
    pub created_at: DateTime<Utc>,
    pub updated_at: DateTime<Utc>,
    pub started_at: Option<DateTime<Utc>>,
    pub terminal_at: Option<DateTime<Utc>>,
}

#[derive(Debug, Clone, sqlx::FromRow, PartialEq)]
pub struct StageWorkerOutputRow {
    pub id: Uuid,
    pub team_plan_id: Uuid,
    pub work_item_id: Uuid,
    pub worker_run_id: Uuid,
    pub operation_id: Uuid,
    pub stage_execution_id: Uuid,
    pub stage_run_unit_id: Uuid,
    pub scope_snapshot_id: Uuid,
    pub organization_id: Uuid,
    pub output_schema: String,
    pub output_version: i32,
    pub business_disposition: String,
    pub canonical_output: Value,
    pub canonical_fact_refs: Value,
    pub evidence_ids: Vec<i64>,
    pub checked_empty_cells: Value,
    pub blocker_codes: Vec<String>,
    pub output_hash: String,
    pub created_at: DateTime<Utc>,
}

#[derive(Debug, Clone, sqlx::FromRow, PartialEq)]
pub struct StageWorkerRequestRow {
    pub id: Uuid,
    pub team_plan_id: Uuid,
    pub operation_id: Uuid,
    pub stage_execution_id: Uuid,
    pub stage_run_unit_id: Uuid,
    pub scope_snapshot_id: Uuid,
    pub organization_id: Uuid,
    pub parent_work_item_id: Uuid,
    pub parent_worker_run_id: Uuid,
    pub dispatch_epoch: i64,
    pub requested_role: String,
    pub request_kind: String,
    pub bounded_subject_refs: Value,
    pub reason_code: String,
    pub expected_output_schema: String,
    pub budget_hint: Value,
    pub dedupe_key: String,
    pub request_payload_hash: String,
    pub status: String,
    pub decision_reason_code: Option<String>,
    pub accepted_work_item_id: Option<Uuid>,
    pub created_at: DateTime<Utc>,
}

#[derive(Debug, Clone, sqlx::FromRow, PartialEq)]
pub struct StageTeamRecoveryDecisionRow {
    pub id: Uuid,
    pub request_id: String,
    pub team_plan_id: Uuid,
    pub work_item_id: Uuid,
    pub worker_run_id: Uuid,
    pub tool_call_record_id: Uuid,
    pub operation_id: Uuid,
    pub stage_execution_id: Uuid,
    pub stage_run_unit_id: Uuid,
    pub scope_snapshot_id: Uuid,
    pub organization_id: Uuid,
    pub expected_work_item_row_version: i64,
    pub expected_checkpoint_version: i64,
    pub expected_attempt_epoch: i64,
    pub resolution_kind: String,
    pub resolution_payload: Value,
    pub resolution_hash: String,
    pub resolved_by: String,
    pub created_at: DateTime<Utc>,
}

#[derive(Debug, Clone, sqlx::FromRow, PartialEq)]
pub struct StageTeamUnitGapRow {
    pub id: Uuid,
    pub request_id: String,
    pub team_plan_id: Uuid,
    pub operation_id: Uuid,
    pub stage_execution_id: Uuid,
    pub stage_run_unit_id: Uuid,
    pub scope_snapshot_id: Uuid,
    pub organization_id: Uuid,
    pub source_dispatch_epoch: i64,
    pub source_manifest_hash: String,
    pub source_attempt_epoch: i64,
    pub source_checkpoint_version: i64,
    pub source_lease_token: Uuid,
    pub source_aggregator_work_item_id: Uuid,
    pub source_aggregator_worker_run_id: Uuid,
    pub deliverable_submission_id: Uuid,
    pub gate_decision_hash: String,
    pub gap_manifest: Value,
    pub gap_manifest_hash: String,
    pub repair_generation: i32,
    pub disposition: String,
    pub created_at: DateTime<Utc>,
}

#[derive(Debug, Clone, sqlx::FromRow, PartialEq)]
pub struct StageTeamRepairGenerationRow {
    pub id: Uuid,
    pub team_plan_id: Uuid,
    pub source_gap_id: Uuid,
    pub operation_id: Uuid,
    pub stage_execution_id: Uuid,
    pub stage_run_unit_id: Uuid,
    pub scope_snapshot_id: Uuid,
    pub organization_id: Uuid,
    pub dispatch_epoch: i64,
    pub repair_work_item_id: Option<Uuid>,
    pub aggregator_work_item_id: Option<Uuid>,
    pub manifest: Value,
    pub manifest_hash: String,
    pub status: String,
    pub created_at: DateTime<Utc>,
    pub sealed_at: Option<DateTime<Utc>>,
}

#[derive(Debug, Clone, PartialEq)]
pub struct NewStageTeamPlan {
    pub id: Uuid,
    pub operation_id: Uuid,
    pub stage_execution_id: Uuid,
    pub stage_run_unit_id: Uuid,
    pub scope_snapshot_id: Uuid,
    pub organization_id: Uuid,
    pub stage_kind: String,
    pub unit_generation: i32,
    pub schema_version: i32,
    pub plan_version: i32,
    pub plan_hash: String,
    pub leader_role: String,
    pub aggregator_kind: String,
    pub aggregator_role: Option<String>,
    pub allowed_worker_roles: Value,
    pub max_workers_total: i32,
    pub max_workers_active: i32,
    pub dynamic_requests_allowed: bool,
    pub dynamic_request_policy: Value,
    pub final_submitter_kind: String,
    pub created_from_stage_spec_hash: String,
}

#[derive(Debug, Clone, PartialEq)]
pub struct NewStageWorkItem {
    pub id: Uuid,
    pub team_plan_id: Uuid,
    pub operation_id: Uuid,
    pub stage_execution_id: Uuid,
    pub stage_run_unit_id: Uuid,
    pub scope_snapshot_id: Uuid,
    pub organization_id: Uuid,
    pub dispatch_epoch: i64,
    pub kind: String,
    pub stable_key: String,
    pub role: String,
    pub input_manifest_hash: String,
    pub input_refs: Value,
    pub required_for_barrier: bool,
    pub conflict_key: Option<String>,
    pub priority: i32,
    pub attempt_policy: Value,
    pub budget: Value,
    pub output_schema: String,
    pub created_by: String,
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct ResolveStageTeamRecoveryRow {
    pub request_id: String,
    pub operation_id: Uuid,
    pub stage_execution_id: Uuid,
    pub stage_run_unit_id: Uuid,
    pub scope_snapshot_id: Uuid,
    pub team_plan_id: Uuid,
    pub work_item_id: Uuid,
    pub worker_run_id: Uuid,
    pub tool_call_record_id: Uuid,
    pub expected_work_item_row_version: i64,
    pub expected_checkpoint_version: i64,
    pub expected_attempt_epoch: i64,
    pub resolved_by: String,
}

#[derive(Debug, Clone, PartialEq)]
pub struct ResolvedStageTeamRecoveryRow {
    pub decision: StageTeamRecoveryDecisionRow,
    pub work_item: StageWorkItemRow,
    pub worker: stage_worker_runs::StageWorkerRunRow,
    pub output: StageWorkerOutputRow,
    pub replayed: bool,
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct StageTeamBarrierRow {
    pub stage_team_plan_id: Uuid,
    pub dispatch_epoch: i64,
    pub requests_closed_at: Option<DateTime<Utc>>,
    pub required_work_items: i64,
    pub terminal_required_work_items: i64,
    pub live_workers: i64,
    pub retry_pending_work_items: i64,
    pub recovery_required_workers: i64,
    pub missing_outputs: i64,
    pub manifest_hash: String,
}

impl StageTeamBarrierRow {
    pub fn ready_to_finalize(&self) -> bool {
        self.requests_closed_at.is_some()
            && self.required_work_items == self.terminal_required_work_items
            && self.live_workers == 0
            && self.retry_pending_work_items == 0
            && self.recovery_required_workers == 0
            && self.missing_outputs == 0
    }
}

fn validate_hash(value: &str) -> bool {
    value.len() == 71
        && value.starts_with("sha256:")
        && value[7..]
            .bytes()
            .all(|byte| byte.is_ascii_digit() || matches!(byte, b'a'..=b'f'))
}

pub async fn insert_plan_with_executor<'e, E>(
    executor: E,
    input: &NewStageTeamPlan,
) -> RuntimeMemoryStoreResult<StageTeamPlanRow>
where
    E: Executor<'e, Database = Postgres>,
{
    if input.schema_version <= 0
        || input.plan_version <= 0
        || !validate_hash(&input.plan_hash)
        || !validate_hash(&input.created_from_stage_spec_hash)
        || input.max_workers_total <= 0
        || input.max_workers_active <= 0
        || input.max_workers_active > input.max_workers_total
    {
        return Err(RuntimeMemoryStoreError::IdentityMismatch {
            code: "invalid_stage_team_plan",
        });
    }
    let sql = format!(
        r#"INSERT INTO stage_team_plans(
               id,operation_id,stage_execution_id,stage_run_unit_id,scope_snapshot_id,
               organization_id,stage_kind,unit_generation,schema_version,plan_version,
               plan_hash,leader_role,aggregator_kind,aggregator_role,allowed_worker_roles,
               max_workers_total,max_workers_active,dynamic_requests_allowed,
               dynamic_request_policy,final_submitter_kind,created_from_stage_spec_hash
           ) VALUES(
               $1,$2,$3,$4,$5,$6,$7,$8,$9,$10,$11,$12,$13,$14,$15,$16,$17,$18,$19,$20,$21
           ) RETURNING {PLAN_COLUMNS}"#,
    );
    Ok(sqlx::query_as::<_, StageTeamPlanRow>(&sql)
        .bind(input.id)
        .bind(input.operation_id)
        .bind(input.stage_execution_id)
        .bind(input.stage_run_unit_id)
        .bind(input.scope_snapshot_id)
        .bind(input.organization_id)
        .bind(&input.stage_kind)
        .bind(input.unit_generation)
        .bind(input.schema_version)
        .bind(input.plan_version)
        .bind(&input.plan_hash)
        .bind(&input.leader_role)
        .bind(&input.aggregator_kind)
        .bind(&input.aggregator_role)
        .bind(&input.allowed_worker_roles)
        .bind(input.max_workers_total)
        .bind(input.max_workers_active)
        .bind(input.dynamic_requests_allowed)
        .bind(&input.dynamic_request_policy)
        .bind(&input.final_submitter_kind)
        .bind(&input.created_from_stage_spec_hash)
        .fetch_one(executor)
        .await?)
}

pub async fn insert_work_item_with_executor<'e, E>(
    executor: E,
    input: &NewStageWorkItem,
) -> RuntimeMemoryStoreResult<StageWorkItemRow>
where
    E: Executor<'e, Database = Postgres>,
{
    if input.kind.trim().is_empty()
        || input.stable_key.trim().is_empty()
        || input.role.trim().is_empty()
        || !validate_hash(&input.input_manifest_hash)
        || !input.input_refs.is_array()
        || input.output_schema.trim().is_empty()
    {
        return Err(RuntimeMemoryStoreError::IdentityMismatch {
            code: "invalid_stage_work_item",
        });
    }
    let sql = format!(
        r#"INSERT INTO stage_work_items(
               id,team_plan_id,operation_id,stage_execution_id,stage_run_unit_id,
               scope_snapshot_id,organization_id,dispatch_epoch,kind,stable_key,role,
               input_manifest_hash,input_refs,required_for_barrier,conflict_key,priority,
               status,attempt_policy,budget,output_schema,created_by
           ) VALUES(
               $1,$2,$3,$4,$5,$6,$7,$8,$9,$10,$11,$12,$13,$14,$15,$16,'queued',$17,$18,$19,$20
           ) RETURNING {ITEM_COLUMNS}"#,
    );
    Ok(sqlx::query_as::<_, StageWorkItemRow>(&sql)
        .bind(input.id)
        .bind(input.team_plan_id)
        .bind(input.operation_id)
        .bind(input.stage_execution_id)
        .bind(input.stage_run_unit_id)
        .bind(input.scope_snapshot_id)
        .bind(input.organization_id)
        .bind(input.dispatch_epoch)
        .bind(&input.kind)
        .bind(&input.stable_key)
        .bind(&input.role)
        .bind(&input.input_manifest_hash)
        .bind(&input.input_refs)
        .bind(input.required_for_barrier)
        .bind(&input.conflict_key)
        .bind(input.priority)
        .bind(&input.attempt_policy)
        .bind(&input.budget)
        .bind(&input.output_schema)
        .bind(&input.created_by)
        .fetch_one(executor)
        .await?)
}

pub async fn get_plan_for_unit_with_executor<'e, E>(
    executor: E,
    stage_run_unit_id: Uuid,
) -> RuntimeMemoryStoreResult<Option<StageTeamPlanRow>>
where
    E: Executor<'e, Database = Postgres>,
{
    let sql = format!("SELECT {PLAN_COLUMNS} FROM stage_team_plans WHERE stage_run_unit_id=$1");
    Ok(sqlx::query_as::<_, StageTeamPlanRow>(&sql)
        .bind(stage_run_unit_id)
        .fetch_optional(executor)
        .await?)
}

pub async fn list_work_items_with_executor<'e, E>(
    executor: E,
    team_plan_id: Uuid,
) -> RuntimeMemoryStoreResult<Vec<StageWorkItemRow>>
where
    E: Executor<'e, Database = Postgres>,
{
    let sql = format!(
        "SELECT {ITEM_COLUMNS} FROM stage_work_items WHERE team_plan_id=$1 ORDER BY priority,id"
    );
    Ok(sqlx::query_as::<_, StageWorkItemRow>(&sql)
        .bind(team_plan_id)
        .fetch_all(executor)
        .await?)
}

pub async fn list_outputs_with_executor<'e, E>(
    executor: E,
    team_plan_id: Uuid,
) -> RuntimeMemoryStoreResult<Vec<StageWorkerOutputRow>>
where
    E: Executor<'e, Database = Postgres>,
{
    let sql = format!(
        "SELECT {OUTPUT_COLUMNS} FROM stage_worker_outputs WHERE team_plan_id=$1 ORDER BY created_at,id"
    );
    Ok(sqlx::query_as::<_, StageWorkerOutputRow>(&sql)
        .bind(team_plan_id)
        .fetch_all(executor)
        .await?)
}

pub async fn list_requests_with_executor<'e, E>(
    executor: E,
    team_plan_id: Uuid,
) -> RuntimeMemoryStoreResult<Vec<StageWorkerRequestRow>>
where
    E: Executor<'e, Database = Postgres>,
{
    let sql = format!(
        "SELECT {REQUEST_COLUMNS} FROM stage_worker_requests WHERE team_plan_id=$1 ORDER BY created_at,id"
    );
    Ok(sqlx::query_as::<_, StageWorkerRequestRow>(&sql)
        .bind(team_plan_id)
        .fetch_all(executor)
        .await?)
}

pub async fn close_request_epoch(
    pool: &PgPool,
    team_plan_id: Uuid,
    expected_dispatch_epoch: i64,
    expected_row_version: i64,
) -> RuntimeMemoryStoreResult<StageTeamPlanRow> {
    let mut tx = pool.begin().await?;
    let select_sql = format!("SELECT {PLAN_COLUMNS} FROM stage_team_plans WHERE id=$1 FOR UPDATE");
    let current = sqlx::query_as::<_, StageTeamPlanRow>(&select_sql)
        .bind(team_plan_id)
        .fetch_optional(&mut *tx)
        .await?
        .ok_or(RuntimeMemoryStoreError::Missing {
            entity: "stage_team_plans",
        })?;
    if current.dispatch_epoch != expected_dispatch_epoch {
        return Err(RuntimeMemoryStoreError::IdentityMismatch {
            code: "stage_team_dispatch_epoch_mismatch",
        });
    }
    if current.requests_closed_at.is_some() {
        if current.row_version == expected_row_version
            || current.row_version == expected_row_version.saturating_add(1)
        {
            tx.commit().await?;
            return Ok(current);
        }
        return Err(RuntimeMemoryStoreError::StaleVersion {
            entity: "stage_team_plans",
            expected: expected_row_version,
            actual: current.row_version,
        });
    }
    if current.row_version != expected_row_version {
        return Err(RuntimeMemoryStoreError::StaleVersion {
            entity: "stage_team_plans",
            expected: expected_row_version,
            actual: current.row_version,
        });
    }
    let sql = format!(
        r#"UPDATE stage_team_plans
              SET requests_closed_at=NOW(),row_version=row_version+1,updated_at=NOW()
            WHERE id=$1 AND dispatch_epoch=$2 AND row_version=$3
              AND requests_closed_at IS NULL AND final_submitter_worker_run_id IS NULL
            RETURNING {PLAN_COLUMNS}"#,
    );
    let closed = sqlx::query_as::<_, StageTeamPlanRow>(&sql)
        .bind(team_plan_id)
        .bind(expected_dispatch_epoch)
        .bind(expected_row_version)
        .fetch_optional(&mut *tx)
        .await?
        .ok_or(RuntimeMemoryStoreError::StaleVersion {
            entity: "stage_team_plans",
            expected: expected_row_version,
            actual: current.row_version,
        })?;
    tx.commit().await?;
    Ok(closed)
}

fn is_aggregator_item(plan: &StageTeamPlanRow, item: &StageWorkItemRow) -> bool {
    plan.aggregator_kind == "worker"
        && plan.aggregator_role.as_deref() == Some(item.role.as_str())
        && !item.required_for_barrier
}

fn barrier_manifest_hash(
    plan: &StageTeamPlanRow,
    items: &[StageWorkItemRow],
    outputs: &[StageWorkerOutputRow],
    requests: &[StageWorkerRequestRow],
) -> String {
    let output_hashes = outputs
        .iter()
        .map(|output| (output.work_item_id.to_string(), output.output_hash.clone()))
        .collect::<std::collections::BTreeMap<_, _>>();
    let producer_items = items
        .iter()
        .filter(|item| !is_aggregator_item(plan, item))
        .map(|item| {
            serde_json::json!({
                "id": item.id,
                "input_manifest_hash": item.input_manifest_hash,
                "kind": item.kind,
                "output_hash": output_hashes.get(&item.id.to_string()),
                "required_for_barrier": item.required_for_barrier,
                "role": item.role,
                "stable_key": item.stable_key,
                "status": item.status,
            })
        })
        .collect::<Vec<_>>();
    // Aggregator claim/finalization mutates its status and binds a Worker.  Its
    // immutable identity belongs in the closed manifest, but those mutable
    // fields must not make the manifest drift between pre-claim and final seal.
    let aggregator_items = items
        .iter()
        .filter(|item| is_aggregator_item(plan, item))
        .map(|item| {
            serde_json::json!({
                "id": item.id,
                "input_manifest_hash": item.input_manifest_hash,
                "kind": item.kind,
                "role": item.role,
                "stable_key": item.stable_key,
            })
        })
        .collect::<Vec<_>>();
    let material = serde_json::json!({
        "dispatch_epoch": plan.dispatch_epoch,
        "aggregator_items": aggregator_items,
        "plan_hash": plan.plan_hash,
        "producer_items": producer_items,
        "requests": requests.iter().map(|request| serde_json::json!({
            "accepted_work_item_id": request.accepted_work_item_id,
            "decision_reason_code": request.decision_reason_code,
            "dedupe_key": request.dedupe_key,
            "id": request.id,
            "parent_work_item_id": request.parent_work_item_id,
            "parent_worker_run_id": request.parent_worker_run_id,
            "request_payload_hash": request.request_payload_hash,
            "status": request.status,
        })).collect::<Vec<_>>(),
    });
    format!(
        "sha256:{}",
        operation_scope_decisions::sha256_json(&material)
    )
}

pub(crate) async fn load_barrier_with_connection_ignoring_worker(
    connection: &mut PgConnection,
    team_plan_id: Uuid,
    ignored_worker_run_id: Option<Uuid>,
) -> RuntimeMemoryStoreResult<StageTeamBarrierRow> {
    let plan_sql = format!("SELECT {PLAN_COLUMNS} FROM stage_team_plans WHERE id=$1 FOR UPDATE");
    let plan = sqlx::query_as::<_, StageTeamPlanRow>(&plan_sql)
        .bind(team_plan_id)
        .fetch_optional(&mut *connection)
        .await?
        .ok_or(RuntimeMemoryStoreError::Missing {
            entity: "stage_team_plans",
        })?;
    let item_sql = format!(
        "SELECT {ITEM_COLUMNS} FROM stage_work_items WHERE team_plan_id=$1 ORDER BY priority,id FOR UPDATE"
    );
    let items = sqlx::query_as::<_, StageWorkItemRow>(&item_sql)
        .bind(team_plan_id)
        .fetch_all(&mut *connection)
        .await?;
    let output_sql = format!(
        "SELECT {OUTPUT_COLUMNS} FROM stage_worker_outputs WHERE team_plan_id=$1 ORDER BY created_at,id FOR SHARE"
    );
    let outputs = sqlx::query_as::<_, StageWorkerOutputRow>(&output_sql)
        .bind(team_plan_id)
        .fetch_all(&mut *connection)
        .await?;
    let request_sql = format!(
        "SELECT {REQUEST_COLUMNS} FROM stage_worker_requests WHERE team_plan_id=$1 ORDER BY created_at,id FOR SHARE"
    );
    let requests = sqlx::query_as::<_, StageWorkerRequestRow>(&request_sql)
        .bind(team_plan_id)
        .fetch_all(&mut *connection)
        .await?;
    let producer_item_ids = items
        .iter()
        .filter(|item| !is_aggregator_item(&plan, item))
        .map(|item| item.id)
        .collect::<std::collections::HashSet<_>>();
    let required = producer_item_ids.len() as i64;
    let terminal_required = items
        .iter()
        .filter(|item| {
            producer_item_ids.contains(&item.id)
                && matches!(item.status.as_str(), "completed" | "exhausted")
        })
        .count() as i64;
    let retry_pending = items
        .iter()
        .filter(|item| producer_item_ids.contains(&item.id) && item.status == "retry_pending")
        .count() as i64;
    let workers = sqlx::query_as::<_, stage_worker_runs::StageWorkerRunRow>(
        r#"SELECT * FROM stage_worker_runs
            WHERE stage_run_unit_id=$1 AND work_item_id IS NOT NULL
            ORDER BY id FOR UPDATE"#,
    )
    .bind(plan.stage_run_unit_id)
    .fetch_all(&mut *connection)
    .await?;
    let producer_workers = workers.iter().filter(|worker| {
        worker.id != ignored_worker_run_id.unwrap_or(Uuid::nil())
            && worker
                .work_item_id
                .is_some_and(|item_id| producer_item_ids.contains(&item_id))
    });
    let live_workers = producer_workers
        .clone()
        .filter(|worker| {
            matches!(
                worker.status.as_str(),
                "queued" | "running" | "waiting_background"
            )
        })
        .count() as i64;
    let recovery_required_workers = producer_workers
        .filter(|worker| worker.status == "recovery_required")
        .count() as i64;
    let output_item_ids = outputs
        .iter()
        .map(|output| output.work_item_id)
        .collect::<std::collections::HashSet<_>>();
    let missing_outputs = items
        .iter()
        .filter(|item| {
            producer_item_ids.contains(&item.id)
                && matches!(item.status.as_str(), "completed" | "exhausted")
                && !output_item_ids.contains(&item.id)
        })
        .count() as i64;
    Ok(StageTeamBarrierRow {
        stage_team_plan_id: plan.id,
        dispatch_epoch: plan.dispatch_epoch,
        requests_closed_at: plan.requests_closed_at,
        required_work_items: required,
        terminal_required_work_items: terminal_required,
        live_workers,
        retry_pending_work_items: retry_pending,
        recovery_required_workers,
        missing_outputs,
        manifest_hash: barrier_manifest_hash(&plan, &items, &outputs, &requests),
    })
}

pub async fn load_barrier_with_connection(
    connection: &mut PgConnection,
    team_plan_id: Uuid,
) -> RuntimeMemoryStoreResult<StageTeamBarrierRow> {
    load_barrier_with_connection_ignoring_worker(connection, team_plan_id, None).await
}

#[derive(Debug, Clone, PartialEq)]
pub struct CompleteStageWorkerRow {
    pub fence: RuntimeMemoryTxFence,
    pub team_plan_id: Uuid,
    pub work_item_id: Uuid,
    pub expected_work_item_row_version: i64,
    pub output_schema: String,
    pub business_disposition: String,
    pub canonical_output: Value,
    pub canonical_fact_refs: Value,
    pub evidence_ids: Vec<i64>,
    pub checked_empty_cells: Value,
    pub blocker_codes: Vec<String>,
    pub output_hash: String,
    pub terminal_checkpoint: Value,
    pub evidence_watermark: Option<i64>,
}

#[derive(Debug, Clone)]
pub struct CompletedStageWorkerRow {
    pub unit: stage_run_units::StageRunUnitRow,
    pub plan: StageTeamPlanRow,
    pub work_item: StageWorkItemRow,
    pub worker: stage_worker_runs::StageWorkerRunRow,
    pub output: StageWorkerOutputRow,
    pub replayed: bool,
}

#[derive(Debug, Clone, PartialEq)]
pub struct RetryStageWorkerRow {
    pub fence: RuntimeMemoryTxFence,
    pub team_plan_id: Uuid,
    pub work_item_id: Uuid,
    pub expected_work_item_row_version: i64,
    pub failure_code: String,
    pub terminal_checkpoint: Value,
}

#[derive(Debug, Clone)]
pub struct RetriedStageWorkerRow {
    pub unit: stage_run_units::StageRunUnitRow,
    pub plan: StageTeamPlanRow,
    pub work_item: StageWorkItemRow,
    pub worker: stage_worker_runs::StageWorkerRunRow,
    pub retry_scheduled: bool,
}

pub(crate) fn work_item_max_attempts(item: &StageWorkItemRow) -> RuntimeMemoryStoreResult<i64> {
    let max_attempts = item
        .attempt_policy
        .get("max_attempts")
        .and_then(Value::as_i64)
        .ok_or(RuntimeMemoryStoreError::IdentityMismatch {
            code: "stage_work_item_attempt_policy_invalid",
        })?;
    if !(1..=32).contains(&max_attempts) {
        return Err(RuntimeMemoryStoreError::IdentityMismatch {
            code: "stage_work_item_attempt_policy_invalid",
        });
    }
    Ok(max_attempts)
}

fn output_replays_exactly(existing: &StageWorkerOutputRow, input: &CompleteStageWorkerRow) -> bool {
    existing.worker_run_id == input.fence.worker_run_id
        && existing.output_schema == input.output_schema
        && existing.output_version == 1
        && existing.business_disposition == input.business_disposition
        && existing.canonical_output == input.canonical_output
        && existing.canonical_fact_refs == input.canonical_fact_refs
        && existing.evidence_ids == input.evidence_ids
        && existing.checked_empty_cells == input.checked_empty_cells
        && existing.blocker_codes == input.blocker_codes
        && existing.output_hash == input.output_hash
}

fn canonical_stage_worker_output_hash(input: &CompleteStageWorkerRow) -> String {
    let material = serde_json::json!({
        "blocker_code": input.blocker_codes.first(),
        "canonical_output": input.canonical_output,
        "checked_empty_units": input.checked_empty_cells,
        "disposition": input.business_disposition,
        "evidence_ids": input.evidence_ids,
        "fact_refs": input.canonical_fact_refs,
        "output_schema": input.output_schema,
        "work_item_id": input.work_item_id,
        "worker_run_id": input.fence.worker_run_id,
    });
    format!(
        "sha256:{}",
        operation_scope_decisions::sha256_json(&material)
    )
}

const STAGE_TEAM_ATTEMPTS_EXHAUSTED: &str = "STAGE_TEAM_PRODUCER_ATTEMPTS_EXHAUSTED";

pub(crate) fn exhausted_stage_worker_output(
    plan: &StageTeamPlanRow,
    item: &StageWorkItemRow,
    worker: &stage_worker_runs::StageWorkerRunRow,
    failure_code: &str,
    attempts_used: i64,
    max_attempts: i64,
) -> StageWorkerOutputRow {
    let canonical_output = serde_json::json!({
        "attempts_used": attempts_used,
        "failure_code": failure_code,
        "kind": "stage_team_attempts_exhausted",
        "max_attempts": max_attempts,
        "schema_version": 1,
        "stable_work_key": item.stable_key,
    });
    let blocker_codes = vec![STAGE_TEAM_ATTEMPTS_EXHAUSTED.to_string()];
    let hash_material = serde_json::json!({
        "blocker_code": blocker_codes.first(),
        "canonical_output": canonical_output,
        "checked_empty_units": [],
        "disposition": "blocked",
        "evidence_ids": [],
        "fact_refs": [],
        "output_schema": item.output_schema,
        "work_item_id": item.id,
        "worker_run_id": worker.id,
    });
    StageWorkerOutputRow {
        id: Uuid::new_v5(&item.id, b"stage-worker-output-v1"),
        team_plan_id: plan.id,
        work_item_id: item.id,
        worker_run_id: worker.id,
        operation_id: plan.operation_id,
        stage_execution_id: plan.stage_execution_id,
        stage_run_unit_id: plan.stage_run_unit_id,
        scope_snapshot_id: plan.scope_snapshot_id,
        organization_id: plan.organization_id,
        output_schema: item.output_schema.clone(),
        output_version: 1,
        business_disposition: "blocked".to_string(),
        canonical_output,
        canonical_fact_refs: serde_json::json!([]),
        evidence_ids: Vec::new(),
        checked_empty_cells: serde_json::json!([]),
        blocker_codes,
        output_hash: format!(
            "sha256:{}",
            operation_scope_decisions::sha256_json(&hash_material)
        ),
        // The INSERT below owns the database timestamp.  Replay comparison
        // deliberately ignores this process-local placeholder.
        created_at: Utc::now(),
    }
}

pub(crate) async fn insert_exhausted_stage_worker_output(
    connection: &mut PgConnection,
    output: &StageWorkerOutputRow,
) -> RuntimeMemoryStoreResult<()> {
    sqlx::query(
        r#"INSERT INTO stage_worker_outputs(
               id,team_plan_id,work_item_id,worker_run_id,operation_id,stage_execution_id,
               stage_run_unit_id,scope_snapshot_id,organization_id,output_schema,output_version,
               business_disposition,canonical_output,canonical_fact_refs,evidence_ids,
               checked_empty_cells,blocker_codes,output_hash
           ) VALUES($1,$2,$3,$4,$5,$6,$7,$8,$9,$10,$11,$12,$13,$14,$15,$16,$17,$18)"#,
    )
    .bind(output.id)
    .bind(output.team_plan_id)
    .bind(output.work_item_id)
    .bind(output.worker_run_id)
    .bind(output.operation_id)
    .bind(output.stage_execution_id)
    .bind(output.stage_run_unit_id)
    .bind(output.scope_snapshot_id)
    .bind(output.organization_id)
    .bind(&output.output_schema)
    .bind(output.output_version)
    .bind(&output.business_disposition)
    .bind(&output.canonical_output)
    .bind(&output.canonical_fact_refs)
    .bind(&output.evidence_ids)
    .bind(&output.checked_empty_cells)
    .bind(&output.blocker_codes)
    .bind(&output.output_hash)
    .execute(&mut *connection)
    .await?;
    Ok(())
}

fn active_tool_recovery_blocked_output(
    plan: &StageTeamPlanRow,
    item: &StageWorkItemRow,
    worker: &stage_worker_runs::StageWorkerRunRow,
    decision: &StageTeamRecoveryDecisionRow,
) -> StageWorkerOutputRow {
    let canonical_output = serde_json::json!({
        "failure_code": "stage_team_active_tool_outcome_unknown",
        "kind": "stage_team_active_tool_recovery_blocked",
        "recovery_decision_id": decision.id,
        "recovery_request_id": decision.request_id,
        "schema_version": 1,
        "stable_work_key": item.stable_key,
        "tool_call_record_id": decision.tool_call_record_id,
    });
    let blocker_codes = vec!["STAGE_TEAM_ACTIVE_TOOL_RECOVERY_BLOCKED".to_string()];
    let hash_material = serde_json::json!({
        "blocker_code": blocker_codes.first(),
        "canonical_output": canonical_output,
        "checked_empty_units": [],
        "disposition": "blocked",
        "evidence_ids": [],
        "fact_refs": [],
        "output_schema": item.output_schema,
        "work_item_id": item.id,
        "worker_run_id": worker.id,
    });
    StageWorkerOutputRow {
        id: Uuid::new_v5(&item.id, b"stage-worker-output-v1"),
        team_plan_id: plan.id,
        work_item_id: item.id,
        worker_run_id: worker.id,
        operation_id: plan.operation_id,
        stage_execution_id: plan.stage_execution_id,
        stage_run_unit_id: plan.stage_run_unit_id,
        scope_snapshot_id: plan.scope_snapshot_id,
        organization_id: plan.organization_id,
        output_schema: item.output_schema.clone(),
        output_version: 1,
        business_disposition: "blocked".to_string(),
        canonical_output,
        canonical_fact_refs: serde_json::json!([]),
        evidence_ids: Vec::new(),
        checked_empty_cells: serde_json::json!([]),
        blocker_codes,
        output_hash: format!(
            "sha256:{}",
            operation_scope_decisions::sha256_json(&hash_material)
        ),
        created_at: Utc::now(),
    }
}

/// Resolve one startup-parked active-tool attempt without replaying the
/// external side effect.  The operator can only mark the outcome unknown and
/// blocked; all mutable identities are reloaded and compared under locks.
/// The decision, failed local tool lifecycle, terminal Worker/WorkItem and
/// immutable blocked output commit atomically, and the same request is an
/// exact response-loss replay.
pub async fn resolve_stage_team_recovery(
    pool: &PgPool,
    input: &ResolveStageTeamRecoveryRow,
) -> RuntimeMemoryStoreResult<ResolvedStageTeamRecoveryRow> {
    if input.request_id.trim().is_empty()
        || input.request_id.len() > 256
        || input.resolved_by.trim().is_empty()
        || input.resolved_by.len() > 256
        || input.expected_work_item_row_version < 0
        || input.expected_checkpoint_version < 0
        || input.expected_attempt_epoch < 0
    {
        return Err(RuntimeMemoryStoreError::IdentityMismatch {
            code: "invalid_stage_team_recovery_resolution",
        });
    }
    let mut tx = pool.begin().await?;
    let existing_sql = format!(
        "SELECT {RECOVERY_DECISION_COLUMNS}
           FROM stage_team_recovery_decisions WHERE worker_run_id=$1 FOR UPDATE"
    );
    if let Some(decision) = sqlx::query_as::<_, StageTeamRecoveryDecisionRow>(&existing_sql)
        .bind(input.worker_run_id)
        .fetch_optional(&mut *tx)
        .await?
    {
        if decision.request_id != input.request_id
            || decision.team_plan_id != input.team_plan_id
            || decision.work_item_id != input.work_item_id
            || decision.tool_call_record_id != input.tool_call_record_id
            || decision.operation_id != input.operation_id
            || decision.stage_execution_id != input.stage_execution_id
            || decision.stage_run_unit_id != input.stage_run_unit_id
            || decision.scope_snapshot_id != input.scope_snapshot_id
            || decision.expected_work_item_row_version != input.expected_work_item_row_version
            || decision.expected_checkpoint_version != input.expected_checkpoint_version
            || decision.expected_attempt_epoch != input.expected_attempt_epoch
            || decision.resolution_kind != "mark_blocked_outcome_unknown"
            || decision.resolved_by != input.resolved_by
        {
            return Err(RuntimeMemoryStoreError::Conflict {
                code: "stage_team_recovery_resolution_replay_mismatch",
            });
        }
        let item_sql = format!("SELECT {ITEM_COLUMNS} FROM stage_work_items WHERE id=$1");
        let item = sqlx::query_as::<_, StageWorkItemRow>(&item_sql)
            .bind(input.work_item_id)
            .fetch_one(&mut *tx)
            .await?;
        let worker = sqlx::query_as::<_, stage_worker_runs::StageWorkerRunRow>(
            "SELECT * FROM stage_worker_runs WHERE id=$1",
        )
        .bind(input.worker_run_id)
        .fetch_one(&mut *tx)
        .await?;
        let output_sql =
            format!("SELECT {OUTPUT_COLUMNS} FROM stage_worker_outputs WHERE work_item_id=$1");
        let output = sqlx::query_as::<_, StageWorkerOutputRow>(&output_sql)
            .bind(input.work_item_id)
            .fetch_optional(&mut *tx)
            .await?
            .ok_or(RuntimeMemoryStoreError::Conflict {
                code: "stage_team_recovery_output_missing",
            })?;
        tx.commit().await?;
        return Ok(ResolvedStageTeamRecoveryRow {
            decision,
            work_item: item,
            worker,
            output,
            replayed: true,
        });
    }

    let operation = sqlx::query_as::<_, (Option<Uuid>, String)>(
        "SELECT superseded_by,runtime_memory_contract
           FROM operation_state WHERE operation_id=$1 FOR UPDATE",
    )
    .bind(input.operation_id)
    .fetch_optional(&mut *tx)
    .await?
    .ok_or(RuntimeMemoryStoreError::Missing {
        entity: "operation_state",
    })?;
    if operation.0.is_some() || operation.1 != "v2_only" {
        return Err(RuntimeMemoryStoreError::Conflict {
            code: "stage_team_recovery_operation_not_active_v2",
        });
    }
    let plan_sql = format!("SELECT {PLAN_COLUMNS} FROM stage_team_plans WHERE id=$1 FOR UPDATE");
    let plan = sqlx::query_as::<_, StageTeamPlanRow>(&plan_sql)
        .bind(input.team_plan_id)
        .fetch_optional(&mut *tx)
        .await?
        .ok_or(RuntimeMemoryStoreError::Missing {
            entity: "stage_team_plans",
        })?;
    let item_sql = format!("SELECT {ITEM_COLUMNS} FROM stage_work_items WHERE id=$1 FOR UPDATE");
    let item = sqlx::query_as::<_, StageWorkItemRow>(&item_sql)
        .bind(input.work_item_id)
        .fetch_optional(&mut *tx)
        .await?
        .ok_or(RuntimeMemoryStoreError::Missing {
            entity: "stage_work_items",
        })?;
    let worker = sqlx::query_as::<_, stage_worker_runs::StageWorkerRunRow>(
        "SELECT * FROM stage_worker_runs WHERE id=$1 FOR UPDATE",
    )
    .bind(input.worker_run_id)
    .fetch_optional(&mut *tx)
    .await?
    .ok_or(RuntimeMemoryStoreError::Missing {
        entity: "stage_worker_runs",
    })?;
    let tool_status = sqlx::query_scalar::<_, String>(
        "SELECT status::text FROM tool_calls WHERE id=$1 FOR UPDATE",
    )
    .bind(input.tool_call_record_id)
    .fetch_optional(&mut *tx)
    .await?
    .ok_or(RuntimeMemoryStoreError::Missing {
        entity: "tool_calls",
    })?;
    if plan.operation_id != input.operation_id
        || plan.stage_execution_id != input.stage_execution_id
        || plan.stage_run_unit_id != input.stage_run_unit_id
        || plan.scope_snapshot_id != input.scope_snapshot_id
        || item.team_plan_id != plan.id
        || item.stage_run_unit_id != plan.stage_run_unit_id
        || item.organization_id != plan.organization_id
        || item.status != "recovery_required"
        || item.row_version != input.expected_work_item_row_version
        || worker.work_item_id != Some(item.id)
        || worker.operation_id != plan.operation_id
        || worker.stage_execution_id != plan.stage_execution_id
        || worker.stage_run_unit_id != plan.stage_run_unit_id
        || worker.organization_id != plan.organization_id
        || worker.status != "recovery_required"
        || worker.checkpoint_version != input.expected_checkpoint_version
        || worker.attempt_epoch != input.expected_attempt_epoch
        || worker.active_tool_call_id != Some(input.tool_call_record_id)
        || !matches!(tool_status.as_str(), "received" | "running")
    {
        return Err(RuntimeMemoryStoreError::Conflict {
            code: "stage_team_recovery_resolution_cas_failed",
        });
    }
    let exact_tool = sqlx::query_scalar::<_, bool>(
        r#"SELECT EXISTS(
               SELECT 1 FROM tool_calls
                WHERE id=$1 AND worker_run_id=$2 AND operation_id=$3
                  AND stage_execution_id=$4 AND stage_run_unit_id=$5
                  AND organization_id=$6 AND attempt_epoch=$7
                  AND lease_token=$8 AND status IN ('received','running')
           )"#,
    )
    .bind(input.tool_call_record_id)
    .bind(worker.id)
    .bind(plan.operation_id)
    .bind(plan.stage_execution_id)
    .bind(plan.stage_run_unit_id)
    .bind(plan.organization_id)
    .bind(worker.attempt_epoch)
    .bind(worker.lease_token)
    .fetch_one(&mut *tx)
    .await?;
    if !exact_tool {
        return Err(RuntimeMemoryStoreError::Conflict {
            code: "stage_team_recovery_active_tool_fence_mismatch",
        });
    }

    let decision_id = Uuid::new_v5(&worker.id, b"stage-team-recovery-decision-v1");
    let resolution_payload = serde_json::json!({
        "decision": "mark_blocked_outcome_unknown",
        "schema_version": 1,
        "tool_call_record_id": input.tool_call_record_id,
        "work_item_id": item.id,
        "worker_run_id": worker.id,
    });
    let resolution_hash = format!(
        "sha256:{}",
        operation_scope_decisions::sha256_json(&resolution_payload)
    );
    let decision_sql = format!(
        r#"INSERT INTO stage_team_recovery_decisions(
               id,request_id,team_plan_id,work_item_id,worker_run_id,tool_call_record_id,
               operation_id,stage_execution_id,stage_run_unit_id,scope_snapshot_id,organization_id,
               expected_work_item_row_version,expected_checkpoint_version,
               expected_attempt_epoch,resolution_kind,resolution_payload,resolution_hash,resolved_by
           ) VALUES($1,$2,$3,$4,$5,$6,$7,$8,$9,$10,$11,$12,$13,$14,
                    'mark_blocked_outcome_unknown',$15,$16,$17)
           RETURNING {RECOVERY_DECISION_COLUMNS}"#
    );
    let decision = sqlx::query_as::<_, StageTeamRecoveryDecisionRow>(&decision_sql)
        .bind(decision_id)
        .bind(input.request_id.trim())
        .bind(plan.id)
        .bind(item.id)
        .bind(worker.id)
        .bind(input.tool_call_record_id)
        .bind(plan.operation_id)
        .bind(plan.stage_execution_id)
        .bind(plan.stage_run_unit_id)
        .bind(plan.scope_snapshot_id)
        .bind(plan.organization_id)
        .bind(input.expected_work_item_row_version)
        .bind(input.expected_checkpoint_version)
        .bind(input.expected_attempt_epoch)
        .bind(&resolution_payload)
        .bind(&resolution_hash)
        .bind(input.resolved_by.trim())
        .fetch_one(&mut *tx)
        .await?;
    let tool_result = serde_json::to_string(&serde_json::json!({
        "decision_id": decision.id,
        "kind": "stage_team_operator_recovery",
        "outcome": "unknown_blocked_without_replay",
        "schema_version": 1,
    }))
    .map_err(|_| RuntimeMemoryStoreError::IdentityMismatch {
        code: "stage_team_recovery_tool_result_invalid",
    })?;
    let tool_rows = sqlx::query(
        r#"UPDATE tool_calls
              SET status='failed',result=$2,updated_at=NOW()
            WHERE id=$1 AND worker_run_id=$3 AND attempt_epoch=$4
              AND status IN ('received','running')"#,
    )
    .bind(input.tool_call_record_id)
    .bind(tool_result)
    .bind(worker.id)
    .bind(worker.attempt_epoch)
    .execute(&mut *tx)
    .await?
    .rows_affected();
    if tool_rows != 1 {
        return Err(RuntimeMemoryStoreError::Conflict {
            code: "stage_team_recovery_tool_cas_failed",
        });
    }
    let terminal_checkpoint = serde_json::json!({
        "stage_team_recovery_resolution": {
            "decision_id": decision.id,
            "kind": "mark_blocked_outcome_unknown",
            "schema_version": 1,
            "tool_call_record_id": input.tool_call_record_id,
        }
    });
    let worker = sqlx::query_as::<_, stage_worker_runs::StageWorkerRunRow>(
        r#"UPDATE stage_worker_runs
              SET status='failed',checkpoint=$6,checkpoint_version=checkpoint_version+1,
                  active_tool_call_id=NULL,active_tool_started_at=NULL,
                  lease_token=NULL,lease_owner=NULL,lease_acquired_at=NULL,
                  lease_expires_at=NULL,heartbeat_at=NULL,terminal_at=NOW(),updated_at=NOW()
            WHERE id=$1 AND status='recovery_required' AND attempt_epoch=$2
              AND checkpoint_version=$3 AND active_tool_call_id=$4
              AND work_item_id=$5
            RETURNING *"#,
    )
    .bind(worker.id)
    .bind(input.expected_attempt_epoch)
    .bind(input.expected_checkpoint_version)
    .bind(input.tool_call_record_id)
    .bind(item.id)
    .bind(&terminal_checkpoint)
    .fetch_optional(&mut *tx)
    .await?
    .ok_or(RuntimeMemoryStoreError::Conflict {
        code: "stage_team_recovery_worker_cas_failed",
    })?;
    let item_sql = format!(
        "UPDATE stage_work_items
            SET status='exhausted',row_version=row_version+1,terminal_at=NOW(),updated_at=NOW()
          WHERE id=$1 AND team_plan_id=$2 AND status='recovery_required' AND row_version=$3
          RETURNING {ITEM_COLUMNS}"
    );
    let item = sqlx::query_as::<_, StageWorkItemRow>(&item_sql)
        .bind(item.id)
        .bind(plan.id)
        .bind(input.expected_work_item_row_version)
        .fetch_optional(&mut *tx)
        .await?
        .ok_or(RuntimeMemoryStoreError::StaleVersion {
            entity: "stage_work_items",
            expected: input.expected_work_item_row_version,
            actual: -1,
        })?;
    let output = active_tool_recovery_blocked_output(&plan, &item, &worker, &decision);
    insert_exhausted_stage_worker_output(&mut tx, &output).await?;
    let output_sql = format!("SELECT {OUTPUT_COLUMNS} FROM stage_worker_outputs WHERE id=$1");
    let output = sqlx::query_as::<_, StageWorkerOutputRow>(&output_sql)
        .bind(output.id)
        .fetch_one(&mut *tx)
        .await?;
    tx.commit().await?;
    Ok(ResolvedStageTeamRecoveryRow {
        decision,
        work_item: item,
        worker,
        output,
        replayed: false,
    })
}

#[derive(Debug, Clone, PartialEq)]
pub(crate) struct ReapedCleanStageWorkerRow {
    pub worker: stage_worker_runs::StageWorkerRunRow,
    pub retry_scheduled: bool,
}

pub(crate) async fn work_item_attempts_used(
    connection: &mut PgConnection,
    work_item_id: Uuid,
) -> RuntimeMemoryStoreResult<i64> {
    Ok(sqlx::query_scalar(
        "SELECT COALESCE(SUM(GREATEST(attempt_epoch, 1)), 0)::BIGINT
           FROM stage_worker_runs
          WHERE work_item_id=$1",
    )
    .bind(work_item_id)
    .fetch_one(connection)
    .await?)
}

async fn is_exact_closed_company_final_submitter_with_submission(
    connection: &mut PgConnection,
    plan: &StageTeamPlanRow,
    item: &StageWorkItemRow,
    worker: &stage_worker_runs::StageWorkerRunRow,
) -> RuntimeMemoryStoreResult<bool> {
    if plan.requests_closed_at.is_none()
        || plan.final_submitter_worker_run_id != Some(worker.id)
        || plan.final_submitter_kind != "worker"
        || plan.aggregator_kind != "worker"
        || plan.aggregator_role.as_deref() != Some(item.role.as_str())
        || plan
            .dynamic_request_policy
            .get("coordination_mode")
            .and_then(serde_json::Value::as_str)
            != Some("company_controller")
        || item.stable_key != "leader:primary"
        || item.required_for_barrier
        || worker.lease_token.is_none()
    {
        return Ok(false);
    }

    Ok(sqlx::query_scalar::<_, bool>(
        r#"SELECT EXISTS(
               SELECT 1
                 FROM stage_deliverable_submissions submission
                WHERE submission.operation_id=$1
                  AND submission.stage_execution_id=$2
                  AND submission.stage_run_unit_id=$3
                  AND submission.organization_id=$4
                  AND submission.worker_run_id=$5
                  AND submission.attempt_epoch=$6
                  AND submission.lease_token=$7
           )"#,
    )
    .bind(plan.operation_id)
    .bind(plan.stage_execution_id)
    .bind(plan.stage_run_unit_id)
    .bind(plan.organization_id)
    .bind(worker.id)
    .bind(worker.attempt_epoch)
    .bind(worker.lease_token)
    .fetch_one(connection)
    .await?)
}

/// Resolve one expired Team worker that has no in-flight tool.  An expired
/// lease consumes an attempt just like an ordinary execution failure.  The
/// stable WorkItem and WorkerRun are requeued only while the frozen attempt
/// allowance can fund a new Turn on the exact same message chain.  The
/// TeamPlan lifetime-worker allowance is not consumed by a continuation Turn;
/// it limits distinct logical workers, not restarts of one worker. Otherwise
/// the final attempt and WorkItem become terminal together and a deterministic
/// blocked output is written for the barrier/Gate to consume.
pub(crate) async fn reap_expired_clean_stage_worker(
    connection: &mut PgConnection,
    plan: &StageTeamPlanRow,
    item: &StageWorkItemRow,
    worker: &stage_worker_runs::StageWorkerRunRow,
) -> RuntimeMemoryStoreResult<ReapedCleanStageWorkerRow> {
    if item.team_plan_id != plan.id
        || item.stage_run_unit_id != plan.stage_run_unit_id
        || item.status != "running"
        || worker.work_item_id != Some(item.id)
        || worker.stage_run_unit_id != plan.stage_run_unit_id
        || worker.organization_id != plan.organization_id
        || worker.active_tool_call_id.is_some()
        || !matches!(
            worker.status.as_str(),
            "running" | "waiting_background" | "gate_blocked"
        )
    {
        return Err(RuntimeMemoryStoreError::IdentityMismatch {
            code: "startup_team_worker_reap_identity_mismatch",
        });
    }

    let max_attempts = work_item_max_attempts(item)?;
    let attempts_used = work_item_attempts_used(&mut *connection, item.id).await?;
    // A closed Company Controller with an exact durable submission is retrying
    // deterministic closeout, not producing another coverage fact. Do not
    // spend producer attempt fuel or forge an attempts-exhausted output for it.
    let finalizer_retry_scheduled =
        is_exact_closed_company_final_submitter_with_submission(connection, plan, item, worker)
            .await?;
    let retry_scheduled = finalizer_retry_scheduled || attempts_used < max_attempts;

    let (next_worker_status, next_item_status, terminal_checkpoint) = if retry_scheduled {
        ("queued", "retry_pending", worker.checkpoint.clone())
    } else {
        (
            "failed",
            "exhausted",
            serde_json::json!({
                "stage_team_execution_failure": {
                    "attempts_used": attempts_used,
                    "code": "stage_team_worker_lease_expired",
                    "max_attempts": max_attempts,
                    "schema_version": 1,
                }
            }),
        )
    };
    let worker_sql = format!(
        r#"UPDATE stage_worker_runs
              SET status='{next_worker_status}',checkpoint=$3,
                  checkpoint_version=checkpoint_version+1,
                  lease_token=NULL,lease_owner=NULL,lease_acquired_at=NULL,
                  lease_expires_at=NULL,heartbeat_at=NULL,
                  terminal_at=CASE WHEN '{next_worker_status}'='queued' THEN NULL ELSE NOW() END,
                  updated_at=NOW()
            WHERE id=$1 AND attempt_epoch=$2
              AND status IN ('running','waiting_background','gate_blocked')
              AND active_tool_call_id IS NULL
            RETURNING *"#,
    );
    let worker = sqlx::query_as::<_, stage_worker_runs::StageWorkerRunRow>(&worker_sql)
        .bind(worker.id)
        .bind(worker.attempt_epoch)
        .bind(&terminal_checkpoint)
        .fetch_optional(&mut *connection)
        .await?
        .ok_or(RuntimeMemoryStoreError::Conflict {
            code: "startup_team_worker_reap_cas_failed",
        })?;
    let item_terminal = if retry_scheduled { "NULL" } else { "NOW()" };
    let item_sql = format!(
        "UPDATE stage_work_items
            SET status='{next_item_status}',row_version=row_version+1,
                terminal_at={item_terminal},updated_at=NOW()
          WHERE id=$1 AND team_plan_id=$2 AND status='running' AND row_version=$3
          RETURNING {ITEM_COLUMNS}"
    );
    let item = sqlx::query_as::<_, StageWorkItemRow>(&item_sql)
        .bind(item.id)
        .bind(plan.id)
        .bind(item.row_version)
        .fetch_optional(&mut *connection)
        .await?
        .ok_or(RuntimeMemoryStoreError::StaleVersion {
            entity: "stage_work_items",
            expected: item.row_version,
            actual: -1,
        })?;
    if retry_scheduled {
        let queue_sql = format!(
            "UPDATE stage_work_items
                SET status='queued',row_version=row_version+1,updated_at=NOW()
              WHERE id=$1 AND team_plan_id=$2 AND status='retry_pending' AND row_version=$3
              RETURNING {ITEM_COLUMNS}"
        );
        sqlx::query_as::<_, StageWorkItemRow>(&queue_sql)
            .bind(item.id)
            .bind(plan.id)
            .bind(item.row_version)
            .fetch_optional(&mut *connection)
            .await?
            .ok_or(RuntimeMemoryStoreError::StaleVersion {
                entity: "stage_work_items",
                expected: item.row_version,
                actual: -1,
            })?;
    } else {
        let output = exhausted_stage_worker_output(
            plan,
            &item,
            &worker,
            "stage_team_worker_lease_expired",
            attempts_used,
            max_attempts,
        );
        insert_exhausted_stage_worker_output(connection, &output).await?;
    }
    Ok(ReapedCleanStageWorkerRow {
        worker,
        retry_scheduled,
    })
}

fn exhausted_output_replays_exactly(
    existing: &StageWorkerOutputRow,
    expected: &StageWorkerOutputRow,
) -> bool {
    existing.id == expected.id
        && existing.team_plan_id == expected.team_plan_id
        && existing.work_item_id == expected.work_item_id
        && existing.worker_run_id == expected.worker_run_id
        && existing.operation_id == expected.operation_id
        && existing.stage_execution_id == expected.stage_execution_id
        && existing.stage_run_unit_id == expected.stage_run_unit_id
        && existing.scope_snapshot_id == expected.scope_snapshot_id
        && existing.organization_id == expected.organization_id
        && existing.output_schema == expected.output_schema
        && existing.output_version == expected.output_version
        && existing.business_disposition == expected.business_disposition
        && existing.canonical_output == expected.canonical_output
        && existing.canonical_fact_refs == expected.canonical_fact_refs
        && existing.evidence_ids == expected.evidence_ids
        && existing.checked_empty_cells == expected.checked_empty_cells
        && existing.blocker_codes == expected.blocker_codes
        && existing.output_hash == expected.output_hash
}

async fn validate_stage_worker_output_authority(
    connection: &mut PgConnection,
    unit: &stage_run_units::StageRunUnitRow,
    plan: &StageTeamPlanRow,
    input: &CompleteStageWorkerRow,
) -> RuntimeMemoryStoreResult<()> {
    let evidence_ids = input
        .evidence_ids
        .iter()
        .copied()
        .collect::<std::collections::BTreeSet<_>>();
    if evidence_ids.len() != input.evidence_ids.len()
        || input.evidence_ids.windows(2).any(|pair| pair[0] >= pair[1])
        || input.evidence_watermark != input.evidence_ids.iter().copied().max()
    {
        return Err(RuntimeMemoryStoreError::IdentityMismatch {
            code: "stage_worker_output_evidence_manifest_invalid",
        });
    }
    if input.blocker_codes.len() > 1
        || input
            .blocker_codes
            .iter()
            .any(|code| code.trim().is_empty())
        || (input.business_disposition == "blocked" && input.blocker_codes.len() != 1)
        || (input.business_disposition != "blocked" && !input.blocker_codes.is_empty())
        || input
            .checked_empty_cells
            .as_array()
            .is_some_and(|cells| cells.iter().any(Value::is_null))
    {
        return Err(RuntimeMemoryStoreError::IdentityMismatch {
            code: "stage_worker_output_disposition_invalid",
        });
    }
    let fact_values =
        input
            .canonical_fact_refs
            .as_array()
            .ok_or(RuntimeMemoryStoreError::IdentityMismatch {
                code: "stage_worker_output_fact_refs_invalid",
            })?;
    if fact_values.len() > canonical_fact_refs::MAX_CANONICAL_REFS {
        return Err(RuntimeMemoryStoreError::IdentityMismatch {
            code: "stage_worker_output_fact_refs_invalid",
        });
    }
    let fact_keys = fact_values
        .iter()
        .cloned()
        .map(|value| {
            serde_json::from_value::<canonical_fact_refs::CanonicalFactKey>(value).map_err(|_| {
                RuntimeMemoryStoreError::IdentityMismatch {
                    code: "stage_worker_output_fact_refs_invalid",
                }
            })
        })
        .collect::<RuntimeMemoryStoreResult<Vec<_>>>()?;
    let checked_empty_cells =
        input
            .checked_empty_cells
            .as_array()
            .ok_or(RuntimeMemoryStoreError::IdentityMismatch {
                code: "stage_worker_output_checked_empty_invalid",
            })?;
    match input.business_disposition.as_str() {
        "found" if fact_keys.is_empty() && input.evidence_ids.is_empty() => {
            return Err(RuntimeMemoryStoreError::IdentityMismatch {
                code: "stage_worker_found_without_authority",
            });
        }
        "checked_empty" if checked_empty_cells.is_empty() || input.evidence_ids.is_empty() => {
            return Err(RuntimeMemoryStoreError::IdentityMismatch {
                code: "stage_worker_checked_empty_without_attestation",
            });
        }
        "blocked" => {}
        "found" | "checked_empty" => {}
        _ => {
            return Err(RuntimeMemoryStoreError::IdentityMismatch {
                code: "invalid_stage_worker_output",
            });
        }
    }
    if canonical_stage_worker_output_hash(input) != input.output_hash {
        return Err(RuntimeMemoryStoreError::IdentityMismatch {
            code: "stage_worker_output_hash_mismatch",
        });
    }
    let (project_path_at_freeze,): (String,) = sqlx::query_as(
        "SELECT project_path_at_freeze FROM operation_org_scope_snapshots
          WHERE id=$1 AND operation_id=$2 AND sealed_at IS NOT NULL FOR SHARE",
    )
    .bind(plan.scope_snapshot_id)
    .bind(plan.operation_id)
    .fetch_optional(&mut *connection)
    .await?
    .ok_or(RuntimeMemoryStoreError::Missing {
        entity: "operation_org_scope_snapshots",
    })?;
    let freshness_floor = unit.started_at.ok_or(RuntimeMemoryStoreError::Conflict {
        code: "stage_worker_unit_has_no_freshness_floor",
    })?;
    let canonical_refs = canonical_fact_refs::resolve_for_handoff(
        connection,
        plan.operation_id,
        plan.organization_id,
        &project_path_at_freeze,
        freshness_floor,
        &fact_keys,
    )
    .await
    .map_err(|error| match error {
        canonical_fact_refs::CanonicalFactRefError::Rejected { code } => {
            RuntimeMemoryStoreError::IdentityMismatch { code }
        }
        canonical_fact_refs::CanonicalFactRefError::Sqlx(error) => {
            RuntimeMemoryStoreError::Sqlx(error)
        }
    })?;
    if canonical_refs
        .iter()
        .flat_map(|fact| fact.evidence_ids.iter())
        .any(|evidence_id| !evidence_ids.contains(evidence_id))
    {
        return Err(RuntimeMemoryStoreError::IdentityMismatch {
            code: "stage_worker_fact_evidence_not_cited",
        });
    }
    super::runtime_memory_tx::validate_final_seal_evidence(
        connection,
        &input.evidence_ids,
        plan.operation_id,
        plan.organization_id,
        &project_path_at_freeze,
        freshness_floor,
        &std::collections::BTreeSet::new(),
    )
    .await
}

pub async fn complete_stage_worker(
    pool: &PgPool,
    input: CompleteStageWorkerRow,
) -> RuntimeMemoryStoreResult<CompletedStageWorkerRow> {
    if !matches!(
        input.business_disposition.as_str(),
        "found" | "checked_empty" | "blocked"
    ) || !input.canonical_output.is_object()
        || !input.canonical_fact_refs.is_array()
        || !input.checked_empty_cells.is_array()
        || !validate_hash(&input.output_hash)
        || (input.business_disposition == "blocked" && input.blocker_codes.is_empty())
    {
        return Err(RuntimeMemoryStoreError::IdentityMismatch {
            code: "invalid_stage_worker_output",
        });
    }
    let mut tx = pool.begin().await?;
    let operation = sqlx::query_as::<_, (Option<Uuid>, String)>(
        "SELECT superseded_by,runtime_memory_contract
           FROM operation_state WHERE operation_id=$1 FOR UPDATE",
    )
    .bind(input.fence.operation_id)
    .fetch_optional(&mut *tx)
    .await?
    .ok_or(RuntimeMemoryStoreError::Missing {
        entity: "operation_state",
    })?;
    if operation.0.is_some() {
        return Err(RuntimeMemoryStoreError::Conflict {
            code: "operation_superseded",
        });
    }
    if operation.1 != "v2_only" {
        return Err(RuntimeMemoryStoreError::Conflict {
            code: "stage_team_requires_v2_only",
        });
    }
    let unit = sqlx::query_as::<_, stage_run_units::StageRunUnitRow>(
        "SELECT * FROM stage_run_units
          WHERE id=$1 AND operation_id=$2 AND stage_execution_id=$3 FOR UPDATE",
    )
    .bind(input.fence.stage_run_unit_id)
    .bind(input.fence.operation_id)
    .bind(input.fence.stage_execution_id)
    .fetch_optional(&mut *tx)
    .await?
    .ok_or(RuntimeMemoryStoreError::Missing {
        entity: "stage_run_units",
    })?;
    let plan_sql = format!("SELECT {PLAN_COLUMNS} FROM stage_team_plans WHERE id=$1 FOR UPDATE");
    let plan = sqlx::query_as::<_, StageTeamPlanRow>(&plan_sql)
        .bind(input.team_plan_id)
        .fetch_optional(&mut *tx)
        .await?
        .ok_or(RuntimeMemoryStoreError::Missing {
            entity: "stage_team_plans",
        })?;
    let item_sql = format!("SELECT {ITEM_COLUMNS} FROM stage_work_items WHERE id=$1 FOR UPDATE");
    let item = sqlx::query_as::<_, StageWorkItemRow>(&item_sql)
        .bind(input.work_item_id)
        .fetch_optional(&mut *tx)
        .await?
        .ok_or(RuntimeMemoryStoreError::Missing {
            entity: "stage_work_items",
        })?;
    if plan.stage_run_unit_id != input.fence.stage_run_unit_id
        || plan.operation_id != input.fence.operation_id
        || plan.stage_execution_id != input.fence.stage_execution_id
        || item.team_plan_id != plan.id
        || item.operation_id != plan.operation_id
        || item.stage_execution_id != plan.stage_execution_id
        || item.stage_run_unit_id != plan.stage_run_unit_id
        || item.organization_id != plan.organization_id
        || item.output_schema != input.output_schema
        || is_aggregator_item(&plan, &item)
    {
        return Err(RuntimeMemoryStoreError::Conflict {
            code: "stage_producer_completion_fence_mismatch",
        });
    }
    if unit.organization_id != plan.organization_id
        || unit.scope_snapshot_id != plan.scope_snapshot_id
        || unit.status != "running"
    {
        return Err(RuntimeMemoryStoreError::Conflict {
            code: "stage_producer_must_not_terminalize_unit",
        });
    }
    validate_stage_worker_output_authority(&mut tx, &unit, &plan, &input).await?;
    let worker = sqlx::query_as::<_, stage_worker_runs::StageWorkerRunRow>(
        "SELECT * FROM stage_worker_runs WHERE id=$1 FOR UPDATE",
    )
    .bind(input.fence.worker_run_id)
    .fetch_optional(&mut *tx)
    .await?
    .ok_or(RuntimeMemoryStoreError::Missing {
        entity: "stage_worker_runs",
    })?;
    if worker.operation_id != plan.operation_id
        || worker.stage_execution_id != plan.stage_execution_id
        || worker.stage_run_unit_id != plan.stage_run_unit_id
        || worker.organization_id != plan.organization_id
        || worker.work_item_id != Some(item.id)
        || worker.attempt_epoch != input.fence.attempt_epoch
    {
        return Err(RuntimeMemoryStoreError::Conflict {
            code: "stage_producer_completion_fence_mismatch",
        });
    }
    let existing_sql = format!(
        "SELECT {OUTPUT_COLUMNS} FROM stage_worker_outputs WHERE work_item_id=$1 FOR SHARE"
    );
    if let Some(existing) = sqlx::query_as::<_, StageWorkerOutputRow>(&existing_sql)
        .bind(item.id)
        .fetch_optional(&mut *tx)
        .await?
    {
        if output_replays_exactly(&existing, &input)
            && item.status == "completed"
            && item.row_version == input.expected_work_item_row_version.saturating_add(1)
            && worker.status == "passed"
            && worker.checkpoint_version
                == input.fence.expected_checkpoint_version.saturating_add(1)
            && worker.checkpoint == input.terminal_checkpoint
            && worker.evidence_watermark == input.evidence_watermark
            && unit.status == "running"
        {
            tx.commit().await?;
            return Ok(CompletedStageWorkerRow {
                unit,
                plan,
                work_item: item,
                worker,
                output: existing,
                replayed: true,
            });
        }
        return Err(RuntimeMemoryStoreError::Conflict {
            code: "stage_worker_output_replay_mismatch",
        });
    }
    if item.row_version != input.expected_work_item_row_version || item.status != "running" {
        return Err(RuntimeMemoryStoreError::StaleVersion {
            entity: "stage_work_items",
            expected: input.expected_work_item_row_version,
            actual: item.row_version,
        });
    }
    if unit.status != "running" {
        return Err(RuntimeMemoryStoreError::Conflict {
            code: "stage_producer_must_not_terminalize_unit",
        });
    }
    let worker = stage_worker_runs::finish_passed_for_stage_output(
        &mut *tx,
        &input.fence,
        &input.terminal_checkpoint,
        input.evidence_watermark,
    )
    .await?;
    let item_update_sql = format!(
        "UPDATE stage_work_items SET status='completed',row_version=row_version+1,
             terminal_at=NOW(),updated_at=NOW()
         WHERE id=$1 AND team_plan_id=$2 AND status='running' AND row_version=$3
         RETURNING {ITEM_COLUMNS}"
    );
    let item = sqlx::query_as::<_, StageWorkItemRow>(&item_update_sql)
        .bind(item.id)
        .bind(plan.id)
        .bind(input.expected_work_item_row_version)
        .fetch_optional(&mut *tx)
        .await?
        .ok_or(RuntimeMemoryStoreError::StaleVersion {
            entity: "stage_work_items",
            expected: input.expected_work_item_row_version,
            actual: -1,
        })?;
    let output_id = Uuid::new_v5(&item.id, b"stage-worker-output-v1");
    let output_sql = format!(
        r#"INSERT INTO stage_worker_outputs(
               id,team_plan_id,work_item_id,worker_run_id,operation_id,stage_execution_id,
               stage_run_unit_id,scope_snapshot_id,organization_id,output_schema,output_version,
               business_disposition,canonical_output,canonical_fact_refs,evidence_ids,
               checked_empty_cells,blocker_codes,output_hash
           ) VALUES($1,$2,$3,$4,$5,$6,$7,$8,$9,$10,1,$11,$12,$13,$14,$15,$16,$17)
           RETURNING {OUTPUT_COLUMNS}"#,
    );
    let output = sqlx::query_as::<_, StageWorkerOutputRow>(&output_sql)
        .bind(output_id)
        .bind(plan.id)
        .bind(item.id)
        .bind(worker.id)
        .bind(plan.operation_id)
        .bind(plan.stage_execution_id)
        .bind(plan.stage_run_unit_id)
        .bind(plan.scope_snapshot_id)
        .bind(plan.organization_id)
        .bind(&input.output_schema)
        .bind(&input.business_disposition)
        .bind(&input.canonical_output)
        .bind(&input.canonical_fact_refs)
        .bind(&input.evidence_ids)
        .bind(&input.checked_empty_cells)
        .bind(&input.blocker_codes)
        .bind(&input.output_hash)
        .fetch_one(&mut *tx)
        .await?;
    let unit = stage_run_units::get_with_executor(&mut *tx, plan.stage_run_unit_id)
        .await?
        .ok_or(RuntimeMemoryStoreError::Missing {
            entity: "stage_run_units",
        })?;
    if unit.status != "running" {
        return Err(RuntimeMemoryStoreError::Conflict {
            code: "stage_producer_must_not_terminalize_unit",
        });
    }
    tx.commit().await?;
    Ok(CompletedStageWorkerRow {
        unit,
        plan,
        work_item: item,
        worker,
        output,
        replayed: false,
    })
}

/// Persist a producer/helper execution failure without forging a business
/// `StageWorkerOutput`. The failed WorkerRun is terminal; the stable WorkItem
/// is either requeued for a fresh sibling WorkerRun/message chain or exhausted
/// when its frozen attempt/lifetime budget has been consumed. The Unit remains
/// running so only scheduler/recovery authority decides what happens next.
pub async fn retry_stage_worker(
    pool: &PgPool,
    input: RetryStageWorkerRow,
) -> RuntimeMemoryStoreResult<RetriedStageWorkerRow> {
    let failure_code = input.failure_code.trim();
    if failure_code.is_empty()
        || failure_code.len() > 128
        || !input.terminal_checkpoint.is_object()
        || input
            .terminal_checkpoint
            .get("stage_team_execution_failure")
            .and_then(|value| value.get("code"))
            .and_then(Value::as_str)
            != Some(failure_code)
    {
        return Err(RuntimeMemoryStoreError::IdentityMismatch {
            code: "invalid_stage_worker_execution_failure",
        });
    }

    let mut tx = pool.begin().await?;
    let operation = sqlx::query_as::<_, (Option<Uuid>, String)>(
        "SELECT superseded_by,runtime_memory_contract
           FROM operation_state WHERE operation_id=$1 FOR UPDATE",
    )
    .bind(input.fence.operation_id)
    .fetch_optional(&mut *tx)
    .await?
    .ok_or(RuntimeMemoryStoreError::Missing {
        entity: "operation_state",
    })?;
    if operation.0.is_some() {
        return Err(RuntimeMemoryStoreError::Conflict {
            code: "operation_superseded",
        });
    }
    if operation.1 != "v2_only" {
        return Err(RuntimeMemoryStoreError::Conflict {
            code: "stage_team_requires_v2_only",
        });
    }

    let unit = sqlx::query_as::<_, stage_run_units::StageRunUnitRow>(
        "SELECT * FROM stage_run_units
          WHERE id=$1 AND operation_id=$2 AND stage_execution_id=$3 FOR UPDATE",
    )
    .bind(input.fence.stage_run_unit_id)
    .bind(input.fence.operation_id)
    .bind(input.fence.stage_execution_id)
    .fetch_optional(&mut *tx)
    .await?
    .ok_or(RuntimeMemoryStoreError::Missing {
        entity: "stage_run_units",
    })?;
    let plan_sql = format!("SELECT {PLAN_COLUMNS} FROM stage_team_plans WHERE id=$1 FOR UPDATE");
    let plan = sqlx::query_as::<_, StageTeamPlanRow>(&plan_sql)
        .bind(input.team_plan_id)
        .fetch_optional(&mut *tx)
        .await?
        .ok_or(RuntimeMemoryStoreError::Missing {
            entity: "stage_team_plans",
        })?;
    let item_sql = format!("SELECT {ITEM_COLUMNS} FROM stage_work_items WHERE id=$1 FOR UPDATE");
    let item = sqlx::query_as::<_, StageWorkItemRow>(&item_sql)
        .bind(input.work_item_id)
        .fetch_optional(&mut *tx)
        .await?
        .ok_or(RuntimeMemoryStoreError::Missing {
            entity: "stage_work_items",
        })?;
    let worker = sqlx::query_as::<_, stage_worker_runs::StageWorkerRunRow>(
        "SELECT * FROM stage_worker_runs WHERE id=$1 FOR UPDATE",
    )
    .bind(input.fence.worker_run_id)
    .fetch_optional(&mut *tx)
    .await?
    .ok_or(RuntimeMemoryStoreError::Missing {
        entity: "stage_worker_runs",
    })?;

    if plan.operation_id != input.fence.operation_id
        || plan.stage_execution_id != input.fence.stage_execution_id
        || plan.stage_run_unit_id != input.fence.stage_run_unit_id
        || plan.requests_closed_at.is_some()
        || unit.organization_id != plan.organization_id
        || unit.scope_snapshot_id != plan.scope_snapshot_id
        || unit.status != "running"
        || item.team_plan_id != plan.id
        || item.stage_run_unit_id != plan.stage_run_unit_id
        || item.organization_id != plan.organization_id
        || is_aggregator_item(&plan, &item)
        || worker.work_item_id != Some(item.id)
        || worker.stage_run_unit_id != plan.stage_run_unit_id
        || worker.organization_id != plan.organization_id
        || worker.attempt_epoch != input.fence.attempt_epoch
    {
        return Err(RuntimeMemoryStoreError::IdentityMismatch {
            code: "stage_worker_retry_fence_mismatch",
        });
    }

    let max_attempts = work_item_max_attempts(&item)?;
    let attempts_used: i64 =
        sqlx::query_scalar("SELECT COUNT(*) FROM stage_worker_runs WHERE work_item_id=$1")
            .bind(item.id)
            .fetch_one(&mut *tx)
            .await?;

    // A lost response is harmless: the same terminal checkpoint identifies
    // the already-failed WorkerRun while the stable WorkItem tells the caller
    // whether a retry was scheduled or the budget was exhausted.
    if worker.status == "failed"
        && worker.checkpoint_version == input.fence.expected_checkpoint_version.saturating_add(1)
        && worker.checkpoint == input.terminal_checkpoint
        && matches!(item.status.as_str(), "queued" | "exhausted")
        && item.row_version > input.expected_work_item_row_version
    {
        if item.status == "exhausted" {
            let expected = exhausted_stage_worker_output(
                &plan,
                &item,
                &worker,
                failure_code,
                attempts_used,
                max_attempts,
            );
            let existing_sql = format!(
                "SELECT {OUTPUT_COLUMNS} FROM stage_worker_outputs WHERE work_item_id=$1 FOR SHARE"
            );
            let existing = sqlx::query_as::<_, StageWorkerOutputRow>(&existing_sql)
                .bind(item.id)
                .fetch_optional(&mut *tx)
                .await?
                .ok_or(RuntimeMemoryStoreError::Conflict {
                    code: "stage_worker_exhaustion_output_missing",
                })?;
            if !exhausted_output_replays_exactly(&existing, &expected) {
                return Err(RuntimeMemoryStoreError::Conflict {
                    code: "stage_worker_exhaustion_output_replay_mismatch",
                });
            }
        }
        tx.commit().await?;
        return Ok(RetriedStageWorkerRow {
            unit,
            plan,
            retry_scheduled: item.status == "queued",
            work_item: item,
            worker,
        });
    }
    if item.status != "running" || item.row_version != input.expected_work_item_row_version {
        return Err(RuntimeMemoryStoreError::StaleVersion {
            entity: "stage_work_items",
            expected: input.expected_work_item_row_version,
            actual: item.row_version,
        });
    }

    let lifetime_total_available = if enforces_lifetime_worker_total(&plan.dynamic_request_policy) {
        let total_workers: i64 =
            sqlx::query_scalar("SELECT COUNT(*) FROM stage_worker_runs WHERE stage_run_unit_id=$1")
                .bind(plan.stage_run_unit_id)
                .fetch_one(&mut *tx)
                .await?;
        total_workers < i64::from(plan.max_workers_total)
    } else {
        true
    };
    let retry_scheduled = attempts_used < max_attempts && lifetime_total_available;

    let worker = stage_worker_runs::finish_attempt_cas(
        &mut *tx,
        &input.fence,
        stage_worker_runs::StageWorkerRunStatus::Running,
        stage_worker_runs::StageWorkerRunStatus::Failed,
        &input.terminal_checkpoint,
        None,
    )
    .await?;
    let next_status = if retry_scheduled {
        "retry_pending"
    } else {
        "exhausted"
    };
    let next_terminal = if retry_scheduled { "NULL" } else { "NOW()" };
    let retry_sql = format!(
        "UPDATE stage_work_items
            SET status='{next_status}',row_version=row_version+1,
                terminal_at={next_terminal},updated_at=NOW()
          WHERE id=$1 AND team_plan_id=$2 AND status='running' AND row_version=$3
          RETURNING {ITEM_COLUMNS}"
    );
    let item = sqlx::query_as::<_, StageWorkItemRow>(&retry_sql)
        .bind(item.id)
        .bind(plan.id)
        .bind(input.expected_work_item_row_version)
        .fetch_optional(&mut *tx)
        .await?
        .ok_or(RuntimeMemoryStoreError::StaleVersion {
            entity: "stage_work_items",
            expected: input.expected_work_item_row_version,
            actual: -1,
        })?;
    let item = if retry_scheduled {
        let queue_sql = format!(
            "UPDATE stage_work_items
                SET status='queued',row_version=row_version+1,updated_at=NOW()
              WHERE id=$1 AND team_plan_id=$2 AND status='retry_pending' AND row_version=$3
              RETURNING {ITEM_COLUMNS}"
        );
        sqlx::query_as::<_, StageWorkItemRow>(&queue_sql)
            .bind(item.id)
            .bind(plan.id)
            .bind(item.row_version)
            .fetch_optional(&mut *tx)
            .await?
            .ok_or(RuntimeMemoryStoreError::StaleVersion {
                entity: "stage_work_items",
                expected: item.row_version,
                actual: -1,
            })?
    } else {
        item
    };
    if !retry_scheduled {
        let exhausted_output = exhausted_stage_worker_output(
            &plan,
            &item,
            &worker,
            failure_code,
            attempts_used,
            max_attempts,
        );
        let output_sql = r#"INSERT INTO stage_worker_outputs(
                   id,team_plan_id,work_item_id,worker_run_id,operation_id,stage_execution_id,
                   stage_run_unit_id,scope_snapshot_id,organization_id,output_schema,output_version,
                   business_disposition,canonical_output,canonical_fact_refs,evidence_ids,
                   checked_empty_cells,blocker_codes,output_hash
               ) VALUES($1,$2,$3,$4,$5,$6,$7,$8,$9,$10,$11,$12,$13,$14,$15,$16,$17,$18)"#;
        sqlx::query(output_sql)
            .bind(exhausted_output.id)
            .bind(exhausted_output.team_plan_id)
            .bind(exhausted_output.work_item_id)
            .bind(exhausted_output.worker_run_id)
            .bind(exhausted_output.operation_id)
            .bind(exhausted_output.stage_execution_id)
            .bind(exhausted_output.stage_run_unit_id)
            .bind(exhausted_output.scope_snapshot_id)
            .bind(exhausted_output.organization_id)
            .bind(&exhausted_output.output_schema)
            .bind(exhausted_output.output_version)
            .bind(&exhausted_output.business_disposition)
            .bind(&exhausted_output.canonical_output)
            .bind(&exhausted_output.canonical_fact_refs)
            .bind(&exhausted_output.evidence_ids)
            .bind(&exhausted_output.checked_empty_cells)
            .bind(&exhausted_output.blocker_codes)
            .bind(&exhausted_output.output_hash)
            .execute(&mut *tx)
            .await?;
    }
    tx.commit().await?;
    Ok(RetriedStageWorkerRow {
        unit,
        plan,
        work_item: item,
        worker,
        retry_scheduled,
    })
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn team_barrier_never_passes_with_open_epoch_or_live_sibling() {
        let base = StageTeamBarrierRow {
            stage_team_plan_id: Uuid::new_v4(),
            dispatch_epoch: 0,
            requests_closed_at: None,
            required_work_items: 2,
            terminal_required_work_items: 2,
            live_workers: 0,
            retry_pending_work_items: 0,
            recovery_required_workers: 0,
            missing_outputs: 0,
            manifest_hash: "sha256:manifest".to_string(),
        };
        assert!(!base.ready_to_finalize());
        assert!(!StageTeamBarrierRow {
            requests_closed_at: Some(Utc::now()),
            live_workers: 1,
            ..base.clone()
        }
        .ready_to_finalize());
        assert!(StageTeamBarrierRow {
            requests_closed_at: Some(Utc::now()),
            ..base
        }
        .ready_to_finalize());
    }

    #[test]
    fn aggregator_mutable_state_is_not_part_of_closed_manifest() {
        let plan = StageTeamPlanRow {
            id: Uuid::new_v4(),
            operation_id: Uuid::new_v4(),
            stage_execution_id: Uuid::new_v4(),
            stage_run_unit_id: Uuid::new_v4(),
            scope_snapshot_id: Uuid::new_v4(),
            organization_id: Uuid::new_v4(),
            stage_kind: "target_intel".into(),
            unit_generation: 0,
            schema_version: 1,
            plan_version: 1,
            plan_hash: format!("sha256:{}", "a".repeat(64)),
            leader_role: "aggregator".into(),
            aggregator_kind: "worker".into(),
            aggregator_role: Some("aggregator".into()),
            allowed_worker_roles: serde_json::json!(["producer", "aggregator"]),
            max_workers_total: 4,
            max_workers_active: 2,
            dynamic_requests_allowed: false,
            dynamic_request_policy: serde_json::json!({}),
            dispatch_epoch: 0,
            requests_closed_at: Some(Utc::now()),
            final_submitter_kind: "worker".into(),
            final_submitter_worker_run_id: None,
            created_from_stage_spec_hash: format!("sha256:{}", "b".repeat(64)),
            row_version: 1,
            created_at: Utc::now(),
            updated_at: Utc::now(),
        };
        let now = Utc::now();
        let aggregator = StageWorkItemRow {
            id: Uuid::new_v4(),
            team_plan_id: plan.id,
            operation_id: plan.operation_id,
            stage_execution_id: plan.stage_execution_id,
            stage_run_unit_id: plan.stage_run_unit_id,
            scope_snapshot_id: plan.scope_snapshot_id,
            organization_id: plan.organization_id,
            dispatch_epoch: 0,
            kind: "aggregate".into(),
            stable_key: "aggregator:final".into(),
            role: "aggregator".into(),
            input_manifest_hash: format!("sha256:{}", "c".repeat(64)),
            input_refs: serde_json::json!([]),
            required_for_barrier: false,
            conflict_key: None,
            priority: i32::MAX,
            status: "queued".into(),
            attempt_policy: serde_json::json!({}),
            budget: serde_json::json!({}),
            output_schema: "stage_unit_aggregate.v1".into(),
            created_by: "server_seed".into(),
            row_version: 0,
            created_at: now,
            updated_at: now,
            started_at: None,
            terminal_at: None,
        };
        let before = barrier_manifest_hash(&plan, std::slice::from_ref(&aggregator), &[], &[]);
        let running = StageWorkItemRow {
            status: "running".into(),
            row_version: 1,
            started_at: Some(Utc::now()),
            updated_at: Utc::now(),
            ..aggregator
        };
        let after = barrier_manifest_hash(&plan, &[running], &[], &[]);
        assert_eq!(before, after);
    }
}
