//! Compound CandidateAttempt, P1 WorkerRun, and execution-lane transactions.

use chrono::{DateTime, Utc};
use sha2::{Digest, Sha256};
use sqlx::{PgPool, Postgres, Transaction};
use std::collections::{BTreeMap, BTreeSet};
use uuid::Uuid;

use super::runtime_memory_tx::RuntimeMemoryTxFence;
use super::stage_worker_runs::{self, NewStageWorkerRun, StageWorkerRunRow, StageWorkerRunStatus};
use super::{attack_execution_lanes, attack_waves, message_chains};

#[derive(Debug, Clone, sqlx::FromRow, PartialEq)]
pub struct CandidateAttemptRow {
    pub id: Uuid,
    pub candidate_id: Uuid,
    pub approval_id: Uuid,
    pub operation_id: Uuid,
    pub scope_snapshot_id: Uuid,
    pub wave_run_id: Uuid,
    pub wave_unit_id: Uuid,
    pub organization_id: Uuid,
    pub target_live_id: Option<Uuid>,
    pub target_type_at_time: String,
    pub target_value_at_time: String,
    pub target_identity_hash: String,
    pub candidate_plan_hash: String,
    pub ordinal: i32,
    pub status: String,
    pub stage_worker_run_id: Option<Uuid>,
    pub result_json: Option<serde_json::Value>,
    pub result_hash: Option<String>,
    pub row_version: i64,
    pub created_at: DateTime<Utc>,
    pub updated_at: DateTime<Utc>,
    pub terminal_at: Option<DateTime<Utc>>,
}

#[derive(Debug, Clone)]
pub struct CandidateClaimQuery {
    pub operation_id: Uuid,
    pub scope_snapshot_id: Uuid,
    pub wave_run_id: Uuid,
    pub wave_unit_id: Uuid,
    pub organization_id: Uuid,
    pub verification_stage_execution_id: Uuid,
    pub verification_stage_run_unit_id: Uuid,
    pub lease_owner: String,
    pub lease_seconds: i32,
}

#[derive(Debug, Clone)]
pub struct ClaimedCandidateAttempt {
    pub attempt: CandidateAttemptRow,
    pub worker: StageWorkerRunRow,
    pub execution_plan: serde_json::Value,
    pub allowed_capability_ids: Vec<String>,
    pub allowed_action_kinds: Vec<String>,
    pub budget: serde_json::Value,
}

#[derive(Debug, Clone, PartialEq)]
pub struct AuthorizedCandidateAction {
    pub action_id: Uuid,
    pub action_ordinal: i32,
    pub target_id: Uuid,
    pub capability_id: String,
    pub action_kind: String,
    pub canonical_args: serde_json::Value,
    pub budget: serde_json::Value,
}

#[derive(Debug, Clone, PartialEq)]
pub enum CandidateActionStart {
    Authorized(AuthorizedCandidateAction),
    ExistingTerminal {
        action_id: Uuid,
        status: String,
        outcome: serde_json::Value,
    },
    OutcomeUnknown {
        action_id: Uuid,
    },
}

#[derive(Debug, Clone)]
pub struct BeginCandidateAction {
    pub operation_id: Uuid,
    pub stage_execution_id: Uuid,
    pub stage_run_unit_id: Uuid,
    pub organization_id: Uuid,
    pub worker_run_id: Uuid,
    pub lease_token: Uuid,
    pub attempt_epoch: i64,
    pub candidate_id: Uuid,
    pub approval_id: Uuid,
    pub attempt_id: Uuid,
    pub candidate_plan_hash: String,
    pub workspace_path_sha256: String,
    pub action_ordinal: i32,
}

#[derive(Debug, Clone)]
pub struct FinishCandidateAction {
    pub operation_id: Uuid,
    pub stage_execution_id: Uuid,
    pub stage_run_unit_id: Uuid,
    pub organization_id: Uuid,
    pub worker_run_id: Uuid,
    pub lease_token: Uuid,
    pub attempt_epoch: i64,
    pub candidate_id: Uuid,
    pub approval_id: Uuid,
    pub attempt_id: Uuid,
    pub candidate_plan_hash: String,
    pub action_id: Uuid,
    pub success: bool,
    pub outcome: serde_json::Value,
    pub error_code: Option<String>,
}

