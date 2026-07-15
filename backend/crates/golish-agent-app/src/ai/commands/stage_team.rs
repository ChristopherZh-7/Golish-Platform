//! Exact, DB-backed Stage Team Scheduler read model.
//!
//! The command accepts only the immutable `(operation_id, stage_execution_id)`
//! owner tuple.  It projects scheduler state from one repeatable-read snapshot
//! and deliberately omits lease tokens/owners, raw checkpoints, tool payloads,
//! budgets, and arbitrary canonical output JSON.

use std::collections::{BTreeMap, BTreeSet};

use chrono::{DateTime, Utc};
use golish_agent_kit::db_traits::{ResolveStageTeamRecovery, RuntimeMemoryRepository};
use golish_app_core::domain::operator::OperatorChannel;
use serde::{Deserialize, Serialize};
use serde_json::{json, Value};
use sha2::{Digest, Sha256};
use sqlx::{FromRow, PgConnection};
use tauri::State;
use ts_rs::TS;
use uuid::Uuid;

use crate::{ai::db_bridge::GolishDbRepoProvider, state::AgentState};

const INVALID_ID: &str = "STAGE_TEAM_INVALID_ID";
const SCOPE_MISMATCH: &str = "STAGE_TEAM_SCOPE_MISMATCH";
const DATABASE: &str = "STAGE_TEAM_DATABASE";
const RECOVERY_CONFLICT: &str = "STAGE_TEAM_RECOVERY_CONFLICT";

#[derive(Debug, Clone, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct StageTeamReadCommandError {
    pub code: String,
    pub message: String,
}

impl StageTeamReadCommandError {
    fn new(code: impl Into<String>, message: impl Into<String>) -> Self {
        Self {
            code: code.into(),
            message: message.into(),
        }
    }

    fn database(error: impl std::fmt::Display) -> Self {
        tracing::warn!(error = %error, "failed to read durable Stage Team state");
        Self::new(DATABASE, "durable Stage Team state could not be read")
    }
}

#[derive(Debug, Clone, Serialize, Deserialize, TS)]
#[serde(rename_all = "camelCase", deny_unknown_fields)]
#[ts(export, export_to = "../../../../frontend/lib/generated/")]
pub struct StageTeamReadRequest {
    pub operation_id: String,
    pub stage_execution_id: String,
}

/// Local-operator-only CAS for an expired Worker whose active external tool
/// outcome is unknown. Every identity/version comes from the sanitized read
/// model; the repository reloads and verifies the complete owner tuple.
#[derive(Debug, Clone, Serialize, Deserialize, TS)]
#[serde(rename_all = "camelCase", deny_unknown_fields)]
#[ts(export, export_to = "../../../../frontend/lib/generated/")]
pub struct StageTeamRecoveryResolveRequest {
    pub request_id: String,
    pub operation_id: String,
    pub stage_execution_id: String,
    pub stage_run_unit_id: String,
    pub scope_snapshot_id: String,
    pub stage_team_plan_id: String,
    pub work_item_id: String,
    pub worker_run_id: String,
    pub tool_call_record_id: String,
    #[ts(type = "number")]
    pub expected_work_item_row_version: i64,
    #[ts(type = "number")]
    pub expected_checkpoint_version: i64,
    #[ts(type = "number")]
    pub expected_attempt_epoch: i64,
}

#[derive(Debug, Clone, Serialize, TS)]
#[serde(rename_all = "camelCase")]
#[ts(export, export_to = "../../../../frontend/lib/generated/")]
pub struct StageTeamRecoveryResolveResponse {
    pub decision_id: String,
    pub decision_sha256: String,
    pub work_item_status: String,
    pub worker_status: String,
    pub output_id: String,
    pub blocker_code: Option<String>,
    pub replayed: bool,
}

#[derive(Debug, Clone, Serialize, TS)]
#[serde(rename_all = "camelCase")]
#[ts(export, export_to = "../../../../frontend/lib/generated/")]
pub struct StageTeamReadModel {
    pub operation_id: String,
    pub stage_execution_id: String,
    pub stage_kind: String,
    pub execution_status: String,
    pub started_at: String,
    pub completed_at: Option<String>,
    pub units: Vec<StageTeamUnitView>,
}

#[derive(Debug, Clone, Serialize, TS)]
#[serde(rename_all = "camelCase")]
#[ts(export, export_to = "../../../../frontend/lib/generated/")]
pub struct StageTeamUnitView {
    pub stage_run_unit_id: String,
    pub scope_snapshot_id: String,
    pub organization_id: String,
    pub organization_name: String,
    pub stage_kind: String,
    #[ts(type = "number")]
    pub generation: i32,
    pub specialist: Option<String>,
    pub status: String,
    pub gate: StageTeamGateView,
    pub plan: Option<StageTeamPlanReadView>,
    pub started_at: Option<String>,
    pub terminal_at: Option<String>,
}

#[derive(Debug, Clone, Serialize, TS)]
#[serde(rename_all = "camelCase")]
#[ts(export, export_to = "../../../../frontend/lib/generated/")]
pub struct StageTeamGateView {
    pub status: String,
    #[ts(type = "number")]
    pub attempt: i32,
    pub pass_watermark_present: bool,
    pub final_handoff_id: Option<String>,
    pub final_handoff_sha256: Option<String>,
    #[ts(type = "number")]
    pub final_handoff_evidence_count: usize,
    #[ts(type = "number | null")]
    pub evidence_watermark: Option<i64>,
    pub gate_passed_at: Option<String>,
}

