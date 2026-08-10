//! Compound Postgres lifecycle for one bounded Investigation nested worker.

use serde_json::{json, Value};
use sqlx::{PgPool, Postgres, Transaction};
use uuid::Uuid;

use crate::models::AgentType;

use super::runtime_memory_tx::{
    stage_worker_request_payload_hash, RequestStageWorkerRow, RuntimeMemoryStoreError,
    RuntimeMemoryTxFence,
};
use super::{
    message_chains, operation_scope_decisions, stage_run_units, stage_teams, stage_worker_runs,
    unified_investigation_runtime,
};

const COGNITIVE_OUTPUT_SCHEMA: &str = "investigation_cognitive_output.v1";
const NESTED_REQUEST_KIND: &str = "analysis_task";

#[derive(Debug, thiserror::Error)]
pub enum InvestigationNestedDispatchStoreError {
    #[error(transparent)]
    Runtime(#[from] RuntimeMemoryStoreError),
    #[error(transparent)]
    Sqlx(#[from] sqlx::Error),
    #[error(transparent)]
    Db(#[from] crate::DbError),
    #[error("invalid nested Investigation dispatch input: {0}")]
    InvalidInput(&'static str),
    #[error("nested Investigation dispatch authority mismatch: {0}")]
    AuthorityMismatch(&'static str),
    #[error("nested Investigation dispatch replay conflict: {0}")]
    ReplayConflict(&'static str),
}

pub type InvestigationNestedDispatchStoreResult<T> =
    Result<T, InvestigationNestedDispatchStoreError>;

#[derive(Debug, Clone, PartialEq)]
pub struct BeginInvestigationNestedDispatchRow {
    pub authority_id: Uuid,
    pub operation_id: Uuid,
    pub stage_execution_id: Uuid,
    pub owning_stage_run_request_id: String,
    pub stage_run_unit_id: Uuid,
    pub scope_snapshot_id: Uuid,
    pub organization_id: Uuid,
    pub stable_request_id: Uuid,
    pub task_plan_id: Uuid,
    pub subtask_id: Uuid,
    pub parent_dispatch_receipt_id: Uuid,
    pub parent_fence: RuntimeMemoryTxFence,
    pub stage_team_plan_id: Uuid,
    pub parent_work_item_id: Uuid,
    pub expected_dispatch_epoch: i64,
    pub nested_tool_request_id: String,
    pub requested_role: String,
    pub objective: String,
    pub args_sha256: String,
    pub snapshot_sha256: String,
    pub dispatch_ordinal: i32,
    pub session_id: Uuid,
    pub agent: AgentType,
    pub model: Option<String>,
    pub provider: Option<String>,
    pub lease_owner: String,
    pub lease_seconds: i32,
    pub initial_chain: Value,
    pub initial_checkpoint: Value,
}

#[derive(Debug, Clone)]
pub struct BegunInvestigationNestedDispatchRow {
    pub begin_receipt_id: Uuid,
    pub stable_request_id: Uuid,
    pub task_plan_id: Uuid,
    pub subtask_id: Uuid,
    pub parent_dispatch_receipt_id: Uuid,
    pub stage_worker_request_id: Uuid,
    pub args_sha256: String,
    pub request_sha256: String,
    pub begin_receipt_sha256: String,
    pub unit: stage_run_units::StageRunUnitRow,
    pub plan: stage_teams::StageTeamPlanRow,
    pub work_item: stage_teams::StageWorkItemRow,
    pub worker: stage_worker_runs::StageWorkerRunRow,
    pub message_chain_id: Uuid,
    pub dispatch: unified_investigation_runtime::PentagiLogicalDispatchRow,
    pub replayed: bool,
}

#[derive(Debug, Clone, PartialEq)]
pub struct FinishInvestigationNestedDispatchRow {
    pub authority_id: Uuid,
    pub operation_id: Uuid,
    pub stage_execution_id: Uuid,
    pub owning_stage_run_request_id: String,
    pub stage_run_unit_id: Uuid,
    pub scope_snapshot_id: Uuid,
    pub organization_id: Uuid,
    pub stable_request_id: Uuid,
    pub begin_receipt_id: Uuid,
    pub task_plan_id: Uuid,
    pub subtask_id: Uuid,
    pub parent_dispatch_receipt_id: Uuid,
    pub dispatch_receipt_id: Uuid,
    pub child_fence: RuntimeMemoryTxFence,
    pub stage_team_plan_id: Uuid,
    pub work_item_id: Uuid,
    pub expected_work_item_row_version: i64,
    pub output: stage_teams::CompleteStageWorkerRow,
    pub outcome: unified_investigation_runtime::PentagiDispatchOutcome,
    pub result_sha256: String,
    pub fence_sha256: String,
}

#[derive(Debug, Clone)]
pub struct FinishedInvestigationNestedDispatchRow {
    pub finish_receipt_id: Uuid,
    pub stable_request_id: Uuid,
    pub begin_receipt_id: Uuid,
    pub task_plan_id: Uuid,
    pub subtask_id: Uuid,
    pub parent_dispatch_receipt_id: Uuid,
    pub dispatch_receipt_id: Uuid,
    pub result_sha256: String,
    pub finish_receipt_sha256: String,
    pub completion: stage_teams::CompletedStageWorkerRow,
    pub dispatch_attempt: unified_investigation_runtime::PentagiDispatchAttemptRow,
    pub replayed: bool,
}

#[derive(Debug, Clone, sqlx::FromRow)]
struct BeginReceiptRow {
    begin_receipt_id: Uuid,
    stable_request_id: Uuid,
    authority_id: Uuid,
    operation_id: Uuid,
    stage_execution_id: Uuid,
    owning_stage_run_request_id: String,
    stage_run_unit_id: Uuid,
    scope_snapshot_id: Uuid,
    organization_id: Uuid,
    task_plan_id: Uuid,
    subtask_id: Uuid,
    parent_dispatch_receipt_id: Uuid,
    parent_worker_run_id: Uuid,
    parent_work_item_id: Uuid,
    parent_lease_token: Uuid,
    parent_attempt_epoch: i64,
    parent_checkpoint_version: i64,
    stage_team_plan_id: Uuid,
    dispatch_epoch: i64,
    nested_tool_request_id: String,
    requested_role: String,
    objective: String,
    args_sha256: String,
    snapshot_sha256: String,
    request_sha256: String,
    dispatch_ordinal: i32,
    stage_worker_request_id: Uuid,
    child_work_item_id: Uuid,
    child_worker_run_id: Uuid,
    child_message_chain_id: Uuid,
    child_dispatch_receipt_id: Uuid,
    begin_receipt_sha256: String,
}

#[derive(Debug, Clone, sqlx::FromRow)]
struct FinishReceiptRow {
    finish_receipt_id: Uuid,
    stable_request_id: Uuid,
    begin_receipt_id: Uuid,
    task_plan_id: Uuid,
    subtask_id: Uuid,
    parent_dispatch_receipt_id: Uuid,
    child_dispatch_receipt_id: Uuid,
    child_worker_run_id: Uuid,
    child_work_item_id: Uuid,
    child_lease_token: Uuid,
    child_attempt_epoch: i64,
    child_checkpoint_version: i64,
    dispatch_attempt_id: Uuid,
    output_id: Uuid,
    outcome: String,
    result_sha256: String,
    fence_sha256: String,
    finish_receipt_sha256: String,
}

#[derive(Debug, Clone)]
pub struct PgInvestigationNestedDispatchRepository {
    pool: std::sync::Arc<PgPool>,
}

impl PgInvestigationNestedDispatchRepository {
    pub fn new(pool: std::sync::Arc<PgPool>) -> Self {
        Self { pool }
    }

    pub async fn begin(
        &self,
        input: &BeginInvestigationNestedDispatchRow,
    ) -> InvestigationNestedDispatchStoreResult<BegunInvestigationNestedDispatchRow> {
        validate_begin(input)?;
        let request = stage_worker_request(input);
        let request_sha256 = stage_worker_request_payload_hash(&request);
        let ids = NestedIds::new(input.stable_request_id);
        let begin_receipt_sha256 = hash_json(&json!({
            "args_sha256": input.args_sha256,
            "begin_receipt_id": ids.begin_receipt_id,
            "child_dispatch_receipt_id": ids.dispatch_receipt_id,
            "child_message_chain_id": ids.message_chain_id,
            "child_work_item_id": ids.work_item_id,
            "child_worker_run_id": ids.worker_run_id,
            "parent_dispatch_receipt_id": input.parent_dispatch_receipt_id,
            "request_sha256": request_sha256,
            "stage_worker_request_id": ids.worker_request_id,
            "stable_request_id": input.stable_request_id,
            "subtask_id": input.subtask_id,
            "task_plan_id": input.task_plan_id,
        }));

        let mut tx = self.pool.begin().await?;
        let locked = lock_begin_authority(&mut tx, input).await?;
        if let Some(existing) = load_begin(&mut tx, input.stable_request_id).await? {
            validate_begin_replay(
                &existing,
                input,
                &ids,
                &request_sha256,
                &begin_receipt_sha256,
            )?;
            let result = load_begun(&mut tx, existing, locked, true).await?;
            tx.commit().await?;
            return Ok(result);
        }

        enforce_new_child_capacity(&mut tx, input, &locked.plan).await?;
        let reason = nested_reason(input);
        let child_input_refs = json!([{
            "assignment_schema": "investigation_task_orchestrator_assignment.v1",
            "objective": input.objective,
            "subject_refs": [],
        }]);
        let input_manifest_hash = hash_json(&json!({
            "parent_work_item_id": input.parent_work_item_id,
            "parent_worker_run_id": input.parent_fence.worker_run_id,
            "reason": input.objective,
            "subject_refs": [],
        }));
        let priority: i32 = sqlx::query_scalar(
            "SELECT COALESCE(MAX(priority),-1)+1 FROM stage_work_items WHERE team_plan_id=$1",
        )
        .bind(input.stage_team_plan_id)
        .fetch_one(&mut *tx)
        .await?;
        let attempt_policy = locked
            .plan
            .dynamic_request_policy
            .get("attempt_policy")
            .filter(|value| value.is_object())
            .cloned()
            .unwrap_or_else(|| json!({"max_attempts": 3}));
        let work_item = stage_teams::insert_work_item_with_executor(
            &mut *tx,
            &stage_teams::NewStageWorkItem {
                id: ids.work_item_id,
                team_plan_id: locked.plan.id,
                operation_id: locked.plan.operation_id,
                stage_execution_id: locked.plan.stage_execution_id,
                stage_run_unit_id: locked.plan.stage_run_unit_id,
                scope_snapshot_id: locked.plan.scope_snapshot_id,
                organization_id: locked.plan.organization_id,
                dispatch_epoch: locked.plan.dispatch_epoch,
                kind: NESTED_REQUEST_KIND.to_string(),
                stable_key: format!("dynamic:{}", ids.worker_request_id),
                role: input.requested_role.clone(),
                input_manifest_hash,
                input_refs: child_input_refs,
                required_for_barrier: true,
                conflict_key: None,
                priority,
                attempt_policy,
                budget: json!({}),
                output_schema: COGNITIVE_OUTPUT_SCHEMA.to_string(),
                created_by: "accepted_worker_request".to_string(),
            },
        )
        .await?;
        sqlx::query(
            r#"INSERT INTO stage_worker_requests(
                   id,team_plan_id,operation_id,stage_execution_id,stage_run_unit_id,
                   scope_snapshot_id,organization_id,parent_work_item_id,parent_worker_run_id,
                   dispatch_epoch,requested_role,request_kind,bounded_subject_refs,reason_code,
                   expected_output_schema,budget_hint,dedupe_key,request_payload_hash,status,
                   decision_reason_code,accepted_work_item_id
               ) VALUES($1,$2,$3,$4,$5,$6,$7,$8,$9,$10,$11,$12,'[]'::jsonb,$13,$14,
                        '{}'::jsonb,$15,$16,'accepted',NULL,$17)"#,
        )
        .bind(ids.worker_request_id)
        .bind(locked.plan.id)
        .bind(locked.plan.operation_id)
        .bind(locked.plan.stage_execution_id)
        .bind(locked.plan.stage_run_unit_id)
        .bind(locked.plan.scope_snapshot_id)
        .bind(locked.plan.organization_id)
        .bind(input.parent_work_item_id)
        .bind(input.parent_fence.worker_run_id)
        .bind(locked.plan.dispatch_epoch)
        .bind(&input.requested_role)
        .bind(NESTED_REQUEST_KIND)
        .bind(reason)
        .bind(COGNITIVE_OUTPUT_SCHEMA)
        .bind(format!("investigation-nested:{}", input.stable_request_id))
        .bind(&request_sha256)
        .bind(ids.work_item_id)
        .execute(&mut *tx)
        .await?;
        let leader_work_item_id: Uuid = sqlx::query_scalar(
            "SELECT id FROM stage_work_items WHERE team_plan_id=$1 AND stable_key='leader:primary' FOR UPDATE",
        )
        .bind(locked.plan.id)
        .fetch_one(&mut *tx)
        .await?;
        sqlx::query(
            r#"INSERT INTO stage_work_item_dependencies(
                   team_plan_id,operation_id,stage_execution_id,stage_run_unit_id,
                   scope_snapshot_id,organization_id,work_item_id,depends_on_work_item_id
               ) VALUES($1,$2,$3,$4,$5,$6,$7,$8)"#,
        )
        .bind(locked.plan.id)
        .bind(locked.plan.operation_id)
        .bind(locked.plan.stage_execution_id)
        .bind(locked.plan.stage_run_unit_id)
        .bind(locked.plan.scope_snapshot_id)
        .bind(locked.plan.organization_id)
        .bind(leader_work_item_id)
        .bind(ids.work_item_id)
        .execute(&mut *tx)
        .await?;
        let work_item = sqlx::query_as::<_, stage_teams::StageWorkItemRow>(
            "UPDATE stage_work_items SET status='running',started_at=NOW(),row_version=row_version+1,updated_at=NOW() WHERE id=$1 AND status='queued' RETURNING *",
        )
        .bind(work_item.id)
        .fetch_one(&mut *tx)
        .await?;
        let worker = stage_worker_runs::insert_with_executor(
            &mut *tx,
            &stage_worker_runs::NewStageWorkerRun {
                id: ids.worker_run_id,
                operation_id: locked.plan.operation_id,
                stage_execution_id: locked.plan.stage_execution_id,
                stage_run_unit_id: locked.plan.stage_run_unit_id,
                work_item_id: Some(work_item.id),
                organization_id: locked.plan.organization_id,
                worker_generation: 0,
                specialist: input.requested_role.clone(),
                work_item_kind: NESTED_REQUEST_KIND.to_string(),
                work_item_key: work_item.stable_key.clone(),
                agent_path: format!(
                    "main>stage_run:investigation>org:{}>nested:{}:{}",
                    locked.plan.organization_id, input.requested_role, input.stable_request_id
                ),
                parent_request_id: Some(input.nested_tool_request_id.clone()),
            },
        )
        .await?;
        let lease_token = Uuid::new_v5(&input.stable_request_id, b"child-lease-token-v1");
        let claimed = stage_worker_runs::claim_cas(
            &mut *tx,
            worker.id,
            locked.unit.id,
            stage_worker_runs::StageWorkerRunStatus::Queued,
            0,
            lease_token,
            &input.lease_owner,
            input.lease_seconds,
        )
        .await?;
        message_chains::create_bound_with_executor(
            &mut *tx,
            ids.message_chain_id,
            input.session_id,
            input.operation_id,
            None,
            input.agent,
            input.model.as_deref(),
            input.provider.as_deref(),
            &input.initial_chain,
        )
        .await?;
        let bound = stage_worker_runs::bind_message_chain_cas(
            &mut *tx,
            claimed.id,
            locked.unit.id,
            lease_token,
            claimed.attempt_epoch,
            ids.message_chain_id,
        )
        .await?;
        let worker = stage_worker_runs::checkpoint_cas(
            &mut *tx,
            &RuntimeMemoryTxFence {
                operation_id: input.operation_id,
                stage_execution_id: input.stage_execution_id,
                stage_run_unit_id: input.stage_run_unit_id,
                worker_run_id: bound.id,
                lease_token,
                attempt_epoch: bound.attempt_epoch,
                expected_checkpoint_version: bound.checkpoint_version,
            },
            &input.initial_checkpoint,
        )
        .await?;

        let dispatch = insert_nested_dispatch(&mut tx, input, &ids, &locked, &worker).await?;
        sqlx::query(
            r#"INSERT INTO investigation_nested_dispatch_begins(
                   begin_receipt_id,stable_request_id,authority_id,operation_id,stage_execution_id,
                   owning_stage_run_request_id,stage_run_unit_id,scope_snapshot_id,organization_id,
                   task_plan_id,subtask_id,parent_dispatch_receipt_id,parent_worker_run_id,
                   parent_work_item_id,parent_lease_token,parent_attempt_epoch,
                   parent_checkpoint_version,stage_team_plan_id,dispatch_epoch,
                   nested_tool_request_id,requested_role,objective,args_sha256,snapshot_sha256,
                   request_sha256,dispatch_ordinal,stage_worker_request_id,child_work_item_id,
                   child_worker_run_id,child_message_chain_id,child_dispatch_receipt_id,
                   begin_receipt_sha256
               ) VALUES($1,$2,$3,$4,$5,$6,$7,$8,$9,$10,$11,$12,$13,$14,$15,$16,$17,$18,
                        $19,$20,$21,$22,$23,$24,$25,$26,$27,$28,$29,$30,$31,$32)"#,
        )
        .bind(ids.begin_receipt_id)
        .bind(input.stable_request_id)
        .bind(input.authority_id)
        .bind(input.operation_id)
        .bind(input.stage_execution_id)
        .bind(&input.owning_stage_run_request_id)
        .bind(input.stage_run_unit_id)
        .bind(input.scope_snapshot_id)
        .bind(input.organization_id)
        .bind(input.task_plan_id)
        .bind(input.subtask_id)
        .bind(input.parent_dispatch_receipt_id)
        .bind(input.parent_fence.worker_run_id)
        .bind(input.parent_work_item_id)
        .bind(input.parent_fence.lease_token)
        .bind(input.parent_fence.attempt_epoch)
        .bind(input.parent_fence.expected_checkpoint_version)
        .bind(input.stage_team_plan_id)
        .bind(input.expected_dispatch_epoch)
        .bind(&input.nested_tool_request_id)
        .bind(&input.requested_role)
        .bind(&input.objective)
        .bind(&input.args_sha256)
        .bind(&input.snapshot_sha256)
        .bind(&request_sha256)
        .bind(input.dispatch_ordinal)
        .bind(ids.worker_request_id)
        .bind(ids.work_item_id)
        .bind(ids.worker_run_id)
        .bind(ids.message_chain_id)
        .bind(ids.dispatch_receipt_id)
        .bind(&begin_receipt_sha256)
        .execute(&mut *tx)
        .await?;
        tx.commit().await?;
        Ok(BegunInvestigationNestedDispatchRow {
            begin_receipt_id: ids.begin_receipt_id,
            stable_request_id: input.stable_request_id,
            task_plan_id: input.task_plan_id,
            subtask_id: input.subtask_id,
            parent_dispatch_receipt_id: input.parent_dispatch_receipt_id,
            stage_worker_request_id: ids.worker_request_id,
            args_sha256: input.args_sha256.clone(),
            request_sha256,
            begin_receipt_sha256,
            unit: locked.unit,
            plan: locked.plan,
            work_item,
            worker,
            message_chain_id: ids.message_chain_id,
            dispatch,
            replayed: false,
        })
    }

    pub async fn finish(
        &self,
        input: &FinishInvestigationNestedDispatchRow,
    ) -> InvestigationNestedDispatchStoreResult<FinishedInvestigationNestedDispatchRow> {
        validate_finish(input)?;
        let finish_receipt_id = Uuid::new_v5(&input.stable_request_id, b"nested-finish-receipt-v1");
        let dispatch_attempt_id =
            Uuid::new_v5(&input.stable_request_id, b"nested-dispatch-attempt-v1");
        let finish_receipt_sha256 = hash_json(&json!({
            "begin_receipt_id": input.begin_receipt_id,
            "child_attempt_epoch": input.child_fence.attempt_epoch,
            "child_checkpoint_version": input.child_fence.expected_checkpoint_version,
            "child_dispatch_receipt_id": input.dispatch_receipt_id,
            "child_lease_token": input.child_fence.lease_token,
            "child_worker_run_id": input.child_fence.worker_run_id,
            "dispatch_attempt_id": dispatch_attempt_id,
            "fence_sha256": input.fence_sha256,
            "finish_receipt_id": finish_receipt_id,
            "outcome": input.outcome.as_str(),
            "output_sha256": input.output.output_hash,
            "result_sha256": input.result_sha256,
            "stable_request_id": input.stable_request_id,
        }));
        let mut tx = self.pool.begin().await?;
        let begin = load_begin_by_id(&mut tx, input.begin_receipt_id).await?;
        validate_finish_owner(&begin, input)?;
        if let Some(existing) = load_finish(&mut tx, input.stable_request_id).await? {
            validate_finish_replay(
                &existing,
                input,
                finish_receipt_id,
                dispatch_attempt_id,
                &finish_receipt_sha256,
            )?;
            let result = load_finished(&mut tx, existing, input, true).await?;
            tx.commit().await?;
            return Ok(result);
        }
        let operation: (Option<Uuid>, String) = sqlx::query_as(
            "SELECT superseded_by,runtime_memory_contract FROM operation_state WHERE operation_id=$1 FOR UPDATE",
        )
        .bind(input.operation_id)
        .fetch_one(&mut *tx)
        .await?;
        if operation.0.is_some() || operation.1 != "v2_only" {
            return Err(InvestigationNestedDispatchStoreError::AuthorityMismatch(
                "operation_not_runnable",
            ));
        }
        let unit = sqlx::query_as::<_, stage_run_units::StageRunUnitRow>(
            "SELECT * FROM stage_run_units WHERE id=$1 AND operation_id=$2 AND stage_execution_id=$3 FOR UPDATE",
        )
        .bind(input.stage_run_unit_id)
        .bind(input.operation_id)
        .bind(input.stage_execution_id)
        .fetch_one(&mut *tx)
        .await?;
        let plan = sqlx::query_as::<_, stage_teams::StageTeamPlanRow>(
            "SELECT * FROM stage_team_plans WHERE id=$1 FOR UPDATE",
        )
        .bind(input.stage_team_plan_id)
        .fetch_one(&mut *tx)
        .await?;
        let item = sqlx::query_as::<_, stage_teams::StageWorkItemRow>(
            "SELECT * FROM stage_work_items WHERE id=$1 FOR UPDATE",
        )
        .bind(input.work_item_id)
        .fetch_one(&mut *tx)
        .await?;
        let worker = sqlx::query_as::<_, stage_worker_runs::StageWorkerRunRow>(
            "SELECT * FROM stage_worker_runs WHERE id=$1 FOR UPDATE",
        )
        .bind(input.child_fence.worker_run_id)
        .fetch_one(&mut *tx)
        .await?;
        if unit.status != "running"
            || unit.stage_kind != "investigation"
            || unit.organization_id != input.organization_id
            || unit.scope_snapshot_id != input.scope_snapshot_id
            || plan.id != begin.stage_team_plan_id
            || plan.operation_id != input.operation_id
            || plan.stage_execution_id != input.stage_execution_id
            || plan.stage_run_unit_id != input.stage_run_unit_id
            || plan.organization_id != input.organization_id
            || item.id != begin.child_work_item_id
            || item.team_plan_id != plan.id
            || item.output_schema != COGNITIVE_OUTPUT_SCHEMA
            || item.status != "running"
            || item.row_version != input.expected_work_item_row_version
            || worker.id != begin.child_worker_run_id
            || worker.work_item_id != Some(item.id)
            || worker.status != "running"
            || worker.lease_token != Some(input.child_fence.lease_token)
            || worker.attempt_epoch != input.child_fence.attempt_epoch
            || worker.checkpoint_version != input.child_fence.expected_checkpoint_version
            || worker
                .lease_expires_at
                .is_none_or(|expires| expires <= chrono::Utc::now())
        {
            return Err(InvestigationNestedDispatchStoreError::AuthorityMismatch(
                "child_finish_fence_mismatch",
            ));
        }
        stage_teams::validate_stage_worker_output_authority(
            &mut tx,
            &unit,
            &plan,
            &item,
            &input.output,
        )
        .await?;
        let worker = stage_worker_runs::finish_passed_for_stage_output(
            &mut *tx,
            &input.child_fence,
            &input.output.terminal_checkpoint,
            input.output.evidence_watermark,
        )
        .await?;
        let work_item = sqlx::query_as::<_, stage_teams::StageWorkItemRow>(
            "UPDATE stage_work_items SET status='completed',row_version=row_version+1,terminal_at=NOW(),updated_at=NOW() WHERE id=$1 AND status='running' AND row_version=$2 RETURNING *",
        )
        .bind(item.id)
        .bind(input.expected_work_item_row_version)
        .fetch_one(&mut *tx)
        .await?;
        let output_id = Uuid::new_v5(&item.id, b"stage-worker-output-v1");
        let output = sqlx::query_as::<_, stage_teams::StageWorkerOutputRow>(
            r#"INSERT INTO stage_worker_outputs(
                   id,team_plan_id,work_item_id,worker_run_id,operation_id,stage_execution_id,
                   stage_run_unit_id,scope_snapshot_id,organization_id,output_schema,output_version,
                   business_disposition,canonical_output,canonical_fact_refs,evidence_ids,
                   checked_empty_cells,blocker_codes,output_hash
               ) VALUES($1,$2,$3,$4,$5,$6,$7,$8,$9,$10,1,$11,$12,$13,$14,$15,$16,$17)
               RETURNING *"#,
        )
        .bind(output_id)
        .bind(plan.id)
        .bind(item.id)
        .bind(worker.id)
        .bind(plan.operation_id)
        .bind(plan.stage_execution_id)
        .bind(plan.stage_run_unit_id)
        .bind(plan.scope_snapshot_id)
        .bind(plan.organization_id)
        .bind(&input.output.output_schema)
        .bind(&input.output.business_disposition)
        .bind(&input.output.canonical_output)
        .bind(&input.output.canonical_fact_refs)
        .bind(&input.output.evidence_ids)
        .bind(&input.output.checked_empty_cells)
        .bind(&input.output.blocker_codes)
        .bind(&input.output.output_hash)
        .fetch_one(&mut *tx)
        .await?;
        let attempt_stable_request_id = Uuid::new_v5(
            &input.stable_request_id,
            b"nested-dispatch-attempt-stable-v1",
        );
        let attempt =
            sqlx::query_as::<_, unified_investigation_runtime::PentagiDispatchAttemptRow>(
                r#"INSERT INTO pentagi_logical_dispatch_attempts(
                   dispatch_attempt_id,stable_request_id,dispatch_receipt_id,attempt_epoch,
                   lease_token,fence_sha256,outcome,result_sha256
               ) VALUES($1,$2,$3,$4,$5,$6,$7,$8) RETURNING *"#,
            )
            .bind(dispatch_attempt_id)
            .bind(attempt_stable_request_id)
            .bind(input.dispatch_receipt_id)
            .bind(input.child_fence.attempt_epoch)
            .bind(input.child_fence.lease_token)
            .bind(&input.fence_sha256)
            .bind(input.outcome.as_str())
            .bind(&input.result_sha256)
            .fetch_one(&mut *tx)
            .await?;
        sqlx::query(
            r#"INSERT INTO investigation_nested_dispatch_finishes(
                   finish_receipt_id,stable_request_id,begin_receipt_id,task_plan_id,subtask_id,
                   parent_dispatch_receipt_id,child_dispatch_receipt_id,child_worker_run_id,
                   child_work_item_id,child_lease_token,child_attempt_epoch,
                   child_checkpoint_version,dispatch_attempt_id,output_id,outcome,result_sha256,
                   fence_sha256,finish_receipt_sha256
               ) VALUES($1,$2,$3,$4,$5,$6,$7,$8,$9,$10,$11,$12,$13,$14,$15,$16,$17,$18)"#,
        )
        .bind(finish_receipt_id)
        .bind(input.stable_request_id)
        .bind(input.begin_receipt_id)
        .bind(input.task_plan_id)
        .bind(input.subtask_id)
        .bind(input.parent_dispatch_receipt_id)
        .bind(input.dispatch_receipt_id)
        .bind(input.child_fence.worker_run_id)
        .bind(input.work_item_id)
        .bind(input.child_fence.lease_token)
        .bind(input.child_fence.attempt_epoch)
        .bind(input.child_fence.expected_checkpoint_version)
        .bind(dispatch_attempt_id)
        .bind(output_id)
        .bind(input.outcome.as_str())
        .bind(&input.result_sha256)
        .bind(&input.fence_sha256)
        .bind(&finish_receipt_sha256)
        .execute(&mut *tx)
        .await?;
        tx.commit().await?;
        Ok(FinishedInvestigationNestedDispatchRow {
            finish_receipt_id,
            stable_request_id: input.stable_request_id,
            begin_receipt_id: input.begin_receipt_id,
            task_plan_id: input.task_plan_id,
            subtask_id: input.subtask_id,
            parent_dispatch_receipt_id: input.parent_dispatch_receipt_id,
            dispatch_receipt_id: input.dispatch_receipt_id,
            result_sha256: input.result_sha256.clone(),
            finish_receipt_sha256,
            completion: stage_teams::CompletedStageWorkerRow {
                unit,
                plan,
                work_item,
                worker,
                output,
                replayed: false,
            },
            dispatch_attempt: attempt,
            replayed: false,
        })
    }
}

