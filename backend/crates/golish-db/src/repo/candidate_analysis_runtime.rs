//! Durable production coordinator for Candidate hypothesis analysis.
//!
//! This module deliberately owns only scheduler identities, leases, response-
//! loss receipts and the transition between immutable Candidate phases. The
//! semantic artifact writers and Gate aggregate remain in
//! [`super::candidate_analysis`] and [`super::hypothesis_registry`].

use std::collections::BTreeSet;

use golish_core::hypothesis_semantic_key::{
    candidate_revision_id, initial_root_id, AtTimeSubjectIdentity, ClaimPolarity,
    HypothesisSemanticKeyV1, PredicateIdentity,
};
use serde_json::{json, Map, Value};
use sha2::{Digest, Sha256};
use sqlx::{PgPool, Postgres, Transaction};
use uuid::Uuid;

use super::candidate_analysis::{
    candidate_chunk_page_hash_on, CandidateChunkPageHashInput, CandidateWriteFenceRow,
};
use crate::{DbError, Result};

const LEASE_SECONDS: i32 = 86_400;
const COORDINATOR_CONTRACT: &str = "candidate_analysis_runtime.v1";

fn conflict(code: &'static str) -> DbError {
    DbError::Other(anyhow::anyhow!(code))
}

async fn hash_json_on(tx: &mut Transaction<'_, Postgres>, value: &Value) -> Result<String> {
    Ok(
        sqlx::query_scalar("SELECT tool_truth_sha256(($1::JSONB)::TEXT)")
            .bind(value)
            .fetch_one(&mut **tx)
            .await?,
    )
}

async fn hash_texts_on(tx: &mut Transaction<'_, Postgres>, values: &[String]) -> Result<String> {
    Ok(
        sqlx::query_scalar("SELECT tool_truth_sha256(to_jsonb($1::TEXT[])::TEXT)")
            .bind(values)
            .fetch_one(&mut **tx)
            .await?,
    )
}

#[derive(Debug, Clone, PartialEq)]
pub struct CandidateRuntimeAttemptRow {
    pub analysis_attempt_id: Uuid,
    pub analysis_attempt_ordinal: i32,
    pub controller_fence: CandidateWriteFenceRow,
    pub snapshot_authority_hash: String,
    pub input_count: i64,
    pub input_chunk_census_set_hash: String,
    pub relationship_cross_index_hash: String,
    pub missed_hypothesis_signals: Vec<Value>,
    pub missed_hypothesis_signal_set_hash: String,
    pub dispatch_replay: Option<CandidateProviderArtifactRow>,
}

#[derive(Debug, Clone, PartialEq)]
pub struct CandidateRuntimeWorkRow {
    pub fence: CandidateWriteFenceRow,
    pub phase: String,
    pub capability: String,
    pub lane_ordinal: i32,
    pub input: Value,
    pub replayed_receipt: Option<CandidateArtifactReceiptRow>,
}

#[derive(Debug, Clone, PartialEq)]
pub struct CandidateProviderArtifactRow {
    pub provider_attempt_id: Uuid,
    pub body: Value,
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct CandidateArtifactReceiptRow {
    pub artifact_id: Uuid,
    pub artifact_hash: String,
    pub replayed: bool,
}

#[derive(Debug, Clone)]
pub struct PersistCandidateWorkerArtifact {
    pub fence: CandidateWriteFenceRow,
    pub provider_attempt_id: Uuid,
    pub artifact_kind: String,
    pub artifact_body: Value,
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub enum CandidateAuthorityRevalidationRow {
    Fresh,
    Invalidated {
        replacement_snapshot_id: Uuid,
        residual_hash: String,
    },
}

/// Re-evaluates mutable temporal/feed heads against a frozen Candidate
/// snapshot using the database clock. On authority drift, this creates the
/// deterministic replacement snapshot through the same checked Plan A
/// callback used by the initial freeze; no caller-supplied roots or time are
/// accepted.
pub async fn revalidate_candidate_runtime_authority(
    pool: &PgPool,
    snapshot_id: Uuid,
    analysis_attempt_id: Uuid,
) -> Result<CandidateAuthorityRevalidationRow> {
    let mut tx = pool.begin().await?;
    let identity: (Uuid, Uuid, Uuid) = sqlx::query_as(
        r#"SELECT snapshot.operation_id,snapshot.scope_snapshot_id,snapshot.organization_id
             FROM candidate_analysis_snapshots snapshot
             JOIN candidate_analysis_attempts attempt
               ON attempt.snapshot_id=snapshot.snapshot_id
            WHERE snapshot.snapshot_id=$1 AND attempt.analysis_attempt_id=$2"#,
    )
    .bind(snapshot_id)
    .bind(analysis_attempt_id)
    .fetch_optional(&mut *tx)
    .await?
    .ok_or_else(|| conflict("CANDIDATE_AUTHORITY_REVALIDATION_SCOPE_INVALID"))?;
    match super::candidate_analysis::reevaluate_candidate_gate_authority_on(&mut tx, snapshot_id)
        .await
    {
        Ok(_) => {
            tx.commit().await?;
            Ok(CandidateAuthorityRevalidationRow::Fresh)
        }
        Err(error)
            if error
                .to_string()
                .contains("HYPOTHESIS_REGISTRY_AUTHORITY_MISMATCH") =>
        {
            tx.rollback().await?;
            let stable_consumer_request_id = Uuid::new_v5(
                &analysis_attempt_id,
                b"candidate_authority_revalidation_replacement.v1",
            );
            let replacement = super::candidate_analysis::freeze_candidate_snapshot(
                pool,
                super::candidate_analysis::FreezeCandidateSnapshotInput {
                    stable_consumer_request_id,
                    operation_id: identity.0,
                    scope_snapshot_id: identity.1,
                    organization_id: identity.2,
                },
            )
            .await?;
            let residual_hash: String = sqlx::query_scalar(
                "SELECT tool_truth_sha256(($1::JSONB)::TEXT)",
            )
            .bind(json!({
                "domain":"candidate_authority_invalidated.v1",
                "snapshot_id":snapshot_id,
                "analysis_attempt_id":analysis_attempt_id,
                "replacement_snapshot_id":replacement.snapshot_id,
                "replacement_disposition":match replacement.disposition {
                    super::candidate_analysis::CandidateSnapshotDispositionRow::SealedReady => "sealed_ready",
                    super::candidate_analysis::CandidateSnapshotDispositionRow::BlockedAuthorityBundle => "blocked_authority_bundle",
                },
            }))
            .fetch_one(pool)
            .await?;
            Ok(CandidateAuthorityRevalidationRow::Invalidated {
                replacement_snapshot_id: replacement.snapshot_id,
                residual_hash,
            })
        }
        Err(error) => Err(error),
    }
}

#[derive(Debug, Clone, PartialEq)]
pub struct CandidateHostCompilationRecipeRow {
    pub stable_compilation_request_id: Uuid,
    pub stable_apply_request_id: Uuid,
    pub controller_fence: CandidateWriteFenceRow,
    pub expected_source_head_version: i64,
    pub recipe: Value,
    pub material_hash: String,
    pub controller_final_input: Value,
}

#[derive(Debug, Clone)]
pub struct PersistHostCompilationMaterial {
    pub snapshot_id: Uuid,
    pub stage_execution_id: Uuid,
    pub attempt_ordinal: i32,
    pub compiler_recipe: Value,
    pub mutations: Value,
    pub mutation_count: i64,
    pub mutation_set_hash: String,
    pub claim_component_count: i64,
    pub claim_component_set_hash: String,
    pub verification_contract_count: i64,
    pub verification_contract_set_hash: String,
    pub verification_plan_count: i64,
    pub verification_plan_set_hash: String,
    pub generation_transition_set_hash: String,
}

#[derive(Debug, sqlx::FromRow)]
struct HostCompilationMaterialDbRow {
    compilation_material_id: Uuid,
    stable_compilation_request_id: Uuid,
    stable_apply_request_id: Uuid,
    analysis_attempt_id: Uuid,
    snapshot_id: Uuid,
    operation_id: Uuid,
    organization_id: Uuid,
    final_submitter_worker_run_id: Uuid,
    compiler_recipe: Value,
    mutations: Value,
    input_dispositions: Value,
    input_relations: Value,
    mutation_count: i64,
    mutation_set_hash: String,
    claim_component_count: i64,
    claim_component_set_hash: String,
    verification_contract_count: i64,
    verification_contract_set_hash: String,
    verification_plan_count: i64,
    verification_plan_set_hash: String,
    generation_transition_count: i64,
    generation_transition_set_hash: String,
    material_hash: String,
}

#[derive(Debug, sqlx::FromRow)]
struct SchedulerIdentityRow {
    operation_id: Uuid,
    scope_snapshot_id: Uuid,
    organization_id: Uuid,
    snapshot_authority_hash: String,
    attempt_id: Uuid,
    attempt_ordinal: i32,
    stage_execution_id: Uuid,
    stage_run_unit_id: Uuid,
    unit_generation: i32,
}

fn team_plan_id(unit_id: Uuid) -> Uuid {
    Uuid::new_v5(&unit_id, b"candidate-analysis-team.v1")
}

fn work_item_id(plan_id: Uuid, stable_key: &str) -> Uuid {
    Uuid::new_v5(&plan_id, stable_key.as_bytes())
}

fn worker_id(work_item_id: Uuid) -> Uuid {
    Uuid::new_v5(&work_item_id, b"candidate-worker.v1")
}

fn lease_token(worker_id: Uuid) -> Uuid {
    Uuid::new_v5(&worker_id, b"candidate-worker-lease.v1")
}

#[allow(clippy::too_many_arguments)]
async fn ensure_queued_work_on(
    tx: &mut Transaction<'_, Postgres>,
    identity: &SchedulerIdentityRow,
    plan_id: Uuid,
    phase: &str,
    capability: &str,
    stable_key: &str,
    candidate_microbatch_key: Option<&str>,
    component_id: Option<Uuid>,
    lane_ordinal: i32,
    input: &Value,
) -> Result<(Uuid, Uuid)> {
    let worker_role = if capability.starts_with("candidate_controller") {
        "controller"
    } else {
        phase
    };
    let candidate_phase = if phase == "analyst" {
        "proposal"
    } else {
        phase
    };
    let item_id = work_item_id(plan_id, stable_key);
    let worker_id = worker_id(item_id);
    let candidate_item_id = Uuid::new_v5(&identity.attempt_id, item_id.as_bytes());
    let created_by = if matches!(
        capability,
        "candidate_controller_dispatch" | "hypothesis_proposal"
    ) {
        "server_seed"
    } else {
        "server_phase_transition"
    };
    let input_hash = hash_json_on(tx, input).await?;
    let item_exists: bool =
        sqlx::query_scalar("SELECT EXISTS(SELECT 1 FROM stage_work_items WHERE id=$1)")
            .bind(item_id)
            .fetch_one(&mut **tx)
            .await?;
    if item_exists {
        let exact_replay: bool = sqlx::query_scalar(
            r#"SELECT EXISTS(
                   SELECT 1
                     FROM stage_work_items item
                     JOIN candidate_analysis_work_items candidate
                       ON candidate.stage_work_item_id=item.id
                     JOIN stage_worker_runs worker
                       ON worker.work_item_id=item.id
                    WHERE item.id=$1 AND item.team_plan_id=$2
                      AND item.operation_id=$3 AND item.stage_execution_id=$4
                      AND item.stage_run_unit_id=$5 AND item.scope_snapshot_id=$6
                      AND item.organization_id=$7 AND item.kind=$8
                      AND item.stable_key=$9 AND item.role=$10
                      AND item.input_manifest_hash=$11
                      AND item.created_by=$18
                      AND candidate.candidate_work_item_id=$12
                      AND candidate.analysis_attempt_id=$13
                      AND candidate.phase=$14 AND candidate.capability=$8
                      AND candidate.microbatch_key IS NOT DISTINCT FROM $15
                      AND candidate.component_id IS NOT DISTINCT FROM $16
                      AND candidate.work_item_hash=$11
                      AND worker.id=$17 AND worker.specialist=$10
                      AND worker.work_item_kind=$8 AND worker.work_item_key=$9)"#,
        )
        .bind(item_id)
        .bind(plan_id)
        .bind(identity.operation_id)
        .bind(identity.stage_execution_id)
        .bind(identity.stage_run_unit_id)
        .bind(identity.scope_snapshot_id)
        .bind(identity.organization_id)
        .bind(capability)
        .bind(stable_key)
        .bind(worker_role)
        .bind(&input_hash)
        .bind(candidate_item_id)
        .bind(identity.attempt_id)
        .bind(candidate_phase)
        .bind(candidate_microbatch_key)
        .bind(component_id)
        .bind(worker_id)
        .bind(created_by)
        .fetch_one(&mut **tx)
        .await?;
        if !exact_replay {
            return Err(conflict("CANDIDATE_WORK_REPLAY_DRIFT"));
        }
        return Ok((item_id, worker_id));
    }
    sqlx::query(
        r#"INSERT INTO stage_work_items(
               id,team_plan_id,operation_id,stage_execution_id,stage_run_unit_id,
               scope_snapshot_id,organization_id,dispatch_epoch,kind,stable_key,role,
               input_manifest_hash,input_refs,required_for_barrier,priority,status,
               attempt_policy,budget,output_schema,created_by)
           VALUES($1,$2,$3,$4,$5,$6,$7,0,$8,$9,$10,$11,'[]',TRUE,$12,'queued',
                  '{"max_attempts":1}','{}','candidate_analysis_artifact_receipt.v1',$13)
           ON CONFLICT(team_plan_id,kind,stable_key) DO NOTHING"#,
    )
    .bind(item_id)
    .bind(plan_id)
    .bind(identity.operation_id)
    .bind(identity.stage_execution_id)
    .bind(identity.stage_run_unit_id)
    .bind(identity.scope_snapshot_id)
    .bind(identity.organization_id)
    .bind(capability)
    .bind(stable_key)
    .bind(worker_role)
    .bind(&input_hash)
    .bind(lane_ordinal)
    .bind(created_by)
    .execute(&mut **tx)
    .await?;
    sqlx::query(
        r#"INSERT INTO candidate_analysis_work_items(
               candidate_work_item_id,stage_work_item_id,analysis_attempt_id,phase,
               capability,microbatch_key,component_id,work_item_hash)
           VALUES($1,$2,$3,$4,$5,$6,$7,$8)
           ON CONFLICT(stage_work_item_id) DO NOTHING"#,
    )
    .bind(candidate_item_id)
    .bind(item_id)
    .bind(identity.attempt_id)
    .bind(candidate_phase)
    .bind(capability)
    .bind(candidate_microbatch_key)
    .bind(component_id)
    .bind(&input_hash)
    .execute(&mut **tx)
    .await?;
    sqlx::query(
        r#"INSERT INTO stage_worker_runs(
               id,operation_id,stage_execution_id,stage_run_unit_id,work_item_id,
               organization_id,worker_generation,specialist,work_item_kind,
               work_item_key,agent_path,status)
           VALUES($1,$2,$3,$4,$5,$6,0,$7,$8,$9,$10,'queued')
           ON CONFLICT(stage_run_unit_id,work_item_kind,work_item_key,worker_generation)
           DO NOTHING"#,
    )
    .bind(worker_id)
    .bind(identity.operation_id)
    .bind(identity.stage_execution_id)
    .bind(identity.stage_run_unit_id)
    .bind(item_id)
    .bind(identity.organization_id)
    .bind(worker_role)
    .bind(capability)
    .bind(stable_key)
    .bind(format!("candidate/{phase}/{lane_ordinal}"))
    .execute(&mut **tx)
    .await?;
    Ok((item_id, worker_id))
}

async fn claim_or_replay_queued_work_on(
    tx: &mut Transaction<'_, Postgres>,
    identity: &SchedulerIdentityRow,
    plan_id: Uuid,
    capability: &str,
    item_id: Uuid,
    worker_id: Uuid,
) -> Result<Option<CandidateWriteFenceRow>> {
    let state: Option<(String, String)> = sqlx::query_as(
        r#"SELECT item.status,worker.status
              FROM stage_work_items item
              JOIN stage_worker_runs worker ON worker.work_item_id=item.id
             WHERE item.id=$1 AND worker.id=$2
             FOR UPDATE OF item,worker"#,
    )
    .bind(item_id)
    .bind(worker_id)
    .fetch_optional(&mut **tx)
    .await?;
    let (item_status, worker_status) =
        state.ok_or_else(|| conflict("CANDIDATE_WORK_IDENTITY_MISSING"))?;
    match (item_status.as_str(), worker_status.as_str()) {
        ("completed", "passed")
            if matches!(
                capability,
                "candidate_controller_dispatch" | "candidate_controller_final"
            ) => {}
        ("completed", "passed") => return Ok(None),
        ("running", "running") => {}
        ("queued", "queued") => {
            let token = lease_token(worker_id);
            sqlx::query(
                r#"UPDATE stage_work_items
                      SET status='running',started_at=COALESCE(started_at,statement_timestamp()),
                          row_version=row_version+1,updated_at=statement_timestamp()
                    WHERE id=$1"#,
            )
            .bind(item_id)
            .execute(&mut **tx)
            .await?;
            sqlx::query(
                r#"UPDATE stage_worker_runs
                      SET status='running',lease_token=$2,lease_owner=$3,
                          lease_acquired_at=statement_timestamp(),
                          lease_expires_at=statement_timestamp()+make_interval(secs=>$4),
                          heartbeat_at=statement_timestamp(),attempt_epoch=attempt_epoch+1,
                          started_at=COALESCE(started_at,statement_timestamp()),
                          updated_at=statement_timestamp()
                    WHERE id=$1"#,
            )
            .bind(worker_id)
            .bind(token)
            .bind(format!(
                "candidate-runtime:{identity_id}",
                identity_id = identity.attempt_id
            ))
            .bind(LEASE_SECONDS)
            .execute(&mut **tx)
            .await?;
        }
        _ => return Err(conflict("CANDIDATE_WORK_STATE_DRIFT")),
    }
    if capability == "candidate_controller_final" {
        let existing_final: Option<Uuid> = sqlx::query_scalar(
            "SELECT final_submitter_worker_run_id FROM stage_team_plans WHERE id=$1 FOR UPDATE",
        )
        .bind(plan_id)
        .fetch_one(&mut **tx)
        .await?;
        match existing_final {
            Some(existing_worker) if existing_worker == worker_id => {}
            Some(_) => return Err(conflict("CANDIDATE_CONTROLLER_FINAL_BINDING_DRIFT")),
            None => {
                sqlx::query(
                    r#"UPDATE stage_team_plans
                          SET requests_closed_at=COALESCE(requests_closed_at,statement_timestamp()),
                              final_submitter_worker_run_id=$2,row_version=row_version+1,
                              updated_at=statement_timestamp()
                        WHERE id=$1 AND final_submitter_worker_run_id IS NULL"#,
                )
                .bind(plan_id)
                .bind(worker_id)
                .execute(&mut **tx)
                .await?;
            }
        }
    }
    let (plan_version, item_version, checkpoint_version, attempt_epoch, persisted_token): (
        i64,
        i64,
        i64,
        i64,
        Option<Uuid>,
    ) = sqlx::query_as(
        r#"SELECT plan.row_version,item.row_version,worker.checkpoint_version,
                  worker.attempt_epoch,worker.lease_token
             FROM stage_team_plans plan
             JOIN stage_work_items item ON item.team_plan_id=plan.id
             JOIN stage_worker_runs worker ON worker.work_item_id=item.id
            WHERE plan.id=$1 AND item.id=$2 AND worker.id=$3 FOR SHARE"#,
    )
    .bind(plan_id)
    .bind(item_id)
    .bind(worker_id)
    .fetch_one(&mut **tx)
    .await?;
    let persisted_token =
        persisted_token.ok_or_else(|| conflict("CANDIDATE_WORKER_LEASE_MISSING"))?;
    Ok(Some(CandidateWriteFenceRow {
        operation_id: identity.operation_id,
        scope_snapshot_id: identity.scope_snapshot_id,
        organization_id: identity.organization_id,
        snapshot_id: sqlx::query_scalar(
            "SELECT snapshot_id FROM candidate_analysis_attempts WHERE analysis_attempt_id=$1",
        )
        .bind(identity.attempt_id)
        .fetch_one(&mut **tx)
        .await?,
        team_plan_id: plan_id,
        work_item_id: item_id,
        worker_run_id: worker_id,
        lease_token: persisted_token,
        lease_epoch: attempt_epoch,
        analysis_attempt_id: identity.attempt_id,
        analysis_attempt_ordinal: identity.attempt_ordinal,
        attempt_epoch,
        expected_snapshot_row_version: 0,
        expected_team_plan_row_version: plan_version,
        expected_work_item_row_version: item_version,
        expected_worker_row_version: checkpoint_version,
        expected_attempt_row_version: 0,
    }))
}

async fn available_live_lanes_on(
    tx: &mut Transaction<'_, Postgres>,
    plan_id: Uuid,
    host_lane_limit: i32,
) -> Result<usize> {
    let running: i64 = sqlx::query_scalar(
        r#"SELECT count(*)
              FROM stage_worker_runs worker
              JOIN stage_work_items item ON item.id=worker.work_item_id
             WHERE item.team_plan_id=$1 AND worker.status='running'
               AND worker.terminal_at IS NULL
               AND worker.lease_expires_at>statement_timestamp()"#,
    )
    .bind(plan_id)
    .fetch_one(&mut **tx)
    .await?;
    let limit = i64::from(host_lane_limit);
    usize::try_from((limit - running).max(0))
        .map_err(|_| conflict("CANDIDATE_LIVE_LANE_COUNT_INVALID"))
}

async fn load_scheduler_identity_on(
    tx: &mut Transaction<'_, Postgres>,
    snapshot_id: Uuid,
    stage_execution_id: Uuid,
    attempt_ordinal: i32,
) -> Result<SchedulerIdentityRow> {
    sqlx::query_as(
        r#"SELECT snapshot.operation_id,snapshot.scope_snapshot_id,
                  snapshot.organization_id,snapshot.candidate_snapshot_authority_hash
                  AS snapshot_authority_hash,attempt.analysis_attempt_id AS attempt_id,
                  attempt.attempt_ordinal,unit.stage_execution_id,
                  unit.id AS stage_run_unit_id,
                  unit.generation AS unit_generation
             FROM candidate_analysis_snapshots snapshot
             JOIN candidate_analysis_attempts attempt
               ON attempt.snapshot_id=snapshot.snapshot_id
             JOIN stage_run_units unit
               ON unit.operation_id=snapshot.operation_id
              AND unit.scope_snapshot_id=snapshot.scope_snapshot_id
              AND unit.organization_id=snapshot.organization_id
              AND unit.stage_execution_id=$2
              AND unit.stage_kind='attack_candidate'
            WHERE snapshot.snapshot_id=$1
              AND snapshot.snapshot_status='sealed_ready'
              AND attempt.attempt_ordinal=$3
            FOR SHARE OF snapshot,attempt,unit"#,
    )
    .bind(snapshot_id)
    .bind(stage_execution_id)
    .bind(attempt_ordinal)
    .fetch_optional(&mut **tx)
    .await?
    .ok_or_else(|| conflict("CANDIDATE_ANALYSIS_RUNTIME_IDENTITY_UNAVAILABLE"))
}

async fn ensure_team_plan_on(
    tx: &mut Transaction<'_, Postgres>,
    identity: &SchedulerIdentityRow,
) -> Result<Uuid> {
    let plan_id = team_plan_id(identity.stage_run_unit_id);
    let plan_hash = hash_json_on(
        tx,
        &json!({
            "contract": COORDINATOR_CONTRACT,
            "snapshot_authority_hash": identity.snapshot_authority_hash,
            "roles": ["controller", "analyst", "critic"],
        }),
    )
    .await?;
    if let Some(persisted) = sqlx::query_as::<_, (Uuid, String)>(
        "SELECT id,plan_hash FROM stage_team_plans WHERE stage_run_unit_id=$1 FOR SHARE",
    )
    .bind(identity.stage_run_unit_id)
    .fetch_optional(&mut **tx)
    .await?
    {
        if persisted != (plan_id, plan_hash) {
            return Err(conflict("CANDIDATE_ANALYSIS_TEAM_PLAN_DRIFT"));
        }
        return Ok(plan_id);
    }
    sqlx::query(
        r#"INSERT INTO stage_team_plans(
               id,operation_id,stage_execution_id,stage_run_unit_id,scope_snapshot_id,
               organization_id,stage_kind,unit_generation,schema_version,plan_version,
               plan_hash,leader_role,aggregator_kind,aggregator_role,allowed_worker_roles,
               max_workers_total,max_workers_active,dynamic_requests_allowed,
               dynamic_request_policy,final_submitter_kind,created_from_stage_spec_hash)
           VALUES($1,$2,$3,$4,$5,$6,'attack_candidate',$7,1,1,$8,'controller',
                  'worker','controller',$9,100000,8,FALSE,$10,'worker',$8)
           "#,
    )
    .bind(plan_id)
    .bind(identity.operation_id)
    .bind(identity.stage_execution_id)
    .bind(identity.stage_run_unit_id)
    .bind(identity.scope_snapshot_id)
    .bind(identity.organization_id)
    .bind(identity.unit_generation)
    .bind(&plan_hash)
    .bind(json!(["controller", "analyst", "critic"]))
    .bind(json!({"coordination_mode":"candidate_analysis"}))
    .execute(&mut **tx)
    .await?;
    let persisted: (Uuid, String) = sqlx::query_as(
        "SELECT id,plan_hash FROM stage_team_plans WHERE stage_run_unit_id=$1 FOR SHARE",
    )
    .bind(identity.stage_run_unit_id)
    .fetch_one(&mut **tx)
    .await?;
    if persisted != (plan_id, plan_hash) {
        return Err(conflict("CANDIDATE_ANALYSIS_TEAM_PLAN_DRIFT"));
    }
    Ok(plan_id)
}

#[allow(clippy::too_many_arguments)]
async fn ensure_claimed_work_on(
    tx: &mut Transaction<'_, Postgres>,
    identity: &SchedulerIdentityRow,
    plan_id: Uuid,
    phase: &str,
    capability: &str,
    stable_key: &str,
    candidate_microbatch_key: Option<&str>,
    component_id: Option<Uuid>,
    lane_ordinal: i32,
    input: &Value,
) -> Result<CandidateWriteFenceRow> {
    let (item_id, worker_id) = ensure_queued_work_on(
        tx,
        identity,
        plan_id,
        phase,
        capability,
        stable_key,
        candidate_microbatch_key,
        component_id,
        lane_ordinal,
        input,
    )
    .await?;
    claim_or_replay_queued_work_on(tx, identity, plan_id, capability, item_id, worker_id)
        .await?
        .ok_or_else(|| conflict("CANDIDATE_WORK_ALREADY_COMPLETED"))
}

async fn load_retry_missed_signals_on(
    tx: &mut Transaction<'_, Postgres>,
    analysis_attempt_id: Uuid,
) -> Result<(Vec<Value>, String)> {
    let rows: Vec<(Uuid, String, i32, String, String, Uuid)> = sqlx::query_as(
        r#"WITH missed(checklist_member_id) AS (
               SELECT jsonb_array_elements_text(subreview.typed_missed_refs)::UUID
                 FROM candidate_analysis_hypothesis_coverage_subreviews subreview
                WHERE subreview.analysis_attempt_id=$1
                  AND subreview.outcome='missed_hypothesis'
               UNION
               SELECT jsonb_array_elements_text(review.typed_missed_refs)::UUID
                 FROM candidate_analysis_hypothesis_coverage_synthesis_reviews review
                WHERE review.analysis_attempt_id=$1
                  AND review.outcome='missed_hypothesis'
           )
           SELECT checklist.checklist_member_id,checklist.attack_class_id,
                  checklist.attack_class_version,checklist.trust_boundary_identity,
                  checklist.trust_boundary_hash,checklist.snapshot_input_id
             FROM missed
             JOIN candidate_analysis_hypothesis_coverage_checklist_members checklist
               USING(checklist_member_id)
            WHERE checklist.analysis_attempt_id=$1
            ORDER BY checklist.checklist_member_id"#,
    )
    .bind(analysis_attempt_id)
    .fetch_all(&mut **tx)
    .await?;
    let signals = rows
        .into_iter()
        .map(
            |(
                checklist_member_id,
                attack_class_id,
                attack_class_version,
                boundary,
                hash,
                input_id,
            )| {
                json!({
                    "checklist_member_id":checklist_member_id,
                    "attack_class_id":attack_class_id,
                    "attack_class_version":attack_class_version,
                    "trust_boundary_identity":boundary,
                    "trust_boundary_hash":hash,
                    "covered_input_ids":[input_id],
                })
            },
        )
        .collect::<Vec<_>>();
    let mut signal_hashes = Vec::with_capacity(signals.len());
    for signal in &signals {
        signal_hashes.push(hash_json_on(tx, signal).await?);
    }
    let signal_set_hash = hash_texts_on(tx, &signal_hashes).await?;
    Ok((signals, signal_set_hash))
}