#[derive(Debug, Clone, Serialize, TS)]
#[serde(rename_all = "camelCase")]
#[ts(export, export_to = "../../../../frontend/lib/generated/")]
pub struct StageTeamPlanReadView {
    pub stage_team_plan_id: String,
    #[ts(type = "number")]
    pub schema_version: i32,
    #[ts(type = "number")]
    pub plan_version: i32,
    pub plan_sha256: String,
    pub leader_role: String,
    pub aggregator_kind: String,
    pub aggregator_role: Option<String>,
    pub allowed_roles: Vec<String>,
    #[ts(type = "number")]
    pub max_workers_total: i32,
    #[ts(type = "number")]
    pub max_workers_active: i32,
    pub dynamic_requests_enabled: bool,
    #[ts(type = "number")]
    pub dispatch_epoch: i64,
    pub requests_closed_at: Option<String>,
    pub final_submitter_kind: String,
    pub final_submitter_worker_run_id: Option<String>,
    pub barrier: StageTeamBarrierReadView,
    pub work_items: Vec<StageTeamWorkItemView>,
    pub requests: Vec<StageTeamWorkerRequestView>,
}

#[derive(Debug, Clone, Serialize, TS)]
#[serde(rename_all = "camelCase")]
#[ts(export, export_to = "../../../../frontend/lib/generated/")]
pub struct StageTeamBarrierReadView {
    #[ts(type = "number")]
    pub dispatch_epoch: i64,
    pub requests_closed: bool,
    #[ts(type = "number")]
    pub required_work_items: i64,
    #[ts(type = "number")]
    pub terminal_required_work_items: i64,
    #[ts(type = "number")]
    pub live_workers: i64,
    #[ts(type = "number")]
    pub retry_pending_work_items: i64,
    #[ts(type = "number")]
    pub recovery_required_workers: i64,
    #[ts(type = "number")]
    pub missing_outputs: i64,
    pub manifest_sha256: String,
    pub ready_to_finalize: bool,
}

#[derive(Debug, Clone, Serialize, TS)]
#[serde(rename_all = "camelCase")]
#[ts(export, export_to = "../../../../frontend/lib/generated/")]
pub struct StageTeamWorkItemView {
    pub work_item_id: String,
    pub kind: String,
    pub stable_key: String,
    pub role: String,
    pub input_manifest_sha256: String,
    #[ts(type = "number")]
    pub subject_ref_count: usize,
    pub required_for_barrier: bool,
    pub is_aggregator: bool,
    pub conflict_key: Option<String>,
    #[ts(type = "number")]
    pub priority: i32,
    pub status: String,
    #[ts(type = "number | null")]
    pub max_attempts: Option<i64>,
    pub output_schema: String,
    pub created_by: String,
    #[ts(type = "number")]
    pub row_version: i64,
    pub dependency_work_item_ids: Vec<String>,
    pub workers: Vec<StageTeamWorkerView>,
    pub output: Option<StageTeamWorkerOutputView>,
    pub started_at: Option<String>,
    pub terminal_at: Option<String>,
}

#[derive(Debug, Clone, Serialize, TS)]
#[serde(rename_all = "camelCase")]
#[ts(export, export_to = "../../../../frontend/lib/generated/")]
pub struct StageTeamWorkerView {
    pub worker_run_id: String,
    #[ts(type = "number")]
    pub generation: i32,
    pub specialist: String,
    pub agent_path: String,
    pub message_chain_id: Option<String>,
    pub status: String,
    #[ts(type = "number")]
    pub gate_attempt: i32,
    #[ts(type = "number")]
    pub attempt_epoch: i64,
    #[ts(type = "number")]
    pub checkpoint_version: i64,
    pub has_active_tool: bool,
    pub active_tool_call_id: Option<String>,
    pub lease_state: String,
    pub recovery_state: String,
    #[ts(type = "number | null")]
    pub evidence_watermark: Option<i64>,
    pub started_at: Option<String>,
    pub updated_at: String,
    pub terminal_at: Option<String>,
}

#[derive(Debug, Clone, Serialize, TS)]
#[serde(rename_all = "camelCase")]
#[ts(export, export_to = "../../../../frontend/lib/generated/")]
pub struct StageTeamWorkerOutputView {
    pub output_id: String,
    pub worker_run_id: String,
    pub output_schema: String,
    #[ts(type = "number")]
    pub output_version: i32,
    pub business_disposition: String,
    #[ts(type = "number")]
    pub canonical_fact_ref_count: usize,
    #[ts(type = "number[]")]
    pub evidence_ids: Vec<i64>,
    #[ts(type = "number")]
    pub checked_empty_cell_count: usize,
    pub blocker_codes: Vec<String>,
    pub output_sha256: String,
    pub created_at: String,
}

#[derive(Debug, Clone, Serialize, TS)]
#[serde(rename_all = "camelCase")]
#[ts(export, export_to = "../../../../frontend/lib/generated/")]
pub struct StageTeamWorkerRequestView {
    pub request_id: String,
    pub parent_work_item_id: String,
    pub parent_worker_run_id: String,
    #[ts(type = "number")]
    pub dispatch_epoch: i64,
    pub requested_role: String,
    pub request_kind: String,
    #[ts(type = "number")]
    pub subject_ref_count: usize,
    pub reason_code: String,
    pub expected_output_schema: String,
    pub dedupe_key: String,
    pub request_sha256: String,
    pub status: String,
    pub decision_reason_code: Option<String>,
    pub accepted_work_item_id: Option<String>,
    pub created_at: String,
}

#[derive(Debug, Clone, FromRow)]
struct ExecutionRow {
    stage_kind: String,
    status: String,
    started_at: DateTime<Utc>,
    completed_at: Option<DateTime<Utc>>,
}