#[derive(Debug)]
struct NestedIds {
    begin_receipt_id: Uuid,
    worker_request_id: Uuid,
    work_item_id: Uuid,
    worker_run_id: Uuid,
    message_chain_id: Uuid,
    dispatch_receipt_id: Uuid,
    dispatch_stable_request_id: Uuid,
}

impl NestedIds {
    fn new(stable_request_id: Uuid) -> Self {
        Self {
            begin_receipt_id: Uuid::new_v5(&stable_request_id, b"nested-begin-receipt-v1"),
            worker_request_id: Uuid::new_v5(&stable_request_id, b"nested-worker-request-v1"),
            work_item_id: Uuid::new_v5(&stable_request_id, b"nested-work-item-v1"),
            worker_run_id: Uuid::new_v5(&stable_request_id, b"nested-worker-run-v1"),
            message_chain_id: Uuid::new_v5(&stable_request_id, b"nested-message-chain-v1"),
            dispatch_receipt_id: Uuid::new_v5(&stable_request_id, b"nested-dispatch-receipt-v1"),
            dispatch_stable_request_id: Uuid::new_v5(
                &stable_request_id,
                b"nested-dispatch-stable-v1",
            ),
        }
    }
}

#[derive(Debug)]
struct LockedBeginAuthority {
    unit: stage_run_units::StageRunUnitRow,
    plan: stage_teams::StageTeamPlanRow,
    parent_dispatch: unified_investigation_runtime::PentagiLogicalDispatchRow,
}