#[derive(Debug, Clone)]
pub struct CandidateExecutionHeartbeat {
    pub operation_id: Uuid,
    pub scope_snapshot_id: Uuid,
    pub wave_run_id: Uuid,
    pub wave_unit_id: Uuid,
    pub organization_id: Uuid,
    pub attempt_id: Uuid,
    pub worker_run_id: Uuid,
    pub stage_execution_id: Uuid,
    pub stage_run_unit_id: Uuid,
    pub lease_token: Uuid,
    pub lease_owner: String,
    pub attempt_epoch: i64,
    pub expected_checkpoint_version: i64,
    pub extend_seconds: i32,
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct HeartbeatOutcome {
    pub lease_expires_at: DateTime<Utc>,
    pub attempt_epoch: i64,
    pub checkpoint_version: i64,
}

#[derive(Debug, Clone)]
pub struct CandidateExecutionRelease {
    pub operation_id: Uuid,
    pub scope_snapshot_id: Uuid,
    pub wave_run_id: Uuid,
    pub wave_unit_id: Uuid,
    pub organization_id: Uuid,
    pub attempt_id: Uuid,
    pub worker_run_id: Uuid,
    pub stage_execution_id: Uuid,
    pub stage_run_unit_id: Uuid,
    pub lease_token: Uuid,
    pub lease_owner: String,
    pub attempt_epoch: i64,
    pub expected_checkpoint_version: i64,
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct ReleaseOutcome {
    pub requeued: bool,
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct AttemptEvidenceLink {
    pub evidence_id: i64,
    pub role: String,
}

#[derive(Debug, Clone)]
pub struct RecordAttemptSubmission {
    pub operation_id: Uuid,
    pub scope_snapshot_id: Uuid,
    pub wave_run_id: Uuid,
    pub wave_unit_id: Uuid,
    pub organization_id: Uuid,
    pub candidate_id: Uuid,
    pub approval_id: Uuid,
    pub attempt_id: Uuid,
    pub candidate_plan_hash: String,
    pub worker_run_id: Uuid,
    pub stage_execution_id: Uuid,
    pub stage_run_unit_id: Uuid,
    pub lease_token: Uuid,
    pub lease_owner: String,
    pub attempt_epoch: i64,
    pub expected_checkpoint_version: i64,
    pub result_json: serde_json::Value,
    pub evidence: Vec<AttemptEvidenceLink>,
}

#[derive(Debug, Clone)]
pub struct RecordedAttemptSubmission {
    pub attempt: CandidateAttemptRow,
    pub replayed: bool,
}

#[derive(Debug, sqlx::FromRow)]
struct ClaimCandidateRow {
    candidate_id: Uuid,
    approval_id: Uuid,
    target_live_id: Option<Uuid>,
    target_type_at_time: String,
    target_value_at_time: String,
    target_identity_hash: String,
    candidate_plan_hash: String,
    execution_plan: serde_json::Value,
    allowed_capability_ids: Vec<String>,
    allowed_action_kinds: Vec<String>,
    budget: serde_json::Value,
}

#[derive(Debug, sqlx::FromRow)]
struct CandidateActionAuthorityRow {
    target_id: Option<Uuid>,
    target_identity_hash: String,
    execution_plan: serde_json::Value,
    allowed_capability_ids: Vec<String>,
    allowed_action_kinds: Vec<String>,
    approval_budget: serde_json::Value,
}

#[derive(Debug, sqlx::FromRow)]
struct CandidateActionJournalRow {
    id: Uuid,
    action_ordinal: i32,
    capability_id: String,
    action_kind: String,
    canonical_args: serde_json::Value,
    status: String,
    outcome: Option<serde_json::Value>,
}

#[derive(Debug, sqlx::FromRow)]
struct TerminalActionJournalRow {
    action_ordinal: i32,
    status: String,
}

const ATTEMPT_COLUMNS: &str = "id,candidate_id,approval_id,operation_id,scope_snapshot_id,\
    wave_run_id,wave_unit_id,organization_id,target_live_id,target_type_at_time,\
    target_value_at_time,target_identity_hash,candidate_plan_hash,ordinal,status,\
    stage_worker_run_id,result_json,result_hash,row_version,created_at,updated_at,terminal_at";

pub(super) async fn validate_terminal_action_journal(
    tx: &mut Transaction<'_, Postgres>,
    attempt_id: Uuid,
    execution_plan: &serde_json::Value,
) -> crate::Result<()> {
    let planned_actions = execution_plan
        .get("actions")
        .and_then(serde_json::Value::as_array)
        .ok_or_else(|| conflict("Candidate execution plan actions are invalid"))?;
    let mut expected_ordinals = BTreeSet::new();
    for action in planned_actions {
        let ordinal = action
            .get("ordinal")
            .and_then(serde_json::Value::as_i64)
            .and_then(|ordinal| i32::try_from(ordinal).ok())
            .filter(|ordinal| *ordinal >= 0)
            .ok_or_else(|| conflict("Candidate execution plan action ordinal is invalid"))?;
        if !expected_ordinals.insert(ordinal) {
            return Err(conflict(
                "Candidate execution plan action ordinal is duplicated",
            ));
        }
    }
    let journal = sqlx::query_as::<_, TerminalActionJournalRow>(
        "SELECT action_ordinal,status FROM candidate_attempt_actions
         WHERE attempt_id=$1 ORDER BY action_ordinal FOR UPDATE",
    )
    .bind(attempt_id)
    .fetch_all(&mut **tx)
    .await?;
    let actual_ordinals = journal
        .iter()
        .map(|action| action.action_ordinal)
        .collect::<BTreeSet<_>>();
    if actual_ordinals != expected_ordinals
        || journal.len() != expected_ordinals.len()
        || journal
            .iter()
            .any(|action| !matches!(action.status.as_str(), "completed" | "failed"))
    {
        return Err(conflict("Candidate action journal is not terminal"));
    }
    Ok(())
}

/// Read-only review projection for one exact operation/project/snapshot/wave.
/// The request has no caller-selected org or actor; every Attempt must join the
/// frozen scope unit and the active project registration for its operation.
pub async fn list_for_review_wave(
    pool: &PgPool,
    operation_id: Uuid,
    wave_run_id: Uuid,
) -> crate::Result<Vec<CandidateAttemptRow>> {
    sqlx::query_as::<_, CandidateAttemptRow>(
        r#"SELECT attempt.id,attempt.candidate_id,attempt.approval_id,
                  attempt.operation_id,attempt.scope_snapshot_id,attempt.wave_run_id,
                  attempt.wave_unit_id,attempt.organization_id,attempt.target_live_id,
                  attempt.target_type_at_time,attempt.target_value_at_time,
                  attempt.target_identity_hash,attempt.candidate_plan_hash,
                  attempt.ordinal,attempt.status,attempt.stage_worker_run_id,
                  attempt.result_json,attempt.result_hash,attempt.row_version,
                  attempt.created_at,attempt.updated_at,attempt.terminal_at
             FROM candidate_attempts attempt
             JOIN attack_wave_runs wave
               ON wave.id=attempt.wave_run_id
              AND wave.operation_id=attempt.operation_id
              AND wave.scope_snapshot_id=attempt.scope_snapshot_id
             JOIN operation_state operation
               ON operation.operation_id=wave.operation_id
              AND operation.project_scope_id IS NOT NULL
             JOIN project_scopes project
               ON project.project_scope_id=operation.project_scope_id
              AND project.retired_at IS NULL
             JOIN operation_org_scope_snapshots snapshot
               ON snapshot.id=attempt.scope_snapshot_id
              AND snapshot.operation_id=attempt.operation_id
              AND snapshot.project_scope_id=operation.project_scope_id
              AND snapshot.sealed_at IS NOT NULL
             JOIN operation_org_scope_units scope_unit
               ON scope_unit.snapshot_id=attempt.scope_snapshot_id
              AND scope_unit.organization_id=attempt.organization_id
            WHERE attempt.operation_id=$1 AND attempt.wave_run_id=$2
            ORDER BY attempt.organization_id,attempt.candidate_id,attempt.ordinal"#,
    )
    .bind(operation_id)
    .bind(wave_run_id)
    .fetch_all(pool)
    .await
    .map_err(Into::into)
}

fn conflict(message: &str) -> crate::DbError {
    crate::DbError::Other(anyhow::anyhow!(message.to_string()))
}

fn runtime_error(error: super::runtime_memory_tx::RuntimeMemoryStoreError) -> crate::DbError {
    crate::DbError::Other(anyhow::Error::new(error))
}

fn canonicalize_json(value: &serde_json::Value) -> serde_json::Value {
    match value {
        serde_json::Value::Object(map) => serde_json::Value::Object(
            map.iter()
                .map(|(key, value)| (key.clone(), canonicalize_json(value)))
                .collect::<BTreeMap<_, _>>()
                .into_iter()
                .collect(),
        ),
        serde_json::Value::Array(items) => {
            serde_json::Value::Array(items.iter().map(canonicalize_json).collect())
        }
        _ => value.clone(),
    }
}

fn canonical_result_hash(value: &serde_json::Value) -> crate::Result<String> {
    let bytes = serde_json::to_vec(&canonicalize_json(value))?;
    let digest = Sha256::digest(bytes)
        .iter()
        .map(|byte| format!("{byte:02x}"))
        .collect::<String>();
    Ok(format!("sha256:{digest}"))
}

async fn lock_v2_operation(
    tx: &mut Transaction<'_, Postgres>,
    operation_id: Uuid,
) -> crate::Result<()> {
    let contracts: Option<(String, String)> = sqlx::query_as(
        "SELECT runtime_memory_contract,attack_execution_contract FROM operation_state
         WHERE operation_id=$1 FOR UPDATE",
    )
    .bind(operation_id)
    .fetch_optional(&mut **tx)
    .await?;
    match contracts {
        Some((runtime, attack)) if runtime == "v2_only" && attack == "v2_only" => Ok(()),
        Some(_) => Err(conflict(
            "operation contracts do not execute Candidate V2 verifier",
        )),
        None => Err(crate::DbError::NotFound("operation_state".to_string())),
    }
}

async fn lock_claim_scope(
    tx: &mut Transaction<'_, Postgres>,
    operation_id: Uuid,
    scope_snapshot_id: Uuid,
    wave_run_id: Uuid,
    wave_unit_id: Uuid,
    organization_id: Uuid,
) -> crate::Result<()> {
    attack_waves::lock_wave(tx, operation_id, scope_snapshot_id, wave_run_id).await?;
    let wave_unit = attack_waves::lock_wave_unit(
        tx,
        operation_id,
        scope_snapshot_id,
        wave_run_id,
        wave_unit_id,
        organization_id,
    )
    .await?;
    if !wave_unit.review_closed || wave_unit.verification_closed || wave_unit.terminal_at.is_some()
    {
        return Err(conflict("WaveUnit is not open for Candidate verification"));
    }
    Ok(())
}

/// Reconcile the single global lane before a new claim. A verifier crash never
/// creates a new Attempt: a side-effect action left in `started` becomes
/// `outcome_unknown` and the same WorkerRun is parked for review; otherwise the
/// same WorkerRun is requeued with its checkpoint intact.
async fn recover_expired_candidate_lane(
    tx: &mut Transaction<'_, Postgres>,
    lane: &attack_execution_lanes::AttackExecutionLaneRow,
) -> crate::Result<bool> {
    let Some(worker_run_id) = lane.stage_worker_run_id else {
        return Ok(true);
    };
    let Some(lease_token) = lane.lease_token else {
        return Err(conflict("occupied Candidate lane has no lease token"));
    };
    if lane
        .lease_expires_at
        .is_some_and(|expiry| expiry > Utc::now())
    {
        return Ok(false);
    }

    let attempt_id: Option<Uuid> = sqlx::query_scalar(
        "SELECT id FROM candidate_attempts
         WHERE stage_worker_run_id=$1 AND status='running' FOR UPDATE",
    )
    .bind(worker_run_id)
    .fetch_optional(&mut **tx)
    .await?;
    let attempt_id = attempt_id
        .ok_or_else(|| conflict("expired Candidate lane has no running Attempt owner"))?;
    let outcome_unknown = sqlx::query(
        "UPDATE candidate_attempt_actions
         SET status='outcome_unknown',error_code='worker_crashed_after_action_start',
             completed_at=NOW(),updated_at=NOW()
         WHERE attempt_id=$1 AND status='started' AND completed_at IS NULL",
    )
    .bind(attempt_id)
    .execute(&mut **tx)
    .await?
    .rows_affected();
    let next_status = if outcome_unknown == 0 {
        "queued"
    } else {
        "recovery_required"
    };
    let cleared = sqlx::query(
        "UPDATE attack_execution_lanes
         SET stage_worker_run_id=NULL,lease_token=NULL,lease_owner=NULL,
             lease_expires_at=NULL,updated_at=NOW()
         WHERE lane_key=$1 AND stage_worker_run_id=$2 AND lease_token=$3
           AND lease_expires_at<=NOW()",
    )
    .bind(attack_execution_lanes::GLOBAL_EXPLOIT_LANE)
    .bind(worker_run_id)
    .bind(lease_token)
    .execute(&mut **tx)
    .await?;
    if cleared.rows_affected() != 1 {
        return Err(conflict("expired Candidate lane recovery CAS lost"));
    }
    let recovered = sqlx::query(
        "UPDATE stage_worker_runs
         SET status=$3,lease_token=NULL,lease_owner=NULL,lease_acquired_at=NULL,
             lease_expires_at=NULL,heartbeat_at=NULL,active_tool_call_id=NULL,
             active_tool_started_at=NULL,updated_at=NOW()
         WHERE id=$1 AND lease_token=$2 AND status='running' AND lease_expires_at<=NOW()",
    )
    .bind(worker_run_id)
    .bind(lease_token)
    .bind(next_status)
    .execute(&mut **tx)
    .await?;
    if recovered.rows_affected() != 1 {
        return Err(conflict("expired Candidate WorkerRun recovery CAS lost"));
    }
    Ok(true)
}

/// Claim one exact approved Candidate and bind its Attempt to a P1 WorkerRun
/// and the global exploit lane with one server-generated lease token.
pub async fn claim_next_candidate_attempt(
    pool: &PgPool,
    query: CandidateClaimQuery,
) -> crate::Result<Option<ClaimedCandidateAttempt>> {
    if query.lease_owner.trim().is_empty() || query.lease_seconds <= 0 {
        return Err(conflict("invalid Candidate claim lease"));
    }
    let mut tx = pool.begin().await?;
    lock_v2_operation(&mut tx, query.operation_id).await?;
    lock_claim_scope(
        &mut tx,
        query.operation_id,
        query.scope_snapshot_id,
        query.wave_run_id,
        query.wave_unit_id,
        query.organization_id,
    )
    .await?;
    let lane = attack_execution_lanes::lock_global(&mut tx).await?;
    if !recover_expired_candidate_lane(&mut tx, &lane).await? {
        tx.rollback().await?;
        return Ok(None);
    }
    let verification_unit: Option<Uuid> = sqlx::query_scalar(
        r#"SELECT unit.id
             FROM stage_run_units unit
             JOIN stage_runs run
               ON run.id=unit.stage_execution_id AND run.operation_id=unit.operation_id
            WHERE unit.id=$1 AND unit.operation_id=$2 AND unit.stage_execution_id=$3
              AND unit.scope_snapshot_id=$4 AND unit.organization_id=$5
              AND unit.stage_kind='verification' AND unit.specialist='candidate_verifier'
              AND unit.status IN ('queued','running')
              AND run.stage_kind='verification' AND run.status='started'
            FOR UPDATE OF unit"#,
    )
    .bind(query.verification_stage_run_unit_id)
    .bind(query.operation_id)
    .bind(query.verification_stage_execution_id)
    .bind(query.scope_snapshot_id)
    .bind(query.organization_id)
    .fetch_optional(&mut *tx)
    .await?;
    if verification_unit.is_none() {
        return Err(conflict("verification StageRunUnit identity mismatch"));
    }
    let candidate = sqlx::query_as::<_, ClaimCandidateRow>(
        r#"SELECT candidate.candidate_id,approval.id AS approval_id,
                  candidate.target_live_id,candidate.target_type_at_time,
                  candidate.target_value_at_time,candidate.target_identity_hash,
                  candidate.candidate_plan_hash,approval.execution_plan,
                  approval.allowed_capability_ids,approval.allowed_action_kinds,approval.budget
             FROM attack_candidates candidate
             JOIN attack_candidate_approvals approval
               ON approval.candidate_id=candidate.candidate_id
              AND approval.operation_id=candidate.operation_uuid
              AND approval.scope_snapshot_id=candidate.scope_snapshot_id
              AND approval.wave_run_id=candidate.wave_run_id
              AND approval.wave_unit_id=candidate.wave_unit_id
              AND approval.organization_id=candidate.organization_id
              AND approval.target_identity_hash=candidate.target_identity_hash
              AND approval.candidate_plan_hash=candidate.candidate_plan_hash
            WHERE candidate.operation_uuid=$1 AND candidate.scope_snapshot_id=$2
              AND candidate.wave_run_id=$3 AND candidate.wave_unit_id=$4
              AND candidate.organization_id=$5 AND candidate.disposition='approved'
              AND approval.status='approved' AND approval.expires_at>NOW()
              AND NOT EXISTS(
                    SELECT 1 FROM candidate_attempts active
                    LEFT JOIN stage_worker_runs active_worker
                      ON active_worker.id=active.stage_worker_run_id
                     WHERE active.candidate_id=candidate.candidate_id
                       AND (
                         active.status='submitted'
                         OR (active.status='running' AND active_worker.status<>'queued')
                       ))
            ORDER BY CASE candidate.priority WHEN 'high' THEN 0 WHEN 'medium' THEN 1 ELSE 2 END,
                     candidate.created_at,candidate.candidate_id
            FOR UPDATE OF candidate,approval SKIP LOCKED
            LIMIT 1"#,
    )
    .bind(query.operation_id)
    .bind(query.scope_snapshot_id)
    .bind(query.wave_run_id)
    .bind(query.wave_unit_id)
    .bind(query.organization_id)
    .fetch_optional(&mut *tx)
    .await?;
    let Some(candidate) = candidate else {
        tx.rollback().await?;
        return Ok(None);
    };

    let select_attempt_sql = format!(
        "SELECT {ATTEMPT_COLUMNS} FROM candidate_attempts
         WHERE candidate_id=$1 AND operation_id=$2 AND scope_snapshot_id=$3
           AND wave_run_id=$4 AND wave_unit_id=$5 AND organization_id=$6
           AND status IN ('queued','running')
         ORDER BY ordinal DESC LIMIT 1 FOR UPDATE"
    );
    let existing = sqlx::query_as::<_, CandidateAttemptRow>(&select_attempt_sql)
        .bind(candidate.candidate_id)
        .bind(query.operation_id)
        .bind(query.scope_snapshot_id)
        .bind(query.wave_run_id)
        .bind(query.wave_unit_id)
        .bind(query.organization_id)
        .fetch_optional(&mut *tx)
        .await?;
    let mut attempt = if let Some(existing) = existing {
        if existing.approval_id != candidate.approval_id
            || existing.candidate_plan_hash != candidate.candidate_plan_hash
            || !matches!(existing.status.as_str(), "queued" | "running")
        {
            return Err(conflict(
                "live Candidate Attempt cannot be rebound to a new plan",
            ));
        }
        existing
    } else {
        let ordinal: i32 = sqlx::query_scalar(
            "SELECT COALESCE(MAX(ordinal),-1)+1 FROM candidate_attempts WHERE candidate_id=$1",
        )
        .bind(candidate.candidate_id)
        .fetch_one(&mut *tx)
        .await?;
        let sql = format!(
            "INSERT INTO candidate_attempts(
                 id,candidate_id,approval_id,operation_id,scope_snapshot_id,wave_run_id,
                 wave_unit_id,organization_id,target_live_id,target_type_at_time,
                 target_value_at_time,target_identity_hash,candidate_plan_hash,ordinal,status)
             VALUES($1,$2,$3,$4,$5,$6,$7,$8,$9,$10,$11,$12,$13,$14,'queued')
             RETURNING {ATTEMPT_COLUMNS}"
        );
        sqlx::query_as::<_, CandidateAttemptRow>(&sql)
            .bind(Uuid::new_v4())
            .bind(candidate.candidate_id)
            .bind(candidate.approval_id)
            .bind(query.operation_id)
            .bind(query.scope_snapshot_id)
            .bind(query.wave_run_id)
            .bind(query.wave_unit_id)
            .bind(query.organization_id)
            .bind(candidate.target_live_id)
            .bind(&candidate.target_type_at_time)
            .bind(&candidate.target_value_at_time)
            .bind(&candidate.target_identity_hash)
            .bind(&candidate.candidate_plan_hash)
            .bind(ordinal)
            .fetch_one(&mut *tx)
            .await?
    };

    let worker = if let Some(worker_id) = attempt.stage_worker_run_id {
        stage_worker_runs::get_with_executor(&mut *tx, worker_id)
            .await
            .map_err(runtime_error)?
            .ok_or_else(|| crate::DbError::NotFound("stage_worker_run".to_string()))?
    } else {
        let worker = stage_worker_runs::insert_with_executor(
            &mut *tx,
            &NewStageWorkerRun {
                id: Uuid::new_v4(),
                operation_id: query.operation_id,
                stage_execution_id: query.verification_stage_execution_id,
                stage_run_unit_id: query.verification_stage_run_unit_id,
                organization_id: query.organization_id,
                worker_generation: attempt.ordinal,
                specialist: "candidate_verifier".to_string(),
                work_item_kind: "candidate_attempt".to_string(),
                work_item_key: attempt.id.to_string(),
                agent_path: format!("main>candidate_verifier:{}", attempt.id),
                parent_request_id: None,
            },
        )
        .await
        .map_err(runtime_error)?;
        let update_sql = format!(
            "UPDATE candidate_attempts SET stage_worker_run_id=$2,row_version=row_version+1,
                 updated_at=NOW() WHERE id=$1 AND stage_worker_run_id IS NULL
             RETURNING {ATTEMPT_COLUMNS}"
        );
        attempt = sqlx::query_as::<_, CandidateAttemptRow>(&update_sql)
            .bind(attempt.id)
            .bind(worker.id)
            .fetch_one(&mut *tx)
            .await?;
        worker
    };
    if worker.status != "queued"
        || worker.stage_run_unit_id != query.verification_stage_run_unit_id
        || worker.work_item_kind != "candidate_attempt"
        || worker.work_item_key != attempt.id.to_string()
    {
        return Err(conflict("Candidate WorkerRun is not safely claimable"));
    }
    let lease_token = Uuid::new_v4();
    let mut worker = stage_worker_runs::claim_cas(
        &mut *tx,
        worker.id,
        query.verification_stage_run_unit_id,
        StageWorkerRunStatus::Queued,
        worker.attempt_epoch,
        lease_token,
        &query.lease_owner,
        query.lease_seconds,
    )
    .await
    .map_err(runtime_error)?;
    let session_id: Uuid = sqlx::query_scalar("SELECT session_id FROM tasks WHERE id=$1")
        .bind(query.operation_id)
        .fetch_optional(&mut *tx)
        .await?
        .ok_or_else(|| conflict("Candidate operation has no durable session"))?;
    match worker.message_chain_id {
        Some(chain_id) => {
            let exact: bool = sqlx::query_scalar(
                "SELECT EXISTS(
                     SELECT 1 FROM message_chains
                      WHERE id=$1 AND session_id=$2 AND task_id=$3 AND agent='pentester')",
            )
            .bind(chain_id)
            .bind(session_id)
            .bind(query.operation_id)
            .fetch_one(&mut *tx)
            .await?;
            if !exact {
                return Err(conflict("Candidate verifier chain identity mismatch"));
            }
        }
        None => {
            let chain_id = Uuid::new_v4();
            message_chains::create_bound_with_executor(
                &mut *tx,
                chain_id,
                session_id,
                query.operation_id,
                None,
                crate::models::AgentType::Pentester,
                None,
                None,
                &serde_json::json!([]),
            )
            .await?;
            worker = stage_worker_runs::bind_message_chain_cas(
                &mut *tx,
                worker.id,
                worker.stage_run_unit_id,
                lease_token,
                worker.attempt_epoch,
                chain_id,
            )
            .await
            .map_err(runtime_error)?;
        }
    }
    let lease_expires_at = worker
        .lease_expires_at
        .ok_or_else(|| conflict("claimed Candidate WorkerRun has no expiry"))?;
    attack_execution_lanes::claim_global(
        &mut tx,
        worker.id,
        lease_token,
        &query.lease_owner,
        lease_expires_at,
    )
    .await?;
    let update_sql = format!(
        "UPDATE candidate_attempts SET status='running',stage_worker_run_id=$2,
             row_version=row_version+1,updated_at=NOW()
         WHERE id=$1 AND status IN ('queued','running') RETURNING {ATTEMPT_COLUMNS}"
    );
    attempt = sqlx::query_as::<_, CandidateAttemptRow>(&update_sql)
        .bind(attempt.id)
        .bind(worker.id)
        .fetch_optional(&mut *tx)
        .await?
        .ok_or_else(|| conflict("Candidate Attempt claim CAS lost"))?;
    tx.commit().await?;
    Ok(Some(ClaimedCandidateAttempt {
        attempt,
        worker,
        execution_plan: candidate.execution_plan,
        allowed_capability_ids: candidate.allowed_capability_ids,
        allowed_action_kinds: candidate.allowed_action_kinds,
        budget: candidate.budget,
    }))
}

/// Extend WorkerRun and lane ownership in one transaction.
pub async fn heartbeat_candidate_execution(
    pool: &PgPool,
    heartbeat: CandidateExecutionHeartbeat,
) -> crate::Result<HeartbeatOutcome> {
    if heartbeat.extend_seconds <= 0 || heartbeat.lease_owner.trim().is_empty() {
        return Err(conflict("invalid Candidate heartbeat"));
    }
    let mut tx = pool.begin().await?;
    lock_v2_operation(&mut tx, heartbeat.operation_id).await?;
    lock_claim_scope(
        &mut tx,
        heartbeat.operation_id,
        heartbeat.scope_snapshot_id,
        heartbeat.wave_run_id,
        heartbeat.wave_unit_id,
        heartbeat.organization_id,
    )
    .await?;
    let lane = attack_execution_lanes::lock_global(&mut tx).await?;
    if lane.stage_worker_run_id != Some(heartbeat.worker_run_id)
        || lane.lease_token != Some(heartbeat.lease_token)
        || lane.lease_owner.as_deref() != Some(heartbeat.lease_owner.as_str())
        || lane
            .lease_expires_at
            .is_none_or(|expires| expires <= Utc::now())
    {
        return Err(conflict("Candidate lane heartbeat fence lost"));
    }
    let attempt_exists: Option<Uuid> = sqlx::query_scalar(
        "SELECT id FROM candidate_attempts
         WHERE id=$1 AND operation_id=$2 AND scope_snapshot_id=$3 AND wave_run_id=$4
           AND wave_unit_id=$5 AND organization_id=$6 AND stage_worker_run_id=$7
           AND status='running' FOR UPDATE",
    )
    .bind(heartbeat.attempt_id)
    .bind(heartbeat.operation_id)
    .bind(heartbeat.scope_snapshot_id)
    .bind(heartbeat.wave_run_id)
    .bind(heartbeat.wave_unit_id)
    .bind(heartbeat.organization_id)
    .bind(heartbeat.worker_run_id)
    .fetch_optional(&mut *tx)
    .await?;
    if attempt_exists.is_none() {
        return Err(conflict("Candidate Attempt heartbeat identity mismatch"));
    }
    let worker = stage_worker_runs::heartbeat_cas(
        &mut *tx,
        &RuntimeMemoryTxFence {
            operation_id: heartbeat.operation_id,
            stage_execution_id: heartbeat.stage_execution_id,
            stage_run_unit_id: heartbeat.stage_run_unit_id,
            worker_run_id: heartbeat.worker_run_id,
            lease_token: heartbeat.lease_token,
            attempt_epoch: heartbeat.attempt_epoch,
            expected_checkpoint_version: heartbeat.expected_checkpoint_version,
        },
        heartbeat.extend_seconds,
    )
    .await
    .map_err(runtime_error)?;
    let lease_expires_at = worker
        .lease_expires_at
        .ok_or_else(|| conflict("heartbeated Candidate WorkerRun has no expiry"))?;
    attack_execution_lanes::heartbeat_global(
        &mut tx,
        heartbeat.worker_run_id,
        heartbeat.lease_token,
        &heartbeat.lease_owner,
        lease_expires_at,
    )
    .await?;
    tx.commit().await?;
    Ok(HeartbeatOutcome {
        lease_expires_at,
        attempt_epoch: worker.attempt_epoch,
        checkpoint_version: worker.checkpoint_version,
    })
}

/// Reload and authorize one exact Candidate action, then durably mark it
/// `started` before the caller can perform any side effect. Every authority
/// field is joined from trusted context and current DB state; model JSON selects
/// only `action_ordinal`.
pub async fn begin_candidate_action(
    pool: &PgPool,
    command: BeginCandidateAction,
) -> crate::Result<CandidateActionStart> {
    if command.action_ordinal < 0
        || command.candidate_plan_hash.trim().is_empty()
        || command.workspace_path_sha256.trim().is_empty()
    {
        return Err(conflict("invalid Candidate action identity"));
    }
    let mut tx = pool.begin().await?;
    lock_v2_operation(&mut tx, command.operation_id).await?;
    let authority = sqlx::query_as::<_, CandidateActionAuthorityRow>(
        r#"SELECT attempt.target_live_id AS target_id,
                  attempt.target_identity_hash,
                  approval.execution_plan,
                  approval.allowed_capability_ids,
                  approval.allowed_action_kinds,
                  approval.budget AS approval_budget
             FROM candidate_attempts attempt
             JOIN attack_candidates candidate
               ON candidate.candidate_id=attempt.candidate_id
              AND candidate.operation_uuid=attempt.operation_id
              AND candidate.scope_snapshot_id=attempt.scope_snapshot_id
              AND candidate.wave_run_id=attempt.wave_run_id
              AND candidate.wave_unit_id=attempt.wave_unit_id
              AND candidate.organization_id=attempt.organization_id
              AND candidate.target_identity_hash=attempt.target_identity_hash
              AND candidate.candidate_plan_hash=attempt.candidate_plan_hash
             JOIN attack_candidate_approvals approval
               ON approval.id=attempt.approval_id
              AND approval.candidate_id=attempt.candidate_id
              AND approval.operation_id=attempt.operation_id
              AND approval.scope_snapshot_id=attempt.scope_snapshot_id
              AND approval.wave_run_id=attempt.wave_run_id
              AND approval.wave_unit_id=attempt.wave_unit_id
              AND approval.organization_id=attempt.organization_id
              AND approval.target_identity_hash=attempt.target_identity_hash
              AND approval.candidate_plan_hash=attempt.candidate_plan_hash
             JOIN operation_state operation
               ON operation.operation_id=attempt.operation_id
              AND operation.project_scope_id IS NOT NULL
             JOIN project_scopes project
               ON project.project_scope_id=operation.project_scope_id
              AND project.retired_at IS NULL AND project.path_sha256=$12
             JOIN operation_org_scope_snapshots snapshot
               ON snapshot.id=attempt.scope_snapshot_id
              AND snapshot.operation_id=attempt.operation_id
              AND snapshot.project_scope_id=operation.project_scope_id
              AND snapshot.sealed_at IS NOT NULL
             JOIN operation_org_scope_units scope_unit
               ON scope_unit.snapshot_id=attempt.scope_snapshot_id
              AND scope_unit.organization_id=attempt.organization_id
             JOIN stage_worker_runs worker
               ON worker.id=attempt.stage_worker_run_id
              AND worker.operation_id=attempt.operation_id
              AND worker.stage_execution_id=$2 AND worker.stage_run_unit_id=$3
              AND worker.organization_id=attempt.organization_id
              AND worker.lease_token=$6 AND worker.attempt_epoch=$7
              AND worker.status='running' AND worker.lease_expires_at>NOW()
             JOIN stage_run_units unit
               ON unit.id=worker.stage_run_unit_id
              AND unit.operation_id=worker.operation_id
              AND unit.stage_execution_id=worker.stage_execution_id
              AND unit.organization_id=worker.organization_id
              AND unit.stage_kind='verification'
              AND unit.specialist='candidate_verifier'
             JOIN attack_execution_lanes lane
               ON lane.lane_key='global:exploit'
              AND lane.stage_worker_run_id=worker.id
              AND lane.lease_token=worker.lease_token
              AND lane.lease_owner=worker.lease_owner
              AND lane.lease_expires_at>NOW()
            WHERE attempt.id=$10 AND attempt.candidate_id=$8
              AND attempt.approval_id=$9 AND attempt.operation_id=$1
              AND attempt.organization_id=$4 AND attempt.stage_worker_run_id=$5
              AND attempt.candidate_plan_hash=$11 AND attempt.status='running'
              AND candidate.disposition='approved'
              AND approval.status='approved' AND approval.expires_at>NOW()
            FOR UPDATE OF attempt,candidate,approval,worker,unit,lane,operation,project"#,
    )
    .bind(command.operation_id)
    .bind(command.stage_execution_id)
    .bind(command.stage_run_unit_id)
    .bind(command.organization_id)
    .bind(command.worker_run_id)
    .bind(command.lease_token)
    .bind(command.attempt_epoch)
    .bind(command.candidate_id)
    .bind(command.approval_id)
    .bind(command.attempt_id)
    .bind(&command.candidate_plan_hash)
    .bind(&command.workspace_path_sha256)
    .fetch_optional(&mut *tx)
    .await?
    .ok_or_else(|| conflict("Candidate action authorization fence lost"))?;

    if canonical_result_hash(&authority.execution_plan)? != command.candidate_plan_hash {
        return Err(conflict("Candidate execution plan hash drift"));
    }
    let plan = authority
        .execution_plan
        .as_object()
        .ok_or_else(|| conflict("Candidate execution plan must be an object"))?;
    let candidate_id_text = command.candidate_id.to_string();
    if plan.get("candidate_id").and_then(serde_json::Value::as_str)
        != Some(candidate_id_text.as_str())
        || plan
            .get("target_identity_hash")
            .and_then(serde_json::Value::as_str)
            != Some(authority.target_identity_hash.as_str())
        || plan
            .get("foreground_only")
            .and_then(serde_json::Value::as_bool)
            != Some(true)
        || plan.get("budget") != Some(&authority.approval_budget)
    {
        return Err(conflict(
            "Candidate plan identity/budget/foreground fence lost",
        ));
    }
    let budget = authority
        .approval_budget
        .as_object()
        .ok_or_else(|| conflict("Candidate budget must be an object"))?;
    let max_actions = budget
        .get("max_actions")
        .and_then(serde_json::Value::as_u64)
        .ok_or_else(|| conflict("Candidate max_actions is invalid"))?;
    let max_requests = budget
        .get("max_requests")
        .and_then(serde_json::Value::as_u64)
        .ok_or_else(|| conflict("Candidate max_requests is invalid"))?;
    let max_runtime_ms = budget
        .get("max_runtime_ms")
        .and_then(serde_json::Value::as_u64)
        .ok_or_else(|| conflict("Candidate max_runtime_ms is invalid"))?;
    let actions = plan
        .get("actions")
        .and_then(serde_json::Value::as_array)
        .ok_or_else(|| conflict("Candidate plan actions are invalid"))?;
    if max_actions == 0
        || max_requests == 0
        || max_runtime_ms == 0
        || actions.len() as u64 > max_actions
        || command.action_ordinal as u64 >= max_actions
    {
        return Err(conflict("Candidate action exceeds approved budget"));
    }

    let mut seen_ordinals = BTreeSet::new();
    for planned in actions {
        let planned = planned
            .as_object()
            .ok_or_else(|| conflict("Candidate planned action must be an object"))?;
        let ordinal = planned
            .get("ordinal")
            .and_then(serde_json::Value::as_u64)
            .and_then(|value| i32::try_from(value).ok())
            .ok_or_else(|| conflict("Candidate action ordinal is invalid"))?;
        let capability_id = planned
            .get("capability_id")
            .and_then(serde_json::Value::as_str)
            .ok_or_else(|| conflict("Candidate capability is invalid"))?;
        let action_kind = planned
            .get("action_kind")
            .and_then(serde_json::Value::as_str)
            .ok_or_else(|| conflict("Candidate action kind is invalid"))?;
        let canonical_args = planned
            .get("canonical_args")
            .filter(|value| value.is_object())
            .ok_or_else(|| conflict("Candidate canonical args are invalid"))?;
        if !seen_ordinals.insert(ordinal)
            || !authority
                .allowed_capability_ids
                .iter()
                .any(|allowed| allowed == capability_id)
            || !authority
                .allowed_action_kinds
                .iter()
                .any(|allowed| allowed == action_kind)
            || canonical_args
                .get("background")
                .and_then(serde_json::Value::as_bool)
                != Some(false)
        {
            return Err(conflict(
                "Candidate capability/action/args authorization fence lost",
            ));
        }
        sqlx::query(
            "INSERT INTO candidate_attempt_actions(
                 attempt_id,action_ordinal,capability_id,action_kind,canonical_args,status)
             VALUES($1,$2,$3,$4,$5,'planned')
             ON CONFLICT(attempt_id,action_ordinal) DO NOTHING",
        )
        .bind(command.attempt_id)
        .bind(ordinal)
        .bind(capability_id)
        .bind(action_kind)
        .bind(canonical_args)
        .execute(&mut *tx)
        .await?;
    }