#[derive(Debug, Clone, FromRow)]
struct UnitRow {
    id: Uuid,
    scope_snapshot_id: Uuid,
    organization_id: Uuid,
    organization_name: String,
    stage_kind: String,
    generation: i32,
    specialist: Option<String>,
    status: String,
    gate_attempt: i32,
    pass_watermark: Value,
    started_at: Option<DateTime<Utc>>,
    terminal_at: Option<DateTime<Utc>>,
    handoff_id: Option<Uuid>,
    handoff_sha256: Option<String>,
    handoff_evidence_ids: Option<Vec<i64>>,
    gate_passed_at: Option<DateTime<Utc>>,
}

#[derive(Debug, Clone, FromRow)]
struct PlanRow {
    id: Uuid,
    stage_run_unit_id: Uuid,
    schema_version: i32,
    plan_version: i32,
    plan_hash: String,
    leader_role: String,
    aggregator_kind: String,
    aggregator_role: Option<String>,
    allowed_worker_roles: Value,
    max_workers_total: i32,
    max_workers_active: i32,
    dynamic_requests_allowed: bool,
    dispatch_epoch: i64,
    requests_closed_at: Option<DateTime<Utc>>,
    final_submitter_kind: String,
    final_submitter_worker_run_id: Option<Uuid>,
}

#[derive(Debug, Clone, FromRow)]
struct WorkItemRow {
    id: Uuid,
    team_plan_id: Uuid,
    kind: String,
    stable_key: String,
    role: String,
    input_manifest_hash: String,
    subject_ref_count: i64,
    required_for_barrier: bool,
    conflict_key: Option<String>,
    priority: i32,
    status: String,
    attempt_policy: Value,
    output_schema: String,
    created_by: String,
    row_version: i64,
    started_at: Option<DateTime<Utc>>,
    terminal_at: Option<DateTime<Utc>>,
}

#[derive(Debug, Clone, FromRow)]
struct DependencyRow {
    work_item_id: Uuid,
    depends_on_work_item_id: Uuid,
}

#[derive(Debug, Clone, FromRow)]
struct WorkerRow {
    id: Uuid,
    work_item_id: Uuid,
    worker_generation: i32,
    specialist: String,
    agent_path: String,
    message_chain_id: Option<Uuid>,
    status: String,
    gate_attempt: i32,
    attempt_epoch: i64,
    checkpoint_version: i64,
    lease_present: bool,
    lease_expired: bool,
    has_active_tool: bool,
    active_tool_call_id: Option<Uuid>,
    evidence_watermark: Option<i64>,
    started_at: Option<DateTime<Utc>>,
    updated_at: DateTime<Utc>,
    terminal_at: Option<DateTime<Utc>>,
}

#[derive(Debug, Clone, FromRow)]
struct OutputRow {
    id: Uuid,
    team_plan_id: Uuid,
    work_item_id: Uuid,
    worker_run_id: Uuid,
    output_schema: String,
    output_version: i32,
    business_disposition: String,
    canonical_fact_ref_count: i64,
    evidence_ids: Vec<i64>,
    checked_empty_cell_count: i64,
    blocker_codes: Vec<String>,
    output_hash: String,
    created_at: DateTime<Utc>,
}

#[derive(Debug, Clone, FromRow)]
struct RequestRow {
    id: Uuid,
    team_plan_id: Uuid,
    parent_work_item_id: Uuid,
    parent_worker_run_id: Uuid,
    dispatch_epoch: i64,
    requested_role: String,
    request_kind: String,
    subject_ref_count: i64,
    reason_code: String,
    expected_output_schema: String,
    dedupe_key: String,
    request_payload_hash: String,
    status: String,
    decision_reason_code: Option<String>,
    accepted_work_item_id: Option<Uuid>,
    created_at: DateTime<Utc>,
}

const EXECUTION_SQL: &str = r#"SELECT stage_kind,status,started_at,completed_at
  FROM stage_runs
 WHERE operation_id=$1 AND id=$2"#;

const UNITS_SQL: &str = r#"SELECT unit.id,unit.scope_snapshot_id,unit.organization_id,
       COALESCE(scope.organization_name_at_freeze,unit.organization_id::text) AS organization_name,
       unit.stage_kind,unit.generation,unit.specialist,unit.status,unit.gate_attempt,
       unit.pass_watermark,unit.started_at,unit.terminal_at,
       handoff.id AS handoff_id,handoff.payload_sha256 AS handoff_sha256,
       handoff.evidence_ids AS handoff_evidence_ids,handoff.gate_passed_at
  FROM stage_run_units AS unit
  LEFT JOIN operation_org_scope_units AS scope
    ON scope.snapshot_id=unit.scope_snapshot_id
   AND scope.organization_id=unit.organization_id
  LEFT JOIN stage_handoffs AS handoff
    ON handoff.source_stage_run_unit_id=unit.id
   AND handoff.operation_id=unit.operation_id
   AND handoff.stage_execution_id=unit.stage_execution_id
   AND handoff.organization_id=unit.organization_id
   AND handoff.invalidated_at IS NULL
 WHERE unit.operation_id=$1 AND unit.stage_execution_id=$2
 ORDER BY scope.ordinal NULLS LAST,unit.organization_id,unit.id"#;

const PLANS_SQL: &str = r#"SELECT id,stage_run_unit_id,schema_version,plan_version,plan_hash,
       leader_role,aggregator_kind,aggregator_role,allowed_worker_roles,
       max_workers_total,max_workers_active,dynamic_requests_allowed,dispatch_epoch,
       requests_closed_at,final_submitter_kind,final_submitter_worker_run_id
  FROM stage_team_plans
 WHERE operation_id=$1 AND stage_execution_id=$2
 ORDER BY organization_id,id"#;