fn valid_sha256(value: &str) -> bool {
    value.strip_prefix("sha256:").is_some_and(|hex| {
        hex.len() == 64
            && hex
                .bytes()
                .all(|byte| byte.is_ascii_digit() || matches!(byte, b'a'..=b'f'))
    })
}

fn hash_json(value: &Value) -> String {
    format!("sha256:{}", operation_scope_decisions::sha256_json(value))
}

fn nested_reason(input: &BeginInvestigationNestedDispatchRow) -> String {
    serde_json::to_string(&json!({
        "schema": "investigation_task_orchestrator_request.v1",
        "parent_tool_request_id": input.nested_tool_request_id,
        "objective": input.objective,
    }))
    .expect("nested reason is JSON-serializable")
}

fn stage_worker_request(input: &BeginInvestigationNestedDispatchRow) -> RequestStageWorkerRow {
    RequestStageWorkerRow {
        fence: input.parent_fence.clone(),
        stage_team_plan_id: input.stage_team_plan_id,
        parent_work_item_id: input.parent_work_item_id,
        expected_dispatch_epoch: input.expected_dispatch_epoch,
        requested_role: input.requested_role.clone(),
        requested_kind: NESTED_REQUEST_KIND.to_string(),
        subject_refs: vec![],
        reason: nested_reason(input),
        output_schema: json!(COGNITIVE_OUTPUT_SCHEMA),
        budget_hint: json!({}),
        dedupe_key: format!("investigation-nested:{}", input.stable_request_id),
        request_sha256: String::new(),
    }
}