    let planned = actions
        .iter()
        .find(|action| {
            action.get("ordinal").and_then(serde_json::Value::as_i64)
                == Some(i64::from(command.action_ordinal))
        })
        .ok_or_else(|| conflict("Candidate action ordinal is not in the approved plan"))?;
    let journal = sqlx::query_as::<_, CandidateActionJournalRow>(
        "SELECT id,action_ordinal,capability_id,action_kind,canonical_args,status,outcome
         FROM candidate_attempt_actions
         WHERE attempt_id=$1 AND action_ordinal=$2 FOR UPDATE",
    )
    .bind(command.attempt_id)
    .bind(command.action_ordinal)
    .fetch_one(&mut *tx)
    .await?;
    if journal.capability_id
        != planned
            .get("capability_id")
            .and_then(serde_json::Value::as_str)
            .unwrap_or_default()
        || journal.action_kind
            != planned
                .get("action_kind")
                .and_then(serde_json::Value::as_str)
                .unwrap_or_default()
        || Some(&journal.canonical_args) != planned.get("canonical_args")
    {
        return Err(conflict("Candidate action journal drift"));
    }

    let result = match journal.status.as_str() {
        "planned" => {
            let started = sqlx::query_as::<_, CandidateActionJournalRow>(
                "UPDATE candidate_attempt_actions
                 SET status='started',started_at=NOW(),updated_at=NOW()
                 WHERE id=$1 AND status='planned'
                 RETURNING id,action_ordinal,capability_id,action_kind,canonical_args,status,outcome",
            )
            .bind(journal.id)
            .fetch_optional(&mut *tx)
            .await?
            .ok_or_else(|| conflict("Candidate action start CAS lost"))?;
            CandidateActionStart::Authorized(AuthorizedCandidateAction {
                action_id: started.id,
                action_ordinal: started.action_ordinal,
                target_id: authority
                    .target_id
                    .ok_or_else(|| conflict("Candidate action has no live target binding"))?,
                capability_id: started.capability_id,
                action_kind: started.action_kind,
                canonical_args: started.canonical_args,
                budget: authority.approval_budget,
            })
        }
        "started" => {
            sqlx::query(
                "UPDATE candidate_attempt_actions
                 SET status='outcome_unknown',error_code='started_without_terminal_outcome',
                     completed_at=NOW(),updated_at=NOW()
                 WHERE id=$1 AND status='started'",
            )
            .bind(journal.id)
            .execute(&mut *tx)
            .await?;
            CandidateActionStart::OutcomeUnknown {
                action_id: journal.id,
            }
        }
        "outcome_unknown" => CandidateActionStart::OutcomeUnknown {
            action_id: journal.id,
        },
        "completed" | "failed" => CandidateActionStart::ExistingTerminal {
            action_id: journal.id,
            status: journal.status,
            outcome: journal.outcome.unwrap_or(serde_json::Value::Null),
        },
        _ => return Err(conflict("unknown Candidate action journal status")),
    };
    tx.commit().await?;
    Ok(result)
}