const WORK_ITEMS_SQL: &str = r#"SELECT id,team_plan_id,kind,stable_key,role,input_manifest_hash,
       jsonb_array_length(input_refs)::bigint AS subject_ref_count,
       required_for_barrier,conflict_key,priority,status,attempt_policy,
       output_schema,created_by,row_version,started_at,terminal_at
  FROM stage_work_items
 WHERE operation_id=$1 AND stage_execution_id=$2
 ORDER BY team_plan_id,priority,id"#;

const DEPENDENCIES_SQL: &str = r#"SELECT work_item_id,depends_on_work_item_id
  FROM stage_work_item_dependencies
 WHERE operation_id=$1 AND stage_execution_id=$2
 ORDER BY work_item_id,depends_on_work_item_id"#;

// Security boundary: this SELECT returns only derived lease booleans plus the
// non-secret CAS versions and active tool record id needed by the local
// recovery command. Lease token/owner/expiry and checkpoint body never cross
// the command boundary.
const WORKERS_SQL: &str = r#"SELECT id,work_item_id,worker_generation,specialist,agent_path,
       message_chain_id,status,gate_attempt,attempt_epoch,checkpoint_version,
       lease_token IS NOT NULL AS lease_present,
       lease_expires_at IS NOT NULL AND lease_expires_at <= NOW() AS lease_expired,
       active_tool_call_id IS NOT NULL AS has_active_tool,active_tool_call_id,
       evidence_watermark,started_at,updated_at,terminal_at
  FROM stage_worker_runs
 WHERE operation_id=$1 AND stage_execution_id=$2 AND work_item_id IS NOT NULL
 ORDER BY work_item_id,worker_generation,id"#;

// `canonical_output` is intentionally absent: the UI needs the immutable
// business disposition and evidence/fact watermarks, not arbitrary prose or a
// possible secret accidentally embedded in an output object.
const OUTPUTS_SQL: &str = r#"SELECT id,team_plan_id,work_item_id,worker_run_id,output_schema,
       output_version,business_disposition,
       jsonb_array_length(canonical_fact_refs)::bigint AS canonical_fact_ref_count,
       evidence_ids,jsonb_array_length(checked_empty_cells)::bigint AS checked_empty_cell_count,
       blocker_codes,output_hash,created_at
  FROM stage_worker_outputs
 WHERE operation_id=$1 AND stage_execution_id=$2
 ORDER BY team_plan_id,created_at,id"#;

// Dynamic request budgets and bounded subject bodies stay server-side.  The
// read model exposes counts plus the immutable decision/hash only.
const REQUESTS_SQL: &str = r#"SELECT id,team_plan_id,parent_work_item_id,parent_worker_run_id,
       dispatch_epoch,requested_role,request_kind,
       jsonb_array_length(bounded_subject_refs)::bigint AS subject_ref_count,reason_code,
       expected_output_schema,dedupe_key,request_payload_hash,status,
       decision_reason_code,accepted_work_item_id,created_at
  FROM stage_worker_requests
 WHERE operation_id=$1 AND stage_execution_id=$2
 ORDER BY team_plan_id,created_at,id"#;

fn parse_uuid(value: &str, field: &str) -> Result<Uuid, StageTeamReadCommandError> {
    Uuid::parse_str(value)
        .map_err(|_| StageTeamReadCommandError::new(INVALID_ID, format!("invalid {field}")))
}

fn canonical_json(value: &Value) -> String {
    match value {
        Value::Null => "null".to_string(),
        Value::Bool(value) => value.to_string(),
        Value::Number(value) => value.to_string(),
        Value::String(value) => serde_json::to_string(value).expect("serialize JSON string"),
        Value::Array(values) => format!(
            "[{}]",
            values
                .iter()
                .map(canonical_json)
                .collect::<Vec<_>>()
                .join(",")
        ),
        Value::Object(object) => {
            let mut keys = object.keys().collect::<Vec<_>>();
            keys.sort_unstable();
            format!(
                "{{{}}}",
                keys.into_iter()
                    .map(|key| format!(
                        "{}:{}",
                        serde_json::to_string(key).expect("serialize JSON key"),
                        canonical_json(&object[key])
                    ))
                    .collect::<Vec<_>>()
                    .join(",")
            )
        }
    }
}

fn sha256_json(value: &Value) -> String {
    let digest = Sha256::digest(canonical_json(value).as_bytes());
    format!(
        "sha256:{}",
        digest
            .iter()
            .map(|byte| format!("{byte:02x}"))
            .collect::<String>()
    )
}

fn count(value: i64) -> usize {
    usize::try_from(value).unwrap_or(0)
}

fn json_string_array(value: &Value) -> Vec<String> {
    value
        .as_array()
        .into_iter()
        .flatten()
        .filter_map(Value::as_str)
        .map(ToOwned::to_owned)
        .collect()
}

fn pass_watermark_present(value: &Value) -> bool {
    !matches!(value, Value::Null)
        && !value.as_object().is_some_and(serde_json::Map::is_empty)
        && !value.as_array().is_some_and(Vec::is_empty)
}

fn is_aggregator(plan: &PlanRow, item: &WorkItemRow) -> bool {
    plan.aggregator_kind == "worker"
        && plan.aggregator_role.as_deref() == Some(item.role.as_str())
        && !item.required_for_barrier
}