pub async fn open_or_replay_attempt_runtime(
    pool: &PgPool,
    snapshot_id: Uuid,
    stage_execution_id: Uuid,
    attempt_ordinal: i32,
) -> Result<CandidateRuntimeAttemptRow> {
    let mut tx = pool.begin().await?;
    let identity =
        load_scheduler_identity_on(&mut tx, snapshot_id, stage_execution_id, attempt_ordinal)
            .await?;
    let plan_id = ensure_team_plan_on(&mut tx, &identity).await?;
    let input_count: i64 = sqlx::query_scalar(
        "SELECT count(*) FROM candidate_analysis_snapshot_inputs WHERE snapshot_id=$1",
    )
    .bind(snapshot_id)
    .fetch_one(&mut *tx)
    .await?;
    let chunk_hashes: Vec<String> = sqlx::query_scalar(
        "SELECT census_hash FROM candidate_analysis_input_chunk_censuses WHERE snapshot_id=$1 ORDER BY snapshot_input_id",
    )
    .bind(snapshot_id)
    .fetch_all(&mut *tx)
    .await?;
    let input_chunk_census_set_hash = hash_texts_on(&mut tx, &chunk_hashes).await?;
    let relation_hashes: Vec<String> = sqlx::query_scalar(
        r#"SELECT member.member_hash
             FROM candidate_analysis_snapshot_source_sets source_set
             JOIN candidate_analysis_snapshot_source_set_members member USING(source_set_id,snapshot_id)
            WHERE source_set.snapshot_id=$1 AND source_set.source_kind='relations'
            ORDER BY member.ordinal"#,
    )
    .bind(snapshot_id)
    .fetch_all(&mut *tx)
    .await?;
    let relationship_cross_index_hash = hash_texts_on(&mut tx, &relation_hashes).await?;
    let predecessor_attempt_id: Option<Uuid> = sqlx::query_scalar(
        "SELECT predecessor_attempt_id FROM candidate_analysis_attempts WHERE analysis_attempt_id=$1",
    )
    .bind(identity.attempt_id)
    .fetch_one(&mut *tx)
    .await?;
    let (missed_hypothesis_signals, missed_hypothesis_signal_set_hash) =
        if let Some(predecessor_attempt_id) = predecessor_attempt_id {
            load_retry_missed_signals_on(&mut tx, predecessor_attempt_id).await?
        } else {
            (Vec::new(), hash_texts_on(&mut tx, &[]).await?)
        };
    let dispatch_input = json!({
        "snapshot_id": snapshot_id,
        "snapshot_authority_hash": identity.snapshot_authority_hash,
        "input_count": input_count,
        "input_chunk_census_set_hash": input_chunk_census_set_hash,
        "relationship_cross_index_hash": relationship_cross_index_hash,
        "missed_hypothesis_signals":&missed_hypothesis_signals,
        "missed_hypothesis_signal_set_hash":&missed_hypothesis_signal_set_hash,
    });
    let controller_fence = ensure_claimed_work_on(
        &mut tx,
        &identity,
        plan_id,
        "controller",
        "candidate_controller_dispatch",
        &format!("controller-dispatch:{}", identity.attempt_id),
        None,
        None,
        0,
        &dispatch_input,
    )
    .await?;
    let dispatch_replay = sqlx::query_as::<_, (Uuid, Value)>(
        r#"SELECT provider_attempt_id,artifact_body
             FROM candidate_analysis_provider_attempts
            WHERE analysis_attempt_id=$1 AND stage_work_item_id=$2
              AND artifact_kind='controller_dispatch.v1'"#,
    )
    .bind(identity.attempt_id)
    .bind(controller_fence.work_item_id)
    .fetch_optional(&mut *tx)
    .await?
    .map(|row| CandidateProviderArtifactRow {
        provider_attempt_id: row.0,
        body: row.1,
    });
    tx.commit().await?;
    Ok(CandidateRuntimeAttemptRow {
        analysis_attempt_id: identity.attempt_id,
        analysis_attempt_ordinal: identity.attempt_ordinal,
        controller_fence,
        snapshot_authority_hash: identity.snapshot_authority_hash,
        input_count,
        input_chunk_census_set_hash,
        relationship_cross_index_hash,
        missed_hypothesis_signals,
        missed_hypothesis_signal_set_hash,
        dispatch_replay,
    })
}

pub async fn persist_controller_dispatch(
    pool: &PgPool,
    fence: &CandidateWriteFenceRow,
    provider_attempt_id: Uuid,
    body: &Value,
) -> Result<()> {
    let mut tx = pool.begin().await?;
    let body_hash = hash_json_on(&mut tx, body).await?;
    let existing: Option<(Uuid, Uuid, String, String, String)> = sqlx::query_as(
        r#"SELECT receipt.provider_attempt_id,receipt.worker_run_id,receipt.artifact_hash,
                  item.status,worker.status
             FROM candidate_analysis_provider_attempts receipt
             JOIN stage_work_items item ON item.id=receipt.stage_work_item_id
             JOIN stage_worker_runs worker ON worker.id=receipt.worker_run_id
            WHERE receipt.analysis_attempt_id=$1 AND receipt.stage_work_item_id=$2
              AND receipt.artifact_kind='controller_dispatch.v1' FOR SHARE"#,
    )
    .bind(fence.analysis_attempt_id)
    .bind(fence.work_item_id)
    .fetch_optional(&mut *tx)
    .await?;
    if let Some((existing_attempt, existing_worker, existing_hash, item_status, worker_status)) =
        existing
    {
        if existing_attempt != provider_attempt_id
            || existing_worker != fence.worker_run_id
            || existing_hash != body_hash
            || item_status != "completed"
            || worker_status != "passed"
        {
            return Err(conflict("CANDIDATE_PROVIDER_ATTEMPT_REPLAY_DRIFT"));
        }
        tx.commit().await?;
        return Ok(());
    } else {
        super::candidate_analysis::validate_write_fence_on(&mut tx, fence).await?;
        sqlx::query(
            r#"INSERT INTO candidate_analysis_provider_attempts(
                   provider_attempt_id,analysis_attempt_id,stage_work_item_id,
                   worker_run_id,artifact_kind,artifact_body,artifact_hash)
               VALUES($1,$2,$3,$4,'controller_dispatch.v1',$5,$6)"#,
        )
        .bind(provider_attempt_id)
        .bind(fence.analysis_attempt_id)
        .bind(fence.work_item_id)
        .bind(fence.worker_run_id)
        .bind(body)
        .bind(body_hash)
        .execute(&mut *tx)
        .await?;
    }
    let item_update = sqlx::query(
        r#"UPDATE stage_work_items
              SET status='completed',terminal_at=statement_timestamp(),
                  row_version=row_version+1,updated_at=statement_timestamp()
            WHERE id=$1 AND team_plan_id=$2 AND status='running' AND row_version=$3"#,
    )
    .bind(fence.work_item_id)
    .bind(fence.team_plan_id)
    .bind(fence.expected_work_item_row_version)
    .execute(&mut *tx)
    .await?;
    let worker_update = sqlx::query(
        r#"UPDATE stage_worker_runs
              SET status='passed',terminal_at=statement_timestamp(),updated_at=statement_timestamp()
            WHERE id=$1 AND work_item_id=$2 AND status='running'
              AND checkpoint_version=$3 AND lease_token=$4 AND attempt_epoch=$5"#,
    )
    .bind(fence.worker_run_id)
    .bind(fence.work_item_id)
    .bind(fence.expected_worker_row_version)
    .bind(fence.lease_token)
    .bind(fence.attempt_epoch)
    .execute(&mut *tx)
    .await?;
    if item_update.rows_affected() != 1 || worker_update.rows_affected() != 1 {
        return Err(conflict(
            "CANDIDATE_CONTROLLER_DISPATCH_TERMINAL_CAS_FAILED",
        ));
    }
    tx.commit().await?;
    Ok(())
}

/// Proves that the server-issued dispatch Controller and the independently
/// leased final Controller are the two closed role turns for one Candidate
/// attempt, and that the latter is the scheduler's unique final submitter.
pub async fn validate_controller_final_authority_binding(
    pool: &PgPool,
    dispatch: &CandidateWriteFenceRow,
    final_authority: &CandidateWriteFenceRow,
) -> Result<()> {
    if dispatch.analysis_attempt_id != final_authority.analysis_attempt_id
        || dispatch.analysis_attempt_ordinal != final_authority.analysis_attempt_ordinal
        || dispatch.team_plan_id != final_authority.team_plan_id
        || dispatch.operation_id != final_authority.operation_id
        || dispatch.scope_snapshot_id != final_authority.scope_snapshot_id
        || dispatch.organization_id != final_authority.organization_id
        || dispatch.snapshot_id != final_authority.snapshot_id
        || dispatch.work_item_id == final_authority.work_item_id
        || dispatch.worker_run_id == final_authority.worker_run_id
    {
        return Err(conflict("CANDIDATE_CONTROLLER_AUTHORITY_BINDING_INVALID"));
    }
    let bound: bool = sqlx::query_scalar(
        r#"SELECT EXISTS(
               SELECT 1
                 FROM stage_team_plans plan
                 JOIN stage_work_items dispatch_item
                   ON dispatch_item.team_plan_id=plan.id
                  AND dispatch_item.id=$2
                  AND dispatch_item.kind='candidate_controller_dispatch'
                  AND dispatch_item.role='controller'
                  AND dispatch_item.status='completed'
                 JOIN stage_worker_runs dispatch_worker
                   ON dispatch_worker.work_item_id=dispatch_item.id
                  AND dispatch_worker.id=$3
                  AND dispatch_worker.specialist='controller'
                  AND dispatch_worker.work_item_kind='candidate_controller_dispatch'
                  AND dispatch_worker.status='passed'
                 JOIN candidate_analysis_work_items dispatch_candidate
                   ON dispatch_candidate.stage_work_item_id=dispatch_item.id
                  AND dispatch_candidate.analysis_attempt_id=$4
                  AND dispatch_candidate.phase='controller'
                  AND dispatch_candidate.capability='candidate_controller_dispatch'
                 JOIN candidate_analysis_provider_attempts dispatch_receipt
                   ON dispatch_receipt.stage_work_item_id=dispatch_item.id
                  AND dispatch_receipt.worker_run_id=dispatch_worker.id
                  AND dispatch_receipt.analysis_attempt_id=dispatch_candidate.analysis_attempt_id
                  AND dispatch_receipt.artifact_kind='controller_dispatch.v1'
                 JOIN stage_work_items final_item
                   ON final_item.team_plan_id=plan.id
                  AND final_item.id=$5
                  AND final_item.kind='candidate_controller_final'
                  AND final_item.role='controller'
                 JOIN stage_worker_runs final_worker
                   ON final_worker.work_item_id=final_item.id
                  AND final_worker.id=$6
                  AND final_worker.specialist='controller'
                  AND final_worker.work_item_kind='candidate_controller_final'
                 JOIN candidate_analysis_work_items final_candidate
                   ON final_candidate.stage_work_item_id=final_item.id
                  AND final_candidate.analysis_attempt_id=dispatch_candidate.analysis_attempt_id
                  AND final_candidate.phase='controller'
                  AND final_candidate.capability='candidate_controller_final'
                WHERE plan.id=$1
                  AND plan.final_submitter_worker_run_id=final_worker.id
                  AND plan.operation_id=$7
                  AND plan.scope_snapshot_id=$8
                  AND plan.organization_id=$9
           )"#,
    )
    .bind(dispatch.team_plan_id)
    .bind(dispatch.work_item_id)
    .bind(dispatch.worker_run_id)
    .bind(dispatch.analysis_attempt_id)
    .bind(final_authority.work_item_id)
    .bind(final_authority.worker_run_id)
    .bind(dispatch.operation_id)
    .bind(dispatch.scope_snapshot_id)
    .bind(dispatch.organization_id)
    .fetch_one(pool)
    .await?;
    if !bound {
        return Err(conflict("CANDIDATE_CONTROLLER_AUTHORITY_BINDING_INVALID"));
    }
    Ok(())
}

/// Rebinds the final Controller fence after its decision receipt atomically
/// closes the worker turn. The receipt transition increments the work-item
/// row version, so canonical Gate seal/apply must use the exact post-receipt
/// version instead of replaying the pre-receipt lease fence.
pub async fn refresh_controller_final_write_fence(
    pool: &PgPool,
    fence: &CandidateWriteFenceRow,
) -> Result<CandidateWriteFenceRow> {
    let refreshed: Option<(i64, i64, i64, i64, Option<Uuid>)> = sqlx::query_as(
        r#"SELECT plan.row_version,item.row_version,worker.checkpoint_version,
                  worker.attempt_epoch,worker.lease_token
             FROM stage_team_plans plan
             JOIN stage_work_items item
               ON item.team_plan_id=plan.id
              AND item.id=$2
              AND item.kind='candidate_controller_final'
              AND item.role='controller'
              AND item.status='completed'
             JOIN stage_worker_runs worker
               ON worker.work_item_id=item.id
              AND worker.id=$3
              AND worker.specialist='controller'
              AND worker.work_item_kind='candidate_controller_final'
              AND worker.status='passed'
             JOIN candidate_analysis_work_items candidate_item
               ON candidate_item.stage_work_item_id=item.id
              AND candidate_item.analysis_attempt_id=$4
              AND candidate_item.phase='controller'
              AND candidate_item.capability='candidate_controller_final'
             JOIN candidate_analysis_provider_attempts receipt
               ON receipt.analysis_attempt_id=candidate_item.analysis_attempt_id
              AND receipt.stage_work_item_id=item.id
              AND receipt.worker_run_id=worker.id
              AND receipt.artifact_kind='controller_decision.v1'
            WHERE plan.id=$1
              AND plan.final_submitter_worker_run_id=worker.id
              AND plan.operation_id=$5
              AND plan.scope_snapshot_id=$6
              AND plan.organization_id=$7
              AND worker.lease_token=$8
              AND worker.attempt_epoch=$9
              AND worker.lease_expires_at>statement_timestamp()"#,
    )
    .bind(fence.team_plan_id)
    .bind(fence.work_item_id)
    .bind(fence.worker_run_id)
    .bind(fence.analysis_attempt_id)
    .bind(fence.operation_id)
    .bind(fence.scope_snapshot_id)
    .bind(fence.organization_id)
    .bind(fence.lease_token)
    .bind(fence.attempt_epoch)
    .fetch_optional(pool)
    .await?;
    let (plan_version, item_version, checkpoint_version, attempt_epoch, lease_token) =
        refreshed.ok_or_else(|| conflict("CANDIDATE_CONTROLLER_FINAL_RECEIPT_FENCE_INVALID"))?;
    let mut refreshed_fence = fence.clone();
    refreshed_fence.expected_team_plan_row_version = plan_version;
    refreshed_fence.expected_work_item_row_version = item_version;
    refreshed_fence.expected_worker_row_version = checkpoint_version;
    refreshed_fence.attempt_epoch = attempt_epoch;
    refreshed_fence.lease_epoch = attempt_epoch;
    refreshed_fence.lease_token =
        lease_token.ok_or_else(|| conflict("CANDIDATE_WORKER_LEASE_MISSING"))?;
    Ok(refreshed_fence)
}

#[derive(Debug, sqlx::FromRow)]
struct AnalystChunkDbRow {
    snapshot_input_id: Uuid,
    stable_input_key: String,
    source_kind: String,
    source_ref: String,
    source_content_hash: String,
    source_size_bytes: i64,
    subject_kind_at_time: String,
    subject_identity_hash: String,
    chunk_id: Uuid,
    ordinal: i32,
    census_hash: String,
    chunking_contract_version: String,
    redaction_contract_version: String,
    chunk_hash: String,
    immutable_redacted_body: Option<Value>,
}

#[derive(Debug, sqlx::FromRow)]
struct KnowledgeFeedPayloadDbRow {
    feed_snapshot_id: Uuid,
    feed_match_member_id: Uuid,
    feed_kind: String,
    feed_version: String,
    published_at_unix_seconds: i64,
    content_hash: String,
    manifest_hash: String,
    provenance_hash: String,
    signature_receipt_hash: String,
    product_version_match_hash: String,
    matcher_hash: String,
    member_hash: String,
}

fn serialized_payload_hash(payload: &Value) -> Result<String> {
    let bytes = serde_json::to_vec(payload)?;
    let digest = Sha256::digest(bytes)
        .iter()
        .map(|byte| format!("{byte:02x}"))
        .collect::<String>();
    Ok(format!("sha256:{digest}"))
}

async fn project_runtime_chunk_on(
    tx: &mut Transaction<'_, Postgres>,
    row: &AnalystChunkDbRow,
) -> Result<Value> {
    let (input_kind, bounded_payload) = match row.source_kind.as_str() {
        "knowledge_signal" => {
            let match_id = row
                .source_ref
                .strip_prefix("candidate_feed_match_member:")
                .and_then(|value| Uuid::parse_str(value).ok())
                .ok_or_else(|| conflict("CANDIDATE_KNOWLEDGE_SIGNAL_AUTHORITY_INVALID"))?;
            let authority = sqlx::query_as::<_, KnowledgeFeedPayloadDbRow>(
                r#"SELECT feed.feed_snapshot_id,
                          match.match_member_id AS feed_match_member_id,
                          expected.source_kind AS feed_kind,feed.feed_version,
                          EXTRACT(EPOCH FROM feed.published_at)::BIGINT AS published_at_unix_seconds,
                          feed.content_hash,feed.signed_manifest_hash AS manifest_hash,
                          tool_truth_sha256(feed.provenance::TEXT) AS provenance_hash,
                          feed.signature_verification_receipt_hash AS signature_receipt_hash,
                          product.member_hash AS product_version_match_hash,
                          census.matcher_contract_digest AS matcher_hash,
                          match.member_hash
                     FROM candidate_analysis_feed_match_census_members match
                     JOIN candidate_analysis_feed_match_censuses census USING(match_census_id)
                     JOIN candidate_analysis_product_version_census_members product
                       ON product.product_member_id=match.product_member_id
                     JOIN candidate_analysis_knowledge_feed_snapshot_members feed
                       ON feed.feed_snapshot_member_id=match.feed_snapshot_member_id
                     JOIN candidate_analysis_knowledge_feed_denominator_members expected
                       ON expected.expected_member_id=feed.expected_member_id
                    WHERE match.match_member_id=$1 AND match.disposition='matched'
                      AND product.disposition='known' AND feed.disposition='current'"#,
            )
            .bind(match_id)
            .fetch_optional(&mut **tx)
            .await?
            .ok_or_else(|| conflict("CANDIDATE_KNOWLEDGE_SIGNAL_AUTHORITY_INVALID"))?;
            (
                "knowledge_signal",
                json!({
                    "kind":"knowledge_feed_match",
                    "feed_snapshot_id":authority.feed_snapshot_id,
                    "feed_match_member_id":authority.feed_match_member_id,
                    "feed_kind":authority.feed_kind,"feed_version":authority.feed_version,
                    "published_at_unix_seconds":authority.published_at_unix_seconds,
                    "content_hash":authority.content_hash,"manifest_hash":authority.manifest_hash,
                    "provenance_hash":authority.provenance_hash,
                    "signature_receipt_hash":authority.signature_receipt_hash,
                    "product_version_match_hash":authority.product_version_match_hash,
                    "matcher_hash":authority.matcher_hash,"member_hash":authority.member_hash,
                    "source_authority":"knowledge_signal_only",
                }),
            )
        }
        "previous_generation" => (
            "previous_generation",
            json!({
                "kind":"previous_generation","revision_hash":row.source_content_hash,
                "lifecycle_state":"frozen_snapshot",
            }),
        ),
        "open_obligations" => (
            "open_obligation",
            json!({
                "kind":"residual_or_obligation","reason_code":"open_obligation",
                "authority_hash":row.source_content_hash,
            }),
        ),
        "managed_knowledge_feed" => (
            "application_context",
            json!({
                "kind":"tool_truth_record",
                "record_schema":"candidate_managed_feed_context.v1",
                "redacted_fields":[
                    ["immutable_body",serde_json::to_string(&row.immutable_redacted_body)?],
                    ["chunk_hash",row.chunk_hash],
                ],
            }),
        ),
        source_kind => (
            candidate_input_kind(source_kind),
            json!({
                "kind":"tool_truth_record",
                "record_schema":format!("candidate_redacted_chunk.v1:{source_kind}"),
                "redacted_fields":[
                    ["immutable_body",serde_json::to_string(&row.immutable_redacted_body)?],
                    ["chunk_hash",row.chunk_hash],
                ],
            }),
        ),
    };
    let bounded_payload_hash = serialized_payload_hash(&bounded_payload)?;
    let chunking_contract_version = row
        .chunking_contract_version
        .parse::<u32>()
        .map_err(|_| conflict("CANDIDATE_CHUNK_CONTRACT_INVALID"))?;
    let redaction_contract_version = row
        .redaction_contract_version
        .parse::<u32>()
        .map_err(|_| conflict("CANDIDATE_CHUNK_CONTRACT_INVALID"))?;
    let provenance = match row.source_kind.as_str() {
        "knowledge_signal" | "managed_knowledge_feed" => "frozen_knowledge_feed",
        "previous_generation" => "previous_generation",
        "open_obligations" => "candidate_residual",
        _ => "tool_truth_authority",
    };
    Ok(json!({
        "snapshot_ready":true,
        "input_id":row.snapshot_input_id,"expected_input_id":row.snapshot_input_id,
        "chunk_id":row.chunk_id,"expected_chunk_id":row.chunk_id,
        "input_key":row.stable_input_key,"input_kind":input_kind,
        "knowledge_feed_eligibility":if row.source_kind=="knowledge_signal" {
            Value::String("current_known_version_signed".to_owned())
        } else { Value::Null },
        "provenance":provenance,
        "at_time_subject":{
            "kind":&row.subject_kind_at_time,"identity_hash":&row.subject_identity_hash,
        },
        "source_hash":row.source_content_hash,
        "source_size":u64::try_from(row.source_size_bytes)
            .map_err(|_|conflict("CANDIDATE_SOURCE_SIZE_INVALID"))?,
        "chunk_ordinal":row.ordinal,"expected_chunk_ordinal":row.ordinal,
        "chunk_census_hash":row.census_hash,"expected_chunk_census_hash":row.census_hash,
        "chunking_contract_version":chunking_contract_version,
        "expected_chunking_contract_version":chunking_contract_version,
        "redaction_contract_version":redaction_contract_version,
        "expected_redaction_contract_version":redaction_contract_version,
        "bounded_payload":bounded_payload,"persisted_payload_hash":bounded_payload_hash,
        "max_chunk_bytes":16384,
        "instruction_authority":false,
    }))
}

fn candidate_input_kind(source_kind: &str) -> &'static str {
    match source_kind {
        "tool_truth_bundle" => "tool_truth_evidence",
        "knowledge_signal" => "knowledge_signal",
        "managed_knowledge_feed" => "application_context",
        "previous_generation" => "previous_generation",
        "relations" => "relation",
        "open_obligations" => "open_obligation",
        "expected_fact_deltas"
        | "unconsumed_fact_deltas"
        | "consumed_fact_deltas"
        | "state_events" => "fact_delta",
        "application_context" => "application_context",
        _ => "tool_truth_fact",
    }
}

pub async fn prepare_analyst_work_batch(
    pool: &PgPool,
    snapshot_id: Uuid,
    stage_execution_id: Uuid,
    attempt_ordinal: i32,
    requested_inputs_per_microbatch: i32,
    host_lane_limit: i32,
) -> Result<Vec<CandidateRuntimeWorkRow>> {
    if requested_inputs_per_microbatch <= 0 || host_lane_limit <= 0 {
        return Err(conflict("CANDIDATE_ANALYST_BATCH_INVALID"));
    }
    let mut tx = pool.begin().await?;
    let identity =
        load_scheduler_identity_on(&mut tx, snapshot_id, stage_execution_id, attempt_ordinal)
            .await?;
    let plan_id = ensure_team_plan_on(&mut tx, &identity).await?;
    let rows = sqlx::query_as::<_, AnalystChunkDbRow>(
        r#"SELECT source.snapshot_input_id,source.stable_input_key,source.source_kind,
                  source.source_ref,source.source_content_hash,
                  source.source_byte_count AS source_size_bytes,source.subject_kind_at_time,
                  source.subject_identity_hash,member.chunk_id,member.ordinal,census.census_hash,
                  census.chunking_contract_version,census.redaction_contract_version,member.chunk_hash,
                  member.immutable_redacted_body
             FROM candidate_analysis_snapshot_inputs source
             JOIN candidate_analysis_input_chunk_censuses census
               ON census.snapshot_input_id=source.snapshot_input_id
             JOIN candidate_analysis_input_chunk_census_members member
               ON member.chunk_census_id=census.chunk_census_id
            WHERE source.snapshot_id=$1 AND census.disposition='complete'
            ORDER BY source.stable_input_key,member.ordinal"#,
    )
    .bind(snapshot_id)
    .fetch_all(&mut *tx)
    .await?;
    let mut per_input = std::collections::BTreeMap::<Uuid, Vec<AnalystChunkDbRow>>::new();
    for row in rows {
        per_input
            .entry(row.snapshot_input_id)
            .or_default()
            .push(row);
    }
    let _requested_batch_size = usize::try_from(requested_inputs_per_microbatch)
        .map_err(|_| conflict("CANDIDATE_ANALYST_BATCH_INVALID"))?;
    // Primary ownership is per input. This keeps the immutable H1/source
    // attribution exact even when the Controller asks for wider prompt
    // batches; rolling lane concurrency still supplies throughput.
    let batch_size = 1usize;
    let grouped = per_input.into_values().collect::<Vec<_>>();
    let mut drafts = Vec::new();
    let relationship_hashes: Vec<String> = sqlx::query_scalar(
        r#"SELECT member.member_hash
             FROM candidate_analysis_snapshot_source_sets source_set
             JOIN candidate_analysis_snapshot_source_set_members member USING(source_set_id,snapshot_id)
            WHERE source_set.snapshot_id=$1 AND source_set.source_kind='relations'
            ORDER BY member.ordinal"#,
    )
    .bind(snapshot_id)
    .fetch_all(&mut *tx)
    .await?;
    let relationship_cross_index_hash = hash_texts_on(&mut tx, &relationship_hashes).await?;
    let predecessor_attempt_id: Option<Uuid> = sqlx::query_scalar(
        "SELECT predecessor_attempt_id FROM candidate_analysis_attempts WHERE analysis_attempt_id=$1",
    )
    .bind(identity.attempt_id)
    .fetch_one(&mut *tx)
    .await?;
    let retry_signals = if let Some(predecessor_attempt_id) = predecessor_attempt_id {
        load_retry_missed_signals_on(&mut tx, predecessor_attempt_id)
            .await?
            .0
    } else {
        Vec::new()
    };
    for (batch_ordinal, input_batch) in grouped.chunks(batch_size).enumerate() {
        let mut chunks = Vec::new();
        let mut trust_boundary_hashes = Vec::new();
        for input_rows in input_batch {
            for row in input_rows {
                trust_boundary_hashes.push(row.subject_identity_hash.clone());
                chunks.push(project_runtime_chunk_on(&mut tx, row).await?);
            }
        }
        let microbatch_id = Uuid::new_v5(
            &identity.attempt_id,
            format!("analyst-microbatch:{batch_ordinal}").as_bytes(),
        );
        let owned_input_ids = input_batch
            .iter()
            .filter_map(|rows| rows.first().map(|row| row.snapshot_input_id))
            .collect::<BTreeSet<_>>();
        let missed_hypothesis_signals = retry_signals
            .iter()
            .filter(|signal| {
                signal
                    .get("covered_input_ids")
                    .and_then(Value::as_array)
                    .is_some_and(|input_ids| {
                        input_ids.iter().any(|input_id| {
                            input_id
                                .as_str()
                                .and_then(|value| Uuid::parse_str(value).ok())
                                .is_some_and(|input_id| owned_input_ids.contains(&input_id))
                        })
                    })
            })
            .cloned()
            .collect::<Vec<_>>();
        let mut missed_signal_hashes = Vec::with_capacity(missed_hypothesis_signals.len());
        for signal in &missed_hypothesis_signals {
            missed_signal_hashes.push(hash_json_on(&mut tx, signal).await?);
        }
        let missed_hypothesis_signal_set_hash =
            hash_texts_on(&mut tx, &missed_signal_hashes).await?;
        let input = json!({
            "microbatch_id":microbatch_id,
            "microbatch_ordinal":batch_ordinal,
            "chunks":chunks,
            "relationship_cross_index_hash":relationship_cross_index_hash,
            "trust_boundary_cross_index_hash":hash_texts_on(&mut tx,&trust_boundary_hashes).await?,
            "missed_hypothesis_signals":missed_hypothesis_signals,
            "missed_hypothesis_signal_set_hash":missed_hypothesis_signal_set_hash,
        });
        let stable_key = format!("analyst:{attempt_ordinal}:{batch_ordinal}");
        let primary_input_key = input_batch
            .first()
            .and_then(|rows| rows.first())
            .map(|row| row.snapshot_input_id.to_string())
            .ok_or_else(|| conflict("CANDIDATE_ANALYST_BATCH_EMPTY"))?;
        let (item_id, worker_id) = ensure_queued_work_on(
            &mut tx,
            &identity,
            plan_id,
            "analyst",
            "hypothesis_proposal",
            &stable_key,
            Some(&primary_input_key),
            None,
            i32::try_from(batch_ordinal).unwrap_or(i32::MAX) % host_lane_limit,
            &input,
        )
        .await?;
        drafts.push((
            item_id,
            worker_id,
            i32::try_from(batch_ordinal).unwrap_or(i32::MAX) % host_lane_limit,
            input,
        ));
    }
    let mut available = available_live_lanes_on(&mut tx, plan_id, host_lane_limit).await?;
    let mut work = Vec::new();
    for (item_id, worker_id, lane_ordinal, input) in drafts {
        let status: String = sqlx::query_scalar("SELECT status FROM stage_worker_runs WHERE id=$1")
            .bind(worker_id)
            .fetch_one(&mut *tx)
            .await?;
        let may_claim = status == "running" || (status == "queued" && available > 0);
        if !may_claim {
            continue;
        }
        let Some(fence) = claim_or_replay_queued_work_on(
            &mut tx,
            &identity,
            plan_id,
            "hypothesis_proposal",
            item_id,
            worker_id,
        )
        .await?
        else {
            continue;
        };
        if status == "queued" {
            available = available.saturating_sub(1);
        }
        ensure_analyst_chunk_page_receipt_on(&mut tx, &fence, &input).await?;
        let replayed_receipt = artifact_receipt_for_work_on(&mut tx, fence.work_item_id).await?;
        work.push(CandidateRuntimeWorkRow {
            fence,
            phase: "analyst".to_owned(),
            capability: "hypothesis_proposal".to_owned(),
            lane_ordinal,
            input,
            replayed_receipt,
        });
    }
    tx.commit().await?;
    Ok(work)
}