/// Finish the exact started journal row under the same current WorkerRun/lane
/// fence. The action result is evidence input only; Attempt submission and all
/// Finding terminalization remain Task 8.
pub async fn finish_candidate_action(
    pool: &PgPool,
    command: FinishCandidateAction,
) -> crate::Result<()> {
    if !command.outcome.is_object() {
        return Err(conflict("Candidate action outcome must be an object"));
    }
    let mut tx = pool.begin().await?;
    lock_v2_operation(&mut tx, command.operation_id).await?;
    let fence: Option<Uuid> = sqlx::query_scalar(
        r#"SELECT action.id
             FROM candidate_attempt_actions action
             JOIN candidate_attempts attempt ON attempt.id=action.attempt_id
             JOIN stage_worker_runs worker
               ON worker.id=attempt.stage_worker_run_id
              AND worker.operation_id=attempt.operation_id
              AND worker.stage_execution_id=$2 AND worker.stage_run_unit_id=$3
              AND worker.organization_id=attempt.organization_id
              AND worker.lease_token=$6 AND worker.attempt_epoch=$7
              AND worker.status='running' AND worker.lease_expires_at>NOW()
             JOIN attack_execution_lanes lane
               ON lane.lane_key='global:exploit'
              AND lane.stage_worker_run_id=worker.id
              AND lane.lease_token=worker.lease_token
              AND lane.lease_owner=worker.lease_owner
              AND lane.lease_expires_at>NOW()
            WHERE action.id=$12 AND action.attempt_id=$10 AND action.status='started'
              AND attempt.candidate_id=$8 AND attempt.approval_id=$9
              AND attempt.operation_id=$1 AND attempt.organization_id=$4
              AND attempt.stage_worker_run_id=$5 AND attempt.candidate_plan_hash=$11
              AND attempt.status='running'
            FOR UPDATE OF action,attempt,worker,lane"#,
    )
    .bind(command.operation_id)
    .bind(command.stage_execution_id)
    .bind(command.stage_run_unit_id)
    .bind(command.organization_id)
    .bind(command.worker_run_id)
    .bind(command.lease_token)
    .bind(command.attempt_epoch)
    .bind(command.candidate_id)
    .bind(command.approval_id)
    .bind(command.attempt_id)
    .bind(&command.candidate_plan_hash)
    .bind(command.action_id)
    .fetch_optional(&mut *tx)
    .await?;
    if fence.is_none() {
        return Err(conflict("Candidate action completion fence lost"));
    }
    let outcome_hash = canonical_result_hash(&command.outcome)?;
    let next_status = if command.success {
        "completed"
    } else {
        "failed"
    };
    let updated = sqlx::query(
        "UPDATE candidate_attempt_actions
         SET status=$2,outcome=$3,outcome_hash=$4,error_code=$5,
             completed_at=NOW(),updated_at=NOW()
         WHERE id=$1 AND status='started'",
    )
    .bind(command.action_id)
    .bind(next_status)
    .bind(&command.outcome)
    .bind(outcome_hash)
    .bind(command.error_code)
    .execute(&mut *tx)
    .await?;
    if updated.rows_affected() != 1 {
        return Err(conflict("Candidate action completion CAS lost"));
    }
    tx.commit().await?;
    Ok(())
}