fn validate_begin(
    input: &BeginInvestigationNestedDispatchRow,
) -> InvestigationNestedDispatchStoreResult<()> {
    if [
        input.authority_id,
        input.operation_id,
        input.stage_execution_id,
        input.stage_run_unit_id,
        input.scope_snapshot_id,
        input.organization_id,
        input.stable_request_id,
        input.task_plan_id,
        input.subtask_id,
        input.parent_dispatch_receipt_id,
        input.stage_team_plan_id,
        input.parent_work_item_id,
        input.session_id,
    ]
    .into_iter()
    .any(|id| id.is_nil())
        || input.parent_fence.operation_id != input.operation_id
        || input.parent_fence.stage_execution_id != input.stage_execution_id
        || input.parent_fence.stage_run_unit_id != input.stage_run_unit_id
        || input.expected_dispatch_epoch < 0
        || input.dispatch_ordinal < 0
        || input.lease_seconds <= 0
        || input.owning_stage_run_request_id.trim().is_empty()
        || input.nested_tool_request_id.trim().is_empty()
        || input.requested_role.trim().is_empty()
        || input.objective.trim().is_empty()
        || input.lease_owner.trim().is_empty()
        || !valid_sha256(&input.args_sha256)
        || !valid_sha256(&input.snapshot_sha256)
        || !input.initial_chain.is_array()
        || !(input.initial_checkpoint.is_array() || input.initial_checkpoint.is_object())
    {
        return Err(InvestigationNestedDispatchStoreError::InvalidInput(
            "begin_contract",
        ));
    }
    Ok(())
}