async fn ensure_analyst_chunk_page_receipt_on(
    tx: &mut Transaction<'_, Postgres>,
    fence: &CandidateWriteFenceRow,
    input: &Value,
) -> Result<()> {
    let chunks = input
        .get("chunks")
        .and_then(Value::as_array)
        .filter(|chunks| !chunks.is_empty())
        .ok_or_else(|| conflict("CANDIDATE_ANALYST_PAGE_EMPTY"))?;
    let input_id = chunks
        .first()
        .and_then(|chunk| chunk.get("input_id"))
        .and_then(Value::as_str)
        .and_then(|value| Uuid::parse_str(value).ok())
        .ok_or_else(|| conflict("CANDIDATE_ANALYST_PAGE_INVALID"))?;
    let supplied_chunk_ids = chunks
        .iter()
        .map(|chunk| {
            let chunk_input_id = chunk
                .get("input_id")
                .and_then(Value::as_str)
                .and_then(|value| Uuid::parse_str(value).ok());
            if chunk_input_id != Some(input_id) {
                return Err(conflict("CANDIDATE_ANALYST_PAGE_OWNER_MISMATCH"));
            }
            chunk
                .get("chunk_id")
                .and_then(Value::as_str)
                .and_then(|value| Uuid::parse_str(value).ok())
                .ok_or_else(|| conflict("CANDIDATE_ANALYST_PAGE_INVALID"))
        })
        .collect::<Result<Vec<_>>>()?;
    let (census_id, census_hash, source_size, chunking_version, redaction_version): (
        Uuid,
        String,
        i64,
        String,
        String,
    ) = sqlx::query_as(
        r#"SELECT chunk_census_id,census_hash,source_byte_count,
                  chunking_contract_version,redaction_contract_version
             FROM candidate_analysis_input_chunk_censuses
            WHERE snapshot_input_id=$1 AND snapshot_id=$2 AND disposition='complete'"#,
    )
    .bind(input_id)
    .bind(fence.snapshot_id)
    .fetch_optional(&mut **tx)
    .await?
    .ok_or_else(|| conflict("CANDIDATE_ANALYST_PAGE_AUTHORITY_MISSING"))?;
    let canonical_chunks: Vec<(Uuid, String, i32)> = sqlx::query_as(
        r#"SELECT chunk_id,chunk_hash,ordinal
             FROM candidate_analysis_input_chunk_census_members
            WHERE chunk_census_id=$1 ORDER BY ordinal"#,
    )
    .bind(census_id)
    .fetch_all(&mut **tx)
    .await?;
    if supplied_chunk_ids != canonical_chunks.iter().map(|row| row.0).collect::<Vec<_>>() {
        return Err(conflict("CANDIDATE_ANALYST_PAGE_EXACT_SET_INVALID"));
    }
    let first = canonical_chunks
        .first()
        .map(|row| row.2)
        .ok_or_else(|| conflict("CANDIDATE_ANALYST_PAGE_EMPTY"))?;
    let limit = i32::try_from(canonical_chunks.len())
        .map_err(|_| conflict("CANDIDATE_ANALYST_PAGE_LIMIT_INVALID"))?;
    let expected_chunk_hashes = canonical_chunks
        .iter()
        .map(|row| row.1.clone())
        .collect::<Vec<_>>();
    let page_request_id = Uuid::new_v5(&fence.work_item_id, b"candidate_analyst_chunk_page.v1");
    let page_hash = ensure_runtime_chunk_page_receipt_on(
        tx,
        RuntimeChunkPageRequest {
            fence,
            stable_request_id: page_request_id,
            snapshot_input_id: input_id,
            chunk_census_id: census_id,
            chunk_census_hash: &census_hash,
            source_size_bytes: source_size,
            chunking_contract_version: &chunking_version,
            redaction_contract_version: &redaction_version,
            first_ordinal: first,
            limit,
            expected_ordered_chunk_hashes: &expected_chunk_hashes,
        },
    )
    .await?;
    seal_candidate_work_page_authority_on(
        tx,
        fence,
        Uuid::new_v5(&page_request_id, b"candidate_page_receipt.v1"),
        &page_hash,
    )
    .await?;
    Ok(())
}

async fn seal_candidate_work_page_authority_on(
    tx: &mut Transaction<'_, Postgres>,
    fence: &CandidateWriteFenceRow,
    page_receipt_id: Uuid,
    page_hash: &str,
) -> Result<()> {
    let candidate_work_item_id: Uuid = sqlx::query_scalar(
        r#"SELECT candidate_work_item_id FROM candidate_analysis_work_items
            WHERE stage_work_item_id=$1 AND analysis_attempt_id=$2"#,
    )
    .bind(fence.work_item_id)
    .bind(fence.analysis_attempt_id)
    .fetch_optional(&mut **tx)
    .await?
    .ok_or_else(|| conflict("CANDIDATE_WORK_ITEM_AUTHORITY_MISSING"))?;
    let authority_id = Uuid::new_v5(&candidate_work_item_id, b"candidate_work_page_authority.v1");
    let page_hashes = [page_hash.to_owned()];
    let page_authority_set_hash = hash_texts_on(tx, &page_hashes).await?;
    sqlx::query(
        r#"INSERT INTO candidate_analysis_work_page_authorities(
               work_page_authority_id,candidate_work_item_id,analysis_attempt_id,
               page_receipt_id,page_authority_set_hash)
           SELECT $1,$2,$3,page.page_receipt_id,$5
             FROM candidate_analysis_page_receipts page
            WHERE page.page_receipt_id=$4
              AND page.analysis_attempt_id=$3
              AND page.consumer_worker_run_id=$6
              AND page.page_hash=$7
           ON CONFLICT(work_page_authority_id) DO NOTHING"#,
    )
    .bind(authority_id)
    .bind(candidate_work_item_id)
    .bind(fence.analysis_attempt_id)
    .bind(page_receipt_id)
    .bind(&page_authority_set_hash)
    .bind(fence.worker_run_id)
    .bind(page_hash)
    .execute(&mut **tx)
    .await?;
    let exact: bool = sqlx::query_scalar(
        r#"SELECT EXISTS(
               SELECT 1
                 FROM candidate_analysis_work_page_authorities authority
                 JOIN candidate_analysis_page_receipts page
                   ON page.page_receipt_id=authority.page_receipt_id
                WHERE authority.work_page_authority_id=$1
                  AND authority.candidate_work_item_id=$2
                  AND authority.analysis_attempt_id=$3
                  AND authority.page_receipt_id=$4
                  AND authority.page_authority_set_hash=$5
                  AND page.analysis_attempt_id=$3
                  AND page.consumer_worker_run_id=$6
                  AND page.page_hash=$7
           )"#,
    )
    .bind(authority_id)
    .bind(candidate_work_item_id)
    .bind(fence.analysis_attempt_id)
    .bind(page_receipt_id)
    .bind(page_authority_set_hash)
    .bind(fence.worker_run_id)
    .bind(page_hash)
    .fetch_one(&mut **tx)
    .await?;
    if !exact {
        return Err(conflict("CANDIDATE_WORK_PAGE_AUTHORITY_DRIFT"));
    }
    Ok(())
}

struct RuntimeChunkPageRequest<'a> {
    fence: &'a CandidateWriteFenceRow,
    stable_request_id: Uuid,
    snapshot_input_id: Uuid,
    chunk_census_id: Uuid,
    chunk_census_hash: &'a str,
    source_size_bytes: i64,
    chunking_contract_version: &'a str,
    redaction_contract_version: &'a str,
    first_ordinal: i32,
    limit: i32,
    expected_ordered_chunk_hashes: &'a [String],
}

async fn ensure_runtime_chunk_page_receipt_on(
    tx: &mut Transaction<'_, Postgres>,
    request: RuntimeChunkPageRequest<'_>,
) -> Result<String> {
    if !(1..=64).contains(&request.limit) {
        return Err(conflict("CANDIDATE_CHUNK_PAGE_LIMIT_INVALID"));
    }
    let chunks: Vec<(i32, String)> = sqlx::query_as(
        r#"SELECT ordinal,chunk_hash
             FROM candidate_analysis_input_chunk_census_members
            WHERE chunk_census_id=$1 AND ordinal>=$2
            ORDER BY ordinal LIMIT $3"#,
    )
    .bind(request.chunk_census_id)
    .bind(request.first_ordinal)
    .bind(i64::from(request.limit))
    .fetch_all(&mut **tx)
    .await?;
    if chunks.is_empty() || chunks.len() > request.limit as usize || chunks.len() > 64 {
        return Err(conflict("CANDIDATE_CHUNK_PAGE_RANGE_INVALID"));
    }
    let first_ordinal = chunks.first().map(|row| row.0);
    let last_ordinal = chunks.last().map(|row| row.0);
    if first_ordinal != Some(request.first_ordinal) {
        return Err(conflict("CANDIDATE_CHUNK_PAGE_RANGE_INVALID"));
    }
    let ordered_chunk_hashes = chunks.iter().map(|row| row.1.clone()).collect::<Vec<_>>();
    if ordered_chunk_hashes != request.expected_ordered_chunk_hashes {
        return Err(conflict("CANDIDATE_CHUNK_PAGE_CONTENT_DRIFT"));
    }
    let page_hash = candidate_chunk_page_hash_on(
        tx,
        &CandidateChunkPageHashInput {
            analysis_attempt_id: request.fence.analysis_attempt_id,
            snapshot_id: request.fence.snapshot_id,
            snapshot_input_id: request.snapshot_input_id,
            chunk_census_id: request.chunk_census_id,
            chunk_census_hash: request.chunk_census_hash.to_owned(),
            consumer_worker_run_id: request.fence.worker_run_id,
            first_ordinal,
            last_ordinal,
            ordered_chunk_hashes,
            source_size_bytes: request.source_size_bytes,
            chunking_contract_version: request.chunking_contract_version.to_owned(),
            redaction_contract_version: request.redaction_contract_version.to_owned(),
        },
    )
    .await?;
    let page_receipt_id = Uuid::new_v5(&request.stable_request_id, b"candidate_page_receipt.v1");
    let cursor = format!("chunk:{}:{}", request.first_ordinal, request.limit);
    sqlx::query(
        r#"INSERT INTO candidate_analysis_page_receipts(
               page_receipt_id,analysis_attempt_id,snapshot_id,page_kind,stable_request_id,
               snapshot_input_id,chunk_census_id,chunk_census_hash,source_size_bytes,
               chunking_contract_version,redaction_contract_version,consumer_worker_run_id,
               server_cursor,first_key,last_key,returned_count,page_hash)
           VALUES($1,$2,$3,'chunk_page',$4,$5,$6,$7,$8,$9,$10,$11,
                  $12,$13,$14,$15,$16)
           ON CONFLICT(analysis_attempt_id,consumer_worker_run_id,stable_request_id) DO NOTHING"#,
    )
    .bind(page_receipt_id)
    .bind(request.fence.analysis_attempt_id)
    .bind(request.fence.snapshot_id)
    .bind(request.stable_request_id)
    .bind(request.snapshot_input_id)
    .bind(request.chunk_census_id)
    .bind(request.chunk_census_hash)
    .bind(request.source_size_bytes)
    .bind(request.chunking_contract_version)
    .bind(request.redaction_contract_version)
    .bind(request.fence.worker_run_id)
    .bind(&cursor)
    .bind(first_ordinal.map(|value| value.to_string()))
    .bind(last_ordinal.map(|value| value.to_string()))
    .bind(i64::try_from(chunks.len()).unwrap_or(i64::MAX))
    .bind(&page_hash)
    .execute(&mut **tx)
    .await?;
    let exact: bool = sqlx::query_scalar(
        r#"SELECT EXISTS(
               SELECT 1 FROM candidate_analysis_page_receipts
                WHERE page_receipt_id=$1 AND analysis_attempt_id=$2 AND snapshot_id=$3
                  AND page_kind='chunk_page' AND stable_request_id=$4
                  AND snapshot_input_id=$5 AND chunk_census_id=$6
                  AND chunk_census_hash=$7 AND source_size_bytes=$8
                  AND chunking_contract_version=$9 AND redaction_contract_version=$10
                  AND consumer_worker_run_id=$11 AND server_cursor=$12
                  AND first_key IS NOT DISTINCT FROM $13
                  AND last_key IS NOT DISTINCT FROM $14
                  AND returned_count=$15 AND page_hash=$16
           )"#,
    )
    .bind(page_receipt_id)
    .bind(request.fence.analysis_attempt_id)
    .bind(request.fence.snapshot_id)
    .bind(request.stable_request_id)
    .bind(request.snapshot_input_id)
    .bind(request.chunk_census_id)
    .bind(request.chunk_census_hash)
    .bind(request.source_size_bytes)
    .bind(request.chunking_contract_version)
    .bind(request.redaction_contract_version)
    .bind(request.fence.worker_run_id)
    .bind(&cursor)
    .bind(first_ordinal.map(|value| value.to_string()))
    .bind(last_ordinal.map(|value| value.to_string()))
    .bind(i64::try_from(chunks.len()).unwrap_or(i64::MAX))
    .bind(&page_hash)
    .fetch_one(&mut **tx)
    .await?;
    if !exact {
        return Err(conflict("CANDIDATE_CHUNK_PAGE_REPLAY_DRIFT"));
    }
    Ok(page_hash)
}

async fn artifact_receipt_for_work_on(
    tx: &mut Transaction<'_, Postgres>,
    work_item_id: Uuid,
) -> Result<Option<CandidateArtifactReceiptRow>> {
    Ok(sqlx::query_as::<_, (Uuid, String)>(
        r#"SELECT artifact.artifact_id,artifact.artifact_hash
             FROM candidate_analysis_work_items candidate_item
             JOIN candidate_analysis_artifacts artifact
               ON artifact.candidate_work_item_id=candidate_item.candidate_work_item_id
             JOIN stage_worker_outputs output
               ON output.id=artifact.stage_worker_output_id
            WHERE candidate_item.stage_work_item_id=$1"#,
    )
    .bind(work_item_id)
    .fetch_optional(&mut **tx)
    .await?
    .map(|row| CandidateArtifactReceiptRow {
        artifact_id: row.0,
        artifact_hash: row.1,
        replayed: true,
    }))
}

pub async fn persist_candidate_worker_artifact(
    pool: &PgPool,
    input: PersistCandidateWorkerArtifact,
) -> Result<CandidateArtifactReceiptRow> {
    if !matches!(
        input.artifact_kind.as_str(),
        "hypothesis_proposal.v1"
            | "proposal_conflict_review.v1"
            | "hypothesis_coverage_subreview.v1"
            | "hypothesis_coverage_synthesis.v1"
            | "controller_decision.v1"
    ) || !input.artifact_body.is_object()
    {
        return Err(conflict("CANDIDATE_ARTIFACT_KIND_INVALID"));
    }
    let mut tx = pool.begin().await?;
    // Serializes proposal ordinals and makes a multi-proposal artifact one
    // indivisible response-loss unit.
    sqlx::query("SELECT analysis_attempt_id FROM candidate_analysis_attempts WHERE analysis_attempt_id=$1 FOR UPDATE")
        .bind(input.fence.analysis_attempt_id)
        .fetch_one(&mut *tx)
        .await?;
    let capability: String = sqlx::query_scalar(
        r#"SELECT capability FROM candidate_analysis_work_items
            WHERE analysis_attempt_id=$1 AND stage_work_item_id=$2"#,
    )
    .bind(input.fence.analysis_attempt_id)
    .bind(input.fence.work_item_id)
    .fetch_optional(&mut *tx)
    .await?
    .ok_or_else(|| conflict("CANDIDATE_WORK_ITEM_AUTHORITY_MISSING"))?;
    let expected_artifact_kind = match capability.as_str() {
        "hypothesis_proposal" => "hypothesis_proposal.v1",
        "proposal_conflict_review" => "proposal_conflict_review.v1",
        "hypothesis_coverage_subreview" => "hypothesis_coverage_subreview.v1",
        "coverage_cross_chunk_synthesis"
        | "coverage_cross_input_partition"
        | "coverage_cross_input_reduce"
        | "coverage_cross_dimension_reduce"
        | "coverage_global_semantic_root" => "hypothesis_coverage_synthesis.v1",
        "candidate_controller_final" => "controller_decision.v1",
        _ => return Err(conflict("CANDIDATE_ARTIFACT_CAPABILITY_INVALID")),
    };
    if input.artifact_kind != expected_artifact_kind {
        return Err(conflict("CANDIDATE_ARTIFACT_CAPABILITY_MISMATCH"));
    }
    if let Some(receipt) = artifact_receipt_for_work_on(&mut tx, input.fence.work_item_id).await? {
        let same_owner: bool = sqlx::query_scalar(
            r#"SELECT EXISTS(
                 SELECT 1
                   FROM candidate_analysis_attempts attempt
                   JOIN candidate_analysis_snapshots snapshot
                     ON snapshot.snapshot_id=attempt.snapshot_id
                   JOIN stage_work_items item
                     ON item.id=$2 AND item.team_plan_id=$3
                    AND item.operation_id=snapshot.operation_id
                    AND item.scope_snapshot_id=snapshot.scope_snapshot_id
                    AND item.organization_id=snapshot.organization_id
                   JOIN stage_worker_runs worker
                     ON worker.id=$4 AND worker.work_item_id=item.id
                  WHERE attempt.analysis_attempt_id=$1
                    AND attempt.attempt_ordinal=$5
                    AND snapshot.snapshot_id=$6
                    AND snapshot.operation_id=$7
                    AND snapshot.scope_snapshot_id=$8
                    AND snapshot.organization_id=$9
               )"#,
        )
        .bind(input.fence.analysis_attempt_id)
        .bind(input.fence.work_item_id)
        .bind(input.fence.team_plan_id)
        .bind(input.fence.worker_run_id)
        .bind(input.fence.analysis_attempt_ordinal)
        .bind(input.fence.snapshot_id)
        .bind(input.fence.operation_id)
        .bind(input.fence.scope_snapshot_id)
        .bind(input.fence.organization_id)
        .fetch_one(&mut *tx)
        .await?;
        if !same_owner {
            return Err(conflict("CANDIDATE_REPLAY_OWNER_MISMATCH"));
        }
        let persisted: (Uuid, String) = sqlx::query_as(
            "SELECT provider_attempt_id,artifact_hash FROM candidate_analysis_provider_attempts WHERE stage_work_item_id=$1",
        )
        .bind(input.fence.work_item_id)
        .fetch_one(&mut *tx)
        .await?;
        let expected_hash = hash_json_on(&mut tx, &input.artifact_body).await?;
        if persisted.0 != input.provider_attempt_id || persisted.1 != expected_hash {
            return Err(conflict("CANDIDATE_PROVIDER_ATTEMPT_REPLAY_DRIFT"));
        }
        tx.commit().await?;
        return Ok(receipt);
    }
    super::candidate_analysis::validate_write_fence_on(&mut tx, &input.fence).await?;
    let provider_output_hash = hash_json_on(&mut tx, &input.artifact_body).await?;
    let artifact_id = Uuid::new_v5(&input.provider_attempt_id, input.artifact_kind.as_bytes());
    let output_id = Uuid::new_v5(&artifact_id, b"candidate_stage_worker_output.v1");
    let candidate_work_item_id: Uuid = sqlx::query_scalar(
        "SELECT candidate_work_item_id FROM candidate_analysis_work_items WHERE analysis_attempt_id=$1 AND stage_work_item_id=$2",
    )
    .bind(input.fence.analysis_attempt_id)
    .bind(input.fence.work_item_id)
    .fetch_optional(&mut *tx)
    .await?
    .ok_or_else(|| conflict("CANDIDATE_WORK_ITEM_AUTHORITY_MISSING"))?;
    let existing_artifact_hash: Option<String> = sqlx::query_scalar(
        "SELECT artifact_hash FROM candidate_analysis_artifacts WHERE artifact_id=$1",
    )
    .bind(artifact_id)
    .fetch_optional(&mut *tx)
    .await?;
    let artifact_hash = if let Some(existing_hash) = existing_artifact_hash {
        existing_hash
    } else {
        sqlx::query(
            r#"INSERT INTO candidate_analysis_artifacts(
               artifact_id,analysis_attempt_id,candidate_work_item_id,worker_run_id,
               stage_worker_output_id,artifact_kind,artifact_body,artifact_hash)
           VALUES($1,$2,$3,$4,$5,$6,$7,$8)"#,
        )
        .bind(artifact_id)
        .bind(input.fence.analysis_attempt_id)
        .bind(candidate_work_item_id)
        .bind(input.fence.worker_run_id)
        .bind(output_id)
        .bind(&input.artifact_kind)
        .bind(&input.artifact_body)
        .bind(&provider_output_hash)
        .execute(&mut *tx)
        .await?;
        if input.artifact_kind == "hypothesis_proposal.v1" {
            persist_proposals_on(&mut tx, &input, artifact_id).await?;
        } else if input.artifact_kind == "proposal_conflict_review.v1" {
            persist_conflict_review_on(&mut tx, &input, artifact_id, &provider_output_hash).await?;
        }
        provider_output_hash.clone()
    };
    sqlx::query(
        r#"INSERT INTO candidate_analysis_provider_attempts(
               provider_attempt_id,analysis_attempt_id,stage_work_item_id,worker_run_id,
               artifact_kind,artifact_body,artifact_hash,artifact_id)
           VALUES($1,$2,$3,$4,$5,$6,$7,$8)"#,
    )
    .bind(input.provider_attempt_id)
    .bind(input.fence.analysis_attempt_id)
    .bind(input.fence.work_item_id)
    .bind(input.fence.worker_run_id)
    .bind(&input.artifact_kind)
    .bind(&input.artifact_body)
    .bind(&provider_output_hash)
    .bind(artifact_id)
    .execute(&mut *tx)
    .await?;
    let canonical_output = json!({
        "schema":"candidate_analysis_artifact_receipt.v1",
        "artifact_id":artifact_id,
        "artifact_hash":artifact_hash,
    });
    let output_hash = hash_json_on(&mut tx, &canonical_output).await?;
    sqlx::query(
        r#"INSERT INTO stage_worker_outputs(
               id,team_plan_id,work_item_id,worker_run_id,operation_id,stage_execution_id,
               stage_run_unit_id,scope_snapshot_id,organization_id,output_schema,output_version,
               business_disposition,canonical_output,canonical_fact_refs,evidence_ids,
               checked_empty_cells,blocker_codes,output_hash)
           SELECT $1,plan.id,item.id,worker.id,plan.operation_id,plan.stage_execution_id,
                  plan.stage_run_unit_id,plan.scope_snapshot_id,plan.organization_id,
                  'candidate_analysis_artifact_receipt.v1',1,'artifact_recorded',$2,
                  '[]',ARRAY[]::BIGINT[],'[]',ARRAY[]::TEXT[],$3
             FROM stage_team_plans plan
             JOIN stage_work_items item ON item.team_plan_id=plan.id
             JOIN stage_worker_runs worker ON worker.work_item_id=item.id
            WHERE plan.id=$4 AND item.id=$5 AND worker.id=$6"#,
    )
    .bind(output_id)
    .bind(canonical_output)
    .bind(output_hash)
    .bind(input.fence.team_plan_id)
    .bind(input.fence.work_item_id)
    .bind(input.fence.worker_run_id)
    .execute(&mut *tx)
    .await?;
    sqlx::query(
        "UPDATE stage_work_items SET status='completed',terminal_at=statement_timestamp(),row_version=row_version+1,updated_at=statement_timestamp() WHERE id=$1",
    )
    .bind(input.fence.work_item_id)
    .execute(&mut *tx)
    .await?;
    sqlx::query(
        "UPDATE stage_worker_runs SET status='passed',terminal_at=statement_timestamp(),updated_at=statement_timestamp() WHERE id=$1",
    )
    .bind(input.fence.worker_run_id)
    .execute(&mut *tx)
    .await?;
    tx.commit().await?;
    Ok(CandidateArtifactReceiptRow {
        artifact_id,
        artifact_hash,
        replayed: false,
    })
}