/// End a failed verifier lease without rewriting history. The running Attempt
/// becomes a terminal `retryable_failed` row and its WorkerRun becomes failed;
/// a later claim creates a new Attempt/Worker ordinal. The lane clear and both
/// terminal writes share one transaction.
pub async fn release_candidate_execution(
    pool: &PgPool,
    release: CandidateExecutionRelease,
) -> crate::Result<ReleaseOutcome> {
    let mut tx = pool.begin().await?;
    lock_v2_operation(&mut tx, release.operation_id).await?;
    lock_claim_scope(
        &mut tx,
        release.operation_id,
        release.scope_snapshot_id,
        release.wave_run_id,
        release.wave_unit_id,
        release.organization_id,
    )
    .await?;
    let lane = attack_execution_lanes::lock_global(&mut tx).await?;
    if lane.stage_worker_run_id != Some(release.worker_run_id)
        || lane.lease_token != Some(release.lease_token)
        || lane.lease_owner.as_deref() != Some(release.lease_owner.as_str())
    {
        return Err(conflict("Candidate lane release fence lost"));
    }
    let attempt_exists: Option<Uuid> = sqlx::query_scalar(
        "SELECT id FROM candidate_attempts
         WHERE id=$1 AND operation_id=$2 AND scope_snapshot_id=$3 AND wave_run_id=$4
           AND wave_unit_id=$5 AND organization_id=$6 AND stage_worker_run_id=$7
           AND status='running' FOR UPDATE",
    )
    .bind(release.attempt_id)
    .bind(release.operation_id)
    .bind(release.scope_snapshot_id)
    .bind(release.wave_run_id)
    .bind(release.wave_unit_id)
    .bind(release.organization_id)
    .bind(release.worker_run_id)
    .fetch_optional(&mut *tx)
    .await?;
    if attempt_exists.is_none() {
        return Err(conflict("Candidate Attempt release identity mismatch"));
    }
    attack_execution_lanes::release_global(
        &mut tx,
        release.worker_run_id,
        release.lease_token,
        &release.lease_owner,
    )
    .await?;
    let released_worker = sqlx::query(
        "UPDATE stage_worker_runs
         SET status='failed',lease_token=NULL,lease_owner=NULL,lease_acquired_at=NULL,
             lease_expires_at=NULL,heartbeat_at=NULL,terminal_at=NOW(),updated_at=NOW()
         WHERE id=$1 AND operation_id=$2 AND stage_execution_id=$3
           AND stage_run_unit_id=$4 AND lease_token=$5 AND attempt_epoch=$6
           AND checkpoint_version=$7 AND status='running' AND active_tool_call_id IS NULL",
    )
    .bind(release.worker_run_id)
    .bind(release.operation_id)
    .bind(release.stage_execution_id)
    .bind(release.stage_run_unit_id)
    .bind(release.lease_token)
    .bind(release.attempt_epoch)
    .bind(release.expected_checkpoint_version)
    .execute(&mut *tx)
    .await?;
    if released_worker.rows_affected() != 1 {
        return Err(conflict("Candidate WorkerRun release CAS lost"));
    }
    let result_json = serde_json::json!({
        "disposition": "retryable_failed",
        "reason_code": "worker_released_for_retry",
        "schema_version": 1,
    });
    let result_hash = canonical_result_hash(&result_json)?;
    let released_attempt = sqlx::query(
        "UPDATE candidate_attempts
         SET status='retryable_failed',result_json=$3,result_hash=$4,terminal_at=NOW(),
             row_version=row_version+1,updated_at=NOW()
         WHERE id=$1 AND status='running' AND stage_worker_run_id=$2
           AND result_json IS NULL AND result_hash IS NULL AND terminal_at IS NULL",
    )
    .bind(release.attempt_id)
    .bind(release.worker_run_id)
    .bind(&result_json)
    .bind(&result_hash)
    .execute(&mut *tx)
    .await?;
    if released_attempt.rows_affected() != 1 {
        return Err(conflict("Candidate Attempt release CAS lost"));
    }
    tx.commit().await?;
    Ok(ReleaseOutcome { requeued: false })
}