async fn lock_begin_authority(
    tx: &mut Transaction<'_, Postgres>,
    input: &BeginInvestigationNestedDispatchRow,
) -> InvestigationNestedDispatchStoreResult<LockedBeginAuthority> {
    let operation: (Option<Uuid>, String, String) = sqlx::query_as(
        "SELECT superseded_by,runtime_memory_contract,current_stage FROM operation_state WHERE operation_id=$1 FOR UPDATE",
    )
    .bind(input.operation_id)
    .fetch_one(&mut **tx)
    .await?;
    if operation.0.is_some() || operation.1 != "v2_only" || operation.2 != "investigation" {
        return Err(InvestigationNestedDispatchStoreError::AuthorityMismatch(
            "operation_not_in_investigation",
        ));
    }
    let unit = sqlx::query_as::<_, stage_run_units::StageRunUnitRow>(
        "SELECT * FROM stage_run_units WHERE id=$1 AND operation_id=$2 AND stage_execution_id=$3 FOR UPDATE",
    )
    .bind(input.stage_run_unit_id)
    .bind(input.operation_id)
    .bind(input.stage_execution_id)
    .fetch_one(&mut **tx)
    .await?;
    let plan = sqlx::query_as::<_, stage_teams::StageTeamPlanRow>(
        "SELECT * FROM stage_team_plans WHERE id=$1 FOR UPDATE",
    )
    .bind(input.stage_team_plan_id)
    .fetch_one(&mut **tx)
    .await?;
    let parent_item = sqlx::query_as::<_, stage_teams::StageWorkItemRow>(
        "SELECT * FROM stage_work_items WHERE id=$1 FOR UPDATE",
    )
    .bind(input.parent_work_item_id)
    .fetch_one(&mut **tx)
    .await?;
    let parent_worker = sqlx::query_as::<_, stage_worker_runs::StageWorkerRunRow>(
        "SELECT * FROM stage_worker_runs WHERE id=$1 FOR UPDATE",
    )
    .bind(input.parent_fence.worker_run_id)
    .fetch_one(&mut **tx)
    .await?;
    let parent_dispatch = sqlx::query_as::<_, unified_investigation_runtime::PentagiLogicalDispatchRow>(
        "SELECT dispatch_receipt_id,stable_request_id,logical_dispatch_key_sha256,task_plan_id,subtask_id,parent_dispatch_receipt_id,dispatch_ordinal,actor_kind,stage_work_item_id,stage_worker_request_id,worker_run_id,operation_id,stage_execution_id,stage_run_unit_id,scope_snapshot_id,organization_id,transcript_request_id,parent_actor_transcript_request_id,parent_dispatch_tool_request_id,snapshot_sha256,receipt_sha256 FROM pentagi_logical_dispatch_receipts WHERE dispatch_receipt_id=$1 FOR SHARE",
    )
    .bind(input.parent_dispatch_receipt_id)
    .fetch_one(&mut **tx)
    .await?;
    let task_plan_open: bool = sqlx::query_scalar(
        "SELECT EXISTS(SELECT 1 FROM investigation_pentagi_task_plans WHERE task_plan_id=$1 AND authority_id=$2 AND stage_team_plan_id=$3 AND operation_id=$4 AND stage_execution_id=$5 AND owning_stage_run_request_id=$6 AND stage_run_unit_id=$7 AND scope_snapshot_id=$8 AND organization_id=$9 AND status='open')",
    )
    .bind(input.task_plan_id)
    .bind(input.authority_id)
    .bind(input.stage_team_plan_id)
    .bind(input.operation_id)
    .bind(input.stage_execution_id)
    .bind(&input.owning_stage_run_request_id)
    .bind(input.stage_run_unit_id)
    .bind(input.scope_snapshot_id)
    .bind(input.organization_id)
    .fetch_one(&mut **tx)
    .await?;
    let subtask_exact: bool = sqlx::query_scalar(
        "SELECT EXISTS(SELECT 1 FROM investigation_pentagi_subtasks WHERE subtask_id=$1 AND task_plan_id=$2 AND runnable AND expected_output_schema=$3)",
    )
    .bind(input.subtask_id)
    .bind(input.task_plan_id)
    .bind(COGNITIVE_OUTPUT_SCHEMA)
    .fetch_one(&mut **tx)
    .await?;
    let role_allowed = plan.allowed_worker_roles.as_array().is_some_and(|roles| {
        roles
            .iter()
            .any(|role| role.as_str() == Some(&input.requested_role))
    });
    let kind_allowed = plan
        .dynamic_request_policy
        .get("allowed_request_kinds")
        .and_then(Value::as_array)
        .is_some_and(|kinds| {
            kinds
                .iter()
                .any(|kind| kind.as_str() == Some(NESTED_REQUEST_KIND))
        });
    if unit.status != "running"
        || unit.stage_kind != "investigation"
        || unit.organization_id != input.organization_id
        || unit.scope_snapshot_id != input.scope_snapshot_id
        || plan.operation_id != input.operation_id
        || plan.stage_execution_id != input.stage_execution_id
        || plan.stage_run_unit_id != input.stage_run_unit_id
        || plan.scope_snapshot_id != input.scope_snapshot_id
        || plan.organization_id != input.organization_id
        || plan.stage_kind != "investigation"
        || plan.dispatch_epoch != input.expected_dispatch_epoch
        || plan.requests_closed_at.is_some()
        || !plan.dynamic_requests_allowed
        || plan
            .dynamic_request_policy
            .get("coordination_mode")
            .and_then(Value::as_str)
            != Some("investigation_task_orchestrator")
        || plan
            .dynamic_request_policy
            .get("child_output_schema")
            .and_then(Value::as_str)
            != Some(COGNITIVE_OUTPUT_SCHEMA)
        || plan
            .dynamic_request_policy
            .get("organization_scope_implicit")
            .and_then(Value::as_bool)
            != Some(true)
        || input.requested_role == plan.leader_role
        || !role_allowed
        || !kind_allowed
        || parent_item.team_plan_id != plan.id
        || parent_item.id != parent_worker.work_item_id.unwrap_or(Uuid::nil())
        || parent_item.status != "running"
        || parent_item.output_schema != COGNITIVE_OUTPUT_SCHEMA
        || parent_worker.status != "running"
        || parent_worker.lease_token != Some(input.parent_fence.lease_token)
        || parent_worker.attempt_epoch != input.parent_fence.attempt_epoch
        || parent_worker.checkpoint_version != input.parent_fence.expected_checkpoint_version
        || parent_worker
            .lease_expires_at
            .is_none_or(|expires| expires <= chrono::Utc::now())
        || parent_dispatch.task_plan_id != input.task_plan_id
        || parent_dispatch.subtask_id != Some(input.subtask_id)
        || parent_dispatch.worker_run_id != input.parent_fence.worker_run_id
        || parent_dispatch.stage_work_item_id != input.parent_work_item_id
        || parent_dispatch.operation_id != input.operation_id
        || parent_dispatch.stage_execution_id != input.stage_execution_id
        || parent_dispatch.stage_run_unit_id != input.stage_run_unit_id
        || parent_dispatch.scope_snapshot_id != input.scope_snapshot_id
        || parent_dispatch.organization_id != input.organization_id
        || !task_plan_open
        || !subtask_exact
    {
        return Err(InvestigationNestedDispatchStoreError::AuthorityMismatch(
            "begin_owner_or_cognitive_contract_mismatch",
        ));
    }
    Ok(LockedBeginAuthority {
        unit,
        plan,
        parent_dispatch,
    })
}