async fn persist_conflict_review_on(
    tx: &mut Transaction<'_, Postgres>,
    input: &PersistCandidateWorkerArtifact,
    artifact_id: Uuid,
    artifact_hash: &str,
) -> Result<()> {
    let object = input
        .artifact_body
        .as_object()
        .ok_or_else(|| conflict("CANDIDATE_CONFLICT_REVIEW_INVALID"))?;
    if object.len() != 4 || object.get("mode").and_then(Value::as_str) != Some("proposal_conflict")
    {
        return Err(conflict("CANDIDATE_CONFLICT_REVIEW_INVALID"));
    }
    let component_id = object
        .get("conflict_component_id")
        .and_then(Value::as_str)
        .and_then(|value| Uuid::parse_str(value).ok())
        .ok_or_else(|| conflict("CANDIDATE_CONFLICT_REVIEW_INVALID"))?;
    let decision = object
        .get("decision")
        .and_then(Value::as_str)
        .ok_or_else(|| conflict("CANDIDATE_CONFLICT_REVIEW_INVALID"))?;
    let decision_kind = match decision {
        "no_conflict" => "keep_distinct",
        "duplicate" => "duplicate",
        "merge" => "merge",
        "split_required" => "split_required",
        "blocked" => "blocked",
        _ => return Err(conflict("CANDIDATE_CONFLICT_REVIEW_INVALID")),
    };
    let mut related_ids = object
        .get("related_proposal_ids")
        .and_then(Value::as_array)
        .ok_or_else(|| conflict("CANDIDATE_CONFLICT_REVIEW_INVALID"))?
        .iter()
        .map(|value| {
            value
                .as_str()
                .and_then(|value| Uuid::parse_str(value).ok())
                .ok_or_else(|| conflict("CANDIDATE_CONFLICT_REVIEW_INVALID"))
        })
        .collect::<Result<Vec<_>>>()?;
    let original_related_count = related_ids.len();
    related_ids.sort_unstable();
    related_ids.dedup();
    if related_ids.len() != original_related_count {
        return Err(conflict("CANDIDATE_CONFLICT_REVIEW_INVALID"));
    }
    let component: (String, String) = sqlx::query_as(
        r#"SELECT component.component_hash,component.proposal_set_hash
              FROM candidate_analysis_conflict_components component
              JOIN candidate_analysis_work_items work
                ON work.analysis_attempt_id=component.analysis_attempt_id
               AND work.stage_work_item_id=$2
               AND work.phase='critic'
               AND work.capability='proposal_conflict_review'
               AND work.component_id=component.conflict_component_id
             WHERE component.conflict_component_id=$1
               AND component.analysis_attempt_id=$3"#,
    )
    .bind(component_id)
    .bind(input.fence.work_item_id)
    .bind(input.fence.analysis_attempt_id)
    .fetch_optional(&mut **tx)
    .await?
    .ok_or_else(|| conflict("CANDIDATE_CONFLICT_REVIEW_AUTHORITY_MISMATCH"))?;
    let component_proposal_ids: Vec<Uuid> = sqlx::query_scalar(
        r#"SELECT proposal_id FROM candidate_analysis_conflict_component_members
             WHERE conflict_component_id=$1 AND analysis_attempt_id=$2
             ORDER BY ordinal,proposal_id"#,
    )
    .bind(component_id)
    .bind(input.fence.analysis_attempt_id)
    .fetch_all(&mut **tx)
    .await?;
    if component_proposal_ids.is_empty()
        || related_ids
            .iter()
            .any(|proposal_id| !component_proposal_ids.contains(proposal_id))
    {
        return Err(conflict("CANDIDATE_CONFLICT_REVIEW_AUTHORITY_MISMATCH"));
    }
    let canonical_decision = json!({
        "domain":"candidate_conflict_review_decision.v1",
        "analysis_attempt_id":input.fence.analysis_attempt_id,
        "conflict_component_id":component_id,
        "component_hash":component.0,
        "source_proposal_ids":component_proposal_ids,
        "source_proposal_set_hash":component.1,
        "artifact_id":artifact_id,
        "artifact_hash":artifact_hash,
        "decision":decision,
        "decision_kind":decision_kind,
        "related_proposal_ids":related_ids,
    });
    let decision_hash = hash_json_on(tx, &canonical_decision).await?;
    let decision_id = Uuid::new_v5(&component_id, b"candidate_conflict_review_decision.v1");
    let existing: Option<(String, String, Value, String)> = sqlx::query_as(
        r#"SELECT decision_kind,source_proposal_set_hash,canonical_decision,decision_hash
              FROM hypothesis_merge_decisions
             WHERE merge_decision_id=$1 AND conflict_component_id=$2"#,
    )
    .bind(decision_id)
    .bind(component_id)
    .fetch_optional(&mut **tx)
    .await?;
    if let Some(existing) = existing {
        if existing
            != (
                decision_kind.to_owned(),
                component.1,
                canonical_decision,
                decision_hash,
            )
        {
            return Err(conflict("CANDIDATE_CONFLICT_REVIEW_REPLAY_DRIFT"));
        }
    } else {
        sqlx::query(
            r#"INSERT INTO hypothesis_merge_decisions(
                   merge_decision_id,analysis_attempt_id,conflict_component_id,decision_kind,
                   source_proposal_set_hash,canonical_decision,decision_hash)
               VALUES($1,$2,$3,$4,$5,$6,$7)"#,
        )
        .bind(decision_id)
        .bind(input.fence.analysis_attempt_id)
        .bind(component_id)
        .bind(decision_kind)
        .bind(component.1)
        .bind(canonical_decision)
        .bind(decision_hash)
        .execute(&mut **tx)
        .await?;
    }
    Ok(())
}

async fn persist_proposals_on(
    tx: &mut Transaction<'_, Postgres>,
    input: &PersistCandidateWorkerArtifact,
    artifact_id: Uuid,
) -> Result<()> {
    let artifact_object = input
        .artifact_body
        .as_object()
        .ok_or_else(|| conflict("CANDIDATE_PROPOSAL_ARTIFACT_INVALID"))?;
    if artifact_object.len() != 1 || !artifact_object.contains_key("proposals") {
        return Err(conflict("CANDIDATE_PROPOSAL_BLOCKED_INPUT_INVALID"));
    }
    let proposals = input
        .artifact_body
        .get("proposals")
        .and_then(Value::as_array)
        .ok_or_else(|| conflict("CANDIDATE_PROPOSAL_ARTIFACT_INVALID"))?;
    if proposals.len() > 16 {
        return Err(conflict("CANDIDATE_PROPOSAL_ARTIFACT_LIMIT_EXCEEDED"));
    }
    let owned_input_id: Uuid = sqlx::query_scalar(
        r#"SELECT candidate.microbatch_key::UUID
             FROM candidate_analysis_work_items candidate
             JOIN candidate_analysis_snapshot_inputs source
               ON source.snapshot_input_id=candidate.microbatch_key::UUID
             JOIN candidate_analysis_attempts attempt
               ON attempt.analysis_attempt_id=candidate.analysis_attempt_id
              AND attempt.snapshot_id=source.snapshot_id
            WHERE candidate.stage_work_item_id=$1
              AND candidate.analysis_attempt_id=$2
              AND candidate.phase='proposal'
              AND candidate.capability='hypothesis_proposal'"#,
    )
    .bind(input.fence.work_item_id)
    .bind(input.fence.analysis_attempt_id)
    .fetch_optional(&mut **tx)
    .await?
    .ok_or_else(|| conflict("CANDIDATE_PROPOSAL_WORK_OWNERSHIP_INVALID"))?;
    // Analyst work is intentionally concurrent, but the proposal ordinal is
    // an attempt-wide immutable sequence. Lock the attempt row before reading
    // MAX so independent artifact transactions cannot allocate the same
    // ordinal from the same snapshot.
    let locked_attempt_id: Uuid = sqlx::query_scalar(
        "SELECT analysis_attempt_id FROM candidate_analysis_attempts WHERE analysis_attempt_id=$1 FOR UPDATE",
    )
    .bind(input.fence.analysis_attempt_id)
    .fetch_one(&mut **tx)
    .await?;
    if locked_attempt_id != input.fence.analysis_attempt_id {
        return Err(conflict("CANDIDATE_PROPOSAL_WORK_OWNERSHIP_INVALID"));
    }
    let mut next_ordinal: i32 = sqlx::query_scalar(
        "SELECT COALESCE(MAX(proposal_ordinal)+1,0)::INTEGER FROM hypothesis_proposals WHERE analysis_attempt_id=$1",
    )
    .bind(input.fence.analysis_attempt_id)
    .fetch_one(&mut **tx)
    .await?;
    for proposal in proposals {
        validate_proposal_knowledge_signals_on(
            tx,
            input.fence.analysis_attempt_id,
            input.fence.work_item_id,
            input.fence.worker_run_id,
            proposal,
        )
        .await?;
        let proposal_id = proposal
            .get("proposal_id")
            .and_then(Value::as_str)
            .and_then(|value| Uuid::parse_str(value).ok())
            .ok_or_else(|| conflict("CANDIDATE_PROPOSAL_ARTIFACT_INVALID"))?;
        let proposal_hash = hash_json_on(tx, proposal).await?;
        sqlx::query(
            r#"INSERT INTO hypothesis_proposals(
                   proposal_id,analysis_attempt_id,artifact_id,proposal_ordinal,
                   structured_proposal,proposal_hash)
               VALUES($1,$2,$3,$4,$5,$6)"#,
        )
        .bind(proposal_id)
        .bind(input.fence.analysis_attempt_id)
        .bind(artifact_id)
        .bind(next_ordinal)
        .bind(proposal)
        .bind(proposal_hash)
        .execute(&mut **tx)
        .await?;
        if let Some(refs) = proposal.get("proof_refs").and_then(Value::as_array) {
            for proof_ref in refs {
                let input_id = proof_ref
                    .get("input_id")
                    .and_then(Value::as_str)
                    .and_then(|value| Uuid::parse_str(value).ok())
                    .ok_or_else(|| conflict("CANDIDATE_PROPOSAL_REF_INVALID"))?;
                let chunk_id = proof_ref
                    .get("chunk_id")
                    .and_then(Value::as_str)
                    .and_then(|value| Uuid::parse_str(value).ok())
                    .ok_or_else(|| conflict("CANDIDATE_PROPOSAL_REF_UNDELIVERED_CHUNK"))?;
                if input_id != owned_input_id {
                    return Err(conflict("CANDIDATE_PROPOSAL_REF_FOREIGN_INPUT"));
                }
                let source_hash = proof_ref
                    .get("source_hash")
                    .and_then(Value::as_str)
                    .ok_or_else(|| conflict("CANDIDATE_PROPOSAL_REF_INVALID"))?;
                let source_role = match proof_ref.get("role").and_then(Value::as_str) {
                    Some("support") => "support",
                    Some("contradiction") => "contradiction",
                    Some("authorization_use") => "application_context",
                    Some("gap") => "gap",
                    _ => return Err(conflict("CANDIDATE_PROPOSAL_REF_INVALID")),
                };
                let (persisted_source_kind, persisted_source_content_hash): (String, String) =
                    sqlx::query_as(
                    r#"SELECT source_kind,source_content_hash FROM candidate_analysis_snapshot_inputs
                        WHERE snapshot_input_id=$1 AND snapshot_id=$2"#,
                )
                .bind(input_id)
                .bind(input.fence.snapshot_id)
                .fetch_optional(&mut **tx)
                .await?
                .ok_or_else(|| conflict("CANDIDATE_PROPOSAL_REF_INVALID"))?;
                if source_hash != persisted_source_content_hash {
                    return Err(conflict("CANDIDATE_PROPOSAL_REF_SOURCE_HASH_MISMATCH"));
                }
                let delivered: bool = sqlx::query_scalar(
                    r#"SELECT EXISTS(
                           SELECT 1
                             FROM candidate_analysis_input_chunk_census_members chunk
                             JOIN candidate_analysis_input_chunk_censuses census
                               ON census.chunk_census_id=chunk.chunk_census_id
                              AND census.snapshot_input_id=chunk.snapshot_input_id
                              AND census.snapshot_id=chunk.snapshot_id
                             JOIN candidate_analysis_page_receipts receipt
                               ON receipt.analysis_attempt_id=$1
                              AND receipt.snapshot_id=chunk.snapshot_id
                              AND receipt.page_kind='chunk_page'
                              AND receipt.snapshot_input_id=chunk.snapshot_input_id
                              AND receipt.chunk_census_id=census.chunk_census_id
                              AND receipt.chunk_census_hash=census.census_hash
                              AND receipt.source_size_bytes=census.source_byte_count
                              AND receipt.chunking_contract_version=census.chunking_contract_version
                              AND receipt.redaction_contract_version=census.redaction_contract_version
                              AND receipt.consumer_worker_run_id=$2
                              AND receipt.first_key::INTEGER<=chunk.ordinal
                              AND receipt.last_key::INTEGER>=chunk.ordinal
                              AND receipt.server_cursor LIKE 'chunk:%'
                             JOIN candidate_analysis_work_items candidate_item
                               ON candidate_item.stage_work_item_id=$3
                              AND candidate_item.analysis_attempt_id=$1
                              AND candidate_item.microbatch_key=chunk.snapshot_input_id::TEXT
                             JOIN candidate_analysis_work_page_authorities page_authority
                               ON page_authority.candidate_work_item_id=
                                      candidate_item.candidate_work_item_id
                              AND page_authority.analysis_attempt_id=
                                      candidate_item.analysis_attempt_id
                              AND page_authority.page_receipt_id=receipt.page_receipt_id
                              AND page_authority.page_authority_set_hash=tool_truth_sha256(
                                  to_jsonb(ARRAY[receipt.page_hash]::TEXT[])::TEXT
                              )
                            WHERE chunk.chunk_id=$4
                              AND chunk.snapshot_input_id=$5
                              AND chunk.snapshot_id=$6
                              AND census.disposition='complete'
                       )"#,
                )
                .bind(input.fence.analysis_attempt_id)
                .bind(input.fence.worker_run_id)
                .bind(input.fence.work_item_id)
                .bind(chunk_id)
                .bind(owned_input_id)
                .bind(input.fence.snapshot_id)
                .fetch_one(&mut **tx)
                .await?;
                if !delivered {
                    return Err(conflict("CANDIDATE_PROPOSAL_REF_UNDELIVERED_CHUNK"));
                }
                if matches!(
                    persisted_source_kind.as_str(),
                    "knowledge_signal" | "managed_knowledge_feed"
                ) || (persisted_source_kind == "application_context"
                    && source_role != "application_context")
                {
                    return Err(conflict("CANDIDATE_NON_PROOF_SOURCE_REJECTED"));
                }
                let ref_hash = hash_json_on(
                    tx,
                    &json!({
                        "proposal_id":proposal_id,
                        "snapshot_input_id":input_id,
                        "chunk_id":chunk_id,
                        "source_role":source_role,
                        "source_hash":source_hash,
                    }),
                )
                .await?;
                sqlx::query(
                    r#"INSERT INTO hypothesis_proposal_refs(
                           proposal_ref_id,proposal_id,analysis_attempt_id,snapshot_input_id,
                           chunk_id,source_role,source_hash,ref_hash)
                       VALUES($1,$2,$3,$4,$5,$6,$7,$8)"#,
                )
                .bind(Uuid::new_v5(&proposal_id, ref_hash.as_bytes()))
                .bind(proposal_id)
                .bind(input.fence.analysis_attempt_id)
                .bind(input_id)
                .bind(chunk_id)
                .bind(source_role)
                .bind(source_hash)
                .bind(ref_hash)
                .execute(&mut **tx)
                .await?;
            }
        }
        next_ordinal = next_ordinal
            .checked_add(1)
            .ok_or_else(|| conflict("CANDIDATE_PROPOSAL_ORDINAL_OVERFLOW"))?;
    }
    Ok(())
}

/// Seals H1, verifies the frozen host catalogs, derives the complete
/// attack-class × trust-boundary checklist for every input, and creates one
/// bounded subreview for every checklist × chunk-partition tuple. Wider
/// recursive synthesis opens only after every map review is durable.
#[derive(Debug, Clone, sqlx::FromRow)]
struct CriticInputDbRow {
    snapshot_input_id: Uuid,
    source_ref: String,
    subject_kind_at_time: String,
    subject_identity_hash: String,
    chunk_census_id: Uuid,
    census_hash: String,
    chunk_count: i64,
    chunking_contract_version: String,
    redaction_contract_version: String,
    chunk_disposition: String,
}

#[derive(Debug, PartialEq, sqlx::FromRow)]
struct ChecklistReplayDbRow {
    checklist_member_id: Uuid,
    attack_class_contract_version: String,
    attack_class_contract_digest: String,
    trust_boundary_contract_version: String,
    trust_boundary_contract_digest: String,
    attack_class_id: String,
    attack_class_version: i32,
    trust_boundary_identity: String,
    trust_boundary_hash: String,
    applicability_basis: Value,
    feed_match_member_refs: Vec<Uuid>,
    applicability_disposition: String,
    enrichment_obligation_id: Option<Uuid>,
    member_hash: String,
}

#[derive(Debug)]
struct CriticWorkDraft {
    input_id: Uuid,
    checklist_id: Uuid,
    partition_id: Uuid,
    item_id: Uuid,
    worker_id: Uuid,
    lane_ordinal: i32,
    provisional_input: Value,
    input_authority: CriticInputDbRow,
    source_size_bytes: i64,
    partition_ordinal: i32,
    chunk_hash: String,
    chunk_ordinal: i32,
}

fn proposal_summary_value(
    proposal_id: Uuid,
    structured: &Value,
    proof_input_ids: Vec<Uuid>,
) -> Result<Value> {
    let object = structured
        .as_object()
        .ok_or_else(|| conflict("CANDIDATE_PROPOSAL_SUMMARY_INVALID"))?;
    let text = |name: &'static str| -> Result<&str> {
        object
            .get(name)
            .and_then(Value::as_str)
            .filter(|value| !value.trim().is_empty())
            .ok_or_else(|| conflict("CANDIDATE_PROPOSAL_SUMMARY_INVALID"))
    };
    let predicate_version = object
        .get("predicate_version")
        .and_then(Value::as_u64)
        .and_then(|value| u32::try_from(value).ok())
        .filter(|value| *value > 0)
        .ok_or_else(|| conflict("CANDIDATE_PROPOSAL_SUMMARY_INVALID"))?;
    Ok(json!({
        "proposal_id":proposal_id,
        "subject_kind":text("subject_kind")?,
        "subject_identity_hash":text("subject_identity_hash")?,
        "predicate_schema":text("predicate_schema")?,
        "predicate_version":predicate_version,
        "polarity":text("polarity")?,
        "trust_boundary":text("trust_boundary")?,
        "readiness":text("readiness")?,
        "proof_input_ids":proof_input_ids,
    }))
}

fn conflict_proposal_summary_value(
    proposal_id: Uuid,
    proposal_hash: &str,
    structured: &Value,
    proof_input_ids: Vec<Uuid>,
    application_context_input_ids: Vec<Uuid>,
    gap_input_ids: Vec<Uuid>,
) -> Result<Value> {
    let object = structured
        .as_object()
        .ok_or_else(|| conflict("CANDIDATE_CONFLICT_SUMMARY_INVALID"))?;
    let text = |name: &'static str, max_len: usize| -> Result<String> {
        object
            .get(name)
            .and_then(Value::as_str)
            .filter(|value| !value.trim().is_empty() && value.len() <= max_len)
            .map(str::to_owned)
            .ok_or_else(|| conflict("CANDIDATE_CONFLICT_SUMMARY_INVALID"))
    };
    let predicate_version = object
        .get("predicate_version")
        .and_then(Value::as_u64)
        .and_then(|value| u32::try_from(value).ok())
        .filter(|value| *value > 0)
        .ok_or_else(|| conflict("CANDIDATE_CONFLICT_SUMMARY_INVALID"))?;
    let mut predicate_arguments = object
        .get("predicate_arguments")
        .and_then(Value::as_array)
        .filter(|values| values.len() <= 32)
        .ok_or_else(|| conflict("CANDIDATE_CONFLICT_SUMMARY_INVALID"))?
        .iter()
        .map(|entry| {
            let pair = entry
                .as_array()
                .filter(|pair| pair.len() == 2)
                .ok_or_else(|| conflict("CANDIDATE_CONFLICT_SUMMARY_INVALID"))?;
            let key = pair[0]
                .as_str()
                .filter(|value| !value.trim().is_empty() && value.len() <= 256)
                .ok_or_else(|| conflict("CANDIDATE_CONFLICT_SUMMARY_INVALID"))?;
            let value = pair[1]
                .as_str()
                .filter(|value| !value.trim().is_empty() && value.len() <= 1024)
                .ok_or_else(|| conflict("CANDIDATE_CONFLICT_SUMMARY_INVALID"))?;
            Ok((key.to_owned(), value.to_owned()))
        })
        .collect::<Result<Vec<_>>>()?;
    predicate_arguments.sort();
    if predicate_arguments
        .windows(2)
        .any(|pair| pair[0].0 == pair[1].0)
    {
        return Err(conflict("CANDIDATE_CONFLICT_SUMMARY_INVALID"));
    }
    let preconditions = object
        .get("preconditions")
        .and_then(Value::as_array)
        .filter(|values| values.len() <= 16)
        .ok_or_else(|| conflict("CANDIDATE_CONFLICT_SUMMARY_INVALID"))?
        .iter()
        .map(|value| {
            value
                .as_str()
                .filter(|value| !value.trim().is_empty() && value.len() <= 1024)
                .map(str::to_owned)
                .ok_or_else(|| conflict("CANDIDATE_CONFLICT_SUMMARY_INVALID"))
        })
        .collect::<Result<Vec<_>>>()?;
    let knowledge_signals = object
        .get("knowledge_signals")
        .and_then(Value::as_array)
        .filter(|values| values.len() <= 32)
        .ok_or_else(|| conflict("CANDIDATE_CONFLICT_SUMMARY_INVALID"))?;
    for signal in knowledge_signals {
        let signal = signal
            .as_object()
            .filter(|signal| signal.len() == 5)
            .ok_or_else(|| conflict("CANDIDATE_CONFLICT_SUMMARY_INVALID"))?;
        for id in ["feed_snapshot_id", "feed_match_member_id"] {
            signal
                .get(id)
                .and_then(Value::as_str)
                .and_then(|value| Uuid::parse_str(value).ok())
                .ok_or_else(|| conflict("CANDIDATE_CONFLICT_SUMMARY_INVALID"))?;
        }
        for hash in ["feed_match_member_hash", "product_version_match_hash"] {
            signal
                .get(hash)
                .and_then(Value::as_str)
                .filter(|value| value.starts_with("sha256:") && value.len() == 71)
                .ok_or_else(|| conflict("CANDIDATE_CONFLICT_SUMMARY_INVALID"))?;
        }
        if signal.get("source_authority").and_then(Value::as_str) != Some("knowledge_signal_only") {
            return Err(conflict("CANDIDATE_CONFLICT_SUMMARY_INVALID"));
        }
    }
    if !proposal_hash.starts_with("sha256:") || proposal_hash.len() != 71 {
        return Err(conflict("CANDIDATE_CONFLICT_SUMMARY_INVALID"));
    }
    Ok(json!({
        "proposal_id":proposal_id,
        "proposal_hash":proposal_hash,
        "subject_kind":text("subject_kind",256)?,
        "subject_identity_hash":text("subject_identity_hash",1024)?,
        "predicate_schema":text("predicate_schema",256)?,
        "predicate_version":predicate_version,
        "predicate_arguments":predicate_arguments,
        "polarity":text("polarity",64)?,
        "trust_boundary":text("trust_boundary",256)?,
        "readiness":text("readiness",64)?,
        "structured_claim":text("structured_claim",4096)?,
        "preconditions":preconditions,
        "impact":text("impact",4096)?,
        "proof_input_ids":proof_input_ids,
        "application_context_input_ids":application_context_input_ids,
        "gap_input_ids":gap_input_ids,
        "knowledge_signals":knowledge_signals,
    }))
}

async fn validate_proposal_knowledge_signals_on(
    tx: &mut Transaction<'_, Postgres>,
    analysis_attempt_id: Uuid,
    stage_work_item_id: Uuid,
    worker_run_id: Uuid,
    structured: &Value,
) -> Result<()> {
    let signals = structured
        .get("knowledge_signals")
        .and_then(Value::as_array)
        .filter(|signals| signals.len() <= 32)
        .ok_or_else(|| conflict("CANDIDATE_KNOWLEDGE_SIGNAL_AUTHORITY_INVALID"))?;
    let mut seen = BTreeSet::new();
    for signal in signals {
        let signal = signal
            .as_object()
            .filter(|signal| signal.len() == 5)
            .ok_or_else(|| conflict("CANDIDATE_KNOWLEDGE_SIGNAL_AUTHORITY_INVALID"))?;
        if signal.get("source_authority").and_then(Value::as_str) != Some("knowledge_signal_only") {
            return Err(conflict("CANDIDATE_KNOWLEDGE_SIGNAL_AUTHORITY_INVALID"));
        }
        let feed_snapshot_id = signal
            .get("feed_snapshot_id")
            .and_then(Value::as_str)
            .and_then(|value| Uuid::parse_str(value).ok())
            .ok_or_else(|| conflict("CANDIDATE_KNOWLEDGE_SIGNAL_AUTHORITY_INVALID"))?;
        let feed_match_member_id = signal
            .get("feed_match_member_id")
            .and_then(Value::as_str)
            .and_then(|value| Uuid::parse_str(value).ok())
            .ok_or_else(|| conflict("CANDIDATE_KNOWLEDGE_SIGNAL_AUTHORITY_INVALID"))?;
        let feed_match_member_hash = signal
            .get("feed_match_member_hash")
            .and_then(Value::as_str)
            .ok_or_else(|| conflict("CANDIDATE_KNOWLEDGE_SIGNAL_AUTHORITY_INVALID"))?;
        let product_version_match_hash = signal
            .get("product_version_match_hash")
            .and_then(Value::as_str)
            .ok_or_else(|| conflict("CANDIDATE_KNOWLEDGE_SIGNAL_AUTHORITY_INVALID"))?;
        if !seen.insert((
            feed_snapshot_id,
            feed_match_member_id,
            feed_match_member_hash.to_owned(),
            product_version_match_hash.to_owned(),
        )) {
            return Err(conflict("CANDIDATE_KNOWLEDGE_SIGNAL_AUTHORITY_INVALID"));
        }
        let authorized: bool = sqlx::query_scalar(
            r#"SELECT EXISTS(
                   SELECT 1
                     FROM candidate_analysis_attempts attempt
                     JOIN candidate_analysis_work_items candidate
                       ON candidate.analysis_attempt_id=attempt.analysis_attempt_id
                      AND candidate.stage_work_item_id=$6
                      AND candidate.phase='proposal'
                      AND candidate.capability='hypothesis_proposal'
                     JOIN candidate_analysis_feed_match_censuses census
                       ON census.snapshot_id=attempt.snapshot_id
                     JOIN candidate_analysis_feed_match_census_members match
                       ON match.match_census_id=census.match_census_id
                      AND match.snapshot_id=census.snapshot_id
                     JOIN candidate_analysis_knowledge_feed_snapshot_members feed
                       ON feed.feed_snapshot_member_id=match.feed_snapshot_member_id
                      AND feed.feed_snapshot_id=census.feed_snapshot_id
                      AND feed.snapshot_id=census.snapshot_id
                     JOIN candidate_analysis_product_version_census_members product
                       ON product.product_member_id=match.product_member_id
                      AND product.product_census_id=census.product_census_id
                      AND product.snapshot_id=census.snapshot_id
                     JOIN candidate_analysis_snapshot_inputs source
                       ON source.snapshot_id=attempt.snapshot_id
                      AND source.snapshot_input_id::TEXT=candidate.microbatch_key
                      AND source.source_kind='knowledge_signal'
                      AND source.source_ref='candidate_feed_match_member:'||match.match_member_id::TEXT
                     JOIN candidate_analysis_input_chunk_censuses chunk_census
                       ON chunk_census.snapshot_input_id=source.snapshot_input_id
                      AND chunk_census.snapshot_id=source.snapshot_id
                      AND chunk_census.disposition='complete'
                     JOIN candidate_analysis_page_receipts page
                       ON page.analysis_attempt_id=attempt.analysis_attempt_id
                      AND page.snapshot_id=attempt.snapshot_id
                      AND page.page_kind='chunk_page'
                      AND page.stable_request_id=uuid_generate_v5(
                          candidate.stage_work_item_id,'candidate_analyst_chunk_page.v1'
                      )
                      AND page.page_receipt_id=uuid_generate_v5(
                          page.stable_request_id,'candidate_page_receipt.v1'
                      )
                      AND page.snapshot_input_id=source.snapshot_input_id
                      AND page.chunk_census_id=chunk_census.chunk_census_id
                      AND page.chunk_census_hash=chunk_census.census_hash
                      AND page.source_size_bytes=chunk_census.source_byte_count
                      AND page.chunking_contract_version=chunk_census.chunking_contract_version
                      AND page.redaction_contract_version=chunk_census.redaction_contract_version
                      AND page.consumer_worker_run_id=$7
                      AND page.returned_count=chunk_census.chunk_count
                      AND page.first_key='0'
                      AND page.last_key=(chunk_census.chunk_count-1)::TEXT
                      AND page.server_cursor='chunk:0:'||chunk_census.chunk_count::TEXT
                     JOIN candidate_analysis_work_page_authorities page_authority
                       ON page_authority.candidate_work_item_id=candidate.candidate_work_item_id
                      AND page_authority.analysis_attempt_id=candidate.analysis_attempt_id
                      AND page_authority.page_receipt_id=page.page_receipt_id
                      AND page_authority.page_authority_set_hash=tool_truth_sha256(
                          to_jsonb(ARRAY[page.page_hash]::TEXT[])::TEXT
                      )
                    WHERE attempt.analysis_attempt_id=$1
                      AND match.match_member_id=$2
                      AND feed.feed_snapshot_id=$3
                      AND match.member_hash=$4
                      AND product.member_hash=$5
                      AND match.disposition='matched'
                      AND feed.disposition='current'
                      AND product.disposition='known'
               )"#,
        )
        .bind(analysis_attempt_id)
        .bind(feed_match_member_id)
        .bind(feed_snapshot_id)
        .bind(feed_match_member_hash)
        .bind(product_version_match_hash)
        .bind(stage_work_item_id)
        .bind(worker_run_id)
        .fetch_one(&mut **tx)
        .await?;
        if !authorized {
            return Err(conflict("CANDIDATE_KNOWLEDGE_SIGNAL_AUTHORITY_INVALID"));
        }
    }
    Ok(())
}