fn recovery_state(worker: &WorkerRow) -> &'static str {
    if worker.status == "recovery_required" || (worker.lease_expired && worker.has_active_tool) {
        "manual_required"
    } else if worker.lease_expired {
        "requeue_eligible"
    } else if worker.lease_present {
        "wait_for_live_lease"
    } else if matches!(
        worker.status.as_str(),
        "passed" | "failed" | "exhausted" | "superseded"
    ) {
        "terminal"
    } else {
        "unleased"
    }
}

fn lease_state(worker: &WorkerRow) -> &'static str {
    if !worker.lease_present {
        "none"
    } else if worker.lease_expired {
        "expired"
    } else {
        "live"
    }
}

fn manifest_sha256(
    plan: &PlanRow,
    items: &[WorkItemRow],
    outputs: &[OutputRow],
    requests: &[RequestRow],
) -> String {
    let output_hashes = outputs
        .iter()
        .map(|output| (output.work_item_id.to_string(), output.output_hash.clone()))
        .collect::<BTreeMap<_, _>>();
    let producer_items = items
        .iter()
        .filter(|item| !is_aggregator(plan, item))
        .map(|item| {
            json!({
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
    let aggregator_items = items
        .iter()
        .filter(|item| is_aggregator(plan, item))
        .map(|item| {
            json!({
                "id": item.id,
                "input_manifest_hash": item.input_manifest_hash,
                "kind": item.kind,
                "role": item.role,
                "stable_key": item.stable_key,
            })
        })
        .collect::<Vec<_>>();
    let material = json!({
        "dispatch_epoch": plan.dispatch_epoch,
        "aggregator_items": aggregator_items,
        "plan_hash": plan.plan_hash,
        "producer_items": producer_items,
        "requests": requests.iter().map(|request| json!({
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
    sha256_json(&material)
}

async fn fetch_all<T>(
    connection: &mut PgConnection,
    sql: &str,
    operation_id: Uuid,
    stage_execution_id: Uuid,
) -> Result<Vec<T>, StageTeamReadCommandError>
where
    T: for<'row> FromRow<'row, sqlx::postgres::PgRow> + Send + Unpin,
{
    sqlx::query_as::<_, T>(sql)
        .bind(operation_id)
        .bind(stage_execution_id)
        .fetch_all(connection)
        .await
        .map_err(StageTeamReadCommandError::database)
}

async fn load_stage_team_read_model(
    connection: &mut PgConnection,
    operation_id: Uuid,
    stage_execution_id: Uuid,
) -> Result<StageTeamReadModel, StageTeamReadCommandError> {
    let execution = sqlx::query_as::<_, ExecutionRow>(EXECUTION_SQL)
        .bind(operation_id)
        .bind(stage_execution_id)
        .fetch_optional(&mut *connection)
        .await
        .map_err(StageTeamReadCommandError::database)?
        .ok_or_else(|| {
            StageTeamReadCommandError::new(
                SCOPE_MISMATCH,
                "stage execution does not belong to the requested operation",
            )
        })?;
    let units = fetch_all::<UnitRow>(
        &mut *connection,
        UNITS_SQL,
        operation_id,
        stage_execution_id,
    )
    .await?;
    let plans = fetch_all::<PlanRow>(
        &mut *connection,
        PLANS_SQL,
        operation_id,
        stage_execution_id,
    )
    .await?;
    let items = fetch_all::<WorkItemRow>(
        &mut *connection,
        WORK_ITEMS_SQL,
        operation_id,
        stage_execution_id,
    )
    .await?;
    let dependencies = fetch_all::<DependencyRow>(
        &mut *connection,
        DEPENDENCIES_SQL,
        operation_id,
        stage_execution_id,
    )
    .await?;
    let workers = fetch_all::<WorkerRow>(
        &mut *connection,
        WORKERS_SQL,
        operation_id,
        stage_execution_id,
    )
    .await?;
    let outputs = fetch_all::<OutputRow>(
        &mut *connection,
        OUTPUTS_SQL,
        operation_id,
        stage_execution_id,
    )
    .await?;
    let requests = fetch_all::<RequestRow>(
        &mut *connection,
        REQUESTS_SQL,
        operation_id,
        stage_execution_id,
    )
    .await?;

    let mut plans_by_unit = plans
        .iter()
        .map(|plan| (plan.stage_run_unit_id, plan))
        .collect::<BTreeMap<_, _>>();
    let mut dependencies_by_item = BTreeMap::<Uuid, Vec<Uuid>>::new();
    for dependency in dependencies {
        dependencies_by_item
            .entry(dependency.work_item_id)
            .or_default()
            .push(dependency.depends_on_work_item_id);
    }
    let mut workers_by_item = BTreeMap::<Uuid, Vec<&WorkerRow>>::new();
    for worker in &workers {
        workers_by_item
            .entry(worker.work_item_id)
            .or_default()
            .push(worker);
    }
    let outputs_by_item = outputs
        .iter()
        .map(|output| (output.work_item_id, output))
        .collect::<BTreeMap<_, _>>();

    let unit_views = units
        .into_iter()
        .map(|unit| {
            let plan = plans_by_unit.remove(&unit.id).map(|plan| {
                let plan_items = items
                    .iter()
                    .filter(|item| item.team_plan_id == plan.id)
                    .cloned()
                    .collect::<Vec<_>>();
                let plan_outputs = outputs
                    .iter()
                    .filter(|output| output.team_plan_id == plan.id)
                    .cloned()
                    .collect::<Vec<_>>();
                let plan_requests = requests
                    .iter()
                    .filter(|request| request.team_plan_id == plan.id)
                    .cloned()
                    .collect::<Vec<_>>();
                let producer_ids = plan_items
                    .iter()
                    .filter(|item| !is_aggregator(plan, item))
                    .map(|item| item.id)
                    .collect::<BTreeSet<_>>();
                let terminal_required = plan_items
                    .iter()
                    .filter(|item| {
                        producer_ids.contains(&item.id)
                            && matches!(item.status.as_str(), "completed" | "exhausted")
                    })
                    .count() as i64;
                let retry_pending = plan_items
                    .iter()
                    .filter(|item| {
                        producer_ids.contains(&item.id) && item.status == "retry_pending"
                    })
                    .count() as i64;
                let producer_workers = workers
                    .iter()
                    .filter(|worker| producer_ids.contains(&worker.work_item_id))
                    .collect::<Vec<_>>();
                let live_workers = producer_workers
                    .iter()
                    .filter(|worker| {
                        matches!(
                            worker.status.as_str(),
                            "queued" | "running" | "waiting_background"
                        )
                    })
                    .count() as i64;
                let recovery_required_workers = producer_workers
                    .iter()
                    .filter(|worker| worker.status == "recovery_required")
                    .count() as i64;
                let output_ids = plan_outputs
                    .iter()
                    .map(|output| output.work_item_id)
                    .collect::<BTreeSet<_>>();
                let missing_outputs = plan_items
                    .iter()
                    .filter(|item| {
                        producer_ids.contains(&item.id)
                            && matches!(item.status.as_str(), "completed" | "exhausted")
                            && !output_ids.contains(&item.id)
                    })
                    .count() as i64;
                let required = producer_ids.len() as i64;
                let requests_closed = plan.requests_closed_at.is_some();
                let ready_to_finalize = requests_closed
                    && required == terminal_required
                    && live_workers == 0
                    && retry_pending == 0
                    && recovery_required_workers == 0
                    && missing_outputs == 0;
                let barrier = StageTeamBarrierReadView {
                    dispatch_epoch: plan.dispatch_epoch,
                    requests_closed,
                    required_work_items: required,
                    terminal_required_work_items: terminal_required,
                    live_workers,
                    retry_pending_work_items: retry_pending,
                    recovery_required_workers,
                    missing_outputs,
                    manifest_sha256: manifest_sha256(
                        plan,
                        &plan_items,
                        &plan_outputs,
                        &plan_requests,
                    ),
                    ready_to_finalize,
                };
                let work_items = plan_items
                    .iter()
                    .map(|item| {
                        let item_workers = workers_by_item
                            .get(&item.id)
                            .into_iter()
                            .flatten()
                            .map(|worker| StageTeamWorkerView {
                                worker_run_id: worker.id.to_string(),
                                generation: worker.worker_generation,
                                specialist: worker.specialist.clone(),
                                agent_path: worker.agent_path.clone(),
                                message_chain_id: worker.message_chain_id.map(|id| id.to_string()),
                                status: worker.status.clone(),
                                gate_attempt: worker.gate_attempt,
                                attempt_epoch: worker.attempt_epoch,
                                checkpoint_version: worker.checkpoint_version,
                                has_active_tool: worker.has_active_tool,
                                active_tool_call_id: worker
                                    .active_tool_call_id
                                    .map(|id| id.to_string()),
                                lease_state: lease_state(worker).to_string(),
                                recovery_state: recovery_state(worker).to_string(),
                                evidence_watermark: worker.evidence_watermark,
                                started_at: worker.started_at.map(|value| value.to_rfc3339()),
                                updated_at: worker.updated_at.to_rfc3339(),
                                terminal_at: worker.terminal_at.map(|value| value.to_rfc3339()),
                            })
                            .collect();
                        let output =
                            outputs_by_item
                                .get(&item.id)
                                .map(|output| StageTeamWorkerOutputView {
                                    output_id: output.id.to_string(),
                                    worker_run_id: output.worker_run_id.to_string(),
                                    output_schema: output.output_schema.clone(),
                                    output_version: output.output_version,
                                    business_disposition: output.business_disposition.clone(),
                                    canonical_fact_ref_count: count(
                                        output.canonical_fact_ref_count,
                                    ),
                                    evidence_ids: output.evidence_ids.clone(),
                                    checked_empty_cell_count: count(
                                        output.checked_empty_cell_count,
                                    ),
                                    blocker_codes: output.blocker_codes.clone(),
                                    output_sha256: output.output_hash.clone(),
                                    created_at: output.created_at.to_rfc3339(),
                                });
                        StageTeamWorkItemView {
                            work_item_id: item.id.to_string(),
                            kind: item.kind.clone(),
                            stable_key: item.stable_key.clone(),
                            role: item.role.clone(),
                            input_manifest_sha256: item.input_manifest_hash.clone(),
                            subject_ref_count: count(item.subject_ref_count),
                            required_for_barrier: item.required_for_barrier,
                            is_aggregator: is_aggregator(plan, item),
                            conflict_key: item.conflict_key.clone(),
                            priority: item.priority,
                            status: item.status.clone(),
                            max_attempts: item
                                .attempt_policy
                                .get("max_attempts")
                                .and_then(Value::as_i64),
                            output_schema: item.output_schema.clone(),
                            created_by: item.created_by.clone(),
                            row_version: item.row_version,
                            dependency_work_item_ids: dependencies_by_item
                                .get(&item.id)
                                .into_iter()
                                .flatten()
                                .map(ToString::to_string)
                                .collect(),
                            workers: item_workers,
                            output,
                            started_at: item.started_at.map(|value| value.to_rfc3339()),
                            terminal_at: item.terminal_at.map(|value| value.to_rfc3339()),
                        }
                    })
                    .collect();
                let request_views = plan_requests
                    .into_iter()
                    .map(|request| StageTeamWorkerRequestView {
                        request_id: request.id.to_string(),
                        parent_work_item_id: request.parent_work_item_id.to_string(),
                        parent_worker_run_id: request.parent_worker_run_id.to_string(),
                        dispatch_epoch: request.dispatch_epoch,
                        requested_role: request.requested_role,
                        request_kind: request.request_kind,
                        subject_ref_count: count(request.subject_ref_count),
                        reason_code: request.reason_code,
                        expected_output_schema: request.expected_output_schema,
                        dedupe_key: request.dedupe_key,
                        request_sha256: request.request_payload_hash,
                        status: request.status,
                        decision_reason_code: request.decision_reason_code,
                        accepted_work_item_id: request
                            .accepted_work_item_id
                            .map(|id| id.to_string()),
                        created_at: request.created_at.to_rfc3339(),
                    })
                    .collect();
                StageTeamPlanReadView {
                    stage_team_plan_id: plan.id.to_string(),
                    schema_version: plan.schema_version,
                    plan_version: plan.plan_version,
                    plan_sha256: plan.plan_hash.clone(),
                    leader_role: plan.leader_role.clone(),
                    aggregator_kind: plan.aggregator_kind.clone(),
                    aggregator_role: plan.aggregator_role.clone(),
                    allowed_roles: json_string_array(&plan.allowed_worker_roles),
                    max_workers_total: plan.max_workers_total,
                    max_workers_active: plan.max_workers_active,
                    dynamic_requests_enabled: plan.dynamic_requests_allowed,
                    dispatch_epoch: plan.dispatch_epoch,
                    requests_closed_at: plan.requests_closed_at.map(|value| value.to_rfc3339()),
                    final_submitter_kind: plan.final_submitter_kind.clone(),
                    final_submitter_worker_run_id: plan
                        .final_submitter_worker_run_id
                        .map(|id| id.to_string()),
                    barrier,
                    work_items,
                    requests: request_views,
                }
            });
            let handoff_evidence_ids = unit.handoff_evidence_ids.unwrap_or_default();
            StageTeamUnitView {
                stage_run_unit_id: unit.id.to_string(),
                scope_snapshot_id: unit.scope_snapshot_id.to_string(),
                organization_id: unit.organization_id.to_string(),
                organization_name: unit.organization_name,
                stage_kind: unit.stage_kind,
                generation: unit.generation,
                specialist: unit.specialist,
                status: unit.status.clone(),
                gate: StageTeamGateView {
                    status: unit.status,
                    attempt: unit.gate_attempt,
                    pass_watermark_present: pass_watermark_present(&unit.pass_watermark),
                    final_handoff_id: unit.handoff_id.map(|id| id.to_string()),
                    final_handoff_sha256: unit.handoff_sha256,
                    final_handoff_evidence_count: handoff_evidence_ids.len(),
                    evidence_watermark: handoff_evidence_ids.into_iter().max(),
                    gate_passed_at: unit.gate_passed_at.map(|value| value.to_rfc3339()),
                },
                plan,
                started_at: unit.started_at.map(|value| value.to_rfc3339()),
                terminal_at: unit.terminal_at.map(|value| value.to_rfc3339()),
            }
        })
        .collect();
    Ok(StageTeamReadModel {
        operation_id: operation_id.to_string(),
        stage_execution_id: stage_execution_id.to_string(),
        stage_kind: execution.stage_kind,
        execution_status: execution.status,
        started_at: execution.started_at.to_rfc3339(),
        completed_at: execution.completed_at.map(|value| value.to_rfc3339()),
        units: unit_views,
    })
}

#[tauri::command]
pub async fn ai_get_stage_team_read_model(
    request: StageTeamReadRequest,
    state: State<'_, AgentState>,
) -> Result<StageTeamReadModel, StageTeamReadCommandError> {
    let principal = state
        .operator_principal_provider
        .current(OperatorChannel::LocalDesktop)
        .await
        .map_err(|_| {
            StageTeamReadCommandError::new(SCOPE_MISMATCH, "Stage Team scope is not authorized")
        })?;
    if principal.channel() != OperatorChannel::LocalDesktop {
        return Err(StageTeamReadCommandError::new(
            SCOPE_MISMATCH,
            "local operator principal is required",
        ));
    }
    let operation_id = parse_uuid(&request.operation_id, "operationId")?;
    let stage_execution_id = parse_uuid(&request.stage_execution_id, "stageExecutionId")?;
    let mut tx = state
        .db_pool
        .begin()
        .await
        .map_err(StageTeamReadCommandError::database)?;
    sqlx::query("SET TRANSACTION ISOLATION LEVEL REPEATABLE READ READ ONLY")
        .execute(&mut *tx)
        .await
        .map_err(StageTeamReadCommandError::database)?;
    let model = load_stage_team_read_model(&mut tx, operation_id, stage_execution_id).await?;
    tx.commit()
        .await
        .map_err(StageTeamReadCommandError::database)?;
    Ok(model)
}

#[tauri::command]
pub async fn ai_resolve_stage_team_recovery(
    request: StageTeamRecoveryResolveRequest,
    state: State<'_, AgentState>,
) -> Result<StageTeamRecoveryResolveResponse, StageTeamReadCommandError> {
    let principal = state
        .operator_principal_provider
        .current(OperatorChannel::LocalDesktop)
        .await
        .map_err(|_| {
            StageTeamReadCommandError::new(
                SCOPE_MISMATCH,
                "Stage Team recovery scope is not authorized",
            )
        })?;
    if principal.channel() != OperatorChannel::LocalDesktop {
        return Err(StageTeamReadCommandError::new(
            SCOPE_MISMATCH,
            "local operator principal is required",
        ));
    }
    if request.request_id.is_empty()
        || request.request_id != request.request_id.trim()
        || request.request_id.len() > 256
        || request.request_id.chars().any(char::is_control)
        || request.expected_work_item_row_version < 0
        || request.expected_checkpoint_version < 0
        || request.expected_attempt_epoch < 0
    {
        return Err(StageTeamReadCommandError::new(
            RECOVERY_CONFLICT,
            "invalid Stage Team recovery CAS request",
        ));
    }

    let provider = GolishDbRepoProvider::new(state.db_pool.clone());
    let resolved = provider
        .resolve_stage_team_recovery(ResolveStageTeamRecovery {
            request_id: request.request_id,
            operation_id: parse_uuid(&request.operation_id, "operationId")?,
            stage_execution_id: parse_uuid(&request.stage_execution_id, "stageExecutionId")?,
            stage_run_unit_id: parse_uuid(&request.stage_run_unit_id, "stageRunUnitId")?,
            scope_snapshot_id: parse_uuid(&request.scope_snapshot_id, "scopeSnapshotId")?,
            stage_team_plan_id: parse_uuid(&request.stage_team_plan_id, "stageTeamPlanId")?,
            work_item_id: parse_uuid(&request.work_item_id, "workItemId")?,
            worker_run_id: parse_uuid(&request.worker_run_id, "workerRunId")?,
            tool_call_record_id: parse_uuid(&request.tool_call_record_id, "toolCallRecordId")?,
            expected_work_item_row_version: request.expected_work_item_row_version,
            expected_checkpoint_version: request.expected_checkpoint_version,
            expected_attempt_epoch: request.expected_attempt_epoch,
            resolved_by: principal.id().as_uuid().to_string(),
        })
        .await
        .map_err(|error| {
            tracing::warn!(error = %error, "Stage Team recovery CAS was rejected");
            StageTeamReadCommandError::new(
                RECOVERY_CONFLICT,
                "Stage Team recovery state changed or does not match this exact owner",
            )
        })?;
    Ok(StageTeamRecoveryResolveResponse {
        decision_id: resolved.decision_id.to_string(),
        decision_sha256: resolved.decision_sha256,
        work_item_status: resolved.work_item.status.as_str().to_string(),
        worker_status: resolved.worker.status.as_str().to_string(),
        output_id: resolved.output.id.to_string(),
        blocker_code: resolved.output.blocker_code,
        replayed: resolved.replayed,
    })
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn read_projection_never_selects_lease_checkpoint_or_raw_output_secrets() {
        assert!(WORKERS_SQL.contains("lease_token IS NOT NULL AS lease_present"));
        assert!(!WORKERS_SQL.contains("lease_owner"));
        assert!(WORKERS_SQL.contains("checkpoint_version"));
        assert!(!WORKERS_SQL.contains("checkpoint,"));
        assert!(!WORKERS_SQL.contains("active_tool_started_at"));
        assert!(!OUTPUTS_SQL.contains("canonical_output"));
        assert!(!REQUESTS_SQL.contains("budget_hint"));
        assert!(OUTPUTS_SQL.contains("jsonb_array_length(canonical_fact_refs)"));
        assert!(REQUESTS_SQL.contains("jsonb_array_length(bounded_subject_refs)"));
    }

    #[test]
    fn every_scheduler_query_is_bound_to_the_exact_operation_execution_tuple() {
        for sql in [
            EXECUTION_SQL,
            UNITS_SQL,
            PLANS_SQL,
            WORK_ITEMS_SQL,
            DEPENDENCIES_SQL,
            WORKERS_SQL,
            OUTPUTS_SQL,
            REQUESTS_SQL,
        ] {
            assert!(sql.contains("operation_id=$1"), "{sql}");
            assert!(
                sql.contains("stage_execution_id=$2") || sql.contains("id=$2"),
                "{sql}"
            );
        }
    }

    #[test]
    fn barrier_manifest_matches_the_scheduler_shape() {
        let plan = PlanRow {
            id: Uuid::from_u128(1),
            stage_run_unit_id: Uuid::from_u128(2),
            schema_version: 1,
            plan_version: 1,
            plan_hash: format!("sha256:{}", "a".repeat(64)),
            leader_role: "producer".into(),
            aggregator_kind: "worker".into(),
            aggregator_role: Some("aggregator".into()),
            allowed_worker_roles: json!(["producer", "aggregator"]),
            max_workers_total: 2,
            max_workers_active: 1,
            dynamic_requests_allowed: false,
            dispatch_epoch: 0,
            requests_closed_at: None,
            final_submitter_kind: "worker".into(),
            final_submitter_worker_run_id: None,
        };
        let item = WorkItemRow {
            id: Uuid::from_u128(3),
            team_plan_id: plan.id,
            kind: "provider".into(),
            stable_key: "provider:a".into(),
            role: "producer".into(),
            input_manifest_hash: format!("sha256:{}", "b".repeat(64)),
            subject_ref_count: 0,
            required_for_barrier: true,
            conflict_key: None,
            priority: 0,
            status: "queued".into(),
            attempt_policy: json!({}),
            output_schema: "stage_worker_output.v1".into(),
            created_by: "server_seed".into(),
            row_version: 0,
            started_at: None,
            terminal_at: None,
        };
        let hash = manifest_sha256(&plan, &[item], &[], &[]);
        assert!(hash.starts_with("sha256:"));
        assert_eq!(hash.len(), 71);
    }
}