/// Persist a validator-approved terminal submission without releasing the
/// WorkerRun/lane. Finding terminalization performs the final atomic release.
pub async fn record_attempt_submission(
    tx: &mut Transaction<'_, Postgres>,
    command: RecordAttemptSubmission,
) -> crate::Result<RecordedAttemptSubmission> {
    if !command.result_json.is_object()
        || command.candidate_plan_hash.trim().is_empty()
        || command.lease_owner.trim().is_empty()
    {
        return Err(conflict("invalid Candidate Attempt submission"));
    }
    let mut evidence_keys = BTreeSet::new();
    if command.evidence.iter().any(|link| {
        link.evidence_id <= 0
            || link.role.trim().is_empty()
            || !evidence_keys.insert((link.evidence_id, link.role.clone()))
    }) {
        return Err(conflict("invalid or duplicate Candidate Attempt evidence"));
    }
    lock_v2_operation(tx, command.operation_id).await?;
    lock_claim_scope(
        tx,
        command.operation_id,
        command.scope_snapshot_id,
        command.wave_run_id,
        command.wave_unit_id,
        command.organization_id,
    )
    .await?;
    let lane = attack_execution_lanes::lock_global(tx).await?;
    if lane.stage_worker_run_id != Some(command.worker_run_id)
        || lane.lease_token != Some(command.lease_token)
        || lane.lease_owner.as_deref() != Some(command.lease_owner.as_str())
        || lane
            .lease_expires_at
            .is_none_or(|expires| expires <= Utc::now())
    {
        return Err(conflict("Candidate submission lane fence lost"));
    }
    let candidate_execution_plan: Option<serde_json::Value> = sqlx::query_scalar(
        r#"SELECT candidate.execution_plan
             FROM attack_candidates candidate
             JOIN attack_candidate_approvals approval
               ON approval.id=$2 AND approval.candidate_id=candidate.candidate_id
              AND approval.operation_id=candidate.operation_uuid
              AND approval.scope_snapshot_id=candidate.scope_snapshot_id
              AND approval.wave_run_id=candidate.wave_run_id
              AND approval.wave_unit_id=candidate.wave_unit_id
              AND approval.organization_id=candidate.organization_id
              AND approval.candidate_plan_hash=candidate.candidate_plan_hash
            WHERE candidate.candidate_id=$1 AND candidate.operation_uuid=$3
              AND candidate.scope_snapshot_id=$4 AND candidate.wave_run_id=$5
              AND candidate.wave_unit_id=$6 AND candidate.organization_id=$7
              AND candidate.candidate_plan_hash=$8 AND candidate.disposition='approved'
              AND approval.status='approved' AND approval.expires_at>NOW()
            FOR UPDATE OF candidate,approval"#,
    )
    .bind(command.candidate_id)
    .bind(command.approval_id)
    .bind(command.operation_id)
    .bind(command.scope_snapshot_id)
    .bind(command.wave_run_id)
    .bind(command.wave_unit_id)
    .bind(command.organization_id)
    .bind(&command.candidate_plan_hash)
    .fetch_optional(&mut **tx)
    .await?;
    let candidate_execution_plan = candidate_execution_plan
        .ok_or_else(|| conflict("Candidate submission approval/plan fence lost"))?;
    let attempt_sql = format!(
        "SELECT {ATTEMPT_COLUMNS} FROM candidate_attempts
         WHERE id=$1 AND candidate_id=$2 AND approval_id=$3 AND operation_id=$4
           AND scope_snapshot_id=$5 AND wave_run_id=$6 AND wave_unit_id=$7
           AND organization_id=$8 AND candidate_plan_hash=$9
           AND stage_worker_run_id=$10 FOR UPDATE"
    );
    let attempt = sqlx::query_as::<_, CandidateAttemptRow>(&attempt_sql)
        .bind(command.attempt_id)
        .bind(command.candidate_id)
        .bind(command.approval_id)
        .bind(command.operation_id)
        .bind(command.scope_snapshot_id)
        .bind(command.wave_run_id)
        .bind(command.wave_unit_id)
        .bind(command.organization_id)
        .bind(&command.candidate_plan_hash)
        .bind(command.worker_run_id)
        .fetch_optional(&mut **tx)
        .await?
        .ok_or_else(|| crate::DbError::NotFound("candidate_attempt".to_string()))?;
    validate_terminal_action_journal(tx, command.attempt_id, &candidate_execution_plan).await?;
    let worker_fence: Option<Uuid> = sqlx::query_scalar(
        "SELECT id FROM stage_worker_runs
         WHERE id=$1 AND operation_id=$2 AND stage_execution_id=$3
           AND stage_run_unit_id=$4 AND organization_id=$5 AND lease_token=$6
           AND lease_owner=$7 AND attempt_epoch=$8 AND checkpoint_version=$9
           AND status='running' AND lease_expires_at>NOW() FOR UPDATE",
    )
    .bind(command.worker_run_id)
    .bind(command.operation_id)
    .bind(command.stage_execution_id)
    .bind(command.stage_run_unit_id)
    .bind(command.organization_id)
    .bind(command.lease_token)
    .bind(&command.lease_owner)
    .bind(command.attempt_epoch)
    .bind(command.expected_checkpoint_version)
    .fetch_optional(&mut **tx)
    .await?;
    if worker_fence.is_none() {
        return Err(conflict("Candidate submission WorkerRun fence lost"));
    }
    let result_hash = canonical_result_hash(&command.result_json)?;
    if attempt.status == "submitted" {
        let actual_evidence: Vec<(i64, String)> = sqlx::query_as(
            "SELECT evidence_id,role FROM candidate_attempt_evidence
             WHERE attempt_id=$1 ORDER BY evidence_id,role",
        )
        .bind(command.attempt_id)
        .fetch_all(&mut **tx)
        .await?;
        let expected_evidence = evidence_keys.into_iter().collect::<Vec<_>>();
        if attempt.result_json.as_ref() != Some(&command.result_json)
            || attempt.result_hash.as_deref() != Some(result_hash.as_str())
            || actual_evidence != expected_evidence
        {
            return Err(conflict("Candidate Attempt submission replay drift"));
        }
        return Ok(RecordedAttemptSubmission {
            attempt,
            replayed: true,
        });
    }
    if attempt.status != "running" || attempt.result_json.is_some() || attempt.result_hash.is_some()
    {
        return Err(conflict("Candidate Attempt is not submit-ready"));
    }
    for link in &command.evidence {
        sqlx::query(
            "INSERT INTO candidate_attempt_evidence(attempt_id,evidence_id,role)
             VALUES($1,$2,$3)",
        )
        .bind(command.attempt_id)
        .bind(link.evidence_id)
        .bind(&link.role)
        .execute(&mut **tx)
        .await?;
    }
    let update_sql = format!(
        "UPDATE candidate_attempts SET status='submitted',result_json=$2,result_hash=$3,
             row_version=row_version+1,updated_at=NOW()
         WHERE id=$1 AND status='running' RETURNING {ATTEMPT_COLUMNS}"
    );
    let attempt = sqlx::query_as::<_, CandidateAttemptRow>(&update_sql)
        .bind(command.attempt_id)
        .bind(&command.result_json)
        .bind(&result_hash)
        .fetch_optional(&mut **tx)
        .await?
        .ok_or_else(|| conflict("Candidate Attempt submission CAS lost"))?;
    Ok(RecordedAttemptSubmission {
        attempt,
        replayed: false,
    })
}