async fn load_proposal_summaries_on(
    tx: &mut Transaction<'_, Postgres>,
    analysis_attempt_id: Uuid,
    covered_input_ids: &[Uuid],
) -> Result<Vec<Value>> {
    if covered_input_ids.is_empty() {
        return Ok(Vec::new());
    }
    let rows: Vec<(Uuid, Value, Vec<Uuid>)> = sqlx::query_as(
        r#"SELECT proposal.proposal_id,proposal.structured_proposal,
                  COALESCE(ARRAY_AGG(DISTINCT reference.snapshot_input_id
                            ORDER BY reference.snapshot_input_id)
                            FILTER (WHERE reference.snapshot_input_id IS NOT NULL),
                           ARRAY[]::UUID[])
             FROM hypothesis_proposals proposal
             JOIN candidate_analysis_artifacts artifact
               ON artifact.artifact_id=proposal.artifact_id
              AND artifact.analysis_attempt_id=proposal.analysis_attempt_id
             JOIN candidate_analysis_work_items candidate
               ON candidate.candidate_work_item_id=artifact.candidate_work_item_id
              AND candidate.analysis_attempt_id=proposal.analysis_attempt_id
             LEFT JOIN hypothesis_proposal_refs reference
               ON reference.proposal_id=proposal.proposal_id
              AND reference.analysis_attempt_id=proposal.analysis_attempt_id
            WHERE proposal.analysis_attempt_id=$1
              AND (candidate.microbatch_key::UUID=ANY($2)
                   OR reference.snapshot_input_id=ANY($2))
            GROUP BY proposal.proposal_id,proposal.structured_proposal,proposal.proposal_ordinal
            ORDER BY proposal.proposal_id"#,
    )
    .bind(analysis_attempt_id)
    .bind(covered_input_ids)
    .fetch_all(&mut **tx)
    .await?;
    if rows.len() > 64 {
        return Err(conflict("CANDIDATE_PROPOSAL_SUMMARY_CAP_EXCEEDED"));
    }
    rows.into_iter()
        .map(|(proposal_id, structured, proof_input_ids)| {
            proposal_summary_value(proposal_id, &structured, proof_input_ids)
        })
        .collect()
}

async fn load_conflict_proposal_summaries_on(
    tx: &mut Transaction<'_, Postgres>,
    analysis_attempt_id: Uuid,
    covered_input_ids: &[Uuid],
) -> Result<Vec<Value>> {
    if covered_input_ids.is_empty() {
        return Ok(Vec::new());
    }
    type SummaryRow = (
        Uuid,
        String,
        Value,
        Vec<Uuid>,
        Vec<Uuid>,
        Vec<Uuid>,
        Uuid,
        Uuid,
    );
    let rows: Vec<SummaryRow> = sqlx::query_as(
        r#"SELECT proposal.proposal_id,proposal.proposal_hash,proposal.structured_proposal,
                  COALESCE(ARRAY_AGG(DISTINCT reference.snapshot_input_id
                    ORDER BY reference.snapshot_input_id) FILTER (
                      WHERE reference.source_role IN ('support','contradiction')),
                    ARRAY[]::UUID[]),
                  COALESCE(ARRAY_AGG(DISTINCT reference.snapshot_input_id
                    ORDER BY reference.snapshot_input_id) FILTER (
                      WHERE reference.source_role='application_context'),
                    ARRAY[]::UUID[]),
                  COALESCE(ARRAY_AGG(DISTINCT reference.snapshot_input_id
                    ORDER BY reference.snapshot_input_id) FILTER (
                      WHERE reference.source_role='gap'),ARRAY[]::UUID[]),
                  candidate.stage_work_item_id,artifact.worker_run_id
             FROM hypothesis_proposals proposal
             JOIN candidate_analysis_artifacts artifact
               ON artifact.artifact_id=proposal.artifact_id
              AND artifact.analysis_attempt_id=proposal.analysis_attempt_id
             JOIN candidate_analysis_work_items candidate
               ON candidate.candidate_work_item_id=artifact.candidate_work_item_id
              AND candidate.analysis_attempt_id=proposal.analysis_attempt_id
             LEFT JOIN hypothesis_proposal_refs reference
               ON reference.proposal_id=proposal.proposal_id
              AND reference.analysis_attempt_id=proposal.analysis_attempt_id
            WHERE proposal.analysis_attempt_id=$1
              AND (candidate.microbatch_key::UUID=ANY($2)
                   OR reference.snapshot_input_id=ANY($2))
            GROUP BY proposal.proposal_id,proposal.proposal_hash,
                     proposal.structured_proposal,proposal.proposal_ordinal,
                     candidate.stage_work_item_id,artifact.worker_run_id
            ORDER BY proposal.proposal_id"#,
    )
    .bind(analysis_attempt_id)
    .bind(covered_input_ids)
    .fetch_all(&mut **tx)
    .await?;
    if rows.len() > 64 {
        return Err(conflict("CANDIDATE_PROPOSAL_SUMMARY_CAP_EXCEEDED"));
    }
    let mut summaries = Vec::with_capacity(rows.len());
    for (
        proposal_id,
        proposal_hash,
        structured,
        proof,
        context,
        gap,
        stage_work_item_id,
        worker_run_id,
    ) in rows
    {
        validate_proposal_knowledge_signals_on(
            tx,
            analysis_attempt_id,
            stage_work_item_id,
            worker_run_id,
            &structured,
        )
        .await?;
        summaries.push(conflict_proposal_summary_value(
            proposal_id,
            &proposal_hash,
            &structured,
            proof,
            context,
            gap,
        )?);
    }
    Ok(summaries)
}

async fn ensure_input_proposal_dispositions_on(
    tx: &mut Transaction<'_, Postgres>,
    analysis_attempt_id: Uuid,
    snapshot_id: Uuid,
) -> Result<()> {
    let inputs: Vec<(Uuid, String)> = sqlx::query_as(
        r#"SELECT source.snapshot_input_id,census.disposition
             FROM candidate_analysis_snapshot_inputs source
             JOIN candidate_analysis_input_chunk_censuses census
               ON census.snapshot_input_id=source.snapshot_input_id
            WHERE source.snapshot_id=$1 ORDER BY source.stable_input_key"#,
    )
    .bind(snapshot_id)
    .fetch_all(&mut **tx)
    .await?;
    for (input_id, chunk_disposition) in inputs {
        let ref_hashes: Vec<String> = sqlx::query_scalar(
            r#"SELECT ref_hash FROM hypothesis_proposal_refs
                WHERE analysis_attempt_id=$1 AND snapshot_input_id=$2
                ORDER BY ref_hash,proposal_ref_id"#,
        )
        .bind(analysis_attempt_id)
        .bind(input_id)
        .fetch_all(&mut **tx)
        .await?;
        let ref_set_hash = hash_texts_on(tx, &ref_hashes).await?;
        let (disposition, blocker_code) = if chunk_disposition == "complete" {
            (
                if ref_hashes.is_empty() {
                    "zero_proposal"
                } else {
                    "has_proposal"
                },
                None,
            )
        } else {
            if !ref_hashes.is_empty() {
                return Err(conflict("CANDIDATE_NONCOMPLETE_INPUT_HAS_PROPOSAL_REF"));
            }
            (
                "blocked",
                Some(format!("candidate_input_{chunk_disposition}")),
            )
        };
        let disposition_hash = hash_json_on(
            tx,
            &json!({
                "analysis_attempt_id":analysis_attempt_id,
                "snapshot_input_id":input_id,
                "proposal_ref_set_hash":ref_set_hash,
                "disposition":disposition,
                "blocker_code":blocker_code,
            }),
        )
        .await?;
        let expected = (
            i64::try_from(ref_hashes.len()).unwrap_or(i64::MAX),
            ref_set_hash.clone(),
            disposition.to_owned(),
            blocker_code.clone(),
            disposition_hash.clone(),
        );
        let existing: Option<(i64, String, String, Option<String>, String)> = sqlx::query_as(
            r#"SELECT proposal_ref_count,proposal_ref_set_hash,disposition,
                      blocker_code,disposition_hash
                 FROM candidate_analysis_input_proposal_dispositions
                WHERE analysis_attempt_id=$1 AND snapshot_input_id=$2"#,
        )
        .bind(analysis_attempt_id)
        .bind(input_id)
        .fetch_optional(&mut **tx)
        .await?;
        if let Some(existing) = existing {
            if existing != expected {
                return Err(conflict("CANDIDATE_INPUT_DISPOSITION_REPLAY_DRIFT"));
            }
        } else {
            sqlx::query(
                r#"INSERT INTO candidate_analysis_input_proposal_dispositions(
                       disposition_id,analysis_attempt_id,snapshot_input_id,proposal_ref_count,
                       proposal_ref_set_hash,disposition,blocker_code,disposition_hash)
                   VALUES($1,$2,$3,$4,$5,$6,$7,$8)"#,
            )
            .bind(Uuid::new_v5(&analysis_attempt_id, input_id.as_bytes()))
            .bind(analysis_attempt_id)
            .bind(input_id)
            .bind(expected.0)
            .bind(&expected.1)
            .bind(&expected.2)
            .bind(&expected.3)
            .bind(&expected.4)
            .execute(&mut **tx)
            .await?;
        }
    }
    Ok(())
}