async fn enforce_new_child_capacity(
    tx: &mut Transaction<'_, Postgres>,
    input: &BeginInvestigationNestedDispatchRow,
    plan: &stage_teams::StageTeamPlanRow,
) -> InvestigationNestedDispatchStoreResult<()> {
    let accepted: i64 = sqlx::query_scalar(
        "SELECT COUNT(*) FROM stage_worker_requests WHERE team_plan_id=$1 AND status='accepted'",
    )
    .bind(plan.id)
    .fetch_one(&mut **tx)
    .await?;
    let max_requests = plan
        .dynamic_request_policy
        .get("max_requests")
        .and_then(Value::as_i64)
        .unwrap_or(0);
    let active: i64 = sqlx::query_scalar(
        "SELECT COUNT(*) FROM stage_worker_runs WHERE stage_run_unit_id=$1 AND status IN ('queued','running','waiting_background')",
    )
    .bind(input.stage_run_unit_id)
    .fetch_one(&mut **tx)
    .await?;
    if max_requests <= 0 || accepted >= max_requests || active >= i64::from(plan.max_workers_active)
    {
        return Err(InvestigationNestedDispatchStoreError::AuthorityMismatch(
            "nested_worker_capacity_exhausted",
        ));
    }
    Ok(())
}

async fn insert_nested_dispatch(
    tx: &mut Transaction<'_, Postgres>,
    input: &BeginInvestigationNestedDispatchRow,
    ids: &NestedIds,
    locked: &LockedBeginAuthority,
    worker: &stage_worker_runs::StageWorkerRunRow,
) -> InvestigationNestedDispatchStoreResult<unified_investigation_runtime::PentagiLogicalDispatchRow>
{
    let nested_transcript_request_id =
        format!("{}::worker:{}", input.nested_tool_request_id, worker.id);
    let logical_key = hash_json(&json!({
        "dispatch_ordinal": input.dispatch_ordinal,
        "nested_tool_request_id": input.nested_tool_request_id,
        "parent_dispatch_receipt_id": input.parent_dispatch_receipt_id,
        "subtask_id": input.subtask_id,
        "task_plan_id": input.task_plan_id,
    }));
    let receipt_sha256 = hash_json(&json!({
        "dispatch_receipt_id": ids.dispatch_receipt_id,
        "logical_dispatch_key_sha256": logical_key,
        "parent_dispatch_receipt_id": input.parent_dispatch_receipt_id,
        "snapshot_sha256": input.snapshot_sha256,
        "stage_worker_request_id": ids.worker_request_id,
        "worker_run_id": worker.id,
    }));
    Ok(
        sqlx::query_as::<_, unified_investigation_runtime::PentagiLogicalDispatchRow>(
            r#"INSERT INTO pentagi_logical_dispatch_receipts(
               dispatch_receipt_id,stable_request_id,logical_dispatch_key_sha256,task_plan_id,
               subtask_id,parent_dispatch_receipt_id,dispatch_ordinal,actor_kind,
               stage_work_item_id,stage_worker_request_id,worker_run_id,operation_id,
               stage_execution_id,stage_run_unit_id,scope_snapshot_id,organization_id,
               transcript_request_id,parent_actor_transcript_request_id,
               parent_dispatch_tool_request_id,snapshot_sha256,receipt_sha256
           ) VALUES($1,$2,$3,$4,$5,$6,$7,'nested_worker',$8,$9,$10,$11,$12,$13,$14,$15,
                    $16,$17,$18,$19,$20)
           RETURNING dispatch_receipt_id,stable_request_id,logical_dispatch_key_sha256,
                     task_plan_id,subtask_id,parent_dispatch_receipt_id,dispatch_ordinal,
                     actor_kind,stage_work_item_id,stage_worker_request_id,worker_run_id,
                     operation_id,stage_execution_id,stage_run_unit_id,scope_snapshot_id,
                     organization_id,transcript_request_id,parent_actor_transcript_request_id,
                     parent_dispatch_tool_request_id,snapshot_sha256,receipt_sha256"#,
        )
        .bind(ids.dispatch_receipt_id)
        .bind(ids.dispatch_stable_request_id)
        .bind(logical_key)
        .bind(input.task_plan_id)
        .bind(input.subtask_id)
        .bind(input.parent_dispatch_receipt_id)
        .bind(input.dispatch_ordinal)
        .bind(ids.work_item_id)
        .bind(ids.worker_request_id)
        .bind(worker.id)
        .bind(input.operation_id)
        .bind(input.stage_execution_id)
        .bind(input.stage_run_unit_id)
        .bind(input.scope_snapshot_id)
        .bind(input.organization_id)
        .bind(&nested_transcript_request_id)
        .bind(&locked.parent_dispatch.transcript_request_id)
        .bind(&input.nested_tool_request_id)
        .bind(&input.snapshot_sha256)
        .bind(receipt_sha256)
        .fetch_one(&mut **tx)
        .await?,
    )
}