pub async fn prepare_critic_work_batch(
    pool: &PgPool,
    snapshot_id: Uuid,
    stage_execution_id: Uuid,
    attempt_ordinal: i32,
    host_lane_limit: i32,
    max_coverage_subreview_work_items: usize,
) -> Result<(String, Vec<CandidateRuntimeWorkRow>, Option<String>)> {
    if host_lane_limit <= 0 || max_coverage_subreview_work_items == 0 {
        return Err(conflict("CANDIDATE_CRITIC_BATCH_INVALID"));
    }
    let opened =
        open_or_replay_attempt_runtime(pool, snapshot_id, stage_execution_id, attempt_ordinal)
            .await?;
    if let Some(terminal) =
        load_terminal_candidate_coverage_closure(pool, opened.analysis_attempt_id).await?
    {
        let h1_hash: Option<String> = sqlx::query_scalar(
            "SELECT census_hash FROM candidate_analysis_proposal_censuses WHERE analysis_attempt_id=$1",
        )
        .bind(opened.analysis_attempt_id)
        .fetch_optional(pool)
        .await?;
        return match terminal {
            CandidateCoverageClosureRow::Blocked { residual_hash } => Ok((
                h1_hash.unwrap_or_else(|| residual_hash.clone()),
                Vec::new(),
                Some(residual_hash),
            )),
            CandidateCoverageClosureRow::RetryAttempt { .. }
            | CandidateCoverageClosureRow::Ready { .. } => Ok((
                h1_hash.ok_or_else(|| conflict("CANDIDATE_TERMINAL_H1_MISSING"))?,
                Vec::new(),
                None,
            )),
        };
    }
    let proposal_count: i64 = sqlx::query_scalar(
        "SELECT count(*) FROM hypothesis_proposals WHERE analysis_attempt_id=$1",
    )
    .bind(opened.analysis_attempt_id)
    .fetch_one(pool)
    .await?;
    if proposal_count > 64 {
        let input_ids: Vec<Uuid> = sqlx::query_scalar(
            "SELECT snapshot_input_id FROM candidate_analysis_snapshot_inputs WHERE snapshot_id=$1 ORDER BY stable_input_key",
        )
        .bind(snapshot_id)
        .fetch_all(pool)
        .await?;
        let mut block_tx = pool.begin().await?;
        let residual_hash = block_candidate_attempt_on(
            &mut block_tx,
            &opened.controller_fence,
            "candidate_h1_proposal_cap_exceeded",
            &input_ids,
        )
        .await?;
        block_tx.commit().await?;
        return Ok((residual_hash.clone(), Vec::new(), Some(residual_hash)));
    }
    let incomplete: i64 = sqlx::query_scalar(
        r#"SELECT count(*) FROM candidate_analysis_work_items candidate_item
             JOIN stage_work_items item ON item.id=candidate_item.stage_work_item_id
            WHERE candidate_item.analysis_attempt_id=$1
              AND candidate_item.phase='proposal'
              AND candidate_item.capability='hypothesis_proposal'
              AND item.status<>'completed'"#,
    )
    .bind(opened.analysis_attempt_id)
    .fetch_one(pool)
    .await?;
    if incomplete != 0 {
        return Err(conflict("CANDIDATE_H1_ANALYST_WAVE_INCOMPLETE"));
    }
    // Build every H1-adjacent denominator and validate the bounded conflict
    // projection before freezing proposals. A malformed empty-proof or
    // knowledge-only proposal therefore cannot strand the attempt behind an
    // already committed H1 seal.
    let mut preseal_tx = pool.begin().await?;
    ensure_input_proposal_dispositions_on(&mut preseal_tx, opened.analysis_attempt_id, snapshot_id)
        .await?;
    let all_input_ids: Vec<Uuid> = sqlx::query_scalar(
        "SELECT snapshot_input_id FROM candidate_analysis_snapshot_inputs WHERE snapshot_id=$1 ORDER BY stable_input_key",
    )
    .bind(snapshot_id)
    .fetch_all(&mut *preseal_tx)
    .await?;
    let preseal_summaries = load_conflict_proposal_summaries_on(
        &mut preseal_tx,
        opened.analysis_attempt_id,
        &all_input_ids,
    )
    .await?;
    if preseal_summaries.len() as i64 != proposal_count {
        return Err(conflict("CANDIDATE_CONFLICT_SUMMARY_EXACT_SET_INVALID"));
    }
    preseal_tx.commit().await?;
    let h1 = super::candidate_analysis::seal_analysis_census(
        pool,
        super::candidate_analysis::SealAnalysisCensusInput {
            fence: opened.controller_fence.clone(),
            stable_census_request_id: Uuid::new_v5(
                &opened.analysis_attempt_id,
                b"candidate_h1_census.v1",
            ),
            census_kind: super::candidate_analysis::AnalysisCensusKindRow::Proposal,
        },
    )
    .await?;
    let mut tx = pool.begin().await?;
    let identity =
        load_scheduler_identity_on(&mut tx, snapshot_id, stage_execution_id, attempt_ordinal)
            .await?;
    let plan_id = ensure_team_plan_on(&mut tx, &identity).await?;
    let canonical_proposals: Vec<(Uuid, String)> = sqlx::query_as(
        r#"SELECT proposal.proposal_id,proposal.proposal_hash
              FROM candidate_analysis_proposal_census_members member
              JOIN hypothesis_proposals proposal ON proposal.proposal_id=member.proposal_id
             WHERE member.analysis_attempt_id=$1
             ORDER BY member.ordinal,member.proposal_id"#,
    )
    .bind(identity.attempt_id)
    .fetch_all(&mut *tx)
    .await?;
    if canonical_proposals.len() > 64 {
        return Err(conflict("CANDIDATE_CONFLICT_COMPONENT_CAP_DRIFT"));
    }
    let conflict_input_ids: Vec<Uuid> = sqlx::query_scalar(
        "SELECT snapshot_input_id FROM candidate_analysis_snapshot_inputs WHERE snapshot_id=$1 ORDER BY stable_input_key",
    )
    .bind(snapshot_id)
    .fetch_all(&mut *tx)
    .await?;
    let conflict_proposal_summaries =
        load_conflict_proposal_summaries_on(&mut tx, identity.attempt_id, &conflict_input_ids)
            .await?;
    let canonical_proposal_ids = canonical_proposals
        .iter()
        .map(|row| row.0)
        .collect::<BTreeSet<_>>();
    let summary_proposal_ids = conflict_proposal_summaries
        .iter()
        .map(|summary| {
            summary
                .get("proposal_id")
                .and_then(Value::as_str)
                .and_then(|value| Uuid::parse_str(value).ok())
                .ok_or_else(|| conflict("CANDIDATE_CONFLICT_SUMMARY_EXACT_SET_INVALID"))
        })
        .collect::<Result<BTreeSet<_>>>()?;
    if conflict_proposal_summaries.len() != canonical_proposals.len()
        || summary_proposal_ids != canonical_proposal_ids
    {
        return Err(conflict("CANDIDATE_CONFLICT_SUMMARY_EXACT_SET_INVALID"));
    }
    let conflict_draft = if canonical_proposals.is_empty() {
        None
    } else {
        let proposal_hashes = canonical_proposals
            .iter()
            .map(|row| row.1.clone())
            .collect::<Vec<_>>();
        let proposal_set_hash = hash_texts_on(&mut tx, &proposal_hashes).await?;
        let conflict_component_id =
            Uuid::new_v5(&identity.attempt_id, b"candidate_conflict_component.v1");
        let component_hash = hash_json_on(
            &mut tx,
            &json!({
                "domain":"candidate_conflict_component.v1",
                "analysis_attempt_id":identity.attempt_id,
                "ordinal":0,
                "proposal_census_id":h1.census_id,
                "proposal_census_hash":h1.census_hash,
                "proposal_ids":canonical_proposals.iter().map(|row|row.0).collect::<Vec<_>>(),
                "proposal_set_hash":proposal_set_hash,
            }),
        )
        .await?;
        let proposal_count = i64::try_from(canonical_proposals.len()).unwrap_or(i64::MAX);
        let existing_component: Option<(i32, i64, String, String)> = sqlx::query_as(
            r#"SELECT ordinal,proposal_count,proposal_set_hash,component_hash
                  FROM candidate_analysis_conflict_components
                 WHERE conflict_component_id=$1 AND analysis_attempt_id=$2"#,
        )
        .bind(conflict_component_id)
        .bind(identity.attempt_id)
        .fetch_optional(&mut *tx)
        .await?;
        let insert_members = existing_component.is_none();
        if let Some(existing) = existing_component {
            if existing
                != (
                    0,
                    proposal_count,
                    proposal_set_hash.clone(),
                    component_hash.clone(),
                )
            {
                return Err(conflict("CANDIDATE_CONFLICT_COMPONENT_REPLAY_DRIFT"));
            }
        } else {
            sqlx::query(
                r#"INSERT INTO candidate_analysis_conflict_components(
                       conflict_component_id,analysis_attempt_id,ordinal,proposal_count,
                       proposal_set_hash,component_hash)
                   VALUES($1,$2,0,$3,$4,$5)"#,
            )
            .bind(conflict_component_id)
            .bind(identity.attempt_id)
            .bind(proposal_count)
            .bind(&proposal_set_hash)
            .bind(&component_hash)
            .execute(&mut *tx)
            .await?;
        }
        let mut expected_members = Vec::with_capacity(canonical_proposals.len());
        for (ordinal, (proposal_id, proposal_hash)) in canonical_proposals.iter().enumerate() {
            let member_hash = hash_json_on(
                &mut tx,
                &json!({
                    "domain":"candidate_conflict_component_member.v1",
                    "analysis_attempt_id":identity.attempt_id,
                    "conflict_component_id":conflict_component_id,
                    "proposal_id":proposal_id,
                    "proposal_hash":proposal_hash,
                    "ordinal":ordinal,
                }),
            )
            .await?;
            let ordinal = i32::try_from(ordinal).unwrap_or(i32::MAX);
            let member_id = Uuid::new_v5(&conflict_component_id, member_hash.as_bytes());
            expected_members.push((member_id, *proposal_id, ordinal, member_hash.clone()));
            if insert_members {
                sqlx::query(
                    r#"INSERT INTO candidate_analysis_conflict_component_members(
                           conflict_member_id,conflict_component_id,analysis_attempt_id,
                           proposal_id,ordinal,member_hash)
                       VALUES($1,$2,$3,$4,$5,$6)"#,
                )
                .bind(member_id)
                .bind(conflict_component_id)
                .bind(identity.attempt_id)
                .bind(proposal_id)
                .bind(ordinal)
                .bind(member_hash)
                .execute(&mut *tx)
                .await?;
            }
        }
        let persisted_members: Vec<(Uuid, Uuid, i32, String)> = sqlx::query_as(
            r#"SELECT conflict_member_id,proposal_id,ordinal,member_hash
                  FROM candidate_analysis_conflict_component_members
                 WHERE conflict_component_id=$1 AND analysis_attempt_id=$2
                 ORDER BY ordinal,proposal_id"#,
        )
        .bind(conflict_component_id)
        .bind(identity.attempt_id)
        .fetch_all(&mut *tx)
        .await?;
        if persisted_members != expected_members {
            return Err(conflict("CANDIDATE_CONFLICT_COMPONENT_REPLAY_DRIFT"));
        }
        let conflict_input = json!({
            "mode":"proposal_conflict",
            "conflict_component_id":conflict_component_id,
            "conflict_component_hash":component_hash,
            "proposals":canonical_proposals.iter().map(|row|json!({
                "proposal_id":row.0,"proposal_hash":row.1,
            })).collect::<Vec<_>>(),
            "proposal_summaries":conflict_proposal_summaries,
        });
        let (item_id, worker_id) = ensure_queued_work_on(
            &mut tx,
            &identity,
            plan_id,
            "critic",
            "proposal_conflict_review",
            &format!("critic-conflict:{attempt_ordinal}:{conflict_component_id}"),
            Some(&conflict_component_id.to_string()),
            Some(conflict_component_id),
            0,
            &conflict_input,
        )
        .await?;
        Some((item_id, worker_id, conflict_input))
    };
    let attempt_policy: (String, String, String, String) = sqlx::query_as(
        r#"SELECT attack_class_checklist_version,attack_class_checklist_digest,
                  trust_boundary_checklist_version,trust_boundary_checklist_digest
             FROM candidate_analysis_attempts WHERE analysis_attempt_id=$1"#,
    )
    .bind(identity.attempt_id)
    .fetch_one(&mut *tx)
    .await?;
    let frozen_boundaries: Vec<(String, String)> = sqlx::query_as(
        r#"SELECT DISTINCT subject_kind_at_time,subject_identity_hash
             FROM candidate_analysis_snapshot_inputs
            WHERE snapshot_id=$1 ORDER BY subject_kind_at_time,subject_identity_hash"#,
    )
    .bind(snapshot_id)
    .fetch_all(&mut *tx)
    .await?;
    let expected_attack_digest = hash_json_on(
        &mut tx,
        &super::candidate_analysis::candidate_attack_class_catalog_manifest_v1(),
    )
    .await?;
    let expected_boundary_digest = hash_json_on(
        &mut tx,
        &json!({
            "contract":"trust_boundary.v1","version":1,
            "boundaries":frozen_boundaries.iter().map(|row|json!({
                "identity":row.0,"hash":row.1,
            })).collect::<Vec<_>>(),
        }),
    )
    .await?;
    if attempt_policy.0 != "attack_class.v1"
        || attempt_policy.1 != expected_attack_digest
        || attempt_policy.2 != "trust_boundary.v1"
        || attempt_policy.3 != expected_boundary_digest
        || frozen_boundaries.is_empty()
    {
        return Err(conflict("CANDIDATE_CHECKLIST_CATALOG_DRIFT"));
    }
    let inputs: Vec<CriticInputDbRow> = sqlx::query_as(
        r#"SELECT source.snapshot_input_id,source.source_ref,
                      source.subject_kind_at_time,
                      source.subject_identity_hash,census.chunk_census_id,
                      census.census_hash,census.chunk_count,
                      census.chunking_contract_version,census.redaction_contract_version,
                      census.disposition AS chunk_disposition
                 FROM candidate_analysis_snapshot_inputs source
                 JOIN candidate_analysis_input_chunk_censuses census
                   ON census.snapshot_input_id=source.snapshot_input_id
                WHERE source.snapshot_id=$1
                ORDER BY source.stable_input_key"#,
    )
    .bind(snapshot_id)
    .fetch_all(&mut *tx)
    .await?;
    for input_row in &inputs {
        let input_id = input_row.snapshot_input_id;
        let proposal_ref_hashes: Vec<String> = sqlx::query_scalar(
            r#"SELECT reference.ref_hash
                 FROM hypothesis_proposal_refs reference
                WHERE reference.analysis_attempt_id=$1
                  AND reference.snapshot_input_id=$2
                ORDER BY reference.ref_hash"#,
        )
        .bind(identity.attempt_id)
        .bind(input_id)
        .fetch_all(&mut *tx)
        .await?;
        let proposal_ref_set_hash = hash_texts_on(&mut tx, &proposal_ref_hashes).await?;
        let (disposition, blocker_code) = if input_row.chunk_disposition == "complete" {
            (
                if proposal_ref_hashes.is_empty() {
                    "zero_proposal"
                } else {
                    "has_proposal"
                },
                None,
            )
        } else {
            if !proposal_ref_hashes.is_empty() {
                return Err(conflict("CANDIDATE_NONCOMPLETE_INPUT_HAS_PROPOSAL_REF"));
            }
            (
                "blocked",
                Some(format!("candidate_input_{}", input_row.chunk_disposition)),
            )
        };
        let disposition_hash = hash_json_on(
            &mut tx,
            &json!({
                "analysis_attempt_id":identity.attempt_id,
                "snapshot_input_id":input_id,
                "proposal_ref_set_hash":proposal_ref_set_hash,
                "disposition":disposition,
                "blocker_code":blocker_code,
            }),
        )
        .await?;
        let existing: Option<(i64, String, String, Option<String>, String)> = sqlx::query_as(
            r#"SELECT proposal_ref_count,proposal_ref_set_hash,disposition,
                      blocker_code,disposition_hash
                 FROM candidate_analysis_input_proposal_dispositions
                WHERE analysis_attempt_id=$1 AND snapshot_input_id=$2"#,
        )
        .bind(identity.attempt_id)
        .bind(input_id)
        .fetch_optional(&mut *tx)
        .await?;
        let expected = (
            i64::try_from(proposal_ref_hashes.len()).unwrap_or(i64::MAX),
            proposal_ref_set_hash,
            disposition.to_owned(),
            blocker_code,
            disposition_hash,
        );
        if existing.as_ref() != Some(&expected) {
            return Err(conflict("CANDIDATE_INPUT_DISPOSITION_REPLAY_DRIFT"));
        }
    }
    let blocked_input_ids: Vec<Uuid> = sqlx::query_scalar(
        r#"SELECT snapshot_input_id FROM candidate_analysis_input_proposal_dispositions
            WHERE analysis_attempt_id=$1 AND disposition='blocked'
            ORDER BY snapshot_input_id"#,
    )
    .bind(identity.attempt_id)
    .fetch_all(&mut *tx)
    .await?;
    if !blocked_input_ids.is_empty() {
        let reason_code = "candidate_noncomplete_input_blocked";
        let residual_hash = block_candidate_attempt_on(
            &mut tx,
            &opened.controller_fence,
            reason_code,
            &blocked_input_ids,
        )
        .await?;
        tx.commit().await?;
        return Ok((h1.census_hash, Vec::new(), Some(residual_hash)));
    }
    let mut drafts = Vec::new();
    let mut subreview_tuple_ordinal = 0usize;
    for input_row in &inputs {
        let input_id = input_row.snapshot_input_id;
        let proposal_refs: Vec<(Uuid, String)> = sqlx::query_as(
            r#"SELECT DISTINCT proposal.proposal_id,proposal.proposal_hash
                 FROM hypothesis_proposals proposal
                 JOIN hypothesis_proposal_refs reference USING(proposal_id)
                WHERE proposal.analysis_attempt_id=$1 AND reference.snapshot_input_id=$2
                ORDER BY proposal.proposal_id"#,
        )
        .bind(identity.attempt_id)
        .bind(input_id)
        .fetch_all(&mut *tx)
        .await?;
        let h1_proposal_summaries =
            load_proposal_summaries_on(&mut tx, identity.attempt_id, &[input_id]).await?;
        let product_member_id = input_row
            .source_ref
            .strip_prefix("candidate_product_version_member:")
            .and_then(|value| Uuid::parse_str(value).ok());
        let feed_member_id = input_row
            .source_ref
            .strip_prefix("candidate_feed_snapshot_member:")
            .and_then(|value| Uuid::parse_str(value).ok());
        let enrichment_obligation_id: Option<Uuid> =
            if let Some(product_member_id) = product_member_id {
                sqlx::query_scalar(
                    r#"SELECT obligation_id FROM candidate_analysis_enrichment_obligations
                    WHERE snapshot_id=$1 AND product_member_id=$2
                      AND obligation_kind='product_version_enrichment'"#,
                )
                .bind(snapshot_id)
                .bind(product_member_id)
                .fetch_optional(&mut *tx)
                .await?
            } else if let Some(feed_member_id) = feed_member_id {
                sqlx::query_scalar(
                    r#"SELECT obligation_id FROM candidate_analysis_enrichment_obligations
                    WHERE snapshot_id=$1 AND feed_snapshot_member_id=$2
                      AND obligation_kind IN ('feed_refresh','feed_matcher_upgrade')
                    ORDER BY obligation_kind LIMIT 1"#,
                )
                .bind(snapshot_id)
                .bind(feed_member_id)
                .fetch_optional(&mut *tx)
                .await?
            } else {
                None
            };
        let feed_match_member_refs: Vec<Uuid> = if let Some(product_member_id) = product_member_id {
            sqlx::query_scalar(
                r#"SELECT match_member_id FROM candidate_analysis_feed_match_census_members
                    WHERE snapshot_id=$1 AND product_member_id=$2 AND disposition='matched'
                    ORDER BY ordinal"#,
            )
            .bind(snapshot_id)
            .bind(product_member_id)
            .fetch_all(&mut *tx)
            .await?
        } else if let Some(feed_member_id) = feed_member_id {
            sqlx::query_scalar(
                r#"SELECT match_member_id FROM candidate_analysis_feed_match_census_members
                    WHERE snapshot_id=$1 AND feed_snapshot_member_id=$2 AND disposition='matched'
                    ORDER BY ordinal"#,
            )
            .bind(snapshot_id)
            .bind(feed_member_id)
            .fetch_all(&mut *tx)
            .await?
        } else {
            Vec::new()
        };
        let applicability_disposition = if enrichment_obligation_id.is_some() {
            if product_member_id.is_some() {
                "blocked_product_version"
            } else {
                "blocked_feed_authority"
            }
        } else {
            "required"
        };
        let mut checklist_ids = Vec::new();
        for (attack_class_id, attack_class_version) in
            super::candidate_analysis::CANDIDATE_ATTACK_CLASS_CATALOG_V1
        {
            for (boundary_identity, boundary_hash) in &frozen_boundaries {
                let ordinal = i32::try_from(checklist_ids.len())
                    .map_err(|_| conflict("CANDIDATE_CHECKLIST_ORDINAL_OVERFLOW"))?;
                let checklist_id = Uuid::new_v5(
                    &identity.attempt_id,
                    format!(
                        "checklist:{input_id}:{attack_class_id}:{attack_class_version}:{boundary_identity}:{boundary_hash}"
                    )
                    .as_bytes(),
                );
                let checklist_hash = hash_json_on(
                    &mut tx,
                    &json!({
                        "analysis_attempt_id":identity.attempt_id,
                        "snapshot_input_id":input_id,"ordinal":ordinal,
                        "attack_class_id":attack_class_id,
                        "attack_class_version":attack_class_version,
                        "trust_boundary_identity":boundary_identity,
                        "trust_boundary_hash":boundary_hash,
                        "attack_class_contract_digest":attempt_policy.1,
                        "trust_boundary_contract_digest":attempt_policy.3,
                        "feed_match_member_refs":feed_match_member_refs,
                        "applicability_disposition":applicability_disposition,
                        "enrichment_obligation_id":enrichment_obligation_id,
                    }),
                )
                .await?;
                let applicability_basis = json!({
                    "source":"server_frozen_catalog_x_boundary",
                    "input_subject_kind":input_row.subject_kind_at_time,
                    "input_subject_identity_hash":input_row.subject_identity_hash,
                });
                let existing: Option<ChecklistReplayDbRow> = sqlx::query_as(
                    r#"SELECT checklist_member_id,attack_class_contract_version,
                              attack_class_contract_digest,trust_boundary_contract_version,
                              trust_boundary_contract_digest,attack_class_id,
                              attack_class_version,trust_boundary_identity,trust_boundary_hash,
                              applicability_basis,feed_match_member_refs,
                              applicability_disposition,enrichment_obligation_id,member_hash
                         FROM candidate_analysis_hypothesis_coverage_checklist_members
                        WHERE analysis_attempt_id=$1 AND snapshot_input_id=$2 AND ordinal=$3"#,
                )
                .bind(identity.attempt_id)
                .bind(input_id)
                .bind(ordinal)
                .fetch_optional(&mut *tx)
                .await?;
                let expected = ChecklistReplayDbRow {
                    checklist_member_id: checklist_id,
                    attack_class_contract_version: attempt_policy.0.clone(),
                    attack_class_contract_digest: attempt_policy.1.clone(),
                    trust_boundary_contract_version: attempt_policy.2.clone(),
                    trust_boundary_contract_digest: attempt_policy.3.clone(),
                    attack_class_id: attack_class_id.to_owned(),
                    attack_class_version,
                    trust_boundary_identity: boundary_identity.clone(),
                    trust_boundary_hash: boundary_hash.clone(),
                    applicability_basis: applicability_basis.clone(),
                    feed_match_member_refs: feed_match_member_refs.clone(),
                    applicability_disposition: applicability_disposition.to_owned(),
                    enrichment_obligation_id,
                    member_hash: checklist_hash.clone(),
                };
                if let Some(existing) = existing {
                    if existing != expected {
                        return Err(conflict("CANDIDATE_CHECKLIST_REPLAY_DRIFT"));
                    }
                } else {
                    sqlx::query(
                        r#"INSERT INTO candidate_analysis_hypothesis_coverage_checklist_members(
                           checklist_member_id,analysis_attempt_id,snapshot_input_id,ordinal,
                           attack_class_contract_version,attack_class_contract_digest,
                           trust_boundary_contract_version,trust_boundary_contract_digest,
                           attack_class_id,attack_class_version,trust_boundary_identity,
                           trust_boundary_hash,applicability_basis,feed_match_member_refs,
                           applicability_disposition,enrichment_obligation_id,member_hash)
                       VALUES($1,$2,$3,$4,$5,$6,$7,$8,$9,$10,$11,$12,$13,$14,$15,$16,$17)"#,
                    )
                    .bind(checklist_id)
                    .bind(identity.attempt_id)
                    .bind(input_id)
                    .bind(ordinal)
                    .bind(&attempt_policy.0)
                    .bind(&attempt_policy.1)
                    .bind(&attempt_policy.2)
                    .bind(&attempt_policy.3)
                    .bind(attack_class_id)
                    .bind(attack_class_version)
                    .bind(boundary_identity)
                    .bind(boundary_hash)
                    .bind(applicability_basis)
                    .bind(&feed_match_member_refs)
                    .bind(applicability_disposition)
                    .bind(enrichment_obligation_id)
                    .bind(&checklist_hash)
                    .execute(&mut *tx)
                    .await?;
                }
                checklist_ids.push(checklist_id);
            }
        }
        let chunk_rows: Vec<AnalystChunkDbRow> = sqlx::query_as(
            r#"SELECT source.snapshot_input_id,source.stable_input_key,source.source_kind,
                      source.source_ref,source.source_content_hash,
                      source.source_byte_count AS source_size_bytes,source.subject_kind_at_time,
                      source.subject_identity_hash,member.chunk_id,member.ordinal,census.census_hash,
                      census.chunking_contract_version,census.redaction_contract_version,member.chunk_hash,
                      member.immutable_redacted_body
                 FROM candidate_analysis_input_chunk_census_members member
                 JOIN candidate_analysis_input_chunk_censuses census USING(chunk_census_id)
                 JOIN candidate_analysis_snapshot_inputs source
                   ON source.snapshot_input_id=member.snapshot_input_id
                WHERE member.chunk_census_id=$1 ORDER BY member.ordinal"#,
        )
        .bind(input_row.chunk_census_id)
        .fetch_all(&mut *tx)
        .await?;
        if chunk_rows.len() as i64 != input_row.chunk_count || chunk_rows.is_empty() {
            return Err(conflict("CANDIDATE_CHUNK_CENSUS_DRIFT"));
        }
        let mut chunks = Vec::with_capacity(chunk_rows.len());
        for row in chunk_rows {
            chunks.push((
                row.ordinal,
                row.chunk_hash.clone(),
                project_runtime_chunk_on(&mut tx, &row).await?,
            ));
        }
        let source_size_bytes: i64 = sqlx::query_scalar(
            "SELECT source_byte_count FROM candidate_analysis_input_chunk_censuses WHERE chunk_census_id=$1",
        )
        .bind(input_row.chunk_census_id)
        .fetch_one(&mut *tx)
        .await?;
        for (partition_ordinal, (chunk_ordinal, chunk_hash, chunk)) in
            chunks.into_iter().enumerate()
        {
            let partition_ordinal = i32::try_from(partition_ordinal)
                .map_err(|_| conflict("CANDIDATE_PARTITION_OVERFLOW"))?;
            let partition_id = Uuid::new_v5(
                &identity.attempt_id,
                format!("partition:{input_id}:{partition_ordinal}").as_bytes(),
            );
            let chunk_set_hash = hash_texts_on(&mut tx, std::slice::from_ref(&chunk_hash)).await?;
            let partition_hash = hash_json_on(
                &mut tx,
                &json!({
                    "analysis_attempt_id":identity.attempt_id,
                    "snapshot_input_id":input_id,
                    "chunk_set_hash":chunk_set_hash,
                    "first":chunk_ordinal,
                    "last":chunk_ordinal,
                }),
            )
            .await?;
            type ExistingPartition = (Uuid, i32, i32, i64, String, i64, String);
            let existing: Option<ExistingPartition> = sqlx::query_as(
                r#"SELECT chunk_partition_id,first_chunk_ordinal,last_chunk_ordinal,
                          chunk_count,chunk_set_hash,bounded_context_budget,partition_hash
                     FROM candidate_analysis_hypothesis_coverage_chunk_partitions
                    WHERE analysis_attempt_id=$1 AND snapshot_input_id=$2
                      AND partition_ordinal=$3"#,
            )
            .bind(identity.attempt_id)
            .bind(input_id)
            .bind(partition_ordinal)
            .fetch_optional(&mut *tx)
            .await?;
            let expected: ExistingPartition = (
                partition_id,
                chunk_ordinal,
                chunk_ordinal,
                1,
                chunk_set_hash.clone(),
                262_144,
                partition_hash.clone(),
            );
            if let Some(existing) = existing {
                if existing != expected {
                    return Err(conflict("CANDIDATE_PARTITION_REPLAY_DRIFT"));
                }
            } else {
                sqlx::query(
                    r#"INSERT INTO candidate_analysis_hypothesis_coverage_chunk_partitions(
                       chunk_partition_id,analysis_attempt_id,snapshot_input_id,partition_ordinal,
                       first_chunk_ordinal,last_chunk_ordinal,chunk_count,chunk_set_hash,
                       bounded_context_budget,partition_hash)
                   VALUES($1,$2,$3,$4,$5,$5,1,$6,262144,$7)"#,
                )
                .bind(partition_id)
                .bind(identity.attempt_id)
                .bind(input_id)
                .bind(partition_ordinal)
                .bind(chunk_ordinal)
                .bind(&chunk_set_hash)
                .bind(partition_hash)
                .execute(&mut *tx)
                .await?;
            }
            for checklist_id in &checklist_ids {
                let checklist_summary: (String, i32, String, String) = sqlx::query_as(
                    r#"SELECT attack_class_id,attack_class_version,
                              trust_boundary_identity,trust_boundary_hash
                         FROM candidate_analysis_hypothesis_coverage_checklist_members
                        WHERE checklist_member_id=$1 AND analysis_attempt_id=$2
                          AND snapshot_input_id=$3"#,
                )
                .bind(checklist_id)
                .bind(identity.attempt_id)
                .bind(input_id)
                .fetch_one(&mut *tx)
                .await?;
                let required = subreview_tuple_ordinal < max_coverage_subreview_work_items;
                subreview_tuple_ordinal = subreview_tuple_ordinal.saturating_add(1);
                let stable_key = format!(
                    "critic-subreview:{attempt_ordinal}:{input_id}:{checklist_id}:{partition_ordinal}"
                );
                let provisional_input = json!({
                    "mode":"coverage_subreview",
                    "subreview_census_id":Uuid::nil(),
                    "subreview_census_member_id":Uuid::nil(),
                    "snapshot_input_id":input_id,
                    "checklist_member_id":checklist_id,
                    "checklist":{
                        "checklist_member_id":checklist_id,
                        "snapshot_input_id":input_id,
                        "attack_class_id":checklist_summary.0,
                        "attack_class_version":checklist_summary.1,
                        "trust_boundary_identity":checklist_summary.2,
                        "trust_boundary_hash":checklist_summary.3,
                    },
                    "chunk_partition_id":partition_id,
                    "designated_chunks":[chunk.clone()],
                    "h1_proposal_refs":proposal_refs.iter().map(|row|json!({"proposal_id":row.0,"proposal_hash":row.1})).collect::<Vec<_>>(),
                    "h1_proposal_summaries":h1_proposal_summaries,
                    "read_receipt_set_hash":hash_texts_on(&mut tx,&[]).await?,
                });
                let lane_ordinal =
                    i32::try_from(drafts.len()).unwrap_or(i32::MAX) % host_lane_limit;
                if required {
                    let (item_id, worker_id) = ensure_queued_work_on(
                        &mut tx,
                        &identity,
                        plan_id,
                        "critic",
                        "hypothesis_coverage_subreview",
                        &stable_key,
                        Some(&partition_id.to_string()),
                        Some(*checklist_id),
                        lane_ordinal,
                        &provisional_input,
                    )
                    .await?;
                    drafts.push(CriticWorkDraft {
                        input_id,
                        checklist_id: *checklist_id,
                        partition_id,
                        item_id,
                        worker_id,
                        lane_ordinal,
                        provisional_input,
                        input_authority: input_row.clone(),
                        source_size_bytes,
                        partition_ordinal,
                        chunk_hash: chunk_hash.clone(),
                        chunk_ordinal,
                    });
                } else {
                    let (item_id, worker_id) = ensure_queued_work_on(
                        &mut tx,
                        &identity,
                        plan_id,
                        "critic",
                        "hypothesis_coverage_sampling_omitted",
                        &stable_key,
                        Some(&partition_id.to_string()),
                        Some(*checklist_id),
                        lane_ordinal,
                        &provisional_input,
                    )
                    .await?;
                    let status: String =
                        sqlx::query_scalar("SELECT status FROM stage_worker_runs WHERE id=$1")
                            .bind(worker_id)
                            .fetch_one(&mut *tx)
                            .await?;
                    if status == "passed" {
                        continue;
                    }
                    let fence = claim_or_replay_queued_work_on(
                        &mut tx,
                        &identity,
                        plan_id,
                        "hypothesis_coverage_sampling_omitted",
                        item_id,
                        worker_id,
                    )
                    .await?
                    .ok_or_else(|| conflict("CANDIDATE_OMITTED_WORK_STATE_DRIFT"))?;
                    sqlx::query(
                        r#"UPDATE stage_work_items
                              SET status='completed',terminal_at=statement_timestamp(),
                                  row_version=row_version+1,updated_at=statement_timestamp()
                            WHERE id=$1 AND status='running'"#,
                    )
                    .bind(fence.work_item_id)
                    .execute(&mut *tx)
                    .await?;
                    sqlx::query(
                        r#"UPDATE stage_worker_runs
                              SET status='passed',terminal_at=statement_timestamp(),
                                  updated_at=statement_timestamp()
                            WHERE id=$1 AND status='running'"#,
                    )
                    .bind(fence.worker_run_id)
                    .execute(&mut *tx)
                    .await?;
                }
            }
        }
    }
    tx.commit().await?;
    for input_id in inputs.iter().map(|row| row.snapshot_input_id) {
        super::candidate_analysis::seal_hypothesis_coverage_subreview_census(
            pool,
            super::candidate_analysis::SealCoverageSubreviewCensusInput {
                fence: opened.controller_fence.clone(),
                stable_census_request_id: Uuid::new_v5(
                    &opened.analysis_attempt_id,
                    b"candidate_subreview_census.v1",
                ),
                input_id,
            },
        )
        .await?;
    }
    let mut claim_tx = pool.begin().await?;
    let claim_identity = load_scheduler_identity_on(
        &mut claim_tx,
        snapshot_id,
        stage_execution_id,
        attempt_ordinal,
    )
    .await?;
    let claim_plan_id = ensure_team_plan_on(&mut claim_tx, &claim_identity).await?;
    let mut available =
        available_live_lanes_on(&mut claim_tx, claim_plan_id, host_lane_limit).await?;
    let mut result = Vec::new();
    if let Some((item_id, worker_id, input)) = conflict_draft {
        let status: String = sqlx::query_scalar("SELECT status FROM stage_worker_runs WHERE id=$1")
            .bind(worker_id)
            .fetch_one(&mut *claim_tx)
            .await?;
        if status == "running" || (status == "queued" && available > 0) {
            if let Some(fence) = claim_or_replay_queued_work_on(
                &mut claim_tx,
                &claim_identity,
                claim_plan_id,
                "proposal_conflict_review",
                item_id,
                worker_id,
            )
            .await?
            {
                if status == "queued" {
                    available = available.saturating_sub(1);
                }
                let replayed_receipt =
                    artifact_receipt_for_work_on(&mut claim_tx, fence.work_item_id).await?;
                result.push(CandidateRuntimeWorkRow {
                    replayed_receipt,
                    fence,
                    phase: "critic".to_owned(),
                    capability: "proposal_conflict_review".to_owned(),
                    lane_ordinal: 0,
                    input,
                });
            }
        }
    }
    for draft in drafts {
        let status: String = sqlx::query_scalar("SELECT status FROM stage_worker_runs WHERE id=$1")
            .bind(draft.worker_id)
            .fetch_one(&mut *claim_tx)
            .await?;
        let may_claim = status == "running" || (status == "queued" && available > 0);
        if !may_claim {
            continue;
        }
        let Some(fence) = claim_or_replay_queued_work_on(
            &mut claim_tx,
            &claim_identity,
            claim_plan_id,
            "hypothesis_coverage_subreview",
            draft.item_id,
            draft.worker_id,
        )
        .await?
        else {
            continue;
        };
        if status == "queued" {
            available = available.saturating_sub(1);
        }
        let (census_id, member_id): (Uuid, Uuid) = sqlx::query_as(
            r#"SELECT census.subreview_census_id,member.subreview_census_member_id
                 FROM candidate_analysis_hypothesis_coverage_subreview_censuses census
                 JOIN candidate_analysis_hypothesis_coverage_subreview_census_members member
                   ON member.subreview_census_id=census.subreview_census_id
                WHERE census.analysis_attempt_id=$1 AND census.snapshot_input_id=$2
                  AND member.checklist_member_id=$3 AND member.chunk_partition_id=$4"#,
        )
        .bind(opened.analysis_attempt_id)
        .bind(draft.input_id)
        .bind(draft.checklist_id)
        .bind(draft.partition_id)
        .fetch_one(&mut *claim_tx)
        .await?;
        let mut value = draft.provisional_input;
        value["subreview_census_id"] = json!(census_id);
        value["subreview_census_member_id"] = json!(member_id);
        let page_request_id = Uuid::new_v5(&fence.work_item_id, draft.partition_id.as_bytes());
        let expected_chunk_hashes = [draft.chunk_hash.clone()];
        let page_hash = ensure_runtime_chunk_page_receipt_on(
            &mut claim_tx,
            RuntimeChunkPageRequest {
                fence: &fence,
                stable_request_id: page_request_id,
                snapshot_input_id: draft.input_id,
                chunk_census_id: draft.input_authority.chunk_census_id,
                chunk_census_hash: &draft.input_authority.census_hash,
                source_size_bytes: draft.source_size_bytes,
                chunking_contract_version: &draft.input_authority.chunking_contract_version,
                redaction_contract_version: &draft.input_authority.redaction_contract_version,
                first_ordinal: draft.chunk_ordinal,
                limit: 1,
                expected_ordered_chunk_hashes: &expected_chunk_hashes,
            },
        )
        .await?;
        if page_hash.is_empty() || draft.partition_ordinal < 0 {
            return Err(conflict("CANDIDATE_CRITIC_PAGE_AUTHORITY_INVALID"));
        }
        seal_candidate_work_page_authority_on(
            &mut claim_tx,
            &fence,
            Uuid::new_v5(&page_request_id, b"candidate_page_receipt.v1"),
            &page_hash,
        )
        .await?;
        let replayed_receipt =
            artifact_receipt_for_work_on(&mut claim_tx, fence.work_item_id).await?;
        result.push(CandidateRuntimeWorkRow {
            replayed_receipt,
            fence,
            phase: "critic".to_owned(),
            capability: "hypothesis_coverage_subreview".to_owned(),
            lane_ordinal: draft.lane_ordinal,
            input: value,
        });
    }
    claim_tx.commit().await?;
    Ok((h1.census_hash, result, None))
}

#[derive(Debug, sqlx::FromRow)]
struct SynthesisWorkDbRow {
    synthesis_census_id: Uuid,
    synthesis_node_id: Uuid,
    node_kind: String,
    level: i32,
    partition_ordinal: i32,
    node_hash: String,
    child_receipt_count: i64,
    child_receipt_set_hash: String,
    descendant_worker_set_hash: String,
    relationship_cross_index_hash: String,
    covered_input_count: i64,
    covered_input_set_hash: String,
    covered_checklist_count: i64,
    covered_checklist_set_hash: String,
}

type SynthesisSemanticChildRow = (
    String,
    Uuid,
    String,
    Option<String>,
    Option<String>,
    Option<Value>,
);

async fn load_synthesis_semantic_input_on(
    tx: &mut Transaction<'_, Postgres>,
    analysis_attempt_id: Uuid,
    node: &SynthesisWorkDbRow,
) -> Result<Option<(Vec<Uuid>, Vec<Uuid>, Vec<Value>, Vec<Value>)>> {
    let child_rows: Vec<SynthesisSemanticChildRow> = sqlx::query_as(
        r#"SELECT child.child_kind,
                  COALESCE(child.child_subreview_id,child.child_synthesis_node_id),
                  child.child_receipt_hash,
                  CASE child.child_kind
                    WHEN 'subreview' THEN subreview.outcome
                    ELSE synthesis.outcome
                  END,
                  CASE child.child_kind
                    WHEN 'subreview' THEN subreview.semantic_summary_hash
                    ELSE synthesis.semantic_summary_hash
                  END,
                  CASE child.child_kind
                    WHEN 'subreview' THEN subreview.semantic_summary
                    ELSE synthesis.semantic_summary
                  END
             FROM candidate_analysis_hypothesis_coverage_synthesis_node_children child
             LEFT JOIN candidate_analysis_hypothesis_coverage_subreviews subreview
               ON child.child_kind='subreview'
              AND subreview.subreview_id=child.child_subreview_id
              AND subreview.analysis_attempt_id=child.analysis_attempt_id
             LEFT JOIN candidate_analysis_hypothesis_coverage_synthesis_reviews synthesis
               ON child.child_kind='synthesis_node'
              AND synthesis.synthesis_node_id=child.child_synthesis_node_id
              AND synthesis.analysis_attempt_id=child.analysis_attempt_id
            WHERE child.analysis_attempt_id=$1 AND child.synthesis_node_id=$2
            ORDER BY child.ordinal"#,
    )
    .bind(analysis_attempt_id)
    .bind(node.synthesis_node_id)
    .fetch_all(&mut **tx)
    .await?;
    if child_rows.len() as i64 != node.child_receipt_count || child_rows.len() > 32 {
        return Err(conflict("CANDIDATE_SYNTHESIS_CHILD_CAP_INVALID"));
    }
    if child_rows
        .iter()
        .any(|row| row.3.is_none() || row.4.is_none() || row.5.is_none())
    {
        return Ok(None);
    }
    let mut covered_input_ids = BTreeSet::new();
    let mut covered_checklist_member_ids = BTreeSet::new();
    let mut observation_count = 0usize;
    let mut children = Vec::with_capacity(child_rows.len());
    for (kind, identity, receipt_hash, outcome, summary_hash, summary) in child_rows {
        let summary = summary.ok_or_else(|| conflict("CANDIDATE_SYNTHESIS_SUMMARY_MISSING"))?;
        let inputs = summary
            .get("covered_input_ids")
            .and_then(Value::as_array)
            .ok_or_else(|| conflict("CANDIDATE_SYNTHESIS_SUMMARY_INVALID"))?;
        for input_id in inputs {
            covered_input_ids.insert(
                input_id
                    .as_str()
                    .and_then(|value| Uuid::parse_str(value).ok())
                    .ok_or_else(|| conflict("CANDIDATE_SYNTHESIS_SUMMARY_INVALID"))?,
            );
        }
        let checklist = summary
            .get("covered_checklist_member_ids")
            .and_then(Value::as_array)
            .ok_or_else(|| conflict("CANDIDATE_SYNTHESIS_SUMMARY_INVALID"))?;
        for checklist_id in checklist {
            covered_checklist_member_ids.insert(
                checklist_id
                    .as_str()
                    .and_then(|value| Uuid::parse_str(value).ok())
                    .ok_or_else(|| conflict("CANDIDATE_SYNTHESIS_SUMMARY_INVALID"))?,
            );
        }
        observation_count = observation_count.saturating_add(
            summary
                .get("semantic_observations")
                .and_then(Value::as_array)
                .map_or(usize::MAX, Vec::len),
        );
        let canonical_outcome = match outcome.as_deref() {
            Some("no_local_miss" | "no_composite_miss") => "no_miss",
            Some("missed_hypothesis") => "missed_hypothesis",
            Some("blocked") => "blocked",
            _ => return Err(conflict("CANDIDATE_SYNTHESIS_SUMMARY_INVALID")),
        };
        children.push(json!({
            "child_kind":kind,
            "child_identity":identity,
            "child_receipt_hash":receipt_hash,
            "outcome":canonical_outcome,
            "semantic_summary_hash":summary_hash,
            "semantic_summary":summary,
        }));
    }
    if observation_count > 64 {
        return Err(conflict(
            "CANDIDATE_SYNTHESIS_SEMANTIC_OBSERVATION_CAP_EXCEEDED",
        ));
    }
    let covered_input_ids = covered_input_ids.into_iter().collect::<Vec<_>>();
    let covered_checklist_member_ids = covered_checklist_member_ids.into_iter().collect::<Vec<_>>();
    let input_hash = hash_texts_on(
        tx,
        &covered_input_ids
            .iter()
            .map(Uuid::to_string)
            .collect::<Vec<_>>(),
    )
    .await?;
    let checklist_hash = hash_texts_on(
        tx,
        &covered_checklist_member_ids
            .iter()
            .map(Uuid::to_string)
            .collect::<Vec<_>>(),
    )
    .await?;
    if covered_input_ids.len() as i64 != node.covered_input_count
        || input_hash != node.covered_input_set_hash
        || covered_checklist_member_ids.len() as i64 != node.covered_checklist_count
        || checklist_hash != node.covered_checklist_set_hash
    {
        return Err(conflict("CANDIDATE_SYNTHESIS_SEMANTIC_EXACT_SET_INVALID"));
    }
    let proposal_summaries =
        load_proposal_summaries_on(tx, analysis_attempt_id, &covered_input_ids).await?;
    Ok(Some((
        covered_input_ids,
        covered_checklist_member_ids,
        proposal_summaries,
        children,
    )))
}

pub async fn prepare_synthesis_work_batch(
    pool: &PgPool,
    snapshot_id: Uuid,
    stage_execution_id: Uuid,
    attempt_ordinal: i32,
    host_lane_limit: i32,
) -> Result<Vec<CandidateRuntimeWorkRow>> {
    if host_lane_limit <= 0 {
        return Err(conflict("CANDIDATE_SYNTHESIS_BATCH_INVALID"));
    }
    let opened =
        open_or_replay_attempt_runtime(pool, snapshot_id, stage_execution_id, attempt_ordinal)
            .await?;
    let census = super::candidate_analysis::seal_hypothesis_coverage_synthesis_census(
        pool,
        super::candidate_analysis::SealCoverageSynthesisCensusInput {
            fence: opened.controller_fence.clone(),
            stable_census_request_id: Uuid::new_v5(
                &opened.analysis_attempt_id,
                b"candidate_synthesis_census.v1",
            ),
        },
    )
    .await?;
    let mut tx = pool.begin().await?;
    let identity =
        load_scheduler_identity_on(&mut tx, snapshot_id, stage_execution_id, attempt_ordinal)
            .await?;
    let plan_id = ensure_team_plan_on(&mut tx, &identity).await?;
    let nodes = sqlx::query_as::<_, SynthesisWorkDbRow>(
        r#"SELECT synthesis_census_id,synthesis_node_id,node_kind,level,
                  partition_ordinal,node_hash,child_receipt_count,
                  child_receipt_set_hash,descendant_worker_set_hash,
                  relationship_cross_index_hash,covered_input_count,
                  covered_input_set_hash,covered_checklist_count,
                  covered_checklist_set_hash
             FROM candidate_analysis_hypothesis_coverage_synthesis_census_members
            WHERE synthesis_census_id=$1
            ORDER BY CASE node_kind
                WHEN 'cross_chunk' THEN 0 WHEN 'cross_input_partition' THEN 100
                WHEN 'cross_input_reduce' THEN 200+level
                WHEN 'cross_dimension_reduce' THEN 400+level ELSE 1000 END,
                partition_ordinal,synthesis_node_id"#,
    )
    .bind(census.census_id)
    .fetch_all(&mut *tx)
    .await?;
    let mut drafts = Vec::with_capacity(nodes.len());
    for (ordinal, node) in nodes.into_iter().enumerate() {
        let mode = match node.node_kind.as_str() {
            "cross_chunk" => "coverage_cross_chunk_synthesis",
            "cross_input_partition" => "coverage_cross_input_partition",
            "cross_input_reduce" => "coverage_cross_input_reduce",
            "cross_dimension_reduce" => "coverage_cross_dimension_reduce",
            "global_semantic_root" => "coverage_global_semantic_root",
            _ => return Err(conflict("CANDIDATE_SYNTHESIS_NODE_KIND_INVALID")),
        };
        let Some((
            covered_input_ids,
            covered_checklist_member_ids,
            h1_proposal_summaries,
            child_semantic_summaries,
        )) = load_synthesis_semantic_input_on(&mut tx, identity.attempt_id, &node).await?
        else {
            continue;
        };
        let value = json!({
            "mode":mode,
            "node":{
                "synthesis_census_id":node.synthesis_census_id,
                "synthesis_node_id":node.synthesis_node_id,
                "level":node.level,
                "partition_ordinal":node.partition_ordinal,
                "node_hash":node.node_hash,
                "child_receipt_count":node.child_receipt_count,
                "child_receipt_set_hash":node.child_receipt_set_hash,
                "descendant_worker_set_hash":node.descendant_worker_set_hash,
                "relationship_cross_index_hash":node.relationship_cross_index_hash,
                "covered_input_ids":covered_input_ids,
                "covered_checklist_member_ids":covered_checklist_member_ids,
                "h1_proposal_summaries":h1_proposal_summaries,
                "child_semantic_summaries":child_semantic_summaries,
            }
        });
        let stable_key = format!(
            "critic-synthesis:{attempt_ordinal}:{}",
            node.synthesis_node_id
        );
        let (item_id, worker_id) = ensure_queued_work_on(
            &mut tx,
            &identity,
            plan_id,
            "critic",
            mode,
            &stable_key,
            Some(&node.synthesis_node_id.to_string()),
            Some(node.synthesis_node_id),
            i32::try_from(ordinal).unwrap_or(i32::MAX) % host_lane_limit,
            &value,
        )
        .await?;
        let rank = match node.node_kind.as_str() {
            "cross_chunk" => 0,
            "cross_input_partition" => 100,
            "cross_input_reduce" => 200 + node.level,
            "cross_dimension_reduce" => 400 + node.level,
            "global_semantic_root" => 1000,
            _ => return Err(conflict("CANDIDATE_SYNTHESIS_NODE_KIND_INVALID")),
        };
        drafts.push((
            item_id,
            worker_id,
            i32::try_from(ordinal).unwrap_or(i32::MAX) % host_lane_limit,
            value,
            mode.to_owned(),
            rank,
        ));
    }
    let mut available = available_live_lanes_on(&mut tx, plan_id, host_lane_limit).await?;
    let mut result = Vec::new();
    let mut active_rank = None;
    for (item_id, worker_id, lane_ordinal, input, capability, rank) in drafts {
        let status: String = sqlx::query_scalar("SELECT status FROM stage_worker_runs WHERE id=$1")
            .bind(worker_id)
            .fetch_one(&mut *tx)
            .await?;
        if status == "passed" {
            continue;
        }
        let required_rank = *active_rank.get_or_insert(rank);
        if rank != required_rank {
            continue;
        }
        let may_claim = status == "running" || (status == "queued" && available > 0);
        if !may_claim {
            continue;
        }
        let Some(fence) = claim_or_replay_queued_work_on(
            &mut tx,
            &identity,
            plan_id,
            &capability,
            item_id,
            worker_id,
        )
        .await?
        else {
            continue;
        };
        if status == "queued" {
            available = available.saturating_sub(1);
        }
        let replayed_receipt = artifact_receipt_for_work_on(&mut tx, fence.work_item_id).await?;
        result.push(CandidateRuntimeWorkRow {
            fence,
            phase: "critic".to_owned(),
            capability,
            lane_ordinal,
            input,
            replayed_receipt,
        });
    }
    tx.commit().await?;
    Ok(result)
}

pub async fn candidate_synthesis_phase_needed(
    pool: &PgPool,
    analysis_attempt_id: Uuid,
) -> Result<bool> {
    let counts: (i64, i64, i64) = sqlx::query_as(
        r#"SELECT
           (SELECT count(*) FROM candidate_analysis_hypothesis_coverage_subreview_census_members
             WHERE analysis_attempt_id=$1 AND disposition='required'),
           (SELECT count(*) FROM candidate_analysis_hypothesis_coverage_subreviews
             WHERE analysis_attempt_id=$1),
           (SELECT count(*) FROM candidate_analysis_hypothesis_coverage_synthesis_censuses
             WHERE analysis_attempt_id=$1)"#,
    )
    .bind(analysis_attempt_id)
    .fetch_one(pool)
    .await?;
    if counts.0 != counts.1 {
        return Err(conflict("CANDIDATE_SUBREVIEW_CLOSURE_INCOMPLETE"));
    }
    Ok(counts.2 == 0)
}

pub async fn candidate_subreview_phase_incomplete(
    pool: &PgPool,
    analysis_attempt_id: Uuid,
) -> Result<bool> {
    let counts: (i64, i64) = sqlx::query_as(
        r#"SELECT
           (SELECT count(*)
              FROM candidate_analysis_hypothesis_coverage_subreview_census_members
             WHERE analysis_attempt_id=$1 AND disposition='required'),
           (SELECT count(*)
              FROM candidate_analysis_hypothesis_coverage_subreviews
             WHERE analysis_attempt_id=$1)"#,
    )
    .bind(analysis_attempt_id)
    .fetch_one(pool)
    .await?;
    Ok(counts.0 != counts.1)
}

pub async fn candidate_synthesis_work_incomplete(
    pool: &PgPool,
    analysis_attempt_id: Uuid,
) -> Result<bool> {
    let counts: (i64, i64, i64) = sqlx::query_as(
        r#"SELECT
           (SELECT count(*)
              FROM candidate_analysis_hypothesis_coverage_synthesis_census_members
             WHERE analysis_attempt_id=$1),
           (SELECT count(*)
              FROM candidate_analysis_hypothesis_coverage_synthesis_reviews
             WHERE analysis_attempt_id=$1),
           (SELECT count(*)
              FROM candidate_analysis_work_items candidate_item
              JOIN stage_work_items item ON item.id=candidate_item.stage_work_item_id
             WHERE candidate_item.analysis_attempt_id=$1
               AND candidate_item.capability IN (
                   'coverage_cross_chunk_synthesis',
                   'coverage_cross_input_partition',
                   'coverage_cross_input_reduce',
                   'coverage_cross_dimension_reduce',
                   'coverage_global_semantic_root'
               )
               AND item.status<>'completed')"#,
    )
    .bind(analysis_attempt_id)
    .fetch_one(pool)
    .await?;
    Ok(counts.0 != counts.1 || counts.2 != 0)
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub enum CandidateCoverageClosureRow {
    Ready {
        proposal_census_hash: String,
        critic_census_hash: String,
        coverage_review_set_hash: String,
    },
    RetryAttempt {
        next_attempt_ordinal: i32,
    },
    Blocked {
        residual_hash: String,
    },
}

pub async fn load_terminal_candidate_coverage_closure(
    pool: &PgPool,
    analysis_attempt_id: Uuid,
) -> Result<Option<CandidateCoverageClosureRow>> {
    let terminal: Option<String> = sqlx::query_scalar(
        r#"SELECT event_kind FROM candidate_analysis_attempt_state_events
            WHERE analysis_attempt_id=$1
              AND event_kind IN ('superseded_missed_hypothesis','sealed','blocked')"#,
    )
    .bind(analysis_attempt_id)
    .fetch_optional(pool)
    .await?;
    let Some(terminal) = terminal else {
        return Ok(None);
    };
    match terminal.as_str() {
        "blocked" => Ok(load_blocked_attempt_residual(pool, analysis_attempt_id)
            .await?
            .map(|residual_hash| CandidateCoverageClosureRow::Blocked { residual_hash })),
        "superseded_missed_hypothesis" => {
            let next_attempt_ordinal: i32 = sqlx::query_scalar(
                r#"SELECT attempt_ordinal FROM candidate_analysis_attempts
                    WHERE predecessor_attempt_id=$1"#,
            )
            .bind(analysis_attempt_id)
            .fetch_one(pool)
            .await?;
            Ok(Some(CandidateCoverageClosureRow::RetryAttempt {
                next_attempt_ordinal,
            }))
        }
        "sealed" => {
            let mut tx = pool.begin().await?;
            let proposal_census_hash: String = sqlx::query_scalar(
                "SELECT census_hash FROM candidate_analysis_proposal_censuses WHERE analysis_attempt_id=$1",
            )
            .bind(analysis_attempt_id)
            .fetch_one(&mut *tx)
            .await?;
            let critic_census_hash: String = sqlx::query_scalar(
                "SELECT census_hash FROM candidate_analysis_critic_censuses WHERE analysis_attempt_id=$1",
            )
            .bind(analysis_attempt_id)
            .fetch_one(&mut *tx)
            .await?;
            let review_hashes: Vec<String> = sqlx::query_scalar(
                r#"SELECT review_hash FROM candidate_analysis_hypothesis_coverage_reviews
                    WHERE analysis_attempt_id=$1 ORDER BY snapshot_input_id"#,
            )
            .bind(analysis_attempt_id)
            .fetch_all(&mut *tx)
            .await?;
            let coverage_review_set_hash = hash_texts_on(&mut tx, &review_hashes).await?;
            tx.commit().await?;
            Ok(Some(CandidateCoverageClosureRow::Ready {
                proposal_census_hash,
                critic_census_hash,
                coverage_review_set_hash,
            }))
        }
        _ => Err(conflict("CANDIDATE_TERMINAL_OUTCOME_INVALID")),
    }
}

async fn block_candidate_attempt_on(
    tx: &mut Transaction<'_, Postgres>,
    fence: &CandidateWriteFenceRow,
    reason_code: &str,
    affected_input_ids: &[Uuid],
) -> Result<String> {
    if !matches!(
        reason_code,
        "candidate_h1_proposal_cap_exceeded"
            | "candidate_noncomplete_input_blocked"
            | "candidate_controller_proposal_page_cap_exceeded"
            | "candidate_conflict_resolution_requires_typed_retry"
            | "candidate_hypothesis_coverage_blocked"
            | "candidate_hypothesis_coverage_retry_exhausted"
    ) {
        return Err(conflict("CANDIDATE_BLOCKED_REASON_INVALID"));
    }
    super::candidate_analysis::validate_write_fence_on(tx, fence).await?;
    let mut affected_input_ids = affected_input_ids.to_vec();
    affected_input_ids.sort_unstable();
    affected_input_ids.dedup();
    let owned_input_count: i64 = sqlx::query_scalar(
        r#"SELECT COUNT(*) FROM candidate_analysis_snapshot_inputs
            WHERE snapshot_id=$1 AND snapshot_input_id=ANY($2)"#,
    )
    .bind(fence.snapshot_id)
    .bind(&affected_input_ids)
    .fetch_one(&mut **tx)
    .await?;
    if owned_input_count != i64::try_from(affected_input_ids.len()).unwrap_or(i64::MAX) {
        return Err(conflict("CANDIDATE_BLOCKED_AFFECTED_INPUT_INVALID"));
    }
    let residual_id = Uuid::new_v5(
        &fence.analysis_attempt_id,
        b"candidate_analysis_blocked_residual.v1",
    );
    let residual_hash = hash_json_on(
        tx,
        &json!({
            "analysis_attempt_id":fence.analysis_attempt_id,
            "snapshot_id":fence.snapshot_id,
            "reason_code":reason_code,
            "affected_input_ids":&affected_input_ids,
        }),
    )
    .await?;
    let inserted = sqlx::query(
        r#"INSERT INTO hypothesis_residual_risks(
               residual_id,operation_id,organization_id,snapshot_id,reason_code,
               owner_kind,affected_inputs,next_action,residual_hash)
           VALUES($1,$2,$3,$4,$5,'candidate_analysis',$6,$7,$8)
           ON CONFLICT(residual_id) DO NOTHING"#,
    )
    .bind(residual_id)
    .bind(fence.operation_id)
    .bind(fence.organization_id)
    .bind(fence.snapshot_id)
    .bind(reason_code)
    .bind(json!(&affected_input_ids))
    .bind(json!({"route":"candidate_analysis_closeout","retry":false}))
    .bind(&residual_hash)
    .execute(&mut **tx)
    .await?;
    if inserted.rows_affected() == 0 {
        let persisted: (String, String) = sqlx::query_as(
            "SELECT reason_code,residual_hash FROM hypothesis_residual_risks WHERE residual_id=$1",
        )
        .bind(residual_id)
        .fetch_one(&mut **tx)
        .await?;
        if persisted != (reason_code.to_owned(), residual_hash.clone()) {
            return Err(conflict("CANDIDATE_BLOCKED_RESIDUAL_REPLAY_DRIFT"));
        }
    }
    let predecessor_event_id: Uuid = sqlx::query_scalar(
        r#"SELECT attempt_event_id FROM candidate_analysis_attempt_state_events
            WHERE analysis_attempt_id=$1 AND event_kind='opened'"#,
    )
    .bind(fence.analysis_attempt_id)
    .fetch_one(&mut **tx)
    .await?;
    let event_hash = hash_json_on(
        tx,
        &json!({
            "attempt":fence.analysis_attempt_id,
            "ordinal":1,
            "event":"blocked",
            "predecessor_event_id":predecessor_event_id,
            "residual_hash":residual_hash,
        }),
    )
    .await?;
    let event_id = Uuid::new_v5(&fence.analysis_attempt_id, b"candidate_attempt_blocked.v1");
    sqlx::query(
        r#"INSERT INTO candidate_analysis_attempt_state_events(
               attempt_event_id,analysis_attempt_id,event_ordinal,event_kind,
               predecessor_event_id,event_hash)
           VALUES($1,$2,1,'blocked',$3,$4)
           ON CONFLICT(attempt_event_id) DO NOTHING"#,
    )
    .bind(event_id)
    .bind(fence.analysis_attempt_id)
    .bind(predecessor_event_id)
    .bind(event_hash)
    .execute(&mut **tx)
    .await?;
    Ok(residual_hash)
}

pub async fn load_blocked_attempt_residual(
    pool: &PgPool,
    analysis_attempt_id: Uuid,
) -> Result<Option<String>> {
    let residual_id = Uuid::new_v5(
        &analysis_attempt_id,
        b"candidate_analysis_blocked_residual.v1",
    );
    let blocked_event_id = Uuid::new_v5(&analysis_attempt_id, b"candidate_attempt_blocked.v1");
    Ok(sqlx::query_scalar(
        r#"SELECT residual.residual_hash
             FROM hypothesis_residual_risks residual
             JOIN candidate_analysis_attempts attempt
               ON attempt.analysis_attempt_id=$1
              AND ROW(attempt.operation_id,attempt.organization_id,attempt.snapshot_id)
                  IS NOT DISTINCT FROM
                      ROW(residual.operation_id,residual.organization_id,residual.snapshot_id)
             JOIN candidate_analysis_attempt_state_events opened
               ON opened.analysis_attempt_id=attempt.analysis_attempt_id
              AND opened.event_kind='opened'
              AND opened.event_ordinal=0
             JOIN candidate_analysis_attempt_state_events event
               ON event.analysis_attempt_id=attempt.analysis_attempt_id
              AND event.event_kind='blocked'
              AND event.event_ordinal=1
              AND event.attempt_event_id=$3
              AND event.predecessor_event_id=opened.attempt_event_id
              AND event.event_hash=tool_truth_sha256(jsonb_build_object(
                    'attempt',attempt.analysis_attempt_id,'ordinal',1,'event','blocked',
                    'predecessor_event_id',opened.attempt_event_id,
                    'residual_hash',residual.residual_hash
                  )::TEXT)
            WHERE residual.residual_id=$2
              AND residual.owner_kind='candidate_analysis'
              AND residual.reason_code IN (
                    'candidate_h1_proposal_cap_exceeded',
                    'candidate_noncomplete_input_blocked',
                    'candidate_controller_proposal_page_cap_exceeded',
                    'candidate_conflict_resolution_requires_typed_retry',
                    'candidate_hypothesis_coverage_blocked',
                    'candidate_hypothesis_coverage_retry_exhausted'
                  )
              AND residual.closed_at IS NULL
              AND jsonb_array_length(residual.affected_inputs)=(
                    SELECT COUNT(DISTINCT affected.value)
                      FROM jsonb_array_elements_text(residual.affected_inputs) affected(value)
                  )
              AND residual.affected_inputs=(
                    SELECT to_jsonb(COALESCE(
                        array_agg(input.snapshot_input_id ORDER BY input.snapshot_input_id),
                        ARRAY[]::UUID[]
                    ))
                      FROM jsonb_array_elements_text(residual.affected_inputs) affected(value)
                      JOIN candidate_analysis_snapshot_inputs input
                        ON input.snapshot_id=attempt.snapshot_id
                       AND input.snapshot_input_id::TEXT=affected.value
                  )
              AND residual.next_action=jsonb_build_object(
                    'route','candidate_analysis_closeout','retry',FALSE
                  )
              AND residual.residual_hash=tool_truth_sha256(jsonb_build_object(
                    'analysis_attempt_id',attempt.analysis_attempt_id,
                    'snapshot_id',attempt.snapshot_id,
                    'reason_code',residual.reason_code,
                    'affected_input_ids',residual.affected_inputs
                  )::TEXT)"#,
    )
    .bind(analysis_attempt_id)
    .bind(residual_id)
    .bind(blocked_event_id)
    .fetch_optional(pool)
    .await?)
}

/// Loads a server-authored, replayable compiler recipe. The recipe contains
/// semantic ingredients and server-derived revision identities, never model
/// supplied exact-set hashes or compiled objects.
pub async fn load_host_compiler_recipe(
    pool: &PgPool,
    fence: &CandidateWriteFenceRow,
) -> Result<Value> {
    let mut tx = pool.begin().await?;
    let sealed_recipe: Option<Value> = sqlx::query_scalar(
        r#"SELECT material.compiler_recipe
              FROM candidate_analysis_host_compilation_materials material
              JOIN candidate_analysis_attempt_state_events terminal
                ON terminal.analysis_attempt_id=material.analysis_attempt_id
               AND terminal.event_kind='sealed'
             WHERE material.analysis_attempt_id=$1 AND material.snapshot_id=$2
               AND material.operation_id=$3 AND material.organization_id=$4"#,
    )
    .bind(fence.analysis_attempt_id)
    .bind(fence.snapshot_id)
    .bind(fence.operation_id)
    .bind(fence.organization_id)
    .fetch_optional(&mut *tx)
    .await?;
    if let Some(recipe) = sealed_recipe {
        tx.commit().await?;
        return Ok(recipe);
    }
    super::candidate_analysis::validate_write_fence_on(&mut tx, fence).await?;
    let unresolved_conflicts: i64 = sqlx::query_scalar(
        r#"SELECT count(*) FROM hypothesis_merge_decisions
            WHERE analysis_attempt_id=$1 AND decision_kind<>'keep_distinct'"#,
    )
    .bind(fence.analysis_attempt_id)
    .fetch_one(&mut *tx)
    .await?;
    if unresolved_conflicts != 0 {
        return Err(conflict("CANDIDATE_CONFLICT_DECISION_UNRESOLVED"));
    }
    let proposals: Vec<(Uuid, Value)> = sqlx::query_as(
        r#"SELECT proposal_id,structured_proposal FROM hypothesis_proposals
            WHERE analysis_attempt_id=$1 ORDER BY proposal_ordinal"#,
    )
    .bind(fence.analysis_attempt_id)
    .fetch_all(&mut *tx)
    .await?;
    if proposals.len() > 64 {
        return Err(conflict("CANDIDATE_CONTROLLER_PROPOSAL_PAGE_CAP_EXCEEDED"));
    }
    let mut items = Vec::new();
    for (proposal_id, structured) in proposals {
        let object = structured
            .as_object()
            .ok_or_else(|| conflict("CANDIDATE_COMPILER_PROPOSAL_INVALID"))?;
        if object
            .get("proposal_id")
            .and_then(Value::as_str)
            .and_then(|value| Uuid::parse_str(value).ok())
            != Some(proposal_id)
        {
            return Err(conflict("CANDIDATE_COMPILER_PROPOSAL_INVALID"));
        }
        let text = |name: &'static str| -> Result<String> {
            object
                .get(name)
                .and_then(Value::as_str)
                .filter(|value| !value.trim().is_empty())
                .map(ToOwned::to_owned)
                .ok_or_else(|| conflict("CANDIDATE_COMPILER_PROPOSAL_INVALID"))
        };
        let subject_kind = text("subject_kind")?;
        let subject_identity_hash = text("subject_identity_hash")?;
        let predicate_schema = text("predicate_schema")?;
        let predicate_version = object
            .get("predicate_version")
            .and_then(Value::as_u64)
            .and_then(|value| u32::try_from(value).ok())
            .filter(|value| *value > 0)
            .ok_or_else(|| conflict("CANDIDATE_COMPILER_PROPOSAL_INVALID"))?;
        let mut predicate_arguments = Map::new();
        for pair in object
            .get("predicate_arguments")
            .and_then(Value::as_array)
            .ok_or_else(|| conflict("CANDIDATE_COMPILER_PROPOSAL_INVALID"))?
        {
            let pair = pair
                .as_array()
                .filter(|pair| pair.len() == 2)
                .ok_or_else(|| conflict("CANDIDATE_COMPILER_PROPOSAL_INVALID"))?;
            let key = pair[0]
                .as_str()
                .ok_or_else(|| conflict("CANDIDATE_COMPILER_PROPOSAL_INVALID"))?;
            let value = pair[1]
                .as_str()
                .ok_or_else(|| conflict("CANDIDATE_COMPILER_PROPOSAL_INVALID"))?;
            if predicate_arguments
                .insert(key.to_owned(), Value::String(value.to_owned()))
                .is_some()
            {
                return Err(conflict("CANDIDATE_COMPILER_PROPOSAL_INVALID"));
            }
        }
        let trust_boundary = text("trust_boundary")?;
        let polarity = ClaimPolarity::try_from(text("polarity")?.as_str())
            .map_err(|error| DbError::Other(anyhow::Error::new(error)))?;
        let structured_claim = text("structured_claim")?;
        let impact = text("impact")?;
        let predicate = PredicateIdentity::new(
            predicate_schema.clone(),
            predicate_version,
            Value::Object(predicate_arguments.clone()),
        )
        .map_err(|error| DbError::Other(anyhow::Error::new(error)))?;
        let semantic_key = HypothesisSemanticKeyV1::new(
            fence.organization_id,
            AtTimeSubjectIdentity::new(subject_kind.clone(), subject_identity_hash.clone())
                .map_err(|error| DbError::Other(anyhow::Error::new(error)))?,
            predicate.clone(),
            trust_boundary.clone(),
            polarity,
        )
        .map_err(|error| DbError::Other(anyhow::Error::new(error)))?;
        let semantic_key_hash = semantic_key
            .hash()
            .map_err(|error| DbError::Other(anyhow::Error::new(error)))?;
        let initial_root = initial_root_id(fence.operation_id, &semantic_key)
            .map_err(|error| DbError::Other(anyhow::Error::new(error)))?;
        let current: Option<(Uuid, Uuid)> = sqlx::query_as(
            r#"SELECT root.root_id,head.head_revision_id FROM attack_hypotheses root
                JOIN attack_hypothesis_heads head USING(root_id)
               WHERE root.operation_id=$1 AND root.organization_id=$2
                 AND head.head_semantic_key_hash=$3 AND head.head_lifecycle_state='current'
               ORDER BY root.root_id LIMIT 1 FOR SHARE OF root,head"#,
        )
        .bind(fence.operation_id)
        .bind(fence.organization_id)
        .bind(&semantic_key_hash)
        .fetch_optional(&mut *tx)
        .await?;
        let generation_transition_hash = hash_json_on(
            &mut tx,
            &json!({
                "domain":"candidate_generation_transition.v1",
                "analysis_attempt_id":fence.analysis_attempt_id,
                "proposal_id":proposal_id,
                "semantic_key_hash":semantic_key_hash,
                "route":if current.is_some(){"attach_current"}else{"create_initial"},
            }),
        )
        .await?;
        let proof_rows: Vec<(String, String)> = sqlx::query_as(
            r#"SELECT source_role,source_hash FROM hypothesis_proposal_refs
                WHERE proposal_id=$1 ORDER BY ref_hash"#,
        )
        .bind(proposal_id)
        .fetch_all(&mut *tx)
        .await?;
        let proof_refs = proof_rows
            .iter()
            .filter(|row| row.0 == "support")
            .map(|row| json!({"kind":"tool_truth_evidence","id":row.1}))
            .collect::<Vec<_>>();
        let refutation_refs = proof_rows
            .iter()
            .filter(|row| row.0 == "contradiction")
            .map(|row| json!({"kind":"tool_truth_evidence","id":row.1}))
            .collect::<Vec<_>>();
        let (route, revision_material) = if let Some((root_id, revision_id)) = current {
            (
                json!({"kind":"attach_current","root_id":root_id,"revision_id":revision_id}),
                Value::Null,
            )
        } else {
            let route = json!({"kind":"create_initial","root_id":initial_root});
            let origin_decision_hash = hash_json_on(
                &mut tx,
                &json!({
                    "proposal_id":proposal_id,"route_kind":"create_initial","root_id":initial_root,
                    "predecessor_revision_id":Value::Null,"semantic_key_hash":semantic_key_hash,
                    "relation_sources":[],"generation_transition_hash":generation_transition_hash,
                    "successor_state":"proposed",
                }),
            )
            .await?;
            let revision_id =
                candidate_revision_id(initial_root, 0, &semantic_key_hash, &origin_decision_hash)
                    .map_err(|error| DbError::Other(anyhow::Error::new(error)))?;
            let proposal_body =
                super::candidate_analysis::AnalysisArtifactBodyRow::HypothesisProposal {
                    proposal_id,
                    subject_kind: subject_kind.clone(),
                    subject_identity_hash: subject_identity_hash.clone(),
                    predicate: predicate.clone(),
                    trust_boundary: trust_boundary.clone(),
                    polarity,
                    prose: structured_claim.clone(),
                    confidence: 0,
                    priority: 0,
                    tags: Vec::new(),
                    evidence_refs: proof_rows.iter().map(|row| row.1.clone()).collect(),
                };
            let revision_ingredients_hash = hash_json_on(
                &mut tx,
                &json!({"proposal":proposal_body,"origin_decision_hash":origin_decision_hash}),
            )
            .await?;
            let revision_hash = hash_json_on(
                &mut tx,
                &json!({
                    "revision_id":revision_id,"root_id":initial_root,"ordinal":0,
                    "semantic_key_hash":semantic_key_hash,"state":"proposed",
                    "ingredients":revision_ingredients_hash,
                }),
            )
            .await?;
            (
                route,
                json!({
                    "revision_id":revision_id,
                    "revision_hash":revision_hash,
                    "revision_ingredients_hash":revision_ingredients_hash,
                    "claim_clause_hash":hash_json_on(&mut tx,&json!({"structured_claim":&structured_claim})).await?,
                    "impact_hash":hash_json_on(&mut tx,&json!({"impact":&impact})).await?,
                    "trust_boundary_hash":hash_json_on(&mut tx,&json!({"trust_boundary":&trust_boundary})).await?,
                    "identity_hash":hash_json_on(&mut tx,&json!({"subject_kind":subject_kind,"subject_identity_hash":subject_identity_hash})).await?,
                    "derivation_digest":hash_json_on(&mut tx,&json!({"contract":"candidate_claim_component_derivation.v1"})).await?,
                    "objective_id":Uuid::new_v5(&revision_id,b"candidate_primary_objective.v1"),
                    "objective_hash":hash_json_on(&mut tx,&json!({"revision_id":revision_id,"objective":"candidate_primary"})).await?,
                    "stopping_criteria_hash":hash_json_on(&mut tx,&json!({"criteria":"plan_c_typed_verification"})).await?,
                    "compiler_digest":hash_json_on(&mut tx,&json!({"compiler":"candidate_contract_compiler.v1"})).await?,
                    "rule_digest":hash_json_on(&mut tx,&json!({"rules":"candidate_predicate_registry.v1"})).await?,
                    "policy_snapshot_hash":hash_json_on(&mut tx,&json!({"snapshot_id":fence.snapshot_id})).await?,
                    "outer_policy_digest":hash_json_on(&mut tx,&json!({"policy":"candidate_outer_aggregation.v1"})).await?,
                }),
            )
        };
        items.push(json!({
            "proposal_id":proposal_id,
            "semantic_key_hash":semantic_key_hash,
            "root_id":route.get("root_id"),
            "generation_transition_hash":generation_transition_hash,
            "state":"proposed",
            "proof_refs":proof_refs,
            "refutation_refs":refutation_refs,
            "route":route,
            "revision":revision_material,
            "predicate_schema":predicate_schema,
            "predicate_version":predicate_version,
            "predicate_arguments":Value::Object(predicate_arguments),
            "polarity":polarity.as_str(),
            "structured_claim":structured_claim,
            "trust_boundary":trust_boundary,
        }));
    }
    tx.commit().await?;
    Ok(json!({
        "schema":"candidate_host_compiler_recipe.v1",
        "analysis_attempt_id":fence.analysis_attempt_id,
        "snapshot_id":fence.snapshot_id,
        "operation_id":fence.operation_id,
        "organization_id":fence.organization_id,
        "items":items,
    }))
}