async fn load_begin(
    tx: &mut Transaction<'_, Postgres>,
    stable_request_id: Uuid,
) -> Result<Option<BeginReceiptRow>, sqlx::Error> {
    sqlx::query_as(
        "SELECT * FROM investigation_nested_dispatch_begins WHERE stable_request_id=$1 FOR SHARE",
    )
    .bind(stable_request_id)
    .fetch_optional(&mut **tx)
    .await
}

async fn load_begin_by_id(
    tx: &mut Transaction<'_, Postgres>,
    begin_receipt_id: Uuid,
) -> Result<BeginReceiptRow, sqlx::Error> {
    sqlx::query_as(
        "SELECT * FROM investigation_nested_dispatch_begins WHERE begin_receipt_id=$1 FOR SHARE",
    )
    .bind(begin_receipt_id)
    .fetch_one(&mut **tx)
    .await
}

fn validate_begin_replay(
    row: &BeginReceiptRow,
    input: &BeginInvestigationNestedDispatchRow,
    ids: &NestedIds,
    request_sha256: &str,
    begin_receipt_sha256: &str,
) -> InvestigationNestedDispatchStoreResult<()> {
    if row.begin_receipt_id != ids.begin_receipt_id
        || row.stable_request_id != input.stable_request_id
        || row.authority_id != input.authority_id
        || row.operation_id != input.operation_id
        || row.stage_execution_id != input.stage_execution_id
        || row.owning_stage_run_request_id != input.owning_stage_run_request_id
        || row.stage_run_unit_id != input.stage_run_unit_id
        || row.scope_snapshot_id != input.scope_snapshot_id
        || row.organization_id != input.organization_id
        || row.task_plan_id != input.task_plan_id
        || row.subtask_id != input.subtask_id
        || row.parent_dispatch_receipt_id != input.parent_dispatch_receipt_id
        || row.parent_worker_run_id != input.parent_fence.worker_run_id
        || row.parent_work_item_id != input.parent_work_item_id
        || row.parent_lease_token != input.parent_fence.lease_token
        || row.parent_attempt_epoch != input.parent_fence.attempt_epoch
        || row.parent_checkpoint_version != input.parent_fence.expected_checkpoint_version
        || row.stage_team_plan_id != input.stage_team_plan_id
        || row.dispatch_epoch != input.expected_dispatch_epoch
        || row.nested_tool_request_id != input.nested_tool_request_id
        || row.requested_role != input.requested_role
        || row.objective != input.objective
        || row.args_sha256 != input.args_sha256
        || row.snapshot_sha256 != input.snapshot_sha256
        || row.request_sha256 != request_sha256
        || row.dispatch_ordinal != input.dispatch_ordinal
        || row.stage_worker_request_id != ids.worker_request_id
        || row.child_work_item_id != ids.work_item_id
        || row.child_worker_run_id != ids.worker_run_id
        || row.child_message_chain_id != ids.message_chain_id
        || row.child_dispatch_receipt_id != ids.dispatch_receipt_id
        || row.begin_receipt_sha256 != begin_receipt_sha256
    {
        return Err(InvestigationNestedDispatchStoreError::ReplayConflict(
            "begin_material_mismatch",
        ));
    }
    Ok(())
}

async fn load_begun(
    tx: &mut Transaction<'_, Postgres>,
    row: BeginReceiptRow,
    locked: LockedBeginAuthority,
    replayed: bool,
) -> InvestigationNestedDispatchStoreResult<BegunInvestigationNestedDispatchRow> {
    let work_item = sqlx::query_as::<_, stage_teams::StageWorkItemRow>(
        "SELECT * FROM stage_work_items WHERE id=$1 FOR SHARE",
    )
    .bind(row.child_work_item_id)
    .fetch_one(&mut **tx)
    .await?;
    let worker = sqlx::query_as::<_, stage_worker_runs::StageWorkerRunRow>(
        "SELECT * FROM stage_worker_runs WHERE id=$1 FOR SHARE",
    )
    .bind(row.child_worker_run_id)
    .fetch_one(&mut **tx)
    .await?;
    let dispatch = sqlx::query_as::<_, unified_investigation_runtime::PentagiLogicalDispatchRow>(
        "SELECT dispatch_receipt_id,stable_request_id,logical_dispatch_key_sha256,task_plan_id,subtask_id,parent_dispatch_receipt_id,dispatch_ordinal,actor_kind,stage_work_item_id,stage_worker_request_id,worker_run_id,operation_id,stage_execution_id,stage_run_unit_id,scope_snapshot_id,organization_id,transcript_request_id,parent_actor_transcript_request_id,parent_dispatch_tool_request_id,snapshot_sha256,receipt_sha256 FROM pentagi_logical_dispatch_receipts WHERE dispatch_receipt_id=$1",
    )
    .bind(row.child_dispatch_receipt_id)
    .fetch_one(&mut **tx)
    .await?;
    let chain_exact: bool = sqlx::query_scalar(
        "SELECT EXISTS(SELECT 1 FROM message_chains WHERE id=$1 AND task_id=$2)",
    )
    .bind(row.child_message_chain_id)
    .bind(row.operation_id)
    .fetch_one(&mut **tx)
    .await?;
    if work_item.status != "running"
        || worker.status != "running"
        || worker.work_item_id != Some(work_item.id)
        || worker.message_chain_id != Some(row.child_message_chain_id)
        || worker
            .lease_expires_at
            .is_none_or(|expires| expires <= chrono::Utc::now())
        || dispatch.actor_kind != "nested_worker"
        || dispatch.worker_run_id != worker.id
        || !chain_exact
    {
        return Err(InvestigationNestedDispatchStoreError::ReplayConflict(
            "begin_child_not_live",
        ));
    }
    Ok(BegunInvestigationNestedDispatchRow {
        begin_receipt_id: row.begin_receipt_id,
        stable_request_id: row.stable_request_id,
        task_plan_id: row.task_plan_id,
        subtask_id: row.subtask_id,
        parent_dispatch_receipt_id: row.parent_dispatch_receipt_id,
        stage_worker_request_id: row.stage_worker_request_id,
        args_sha256: row.args_sha256,
        request_sha256: row.request_sha256,
        begin_receipt_sha256: row.begin_receipt_sha256,
        unit: locked.unit,
        plan: locked.plan,
        work_item,
        worker,
        message_chain_id: row.child_message_chain_id,
        dispatch,
        replayed,
    })
}

fn validate_finish(
    input: &FinishInvestigationNestedDispatchRow,
) -> InvestigationNestedDispatchStoreResult<()> {
    if [
        input.authority_id,
        input.operation_id,
        input.stage_execution_id,
        input.stage_run_unit_id,
        input.scope_snapshot_id,
        input.organization_id,
        input.stable_request_id,
        input.begin_receipt_id,
        input.task_plan_id,
        input.subtask_id,
        input.parent_dispatch_receipt_id,
        input.dispatch_receipt_id,
        input.stage_team_plan_id,
        input.work_item_id,
    ]
    .into_iter()
    .any(|id| id.is_nil())
        || input.child_fence.operation_id != input.operation_id
        || input.child_fence.stage_execution_id != input.stage_execution_id
        || input.child_fence.stage_run_unit_id != input.stage_run_unit_id
        || input.output.fence != input.child_fence
        || input.output.team_plan_id != input.stage_team_plan_id
        || input.output.work_item_id != input.work_item_id
        || input.output.expected_work_item_row_version != input.expected_work_item_row_version
        || input.output.output_schema != COGNITIVE_OUTPUT_SCHEMA
        || !valid_sha256(&input.output.output_hash)
        || !valid_sha256(&input.result_sha256)
        || !valid_sha256(&input.fence_sha256)
    {
        return Err(InvestigationNestedDispatchStoreError::InvalidInput(
            "finish_contract",
        ));
    }
    Ok(())
}

fn validate_finish_owner(
    begin: &BeginReceiptRow,
    input: &FinishInvestigationNestedDispatchRow,
) -> InvestigationNestedDispatchStoreResult<()> {
    if begin.authority_id != input.authority_id
        || begin.operation_id != input.operation_id
        || begin.stage_execution_id != input.stage_execution_id
        || begin.owning_stage_run_request_id != input.owning_stage_run_request_id
        || begin.stage_run_unit_id != input.stage_run_unit_id
        || begin.scope_snapshot_id != input.scope_snapshot_id
        || begin.organization_id != input.organization_id
        || begin.task_plan_id != input.task_plan_id
        || begin.subtask_id != input.subtask_id
        || begin.parent_dispatch_receipt_id != input.parent_dispatch_receipt_id
        || begin.child_dispatch_receipt_id != input.dispatch_receipt_id
        || begin.child_worker_run_id != input.child_fence.worker_run_id
        || begin.child_work_item_id != input.work_item_id
        || begin.stage_team_plan_id != input.stage_team_plan_id
    {
        return Err(InvestigationNestedDispatchStoreError::AuthorityMismatch(
            "finish_owner_mismatch",
        ));
    }
    Ok(())
}

async fn load_finish(
    tx: &mut Transaction<'_, Postgres>,
    stable_request_id: Uuid,
) -> Result<Option<FinishReceiptRow>, sqlx::Error> {
    sqlx::query_as(
        "SELECT * FROM investigation_nested_dispatch_finishes WHERE stable_request_id=$1 FOR SHARE",
    )
    .bind(stable_request_id)
    .fetch_optional(&mut **tx)
    .await
}

fn validate_finish_replay(
    row: &FinishReceiptRow,
    input: &FinishInvestigationNestedDispatchRow,
    finish_receipt_id: Uuid,
    dispatch_attempt_id: Uuid,
    finish_receipt_sha256: &str,
) -> InvestigationNestedDispatchStoreResult<()> {
    if row.finish_receipt_id != finish_receipt_id
        || row.stable_request_id != input.stable_request_id
        || row.begin_receipt_id != input.begin_receipt_id
        || row.task_plan_id != input.task_plan_id
        || row.subtask_id != input.subtask_id
        || row.parent_dispatch_receipt_id != input.parent_dispatch_receipt_id
        || row.child_dispatch_receipt_id != input.dispatch_receipt_id
        || row.child_worker_run_id != input.child_fence.worker_run_id
        || row.child_work_item_id != input.work_item_id
        || row.child_lease_token != input.child_fence.lease_token
        || row.child_attempt_epoch != input.child_fence.attempt_epoch
        || row.child_checkpoint_version != input.child_fence.expected_checkpoint_version
        || row.dispatch_attempt_id != dispatch_attempt_id
        || row.outcome != input.outcome.as_str()
        || row.result_sha256 != input.result_sha256
        || row.fence_sha256 != input.fence_sha256
        || row.finish_receipt_sha256 != finish_receipt_sha256
    {
        return Err(InvestigationNestedDispatchStoreError::ReplayConflict(
            "finish_material_mismatch",
        ));
    }
    Ok(())
}

async fn load_finished(
    tx: &mut Transaction<'_, Postgres>,
    row: FinishReceiptRow,
    input: &FinishInvestigationNestedDispatchRow,
    replayed: bool,
) -> InvestigationNestedDispatchStoreResult<FinishedInvestigationNestedDispatchRow> {
    let unit = sqlx::query_as::<_, stage_run_units::StageRunUnitRow>(
        "SELECT * FROM stage_run_units WHERE id=$1",
    )
    .bind(input.stage_run_unit_id)
    .fetch_one(&mut **tx)
    .await?;
    let plan = sqlx::query_as::<_, stage_teams::StageTeamPlanRow>(
        "SELECT * FROM stage_team_plans WHERE id=$1",
    )
    .bind(input.stage_team_plan_id)
    .fetch_one(&mut **tx)
    .await?;
    let work_item = sqlx::query_as::<_, stage_teams::StageWorkItemRow>(
        "SELECT * FROM stage_work_items WHERE id=$1",
    )
    .bind(row.child_work_item_id)
    .fetch_one(&mut **tx)
    .await?;
    let worker = sqlx::query_as::<_, stage_worker_runs::StageWorkerRunRow>(
        "SELECT * FROM stage_worker_runs WHERE id=$1",
    )
    .bind(row.child_worker_run_id)
    .fetch_one(&mut **tx)
    .await?;
    let output = sqlx::query_as::<_, stage_teams::StageWorkerOutputRow>(
        "SELECT * FROM stage_worker_outputs WHERE id=$1",
    )
    .bind(row.output_id)
    .fetch_one(&mut **tx)
    .await?;
    let dispatch_attempt = sqlx::query_as::<_, unified_investigation_runtime::PentagiDispatchAttemptRow>(
        "SELECT dispatch_attempt_id,stable_request_id,dispatch_receipt_id,attempt_epoch,lease_token,fence_sha256,outcome,result_sha256 FROM pentagi_logical_dispatch_attempts WHERE dispatch_attempt_id=$1",
    )
    .bind(row.dispatch_attempt_id)
    .fetch_one(&mut **tx)
    .await?;
    if work_item.status != "completed"
        || worker.status != "passed"
        || output.work_item_id != work_item.id
        || output.worker_run_id != worker.id
        || dispatch_attempt.dispatch_receipt_id != row.child_dispatch_receipt_id
    {
        return Err(InvestigationNestedDispatchStoreError::ReplayConflict(
            "finish_terminal_rows_mismatch",
        ));
    }
    Ok(FinishedInvestigationNestedDispatchRow {
        finish_receipt_id: row.finish_receipt_id,
        stable_request_id: row.stable_request_id,
        begin_receipt_id: row.begin_receipt_id,
        task_plan_id: row.task_plan_id,
        subtask_id: row.subtask_id,
        parent_dispatch_receipt_id: row.parent_dispatch_receipt_id,
        dispatch_receipt_id: row.child_dispatch_receipt_id,
        result_sha256: row.result_sha256,
        finish_receipt_sha256: row.finish_receipt_sha256,
        completion: stage_teams::CompletedStageWorkerRow {
            unit,
            plan,
            work_item,
            worker,
            output,
            replayed,
        },
        dispatch_attempt,
        replayed,
    })
}