pub async fn persist_host_compilation_material_and_prepare_final(
    pool: &PgPool,
    mut input: PersistHostCompilationMaterial,
) -> Result<CandidateHostCompilationRecipeRow> {
    let counts = [
        input.mutation_count,
        input.claim_component_count,
        input.verification_contract_count,
        input.verification_plan_count,
    ];
    if counts.iter().any(|count| *count < 0)
        || (input.mutation_count == 0 && counts.iter().any(|count| *count != 0))
        || (input.mutation_count > 0 && counts.iter().skip(1).any(|count| *count == 0))
        || input.mutations.as_array().map(Vec::len) != usize::try_from(input.mutation_count).ok()
    {
        return Err(conflict("CANDIDATE_COMPILATION_MATERIAL_INVALID"));
    }
    let mut tx = pool.begin().await?;
    let identity = load_scheduler_identity_on(
        &mut tx,
        input.snapshot_id,
        input.stage_execution_id,
        input.attempt_ordinal,
    )
    .await?;
    let plan_id = ensure_team_plan_on(&mut tx, &identity).await?;
    let input_ids: Vec<Uuid> = sqlx::query_scalar(
        "SELECT snapshot_input_id FROM candidate_analysis_snapshot_inputs WHERE snapshot_id=$1 ORDER BY stable_input_key",
    )
    .bind(input.snapshot_id)
    .fetch_all(&mut *tx)
    .await?;
    let mut dispositions = Vec::with_capacity(input_ids.len());
    for input_id in &input_ids {
        let decision_hash = hash_json_on(
            &mut tx,
            &json!({
                "input_id":input_id,
                "disposition":"analyzed",
                "reason_code":"candidate_coverage_adequate",
            }),
        )
        .await?;
        dispositions.push(json!({
            "input_id":input_id,
            "disposition":"analyzed",
            "reason_code":"candidate_coverage_adequate",
            "decision_hash":decision_hash,
        }));
    }
    let routes = input
        .compiler_recipe
        .get("items")
        .and_then(Value::as_array)
        .cloned()
        .ok_or_else(|| conflict("CANDIDATE_COMPILATION_MATERIAL_INVALID"))?;
    let mut relations = Vec::new();
    for item in &routes {
        let proposal_id = item
            .get("proposal_id")
            .and_then(Value::as_str)
            .and_then(|value| Uuid::parse_str(value).ok())
            .ok_or_else(|| conflict("CANDIDATE_COMPILATION_MATERIAL_INVALID"))?;
        let root_id = item
            .get("route")
            .and_then(|value| value.get("root_id"))
            .and_then(Value::as_str)
            .and_then(|value| Uuid::parse_str(value).ok())
            .ok_or_else(|| conflict("CANDIDATE_COMPILATION_MATERIAL_INVALID"))?;
        let relation_kind = if item
            .get("route")
            .and_then(|value| value.get("kind"))
            .and_then(Value::as_str)
            == Some("attach_current")
        {
            "supports_existing"
        } else {
            "creates_hypothesis"
        };
        let referenced_inputs: Vec<Uuid> = sqlx::query_scalar(
            "SELECT DISTINCT snapshot_input_id FROM hypothesis_proposal_refs WHERE proposal_id=$1 ORDER BY snapshot_input_id",
        )
        .bind(proposal_id)
        .fetch_all(&mut *tx)
        .await?;
        for input_id in referenced_inputs {
            let decision_hash = hash_json_on(
                &mut tx,
                &json!({
                    "input_id":input_id,"root_id":root_id,"relation_kind":relation_kind,
                }),
            )
            .await?;
            relations.push(json!({
                "input_id":input_id,
                "root_id":root_id,
                "relation_kind":relation_kind,
                "decision_hash":decision_hash,
            }));
        }
    }
    let stable_compilation_request_id =
        Uuid::new_v5(&identity.attempt_id, b"candidate_host_compilation.v1");
    let stable_apply_request_id = Uuid::new_v5(&identity.attempt_id, b"candidate_gate_apply.v1");
    let persisted_recipe: Option<Value> = sqlx::query_scalar(
        "SELECT compiler_recipe FROM candidate_analysis_host_compilation_materials WHERE analysis_attempt_id=$1",
    )
    .bind(identity.attempt_id)
    .fetch_optional(&mut *tx)
    .await?;
    let material_exists = persisted_recipe.is_some();
    let expected_source_head_version = if let Some(recipe) = persisted_recipe {
        recipe
            .get("expected_source_head_version")
            .and_then(Value::as_i64)
            .ok_or_else(|| conflict("CANDIDATE_COMPILATION_MATERIAL_REPLAY_DRIFT"))?
    } else {
        sqlx::query_scalar(
            "SELECT last_source_batch_seq FROM investigation_projection_source_heads WHERE operation_id=$1",
        )
        .bind(identity.operation_id)
        .fetch_one(&mut *tx)
        .await?
    };
    input.compiler_recipe["expected_source_head_version"] = json!(expected_source_head_version);
    let material_id = Uuid::new_v5(
        &stable_compilation_request_id,
        b"candidate_host_compilation_material.v1",
    );
    let proposal_census_hash: String = sqlx::query_scalar(
        "SELECT census_hash FROM candidate_analysis_proposal_censuses WHERE analysis_attempt_id=$1",
    )
    .bind(identity.attempt_id)
    .fetch_one(&mut *tx)
    .await?;
    let critic_census_hash: String = sqlx::query_scalar(
        "SELECT census_hash FROM candidate_analysis_critic_censuses WHERE analysis_attempt_id=$1",
    )
    .bind(identity.attempt_id)
    .fetch_one(&mut *tx)
    .await?;
    let coverage_hashes: Vec<String> = sqlx::query_scalar(
        "SELECT review_hash FROM candidate_analysis_hypothesis_coverage_reviews WHERE analysis_attempt_id=$1 ORDER BY snapshot_input_id",
    )
    .bind(identity.attempt_id)
    .fetch_all(&mut *tx)
    .await?;
    let coverage_review_set_hash = hash_texts_on(&mut tx, &coverage_hashes).await?;
    let proposal_summaries = routes
        .iter()
        .map(|item| -> Result<Value> {
            let array_len = |name: &'static str| -> Result<u32> {
                item.get(name)
                    .and_then(Value::as_array)
                    .and_then(|values| u32::try_from(values.len()).ok())
                    .ok_or_else(|| conflict("CANDIDATE_COMPILATION_MATERIAL_INVALID"))
            };
            Ok(json!({
                "proposal_id":item.get("proposal_id").ok_or_else(||conflict("CANDIDATE_COMPILATION_MATERIAL_INVALID"))?,
                "semantic_key_hash":item.get("semantic_key_hash").ok_or_else(||conflict("CANDIDATE_COMPILATION_MATERIAL_INVALID"))?,
                "structured_claim":item.get("structured_claim").ok_or_else(||conflict("CANDIDATE_COMPILATION_MATERIAL_INVALID"))?,
                "trust_boundary":item.get("trust_boundary").ok_or_else(||conflict("CANDIDATE_COMPILATION_MATERIAL_INVALID"))?,
                "polarity":item.get("polarity").ok_or_else(||conflict("CANDIDATE_COMPILATION_MATERIAL_INVALID"))?,
                "route_kind":item.get("route").and_then(|route|route.get("kind")).ok_or_else(||conflict("CANDIDATE_COMPILATION_MATERIAL_INVALID"))?,
                "proof_ref_count":array_len("proof_refs")?,
                "refutation_ref_count":array_len("refutation_refs")?,
            }))
        })
        .collect::<Result<Vec<_>>>()?;
    let mut proposal_pages = Vec::new();
    let mut proposal_page_hashes = Vec::new();
    for (ordinal, proposals) in proposal_summaries.chunks(16).enumerate() {
        let page_body = json!({
            "page_ordinal":u32::try_from(ordinal).map_err(|_|conflict("CANDIDATE_COMPILATION_MATERIAL_INVALID"))?,
            "proposal_count":u32::try_from(proposals.len()).map_err(|_|conflict("CANDIDATE_COMPILATION_MATERIAL_INVALID"))?,
            "proposals":proposals,
        });
        let page_hash = hash_json_on(&mut tx, &page_body).await?;
        proposal_page_hashes.push(page_hash.clone());
        let mut page = page_body;
        page["page_hash"] = json!(page_hash);
        proposal_pages.push(page);
    }
    let proposal_page_set_hash = hash_texts_on(&mut tx, &proposal_page_hashes).await?;
    let final_input = json!({
        "snapshot_id":input.snapshot_id,
        "analysis_attempt_id":identity.attempt_id,
        "proposal_census_hash":proposal_census_hash,
        "critic_census_hash":critic_census_hash,
        "coverage_review_set_hash":coverage_review_set_hash,
        "claim_component_set_hash":&input.claim_component_set_hash,
        "verification_contract_set_hash":&input.verification_contract_set_hash,
        "verification_plan_set_hash":&input.verification_plan_set_hash,
        "proposal_pages":proposal_pages,
        "proposal_page_set_hash":proposal_page_set_hash,
    });
    let final_fence = ensure_claimed_work_on(
        &mut tx,
        &identity,
        plan_id,
        "controller",
        "candidate_controller_final",
        &format!("controller-final:{}", identity.attempt_id),
        None,
        None,
        0,
        &final_input,
    )
    .await?;
    let material_hash = hash_json_on(
        &mut tx,
        &json!({
            "compiler_recipe":&input.compiler_recipe,
            "mutations":&input.mutations,
            "input_dispositions":dispositions,
            "input_relations":relations,
            "mutation_count":input.mutation_count,
            "mutation_set_hash":&input.mutation_set_hash,
            "claim_component_count":input.claim_component_count,
            "claim_component_set_hash":&input.claim_component_set_hash,
            "verification_contract_count":input.verification_contract_count,
            "verification_contract_set_hash":&input.verification_contract_set_hash,
            "verification_plan_count":input.verification_plan_count,
            "verification_plan_set_hash":&input.verification_plan_set_hash,
            "generation_transition_count":input.mutation_count,
            "generation_transition_set_hash":&input.generation_transition_set_hash,
        }),
    )
    .await?;
    if !material_exists {
        sqlx::query(
            r#"INSERT INTO candidate_analysis_host_compilation_materials(
               compilation_material_id,stable_compilation_request_id,stable_apply_request_id,
               analysis_attempt_id,snapshot_id,operation_id,organization_id,
               final_submitter_worker_run_id,compiler_recipe,mutations,input_dispositions,
               input_relations,mutation_count,mutation_set_hash,claim_component_count,
               claim_component_set_hash,verification_contract_count,
               verification_contract_set_hash,verification_plan_count,
               verification_plan_set_hash,generation_transition_count,
               generation_transition_set_hash,material_hash)
           VALUES($1,$2,$3,$4,$5,$6,$7,$8,$9,$10,$11,$12,$13,$14,$15,$16,$17,$18,
                  $19,$20,$13,$21,$22)
           ON CONFLICT(analysis_attempt_id) DO NOTHING"#,
        )
        .bind(material_id)
        .bind(stable_compilation_request_id)
        .bind(stable_apply_request_id)
        .bind(identity.attempt_id)
        .bind(input.snapshot_id)
        .bind(identity.operation_id)
        .bind(identity.organization_id)
        .bind(final_fence.worker_run_id)
        .bind(&input.compiler_recipe)
        .bind(&input.mutations)
        .bind(json!(&dispositions))
        .bind(json!(&relations))
        .bind(input.mutation_count)
        .bind(&input.mutation_set_hash)
        .bind(input.claim_component_count)
        .bind(&input.claim_component_set_hash)
        .bind(input.verification_contract_count)
        .bind(&input.verification_contract_set_hash)
        .bind(input.verification_plan_count)
        .bind(&input.verification_plan_set_hash)
        .bind(&input.generation_transition_set_hash)
        .bind(&material_hash)
        .execute(&mut *tx)
        .await?;
    }
    let persisted = sqlx::query_as::<_, HostCompilationMaterialDbRow>(
        r#"SELECT compilation_material_id,stable_compilation_request_id,stable_apply_request_id,
                  analysis_attempt_id,snapshot_id,operation_id,organization_id,
                  final_submitter_worker_run_id,compiler_recipe,mutations,input_dispositions,
                  input_relations,mutation_count,mutation_set_hash,claim_component_count,
                  claim_component_set_hash,verification_contract_count,
                  verification_contract_set_hash,verification_plan_count,
                  verification_plan_set_hash,generation_transition_count,
                  generation_transition_set_hash,material_hash
             FROM candidate_analysis_host_compilation_materials
            WHERE analysis_attempt_id=$1"#,
    )
    .bind(identity.attempt_id)
    .fetch_one(&mut *tx)
    .await?;
    if persisted.compilation_material_id != material_id
        || persisted.stable_compilation_request_id != stable_compilation_request_id
        || persisted.stable_apply_request_id != stable_apply_request_id
        || persisted.analysis_attempt_id != identity.attempt_id
        || persisted.snapshot_id != input.snapshot_id
        || persisted.operation_id != identity.operation_id
        || persisted.organization_id != identity.organization_id
        || persisted.final_submitter_worker_run_id != final_fence.worker_run_id
        || persisted.compiler_recipe != input.compiler_recipe
        || persisted.mutations != input.mutations
        || persisted.input_dispositions != json!(&dispositions)
        || persisted.input_relations != json!(&relations)
        || persisted.mutation_count != input.mutation_count
        || persisted.mutation_set_hash != input.mutation_set_hash
        || persisted.claim_component_count != input.claim_component_count
        || persisted.claim_component_set_hash != input.claim_component_set_hash
        || persisted.verification_contract_count != input.verification_contract_count
        || persisted.verification_contract_set_hash != input.verification_contract_set_hash
        || persisted.verification_plan_count != input.verification_plan_count
        || persisted.verification_plan_set_hash != input.verification_plan_set_hash
        || persisted.generation_transition_count != input.mutation_count
        || persisted.generation_transition_set_hash != input.generation_transition_set_hash
        || persisted.material_hash != material_hash
    {
        return Err(conflict("CANDIDATE_COMPILATION_MATERIAL_REPLAY_DRIFT"));
    }
    tx.commit().await?;
    Ok(CandidateHostCompilationRecipeRow {
        stable_compilation_request_id,
        stable_apply_request_id,
        controller_fence: final_fence,
        expected_source_head_version,
        recipe: input.compiler_recipe,
        material_hash,
        controller_final_input: final_input,
    })
}

pub async fn reduce_coverage_and_seal_h2(
    pool: &PgPool,
    snapshot_id: Uuid,
    stage_execution_id: Uuid,
    attempt_ordinal: i32,
) -> Result<CandidateCoverageClosureRow> {
    let opened =
        open_or_replay_attempt_runtime(pool, snapshot_id, stage_execution_id, attempt_ordinal)
            .await?;
    let input_ids: Vec<Uuid> = sqlx::query_scalar(
        "SELECT snapshot_input_id FROM candidate_analysis_snapshot_inputs WHERE snapshot_id=$1 ORDER BY stable_input_key",
    )
    .bind(snapshot_id)
    .fetch_all(pool)
    .await?;
    if input_ids.is_empty() {
        return Err(conflict("CANDIDATE_INPUT_CENSUS_EMPTY"));
    }
    let proposal_count: i64 = sqlx::query_scalar(
        "SELECT count(*) FROM hypothesis_proposals WHERE analysis_attempt_id=$1",
    )
    .bind(opened.analysis_attempt_id)
    .fetch_one(pool)
    .await?;
    if proposal_count > 64 {
        let mut tx = pool.begin().await?;
        let residual_hash = block_candidate_attempt_on(
            &mut tx,
            &opened.controller_fence,
            "candidate_controller_proposal_page_cap_exceeded",
            &input_ids,
        )
        .await?;
        tx.commit().await?;
        return Ok(CandidateCoverageClosureRow::Blocked { residual_hash });
    }
    let mut review_hashes = Vec::with_capacity(input_ids.len());
    let mut missed = false;
    let mut blocked = false;
    for input_id in &input_ids {
        let review = super::candidate_analysis::reduce_hypothesis_coverage_review(
            pool,
            super::candidate_analysis::ReduceCoverageReviewInput {
                fence: opened.controller_fence.clone(),
                stable_reduction_request_id: Uuid::new_v5(
                    &opened.analysis_attempt_id,
                    format!("candidate_coverage_reduction:{input_id}.v1").as_bytes(),
                ),
                input_id: *input_id,
            },
        )
        .await?;
        review_hashes.push(review.coverage_review_hash);
        missed |= review.outcome == "missed_hypothesis";
        blocked |= review.outcome == "blocked";
    }
    let coverage_review_set_hash = {
        let mut tx = pool.begin().await?;
        let hash = hash_texts_on(&mut tx, &review_hashes).await?;
        tx.commit().await?;
        hash
    };
    let unresolved_conflict_decisions: i64 = sqlx::query_scalar(
        r#"SELECT count(*) FROM hypothesis_merge_decisions
            WHERE analysis_attempt_id=$1 AND decision_kind<>'keep_distinct'"#,
    )
    .bind(opened.analysis_attempt_id)
    .fetch_one(pool)
    .await?;
    if unresolved_conflict_decisions > 0 {
        let mut tx = pool.begin().await?;
        let residual_hash = block_candidate_attempt_on(
            &mut tx,
            &opened.controller_fence,
            "candidate_conflict_resolution_requires_typed_retry",
            &input_ids,
        )
        .await?;
        tx.commit().await?;
        return Ok(CandidateCoverageClosureRow::Blocked { residual_hash });
    }
    if missed {
        return supersede_attempt_after_miss(pool, &opened.controller_fence).await;
    }
    if blocked {
        let mut tx = pool.begin().await?;
        let residual_hash = block_candidate_attempt_on(
            &mut tx,
            &opened.controller_fence,
            "candidate_hypothesis_coverage_blocked",
            &input_ids,
        )
        .await?;
        tx.commit().await?;
        return Ok(CandidateCoverageClosureRow::Blocked { residual_hash });
    }
    let h2 = super::candidate_analysis::seal_analysis_census(
        pool,
        super::candidate_analysis::SealAnalysisCensusInput {
            fence: opened.controller_fence,
            stable_census_request_id: Uuid::new_v5(
                &opened.analysis_attempt_id,
                b"candidate_h2_census.v1",
            ),
            census_kind: super::candidate_analysis::AnalysisCensusKindRow::Critic,
        },
    )
    .await?;
    let proposal_census_hash: String = sqlx::query_scalar(
        "SELECT census_hash FROM candidate_analysis_proposal_censuses WHERE analysis_attempt_id=$1",
    )
    .bind(opened.analysis_attempt_id)
    .fetch_one(pool)
    .await?;
    Ok(CandidateCoverageClosureRow::Ready {
        proposal_census_hash,
        critic_census_hash: h2.census_hash,
        coverage_review_set_hash,
    })
}

async fn supersede_attempt_after_miss(
    pool: &PgPool,
    fence: &CandidateWriteFenceRow,
) -> Result<CandidateCoverageClosureRow> {
    let mut tx = pool.begin().await?;
    let (retry_limit, predecessor_attempt_input_hash): (i32, String) = sqlx::query_as(
        "SELECT retry_limit,attempt_input_hash FROM candidate_analysis_attempts WHERE analysis_attempt_id=$1 FOR SHARE",
    )
    .bind(fence.analysis_attempt_id)
    .fetch_one(&mut *tx)
    .await?;
    let next_ordinal = fence
        .analysis_attempt_ordinal
        .checked_add(1)
        .ok_or_else(|| conflict("CANDIDATE_ATTEMPT_ORDINAL_OVERFLOW"))?;
    let opened_event_id = Uuid::new_v5(&fence.analysis_attempt_id, b"candidate_attempt_opened.v1");
    let terminal_event_id = Uuid::new_v5(
        &fence.analysis_attempt_id,
        b"candidate_attempt_superseded_missed.v1",
    );
    let terminal_hash = hash_json_on(
        &mut tx,
        &json!({
            "attempt":fence.analysis_attempt_id,
            "ordinal":1,
            "event":"superseded_missed_hypothesis",
            "predecessor_event_id":opened_event_id,
        }),
    )
    .await?;
    let (missed_hypothesis_signals, missed_hypothesis_signal_set_hash) =
        load_retry_missed_signals_on(&mut tx, fence.analysis_attempt_id).await?;
    if missed_hypothesis_signals.is_empty() {
        return Err(conflict("CANDIDATE_RETRY_MISS_SIGNAL_EMPTY"));
    }
    let successor_attempt_input_hash = hash_json_on(
        &mut tx,
        &json!({
            "schema":"candidate_retry_attempt_input.v1",
            "predecessor_attempt_id":fence.analysis_attempt_id,
            "predecessor_attempt_input_hash":predecessor_attempt_input_hash,
            "predecessor_terminal_event_id":terminal_event_id,
            "predecessor_terminal_event_hash":terminal_hash,
            "missed_hypothesis_signal_count":missed_hypothesis_signals.len(),
            "missed_hypothesis_signal_set_hash":missed_hypothesis_signal_set_hash,
        }),
    )
    .await?;
    let successor_id = Uuid::new_v5(
        &fence.snapshot_id,
        format!("candidate_analysis_attempt:{next_ordinal}").as_bytes(),
    );
    let existing_terminal: Option<(Uuid, String, String)> = sqlx::query_as(
        r#"SELECT attempt_event_id,event_kind,event_hash
              FROM candidate_analysis_attempt_state_events
             WHERE analysis_attempt_id=$1
               AND event_kind IN ('superseded_missed_hypothesis','sealed','blocked')"#,
    )
    .bind(fence.analysis_attempt_id)
    .fetch_optional(&mut *tx)
    .await?;
    if let Some((existing_event_id, event_kind, existing_event_hash)) = existing_terminal {
        let existing_successor: Option<(Uuid, Option<Uuid>, String)> = sqlx::query_as(
            r#"SELECT analysis_attempt_id,predecessor_attempt_id,attempt_input_hash
                  FROM candidate_analysis_attempts
                 WHERE snapshot_id=$1 AND attempt_ordinal=$2"#,
        )
        .bind(fence.snapshot_id)
        .bind(next_ordinal)
        .fetch_optional(&mut *tx)
        .await?;
        if event_kind != "superseded_missed_hypothesis"
            || existing_event_id != terminal_event_id
            || existing_event_hash != terminal_hash
            || existing_successor
                != Some((
                    successor_id,
                    Some(fence.analysis_attempt_id),
                    successor_attempt_input_hash,
                ))
        {
            return Err(conflict("CANDIDATE_ATTEMPT_REPLAY_DRIFT"));
        }
        tx.commit().await?;
        return Ok(CandidateCoverageClosureRow::RetryAttempt {
            next_attempt_ordinal: next_ordinal,
        });
    }
    super::candidate_analysis::validate_write_fence_on(&mut tx, fence).await?;
    if next_ordinal > retry_limit {
        let affected_input_ids: Vec<Uuid> = sqlx::query_scalar(
            "SELECT snapshot_input_id FROM candidate_analysis_snapshot_inputs WHERE snapshot_id=$1 ORDER BY stable_input_key",
        )
        .bind(fence.snapshot_id)
        .fetch_all(&mut *tx)
        .await?;
        let residual_hash = block_candidate_attempt_on(
            &mut tx,
            fence,
            "candidate_hypothesis_coverage_retry_exhausted",
            &affected_input_ids,
        )
        .await?;
        tx.commit().await?;
        return Ok(CandidateCoverageClosureRow::Blocked { residual_hash });
    }
    let existing: Option<Uuid> = sqlx::query_scalar(
        "SELECT analysis_attempt_id FROM candidate_analysis_attempts WHERE snapshot_id=$1 AND attempt_ordinal=$2",
    )
    .bind(fence.snapshot_id)
    .bind(next_ordinal)
    .fetch_optional(&mut *tx)
    .await?;
    if let Some(existing_id) = existing {
        let _ = existing_id;
        return Err(conflict("CANDIDATE_ATTEMPT_ORPHAN_SUCCESSOR"));
    }
    sqlx::query(
        r#"INSERT INTO candidate_analysis_attempt_state_events(
               attempt_event_id,analysis_attempt_id,event_ordinal,event_kind,
               predecessor_event_id,event_hash)
           VALUES($1,$2,1,'superseded_missed_hypothesis',$3,$4)"#,
    )
    .bind(terminal_event_id)
    .bind(fence.analysis_attempt_id)
    .bind(opened_event_id)
    .bind(terminal_hash)
    .execute(&mut *tx)
    .await?;
    sqlx::query(
        r#"INSERT INTO candidate_analysis_attempts(
               analysis_attempt_id,snapshot_id,operation_id,organization_id,attempt_ordinal,
               predecessor_attempt_id,attempt_input_hash,attack_class_checklist_version,
               attack_class_checklist_digest,trust_boundary_checklist_version,
               trust_boundary_checklist_digest,coverage_sampling_contract_version,
               coverage_sampling_contract_digest,retry_limit)
           SELECT $1,snapshot_id,operation_id,organization_id,$2,analysis_attempt_id,
                  $4,attack_class_checklist_version,
                  attack_class_checklist_digest,trust_boundary_checklist_version,
                  trust_boundary_checklist_digest,coverage_sampling_contract_version,
                  coverage_sampling_contract_digest,retry_limit
             FROM candidate_analysis_attempts WHERE analysis_attempt_id=$3"#,
    )
    .bind(successor_id)
    .bind(next_ordinal)
    .bind(fence.analysis_attempt_id)
    .bind(&successor_attempt_input_hash)
    .execute(&mut *tx)
    .await?;
    let successor_opened_hash = hash_json_on(
        &mut tx,
        &json!({
            "attempt":successor_id,
            "ordinal":0,
            "event":"opened",
            "predecessor_attempt_id":fence.analysis_attempt_id,
            "predecessor_terminal_event_id":terminal_event_id,
        }),
    )
    .await?;
    sqlx::query(
        r#"INSERT INTO candidate_analysis_attempt_state_events(
               attempt_event_id,analysis_attempt_id,event_ordinal,event_kind,event_hash)
           VALUES($1,$2,0,'opened',$3)"#,
    )
    .bind(Uuid::new_v5(&successor_id, b"candidate_attempt_opened.v1"))
    .bind(successor_id)
    .bind(successor_opened_hash)
    .execute(&mut *tx)
    .await?;
    tx.commit().await?;
    Ok(CandidateCoverageClosureRow::RetryAttempt {
        next_attempt_ordinal: next_ordinal,
    })
}

pub async fn load_artifact_receipt_by_provider_attempt(
    pool: &PgPool,
    provider_attempt_id: Uuid,
) -> Result<Option<CandidateArtifactReceiptRow>> {
    Ok(sqlx::query_as::<_, (Uuid, String)>(
        r#"SELECT artifact.artifact_id,artifact.artifact_hash
             FROM candidate_analysis_provider_attempts provider
             JOIN candidate_analysis_artifacts artifact
               ON artifact.artifact_id=provider.artifact_id
             JOIN stage_worker_outputs output
               ON output.id=artifact.stage_worker_output_id
            WHERE provider.provider_attempt_id=$1"#,
    )
    .bind(provider_attempt_id)
    .fetch_optional(pool)
    .await?
    .map(|row| CandidateArtifactReceiptRow {
        artifact_id: row.0,
        artifact_hash: row.1,
        replayed: true,
    }))
}
