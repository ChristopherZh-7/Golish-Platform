//! Automatic admission and durable HypothesisVerificationTask authority.
//!
//! A task is an orchestration aggregate, never a security verdict.  Stable
//! identity excludes admission generation, timestamps, and the wrapper
//! evidence-snapshot UUID.  Objective assignments are immutable denominators;
//! only campaign assignments receive terminal outcome members.

use std::collections::{BTreeMap, BTreeSet};

use golish_core::hypothesis_verification_task::{
    HypothesisVerificationTaskHeaderV1, HypothesisVerificationTaskStateV1,
    NewHypothesisVerificationTaskV1, TaskObjectiveResidualKindV1,
    VerificationAdmissionDispositionV1,
};
use serde::Serialize;
use sha2::{Digest, Sha256};
use sqlx::{PgPool, Postgres, Transaction};
use uuid::Uuid;

#[derive(Debug, thiserror::Error)]
pub enum HypothesisVerificationTaskStoreError {
    #[error(transparent)]
    Sqlx(#[from] sqlx::Error),
    #[error("invalid hypothesis verification task input: {0}")]
    InvalidInput(&'static str),
    #[error("hypothesis verification task identity conflict: {0}")]
    IdentityConflict(&'static str),
    #[error("hypothesis verification task CAS conflict: {0}")]
    CasConflict(&'static str),
}

pub type HypothesisVerificationTaskStoreResult<T> = Result<T, HypothesisVerificationTaskStoreError>;

#[derive(Debug, Clone, sqlx::FromRow, PartialEq, Eq)]
pub struct HypothesisVerificationTaskRow {
    pub task_id: Uuid,
    pub stable_task_key_sha256: String,
    pub operation_id: Uuid,
    pub project_scope_id: Uuid,
    pub stage_execution_id: Uuid,
    pub stage_run_unit_id: Uuid,
    pub scope_snapshot_id: Uuid,
    pub organization_id: Uuid,
    pub hypothesis_revision_id: Uuid,
    pub hypothesis_revision_sha256: String,
    pub verification_plan_id: Uuid,
    pub verification_plan_sha256: String,
    pub relevant_evidence_snapshot_id: Uuid,
    pub semantic_evidence_set_sha256: String,
    pub open_obligation_set_sha256: String,
    pub semantic_attempt_fingerprint: String,
    pub task_contract_version: String,
    pub first_admission_generation_id: Uuid,
    pub host_rerun_receipt_id: Option<Uuid>,
    pub host_rerun_receipt_sha256: Option<String>,
    pub rerun_contract_version: Option<i32>,
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct CreateTaskReceipt {
    pub task: HypothesisVerificationTaskRow,
    pub replayed: bool,
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct HostRerunReceiptInput {
    pub rerun_receipt_id: Uuid,
    pub operation_id: Uuid,
    pub stage_execution_id: Uuid,
    pub stage_run_unit_id: Uuid,
    pub scope_snapshot_id: Uuid,
    pub organization_id: Uuid,
    pub hypothesis_revision_id: Uuid,
    pub rerun_contract_version: i32,
    pub reason_code: String,
    pub authority_receipt_sha256: String,
    pub rerun_receipt_sha256: String,
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct AdmissionMemberInput {
    pub admission_member_id: Uuid,
    pub generation_member_id: Uuid,
    pub disposition: VerificationAdmissionDispositionV1,
    pub reason_code: String,
    pub semantic_attempt_fingerprint: String,
    pub task_id: Option<Uuid>,
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct SealAdmissionSetInput {
    pub admission_set_id: Uuid,
    pub stable_request_id: Uuid,
    pub operation_id: Uuid,
    pub stage_execution_id: Uuid,
    pub stage_run_unit_id: Uuid,
    pub scope_snapshot_id: Uuid,
    pub organization_id: Uuid,
    pub generation_id: Uuid,
    pub members: Vec<AdmissionMemberInput>,
}

#[derive(Debug, Clone, sqlx::FromRow, PartialEq, Eq)]
pub struct AdmissionSetRow {
    pub admission_set_id: Uuid,
    pub generation_id: Uuid,
    pub status: String,
    pub member_count: Option<i64>,
    pub member_set_sha256: Option<String>,
    pub row_version: i64,
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub enum ObjectiveAssignmentInput {
    Campaign {
        campaign_id: Uuid,
    },
    AlreadySatisfied {
        objective_outcome_receipt_id: Uuid,
        outcome_receipt_sha256: String,
        semantic_evidence_set_sha256: String,
    },
    Residual {
        residual_kind: TaskObjectiveResidualKindV1,
        reason_code: String,
        owner: String,
        next_action: String,
        residual_receipt_id: Uuid,
        residual_receipt_sha256: String,
    },
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct ObjectiveAssignmentMemberInput {
    pub assignment_member_id: Uuid,
    pub plan_objective_id: Uuid,
    pub assignment: ObjectiveAssignmentInput,
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct SealObjectiveAssignmentsInput {
    pub assignment_set_id: Uuid,
    pub stable_request_id: Uuid,
    pub task_id: Uuid,
    pub members: Vec<ObjectiveAssignmentMemberInput>,
}

#[derive(Debug, Clone, sqlx::FromRow, PartialEq, Eq)]
pub struct ObjectiveAssignmentSetRow {
    pub assignment_set_id: Uuid,
    pub task_id: Uuid,
    pub status: String,
    pub member_count: Option<i64>,
    pub member_set_sha256: Option<String>,
    pub row_version: i64,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum CampaignOutcomeKind {
    Completed,
    Blocked,
    CancelledBeforeStart,
    RecoveryRequired,
}

impl CampaignOutcomeKind {
    const fn as_str(self) -> &'static str {
        match self {
            Self::Completed => "completed",
            Self::Blocked => "blocked",
            Self::CancelledBeforeStart => "cancelled_before_start",
            Self::RecoveryRequired => "recovery_required",
        }
    }
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct CampaignOutcomeInput {
    pub outcome_member_id: Uuid,
    pub campaign_id: Uuid,
    pub outcome_kind: CampaignOutcomeKind,
    pub terminal_receipt_id: Uuid,
    pub terminal_receipt_sha256: String,
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct SealCampaignOutcomesInput {
    pub outcome_set_id: Uuid,
    pub stable_request_id: Uuid,
    pub assignment_set_id: Uuid,
    pub task_id: Uuid,
    pub outcomes: Vec<CampaignOutcomeInput>,
}

#[derive(Debug, Clone, sqlx::FromRow, PartialEq, Eq)]
pub struct CampaignOutcomeSetRow {
    pub outcome_set_id: Uuid,
    pub task_id: Uuid,
    pub status: String,
    pub member_count: Option<i64>,
    pub member_set_sha256: Option<String>,
    pub row_version: i64,
}

#[derive(Debug, Clone, sqlx::FromRow, PartialEq, Eq)]
pub struct TaskStateHeadRow {
    pub task_id: Uuid,
    pub current_state: String,
    pub latest_event_id: Uuid,
    pub head_version: i64,
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct FinalizeTaskFromCampaignTruthInput {
    pub task_id: Uuid,
    pub operation_id: Uuid,
    pub stage_execution_id: Uuid,
    pub stage_run_unit_id: Uuid,
    pub scope_snapshot_id: Uuid,
    pub organization_id: Uuid,
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct AdvanceTaskToRunningInput {
    pub task_id: Uuid,
    pub operation_id: Uuid,
    pub stage_execution_id: Uuid,
    pub stage_run_unit_id: Uuid,
    pub scope_snapshot_id: Uuid,
    pub organization_id: Uuid,
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct FinalizedTaskFromCampaignTruth {
    pub task_id: Uuid,
    pub outcome_set_id: Uuid,
    pub outcome_member_count: i64,
    pub outcome_member_set_sha256: String,
    pub terminal_state: String,
    pub head_version: i64,
    pub replayed: bool,
}

#[derive(Debug, Serialize)]
struct AdmissionMemberHashMaterial<'a> {
    admission_set_id: Uuid,
    generation_member_id: Uuid,
    hypothesis_revision_id: Uuid,
    disposition: &'a str,
    reason_code: &'a str,
    semantic_attempt_fingerprint: &'a str,
    task_id: Option<Uuid>,
}

#[derive(Debug, Serialize)]
struct AssignmentMemberHashMaterial<'a> {
    assignment_set_id: Uuid,
    task_id: Uuid,
    plan_objective_id: Uuid,
    verification_objective_id: Uuid,
    assignment_kind: &'a str,
    authority: serde_json::Value,
}

#[derive(Debug, Serialize)]
struct OutcomeMemberHashMaterial<'a> {
    outcome_set_id: Uuid,
    task_id: Uuid,
    campaign_id: Uuid,
    outcome_kind: &'a str,
    terminal_receipt_id: Uuid,
    terminal_receipt_sha256: &'a str,
}

pub async fn insert_host_rerun_receipt(
    pool: &PgPool,
    input: &HostRerunReceiptInput,
) -> HypothesisVerificationTaskStoreResult<()> {
    validate_ids(&[
        input.rerun_receipt_id,
        input.operation_id,
        input.stage_execution_id,
        input.stage_run_unit_id,
        input.scope_snapshot_id,
        input.organization_id,
        input.hypothesis_revision_id,
    ])?;
    if input.rerun_contract_version <= 0 || input.reason_code.trim().is_empty() {
        return Err(HypothesisVerificationTaskStoreError::InvalidInput(
            "rerun_receipt",
        ));
    }
    validate_sha256(&input.authority_receipt_sha256)?;
    validate_sha256(&input.rerun_receipt_sha256)?;
    sqlx::query(
        r#"INSERT INTO hypothesis_verification_rerun_receipts(
               rerun_receipt_id,operation_id,stage_execution_id,stage_run_unit_id,
               scope_snapshot_id,organization_id,hypothesis_revision_id,
               rerun_contract_version,reason_code,authority_receipt_sha256,
               rerun_receipt_sha256
           ) VALUES($1,$2,$3,$4,$5,$6,$7,$8,$9,$10,$11)
           ON CONFLICT(rerun_receipt_id) DO NOTHING"#,
    )
    .bind(input.rerun_receipt_id)
    .bind(input.operation_id)
    .bind(input.stage_execution_id)
    .bind(input.stage_run_unit_id)
    .bind(input.scope_snapshot_id)
    .bind(input.organization_id)
    .bind(input.hypothesis_revision_id)
    .bind(input.rerun_contract_version)
    .bind(&input.reason_code)
    .bind(&input.authority_receipt_sha256)
    .bind(&input.rerun_receipt_sha256)
    .execute(pool)
    .await?;
    let stored: (Uuid, i32, String, String, String) = sqlx::query_as(
        r#"SELECT hypothesis_revision_id,rerun_contract_version,reason_code,
                  authority_receipt_sha256,rerun_receipt_sha256
             FROM hypothesis_verification_rerun_receipts WHERE rerun_receipt_id=$1"#,
    )
    .bind(input.rerun_receipt_id)
    .fetch_one(pool)
    .await?;
    if stored
        != (
            input.hypothesis_revision_id,
            input.rerun_contract_version,
            input.reason_code.clone(),
            input.authority_receipt_sha256.clone(),
            input.rerun_receipt_sha256.clone(),
        )
    {
        return Err(HypothesisVerificationTaskStoreError::IdentityConflict(
            "rerun_receipt_replay_mismatch",
        ));
    }
    Ok(())
}

pub async fn create_or_replay_task(
    pool: &PgPool,
    header: &HypothesisVerificationTaskHeaderV1,
) -> HypothesisVerificationTaskStoreResult<CreateTaskReceipt> {
    validate_task_header(header)?;
    let (project_scope_id, verification_plan_id): (Uuid, Uuid) = sqlx::query_as(
        r#"SELECT operation.project_scope_id,plan.plan_id
             FROM operation_state operation
             JOIN attack_hypothesis_verification_plans plan
               ON plan.revision_id=$2 AND plan.plan_hash=$3
            WHERE operation.operation_id=$1 AND operation.project_scope_id IS NOT NULL"#,
    )
    .bind(header.operation_id)
    .bind(header.hypothesis_revision_id)
    .bind(&header.verification_plan_sha256)
    .fetch_one(pool)
    .await?;

    let mut tx = pool.begin().await?;
    let inserted = sqlx::query(
        r#"INSERT INTO hypothesis_verification_tasks(
               task_id,stable_task_key_sha256,operation_id,project_scope_id,
               stage_execution_id,stage_run_unit_id,scope_snapshot_id,organization_id,
               hypothesis_revision_id,hypothesis_revision_sha256,verification_plan_id,
               verification_plan_sha256,relevant_evidence_snapshot_id,
               semantic_evidence_set_sha256,open_obligation_set_sha256,
               semantic_attempt_fingerprint,task_contract_version,
               first_admission_generation_id,host_rerun_receipt_id,
               host_rerun_receipt_sha256,rerun_contract_version
           ) VALUES($1,$2,$3,$4,$5,$6,$7,$8,$9,$10,$11,$12,$13,$14,$15,$16,$17,$18,$19,$20,$21)
           ON CONFLICT(stable_task_key_sha256) DO NOTHING"#,
    )
    .bind(header.task_id)
    .bind(&header.stable_task_key_sha256)
    .bind(header.operation_id)
    .bind(project_scope_id)
    .bind(header.stage_execution_id)
    .bind(header.stage_run_unit_id)
    .bind(header.scope_snapshot_id)
    .bind(header.organization_id)
    .bind(header.hypothesis_revision_id)
    .bind(&header.hypothesis_revision_sha256)
    .bind(verification_plan_id)
    .bind(&header.verification_plan_sha256)
    .bind(header.relevant_evidence_snapshot_id)
    .bind(&header.semantic_evidence_set_sha256)
    .bind(&header.open_obligation_set_sha256)
    .bind(&header.semantic_attempt_fingerprint)
    .bind(&header.task_contract_version)
    .bind(header.first_admission_generation_id)
    .bind(header.host_rerun_receipt_id)
    .bind(&header.host_rerun_receipt_sha256)
    .bind(header.rerun_contract_version.map(|value| value as i32))
    .execute(&mut *tx)
    .await?
    .rows_affected()
        == 1;
    if inserted {
        append_state_event_on(
            &mut tx,
            header.task_id,
            Uuid::new_v5(&header.task_id, b"state:admitted:event"),
            Uuid::new_v5(&header.task_id, b"state:admitted:request"),
            0,
            None,
            HypothesisVerificationTaskStateV1::Admitted,
            "automatic_admission",
        )
        .await?;
    }
    let task = load_task_on(&mut tx, header.stable_task_key_sha256.as_str()).await?;
    validate_stable_task_replay(&task, header, verification_plan_id)?;
    tx.commit().await?;
    Ok(CreateTaskReceipt {
        task,
        replayed: !inserted,
    })
}

pub async fn seal_admission_set(
    pool: &PgPool,
    input: &SealAdmissionSetInput,
) -> HypothesisVerificationTaskStoreResult<AdmissionSetRow> {
    validate_ids(&[
        input.admission_set_id,
        input.stable_request_id,
        input.operation_id,
        input.stage_execution_id,
        input.stage_run_unit_id,
        input.scope_snapshot_id,
        input.organization_id,
        input.generation_id,
    ])?;
    let mut tx = pool.begin().await?;
    if let Some(existing) = sqlx::query_as::<_, AdmissionSetRow>(
        r#"SELECT admission_set_id,generation_id,status,member_count,
                  member_set_sha256,row_version
             FROM verification_admission_sets WHERE stable_request_id=$1"#,
    )
    .bind(input.stable_request_id)
    .fetch_optional(&mut *tx)
    .await?
    {
        if existing.admission_set_id != input.admission_set_id
            || existing.generation_id != input.generation_id
            || existing.status != "sealed"
        {
            return Err(HypothesisVerificationTaskStoreError::IdentityConflict(
                "admission_set_replay_mismatch",
            ));
        }
        tx.commit().await?;
        return Ok(existing);
    }
    sqlx::query(
        r#"INSERT INTO verification_admission_sets(
               admission_set_id,stable_request_id,operation_id,stage_execution_id,
               stage_run_unit_id,scope_snapshot_id,organization_id,generation_id
           ) VALUES($1,$2,$3,$4,$5,$6,$7,$8)"#,
    )
    .bind(input.admission_set_id)
    .bind(input.stable_request_id)
    .bind(input.operation_id)
    .bind(input.stage_execution_id)
    .bind(input.stage_run_unit_id)
    .bind(input.scope_snapshot_id)
    .bind(input.organization_id)
    .bind(input.generation_id)
    .execute(&mut *tx)
    .await?;

    let mut seen = BTreeSet::new();
    for member in &input.members {
        if !seen.insert(member.generation_member_id)
            || member.reason_code.trim().is_empty()
            || !matches!(
                (member.disposition, member.task_id),
                (VerificationAdmissionDispositionV1::Scheduled, Some(_))
                    | (
                        VerificationAdmissionDispositionV1::NeedsEnrichment
                            | VerificationAdmissionDispositionV1::Deferred
                            | VerificationAdmissionDispositionV1::OutOfScope
                            | VerificationAdmissionDispositionV1::Unsafe
                            | VerificationAdmissionDispositionV1::AlreadyTerminal
                            | VerificationAdmissionDispositionV1::NoNewObligation,
                        None
                    )
            )
        {
            return Err(HypothesisVerificationTaskStoreError::InvalidInput(
                "admission_member",
            ));
        }
        validate_sha256(&member.semantic_attempt_fingerprint)?;
        let revision_id: Uuid = sqlx::query_scalar(
            r#"SELECT revision_id FROM hypothesis_generation_members
                WHERE generation_member_id=$1 AND generation_id=$2"#,
        )
        .bind(member.generation_member_id)
        .bind(input.generation_id)
        .fetch_one(&mut *tx)
        .await?;
        let disposition = admission_disposition_str(member.disposition);
        let member_sha256 = sha256_json(&AdmissionMemberHashMaterial {
            admission_set_id: input.admission_set_id,
            generation_member_id: member.generation_member_id,
            hypothesis_revision_id: revision_id,
            disposition,
            reason_code: &member.reason_code,
            semantic_attempt_fingerprint: &member.semantic_attempt_fingerprint,
            task_id: member.task_id,
        });
        sqlx::query(
            r#"INSERT INTO verification_admission_members(
                   admission_member_id,admission_set_id,operation_id,stage_execution_id,
                   stage_run_unit_id,scope_snapshot_id,organization_id,
                   generation_member_id,hypothesis_revision_id,disposition,reason_code,
                   semantic_attempt_fingerprint,task_id,member_sha256
               ) VALUES($1,$2,$3,$4,$5,$6,$7,$8,$9,$10,$11,$12,$13,$14)"#,
        )
        .bind(member.admission_member_id)
        .bind(input.admission_set_id)
        .bind(input.operation_id)
        .bind(input.stage_execution_id)
        .bind(input.stage_run_unit_id)
        .bind(input.scope_snapshot_id)
        .bind(input.organization_id)
        .bind(member.generation_member_id)
        .bind(revision_id)
        .bind(disposition)
        .bind(&member.reason_code)
        .bind(&member.semantic_attempt_fingerprint)
        .bind(member.task_id)
        .bind(member_sha256)
        .execute(&mut *tx)
        .await?;
    }
    let (count, set_hash) = exact_set_hash_on(
        &mut tx,
        "verification_admission_members.v1",
        "verification_admission_members",
        "member_sha256",
        "hypothesis_revision_id",
        "admission_set_id",
        input.admission_set_id,
    )
    .await?;
    let row = sqlx::query_as::<_, AdmissionSetRow>(
        r#"UPDATE verification_admission_sets
              SET status='sealed',member_count=$2,member_set_sha256=$3,
                  row_version=1,sealed_at=statement_timestamp()
            WHERE admission_set_id=$1 AND status='open' AND row_version=0
            RETURNING admission_set_id,generation_id,status,member_count,
                      member_set_sha256,row_version"#,
    )
    .bind(input.admission_set_id)
    .bind(count)
    .bind(set_hash)
    .fetch_one(&mut *tx)
    .await?;
    tx.commit().await?;
    Ok(row)
}

pub async fn seal_objective_assignments(
    pool: &PgPool,
    input: &SealObjectiveAssignmentsInput,
) -> HypothesisVerificationTaskStoreResult<ObjectiveAssignmentSetRow> {
    validate_ids(&[
        input.assignment_set_id,
        input.stable_request_id,
        input.task_id,
    ])?;
    let mut tx = pool.begin().await?;
    let task = load_task_by_id_on(&mut tx, input.task_id).await?;
    let objectives: Vec<(Uuid, Uuid)> = sqlx::query_as(
        r#"SELECT plan_objective_id,objective_id
             FROM attack_hypothesis_verification_plan_objectives
            WHERE plan_id=$1 ORDER BY ordinal,plan_objective_id"#,
    )
    .bind(task.verification_plan_id)
    .fetch_all(&mut *tx)
    .await?;
    let inputs = input
        .members
        .iter()
        .map(|member| (member.plan_objective_id, member))
        .collect::<BTreeMap<_, _>>();
    if inputs.len() != input.members.len()
        || inputs.len() != objectives.len()
        || objectives.iter().any(|(id, _)| !inputs.contains_key(id))
    {
        return Err(HypothesisVerificationTaskStoreError::InvalidInput(
            "objective_assignment_exact_set",
        ));
    }
    sqlx::query(
        r#"INSERT INTO hypothesis_verification_task_assignment_sets(
               assignment_set_id,stable_request_id,task_id,hypothesis_revision_id,
               verification_plan_id
           ) VALUES($1,$2,$3,$4,$5)"#,
    )
    .bind(input.assignment_set_id)
    .bind(input.stable_request_id)
    .bind(input.task_id)
    .bind(task.hypothesis_revision_id)
    .bind(task.verification_plan_id)
    .execute(&mut *tx)
    .await?;

    for (plan_objective_id, objective_id) in objectives {
        let member = inputs[&plan_objective_id];
        let (
            assignment_kind,
            campaign_id,
            already_id,
            already_hash,
            semantic_hash,
            residual_kind,
            residual_reason,
            residual_owner,
            residual_next,
            residual_id,
            residual_hash,
            authority,
        ) = assignment_columns(&member.assignment)?;
        if let Some(campaign_id) = campaign_id {
            let reservation_sha256 = sha256_json(&(
                input.assignment_set_id,
                input.task_id,
                plan_objective_id,
                objective_id,
                "task_campaign_reservation.v1",
            ));
            sqlx::query(
                r#"INSERT INTO hypothesis_verification_task_campaigns(
                       campaign_id,assignment_set_id,task_id,hypothesis_revision_id,
                       verification_plan_id,plan_objective_id,verification_objective_id,
                       reservation_sha256
                   ) VALUES($1,$2,$3,$4,$5,$6,$7,$8)"#,
            )
            .bind(campaign_id)
            .bind(input.assignment_set_id)
            .bind(input.task_id)
            .bind(task.hypothesis_revision_id)
            .bind(task.verification_plan_id)
            .bind(plan_objective_id)
            .bind(objective_id)
            .bind(reservation_sha256)
            .execute(&mut *tx)
            .await?;
        }
        let member_sha256 = sha256_json(&AssignmentMemberHashMaterial {
            assignment_set_id: input.assignment_set_id,
            task_id: input.task_id,
            plan_objective_id,
            verification_objective_id: objective_id,
            assignment_kind,
            authority,
        });
        sqlx::query(
            r#"INSERT INTO hypothesis_verification_task_assignment_members(
                   assignment_member_id,assignment_set_id,task_id,hypothesis_revision_id,
                   verification_plan_id,plan_objective_id,verification_objective_id,
                   assignment_kind,campaign_id,already_satisfied_receipt_id,
                   already_satisfied_receipt_sha256,semantic_evidence_set_sha256,
                   residual_kind,residual_reason_code,residual_owner,residual_next_action,
                   residual_receipt_id,residual_receipt_sha256,member_sha256
               ) VALUES($1,$2,$3,$4,$5,$6,$7,$8,$9,$10,$11,$12,$13,$14,$15,$16,$17,$18,$19)"#,
        )
        .bind(member.assignment_member_id)
        .bind(input.assignment_set_id)
        .bind(input.task_id)
        .bind(task.hypothesis_revision_id)
        .bind(task.verification_plan_id)
        .bind(plan_objective_id)
        .bind(objective_id)
        .bind(assignment_kind)
        .bind(campaign_id)
        .bind(already_id)
        .bind(already_hash)
        .bind(semantic_hash)
        .bind(residual_kind)
        .bind(residual_reason)
        .bind(residual_owner)
        .bind(residual_next)
        .bind(residual_id)
        .bind(residual_hash)
        .bind(member_sha256)
        .execute(&mut *tx)
        .await?;
    }
    let (count, set_hash) = exact_set_hash_on(
        &mut tx,
        "hypothesis_verification_task_assignments.v1",
        "hypothesis_verification_task_assignment_members",
        "member_sha256",
        "plan_objective_id",
        "assignment_set_id",
        input.assignment_set_id,
    )
    .await?;
    let row = sqlx::query_as::<_, ObjectiveAssignmentSetRow>(
        r#"UPDATE hypothesis_verification_task_assignment_sets
              SET status='sealed',member_count=$2,member_set_sha256=$3,
                  row_version=1,sealed_at=statement_timestamp()
            WHERE assignment_set_id=$1 AND status='open' AND row_version=0
            RETURNING assignment_set_id,task_id,status,member_count,
                      member_set_sha256,row_version"#,
    )
    .bind(input.assignment_set_id)
    .bind(count)
    .bind(set_hash)
    .fetch_one(&mut *tx)
    .await?;
    tx.commit().await?;
    Ok(row)
}

pub async fn seal_campaign_outcomes(
    pool: &PgPool,
    input: &SealCampaignOutcomesInput,
) -> HypothesisVerificationTaskStoreResult<CampaignOutcomeSetRow> {
    validate_ids(&[
        input.outcome_set_id,
        input.stable_request_id,
        input.assignment_set_id,
        input.task_id,
    ])?;
    let mut tx = pool.begin().await?;
    sqlx::query(
        r#"INSERT INTO hypothesis_verification_task_outcome_sets(
               outcome_set_id,stable_request_id,assignment_set_id,task_id
           ) VALUES($1,$2,$3,$4)"#,
    )
    .bind(input.outcome_set_id)
    .bind(input.stable_request_id)
    .bind(input.assignment_set_id)
    .bind(input.task_id)
    .execute(&mut *tx)
    .await?;
    let mut seen = BTreeSet::new();
    for outcome in &input.outcomes {
        if !seen.insert(outcome.campaign_id)
            || outcome.outcome_member_id.is_nil()
            || outcome.terminal_receipt_id.is_nil()
        {
            return Err(HypothesisVerificationTaskStoreError::InvalidInput(
                "campaign_outcome",
            ));
        }
        validate_sha256(&outcome.terminal_receipt_sha256)?;
        let outcome_kind = outcome.outcome_kind.as_str();
        let member_sha256 = sha256_json(&OutcomeMemberHashMaterial {
            outcome_set_id: input.outcome_set_id,
            task_id: input.task_id,
            campaign_id: outcome.campaign_id,
            outcome_kind,
            terminal_receipt_id: outcome.terminal_receipt_id,
            terminal_receipt_sha256: &outcome.terminal_receipt_sha256,
        });
        sqlx::query(
            r#"INSERT INTO hypothesis_verification_task_outcome_members(
                   outcome_member_id,outcome_set_id,assignment_set_id,task_id,
                   campaign_id,outcome_kind,terminal_receipt_id,
                   terminal_receipt_sha256,member_sha256
               ) VALUES($1,$2,$3,$4,$5,$6,$7,$8,$9)"#,
        )
        .bind(outcome.outcome_member_id)
        .bind(input.outcome_set_id)
        .bind(input.assignment_set_id)
        .bind(input.task_id)
        .bind(outcome.campaign_id)
        .bind(outcome_kind)
        .bind(outcome.terminal_receipt_id)
        .bind(&outcome.terminal_receipt_sha256)
        .bind(member_sha256)
        .execute(&mut *tx)
        .await?;
    }
    let (count, set_hash) = exact_set_hash_on(
        &mut tx,
        "hypothesis_verification_task_outcomes.v1",
        "hypothesis_verification_task_outcome_members",
        "member_sha256",
        "campaign_id",
        "outcome_set_id",
        input.outcome_set_id,
    )
    .await?;
    let row = sqlx::query_as::<_, CampaignOutcomeSetRow>(
        r#"UPDATE hypothesis_verification_task_outcome_sets
              SET status='sealed',member_count=$2,member_set_sha256=$3,
                  row_version=1,sealed_at=statement_timestamp()
            WHERE outcome_set_id=$1 AND status='open' AND row_version=0
            RETURNING outcome_set_id,task_id,status,member_count,
                      member_set_sha256,row_version"#,
    )
    .bind(input.outcome_set_id)
    .bind(count)
    .bind(set_hash)
    .fetch_one(&mut *tx)
    .await?;
    tx.commit().await?;
    Ok(row)
}

pub async fn append_task_state(
    pool: &PgPool,
    task_id: Uuid,
    event_id: Uuid,
    stable_request_id: Uuid,
    expected_head_version: i64,
    next: HypothesisVerificationTaskStateV1,
    reason_code: &str,
) -> HypothesisVerificationTaskStoreResult<TaskStateHeadRow> {
    validate_ids(&[task_id, event_id, stable_request_id])?;
    let mut tx = pool.begin().await?;
    let head = load_state_head_on(&mut tx, task_id).await?;
    if head.head_version != expected_head_version {
        return Err(HypothesisVerificationTaskStoreError::CasConflict(
            "task_state_head",
        ));
    }
    let current = parse_task_state(&head.current_state)?;
    append_state_event_on(
        &mut tx,
        task_id,
        event_id,
        stable_request_id,
        expected_head_version,
        Some(current),
        next,
        reason_code,
    )
    .await?;
    let next_head = load_state_head_on(&mut tx, task_id).await?;
    tx.commit().await?;
    Ok(next_head)
}

/// Advance the durable VerificationTask aggregate to `running` before its AI
/// Primary is allowed to plan. The complete legal prefix is applied in one
/// transaction and exact re-entry is a no-op.
pub async fn advance_task_to_running(
    pool: &PgPool,
    input: &AdvanceTaskToRunningInput,
) -> HypothesisVerificationTaskStoreResult<TaskStateHeadRow> {
    validate_ids(&[
        input.task_id,
        input.operation_id,
        input.stage_execution_id,
        input.stage_run_unit_id,
        input.scope_snapshot_id,
        input.organization_id,
    ])?;
    let mut tx = pool.begin().await?;
    let task = load_task_by_id_on(&mut tx, input.task_id).await?;
    if task.operation_id != input.operation_id
        || task.stage_execution_id != input.stage_execution_id
        || task.stage_run_unit_id != input.stage_run_unit_id
        || task.scope_snapshot_id != input.scope_snapshot_id
        || task.organization_id != input.organization_id
    {
        return Err(HypothesisVerificationTaskStoreError::IdentityConflict(
            "task_running_scope",
        ));
    }
    loop {
        let head = load_state_head_on(&mut tx, input.task_id).await?;
        let current = parse_task_state(&head.current_state)?;
        let next = match current {
            HypothesisVerificationTaskStateV1::Admitted => {
                Some(HypothesisVerificationTaskStateV1::Queued)
            }
            HypothesisVerificationTaskStateV1::Queued => {
                Some(HypothesisVerificationTaskStateV1::Planning)
            }
            HypothesisVerificationTaskStateV1::Planning => {
                Some(HypothesisVerificationTaskStateV1::Running)
            }
            HypothesisVerificationTaskStateV1::Running
            | HypothesisVerificationTaskStateV1::AwaitingAuthorization
            | HypothesisVerificationTaskStateV1::Consolidating
            | HypothesisVerificationTaskStateV1::StopPending
            | HypothesisVerificationTaskStateV1::Draining
            | HypothesisVerificationTaskStateV1::Terminal => None,
            HypothesisVerificationTaskStateV1::Blocked
            | HypothesisVerificationTaskStateV1::Cancelled
            | HypothesisVerificationTaskStateV1::RecoveryRequired => {
                return Err(HypothesisVerificationTaskStoreError::CasConflict(
                    "task_not_runnable",
                ));
            }
        };
        let Some(next) = next else {
            tx.commit().await?;
            return Ok(head);
        };
        let next_name = task_state_str(next);
        append_state_event_on(
            &mut tx,
            input.task_id,
            Uuid::new_v5(
                &input.task_id,
                format!("task-running-state:{}:{next_name}", head.head_version + 1).as_bytes(),
            ),
            Uuid::new_v5(
                &input.task_id,
                format!("task-running-request:{}:{next_name}", head.head_version + 1).as_bytes(),
            ),
            head.head_version,
            Some(current),
            next,
            "verification_primary_admission",
        )
        .await?;
    }
}

/// Seal the exact Campaign terminal-receipt set and advance the Task aggregate
/// through its legal state path in one response-loss-safe transaction. The
/// caller supplies only immutable Task scope; Campaign members are read from
/// canonical reservation and terminal-decision truth under lock.
pub async fn finalize_task_from_campaign_truth(
    pool: &PgPool,
    input: &FinalizeTaskFromCampaignTruthInput,
) -> HypothesisVerificationTaskStoreResult<FinalizedTaskFromCampaignTruth> {
    validate_ids(&[
        input.task_id,
        input.operation_id,
        input.stage_execution_id,
        input.stage_run_unit_id,
        input.scope_snapshot_id,
        input.organization_id,
    ])?;
    let mut tx = pool.begin().await?;
    let task = load_task_by_id_on(&mut tx, input.task_id).await?;
    if task.operation_id != input.operation_id
        || task.stage_execution_id != input.stage_execution_id
        || task.stage_run_unit_id != input.stage_run_unit_id
        || task.scope_snapshot_id != input.scope_snapshot_id
        || task.organization_id != input.organization_id
    {
        return Err(HypothesisVerificationTaskStoreError::IdentityConflict(
            "campaign_truth_task_scope",
        ));
    }
    let assignment_set_id: Uuid = sqlx::query_scalar(
        "SELECT assignment_set_id FROM hypothesis_verification_task_assignment_sets
          WHERE task_id=$1 AND status='sealed' FOR SHARE",
    )
    .bind(input.task_id)
    .fetch_one(&mut *tx)
    .await?;
    let terminals = sqlx::query_as::<_, (Uuid, Uuid, String, String)>(
        r#"SELECT reservation.campaign_id,terminal.campaign_terminal_decision_id,
                  terminal.terminal_decision,terminal.terminal_hash
             FROM hypothesis_verification_task_campaigns reservation
             JOIN verification_campaign_terminal_decisions terminal
               ON terminal.campaign_id=reservation.campaign_id
              AND terminal.operation_id=$3
            WHERE reservation.task_id=$1 AND reservation.assignment_set_id=$2
            ORDER BY reservation.campaign_id
            FOR SHARE OF terminal"#,
    )
    .bind(input.task_id)
    .bind(assignment_set_id)
    .bind(input.operation_id)
    .fetch_all(&mut *tx)
    .await?;
    let expected_campaign_count: i64 = sqlx::query_scalar(
        "SELECT COUNT(*) FROM hypothesis_verification_task_campaigns
          WHERE task_id=$1 AND assignment_set_id=$2",
    )
    .bind(input.task_id)
    .bind(assignment_set_id)
    .fetch_one(&mut *tx)
    .await?;
    if expected_campaign_count == 0
        || i64::try_from(terminals.len()).ok() != Some(expected_campaign_count)
    {
        return Err(HypothesisVerificationTaskStoreError::CasConflict(
            "campaign_truth_not_terminal",
        ));
    }
    let outcome_set_id = Uuid::new_v5(&input.task_id, b"campaign_outcomes.v1");
    let existing = sqlx::query_as::<_, CampaignOutcomeSetRow>(
        r#"SELECT outcome_set_id,task_id,status,member_count,member_set_sha256,row_version
             FROM hypothesis_verification_task_outcome_sets WHERE task_id=$1 FOR SHARE"#,
    )
    .bind(input.task_id)
    .fetch_optional(&mut *tx)
    .await?;
    let replayed = existing.is_some();
    let outcome_set = if let Some(existing) = existing {
        if existing.outcome_set_id != outcome_set_id
            || existing.status != "sealed"
            || existing.member_count != Some(expected_campaign_count)
            || existing.member_set_sha256.is_none()
        {
            return Err(HypothesisVerificationTaskStoreError::IdentityConflict(
                "campaign_truth_outcome_replay",
            ));
        }
        existing
    } else {
        let stable_request_id = Uuid::new_v5(&input.task_id, b"campaign_outcomes:request");
        sqlx::query(
            r#"INSERT INTO hypothesis_verification_task_outcome_sets(
                   outcome_set_id,stable_request_id,assignment_set_id,task_id)
               VALUES($1,$2,$3,$4)"#,
        )
        .bind(outcome_set_id)
        .bind(stable_request_id)
        .bind(assignment_set_id)
        .bind(input.task_id)
        .execute(&mut *tx)
        .await?;
        for (campaign_id, terminal_receipt_id, terminal_decision, terminal_hash) in &terminals {
            validate_sha256(terminal_hash)?;
            let outcome_kind = if terminal_decision == "blocked" {
                CampaignOutcomeKind::Blocked
            } else {
                CampaignOutcomeKind::Completed
            };
            let outcome_member_id = Uuid::new_v5(&outcome_set_id, campaign_id.as_bytes());
            let member_sha256 = sha256_json(&OutcomeMemberHashMaterial {
                outcome_set_id,
                task_id: input.task_id,
                campaign_id: *campaign_id,
                outcome_kind: outcome_kind.as_str(),
                terminal_receipt_id: *terminal_receipt_id,
                terminal_receipt_sha256: terminal_hash,
            });
            sqlx::query(
                r#"INSERT INTO hypothesis_verification_task_outcome_members(
                       outcome_member_id,outcome_set_id,assignment_set_id,task_id,
                       campaign_id,outcome_kind,terminal_receipt_id,
                       terminal_receipt_sha256,member_sha256)
                   VALUES($1,$2,$3,$4,$5,$6,$7,$8,$9)"#,
            )
            .bind(outcome_member_id)
            .bind(outcome_set_id)
            .bind(assignment_set_id)
            .bind(input.task_id)
            .bind(campaign_id)
            .bind(outcome_kind.as_str())
            .bind(terminal_receipt_id)
            .bind(terminal_hash)
            .bind(member_sha256)
            .execute(&mut *tx)
            .await?;
        }
        let (member_count, member_set_sha256) = exact_set_hash_on(
            &mut tx,
            "hypothesis_verification_task_outcomes.v1",
            "hypothesis_verification_task_outcome_members",
            "member_sha256",
            "campaign_id",
            "outcome_set_id",
            outcome_set_id,
        )
        .await?;
        sqlx::query_as::<_, CampaignOutcomeSetRow>(
            r#"UPDATE hypothesis_verification_task_outcome_sets
                  SET status='sealed',member_count=$2,member_set_sha256=$3,
                      row_version=1,sealed_at=statement_timestamp()
                WHERE outcome_set_id=$1 AND status='open' AND row_version=0
                RETURNING outcome_set_id,task_id,status,member_count,
                          member_set_sha256,row_version"#,
        )
        .bind(outcome_set_id)
        .bind(member_count)
        .bind(member_set_sha256)
        .fetch_one(&mut *tx)
        .await?
    };
    let blocked = terminals
        .iter()
        .any(|(_, _, decision, _)| decision == "blocked");
    loop {
        let head = load_state_head_on(&mut tx, input.task_id).await?;
        let current = parse_task_state(&head.current_state)?;
        let next = match current {
            HypothesisVerificationTaskStateV1::Admitted => {
                Some(HypothesisVerificationTaskStateV1::Queued)
            }
            HypothesisVerificationTaskStateV1::Queued => {
                Some(HypothesisVerificationTaskStateV1::Planning)
            }
            HypothesisVerificationTaskStateV1::Planning => {
                Some(HypothesisVerificationTaskStateV1::Running)
            }
            HypothesisVerificationTaskStateV1::Running => {
                Some(HypothesisVerificationTaskStateV1::Consolidating)
            }
            HypothesisVerificationTaskStateV1::AwaitingAuthorization => {
                Some(HypothesisVerificationTaskStateV1::Running)
            }
            HypothesisVerificationTaskStateV1::Consolidating => Some(if blocked {
                HypothesisVerificationTaskStateV1::Blocked
            } else {
                HypothesisVerificationTaskStateV1::Terminal
            }),
            HypothesisVerificationTaskStateV1::RecoveryRequired => {
                Some(HypothesisVerificationTaskStateV1::Blocked)
            }
            HypothesisVerificationTaskStateV1::Terminal
            | HypothesisVerificationTaskStateV1::Blocked
            | HypothesisVerificationTaskStateV1::Cancelled => None,
            HypothesisVerificationTaskStateV1::StopPending => {
                Some(HypothesisVerificationTaskStateV1::Draining)
            }
            HypothesisVerificationTaskStateV1::Draining => {
                Some(HypothesisVerificationTaskStateV1::Consolidating)
            }
        };
        let Some(next) = next else {
            let terminal = load_state_head_on(&mut tx, input.task_id).await?;
            let outcome_member_set_sha256 = outcome_set.member_set_sha256.clone().ok_or(
                HypothesisVerificationTaskStoreError::IdentityConflict(
                    "campaign_truth_outcome_unsealed",
                ),
            )?;
            tx.commit().await?;
            return Ok(FinalizedTaskFromCampaignTruth {
                task_id: input.task_id,
                outcome_set_id,
                outcome_member_count: outcome_set.member_count.unwrap_or_default(),
                outcome_member_set_sha256,
                terminal_state: terminal.current_state,
                head_version: terminal.head_version,
                replayed,
            });
        };
        let next_name = task_state_str(next);
        append_state_event_on(
            &mut tx,
            input.task_id,
            Uuid::new_v5(
                &input.task_id,
                format!(
                    "campaign-truth-state:{}:{}",
                    head.head_version + 1,
                    next_name
                )
                .as_bytes(),
            ),
            Uuid::new_v5(
                &input.task_id,
                format!(
                    "campaign-truth-state-request:{}:{}",
                    head.head_version + 1,
                    next_name
                )
                .as_bytes(),
            ),
            head.head_version,
            Some(current),
            next,
            "campaign_truth_finalization",
        )
        .await?;
    }
}

#[allow(clippy::too_many_arguments)]
async fn append_state_event_on(
    tx: &mut Transaction<'_, Postgres>,
    task_id: Uuid,
    event_id: Uuid,
    stable_request_id: Uuid,
    expected_head_version: i64,
    from: Option<HypothesisVerificationTaskStateV1>,
    to: HypothesisVerificationTaskStateV1,
    reason_code: &str,
) -> HypothesisVerificationTaskStoreResult<()> {
    if reason_code.trim().is_empty() || expected_head_version < 0 {
        return Err(HypothesisVerificationTaskStoreError::InvalidInput(
            "task_state_event",
        ));
    }
    let event_ordinal = if from.is_some() {
        expected_head_version + 1
    } else {
        0
    };
    let from_state = from.map(task_state_str);
    let to_state = task_state_str(to);
    let event_sha256 = sha256_json(&(
        task_id,
        event_ordinal,
        expected_head_version,
        from_state,
        to_state,
        reason_code,
    ));
    sqlx::query(
        r#"INSERT INTO hypothesis_verification_task_state_events(
               event_id,stable_request_id,task_id,event_ordinal,
               expected_head_version,from_state,to_state,reason_code,event_sha256
           ) VALUES($1,$2,$3,$4,$5,$6,$7,$8,$9)"#,
    )
    .bind(event_id)
    .bind(stable_request_id)
    .bind(task_id)
    .bind(event_ordinal)
    .bind(expected_head_version)
    .bind(from_state)
    .bind(to_state)
    .bind(reason_code)
    .bind(event_sha256)
    .execute(&mut **tx)
    .await?;
    Ok(())
}

fn validate_task_header(
    header: &HypothesisVerificationTaskHeaderV1,
) -> HypothesisVerificationTaskStoreResult<()> {
    let rebuilt =
        HypothesisVerificationTaskHeaderV1::host_create(NewHypothesisVerificationTaskV1 {
            operation_id: header.operation_id,
            stage_execution_id: header.stage_execution_id,
            stage_run_unit_id: header.stage_run_unit_id,
            organization_id: header.organization_id,
            scope_snapshot_id: header.scope_snapshot_id,
            hypothesis_revision_id: header.hypothesis_revision_id,
            hypothesis_revision_sha256: header.hypothesis_revision_sha256.clone(),
            verification_plan_sha256: header.verification_plan_sha256.clone(),
            relevant_evidence_snapshot_id: header.relevant_evidence_snapshot_id,
            semantic_evidence_set_sha256: header.semantic_evidence_set_sha256.clone(),
            open_obligation_set_sha256: header.open_obligation_set_sha256.clone(),
            semantic_attempt_fingerprint: header.semantic_attempt_fingerprint.clone(),
            first_admission_generation_id: header.first_admission_generation_id,
            host_rerun_receipt_id: header.host_rerun_receipt_id,
            host_rerun_receipt_sha256: header.host_rerun_receipt_sha256.clone(),
            rerun_contract_version: header.rerun_contract_version,
        })
        .map_err(|_| HypothesisVerificationTaskStoreError::InvalidInput("task_header"))?;
    if rebuilt != *header {
        return Err(HypothesisVerificationTaskStoreError::IdentityConflict(
            "caller_supplied_task_key",
        ));
    }
    Ok(())
}

fn validate_stable_task_replay(
    stored: &HypothesisVerificationTaskRow,
    header: &HypothesisVerificationTaskHeaderV1,
    plan_id: Uuid,
) -> HypothesisVerificationTaskStoreResult<()> {
    if stored.task_id != header.task_id
        || stored.stable_task_key_sha256 != header.stable_task_key_sha256
        || stored.operation_id != header.operation_id
        || stored.stage_execution_id != header.stage_execution_id
        || stored.stage_run_unit_id != header.stage_run_unit_id
        || stored.scope_snapshot_id != header.scope_snapshot_id
        || stored.organization_id != header.organization_id
        || stored.hypothesis_revision_id != header.hypothesis_revision_id
        || stored.hypothesis_revision_sha256 != header.hypothesis_revision_sha256
        || stored.verification_plan_id != plan_id
        || stored.verification_plan_sha256 != header.verification_plan_sha256
        || stored.semantic_evidence_set_sha256 != header.semantic_evidence_set_sha256
        || stored.open_obligation_set_sha256 != header.open_obligation_set_sha256
        || stored.semantic_attempt_fingerprint != header.semantic_attempt_fingerprint
        || stored.task_contract_version != header.task_contract_version
        || stored.host_rerun_receipt_id != header.host_rerun_receipt_id
        || stored.host_rerun_receipt_sha256 != header.host_rerun_receipt_sha256
        || stored.rerun_contract_version != header.rerun_contract_version.map(|value| value as i32)
    {
        return Err(HypothesisVerificationTaskStoreError::IdentityConflict(
            "stable_task_key_collision",
        ));
    }
    Ok(())
}

async fn load_task_on(
    tx: &mut Transaction<'_, Postgres>,
    stable_key: &str,
) -> HypothesisVerificationTaskStoreResult<HypothesisVerificationTaskRow> {
    Ok(sqlx::query_as::<_, HypothesisVerificationTaskRow>(&format!(
        "SELECT {TASK_COLUMNS} FROM hypothesis_verification_tasks WHERE stable_task_key_sha256=$1"
    ))
    .bind(stable_key)
    .fetch_one(&mut **tx)
    .await?)
}

async fn load_task_by_id_on(
    tx: &mut Transaction<'_, Postgres>,
    task_id: Uuid,
) -> HypothesisVerificationTaskStoreResult<HypothesisVerificationTaskRow> {
    Ok(sqlx::query_as::<_, HypothesisVerificationTaskRow>(&format!(
        "SELECT {TASK_COLUMNS} FROM hypothesis_verification_tasks WHERE task_id=$1 FOR SHARE"
    ))
    .bind(task_id)
    .fetch_one(&mut **tx)
    .await?)
}

async fn load_state_head_on(
    tx: &mut Transaction<'_, Postgres>,
    task_id: Uuid,
) -> HypothesisVerificationTaskStoreResult<TaskStateHeadRow> {
    Ok(sqlx::query_as::<_, TaskStateHeadRow>(
        r#"SELECT task_id,current_state,latest_event_id,head_version
             FROM hypothesis_verification_task_state_heads
            WHERE task_id=$1 FOR UPDATE"#,
    )
    .bind(task_id)
    .fetch_one(&mut **tx)
    .await?)
}

const TASK_COLUMNS: &str = r#"task_id,stable_task_key_sha256,operation_id,
    project_scope_id,stage_execution_id,stage_run_unit_id,scope_snapshot_id,
    organization_id,hypothesis_revision_id,hypothesis_revision_sha256,
    verification_plan_id,verification_plan_sha256,relevant_evidence_snapshot_id,
    semantic_evidence_set_sha256,open_obligation_set_sha256,
    semantic_attempt_fingerprint,task_contract_version,
    first_admission_generation_id,host_rerun_receipt_id,
    host_rerun_receipt_sha256,rerun_contract_version"#;

#[allow(clippy::too_many_arguments)]
async fn exact_set_hash_on(
    tx: &mut Transaction<'_, Postgres>,
    contract: &str,
    table: &str,
    member_column: &str,
    order_column: &str,
    owner_column: &str,
    owner_id: Uuid,
) -> HypothesisVerificationTaskStoreResult<(i64, String)> {
    let sql = format!(
        "SELECT COUNT(*),unified_investigation_exact_set_hash($1,\
         COALESCE(array_agg({member_column} ORDER BY {order_column}),ARRAY[]::TEXT[])) \
         FROM {table} WHERE {owner_column}=$2"
    );
    Ok(sqlx::query_as(&sql)
        .bind(contract)
        .bind(owner_id)
        .fetch_one(&mut **tx)
        .await?)
}

type AssignmentColumns = (
    &'static str,
    Option<Uuid>,
    Option<Uuid>,
    Option<String>,
    Option<String>,
    Option<&'static str>,
    Option<String>,
    Option<String>,
    Option<String>,
    Option<Uuid>,
    Option<String>,
    serde_json::Value,
);

fn assignment_columns(
    assignment: &ObjectiveAssignmentInput,
) -> HypothesisVerificationTaskStoreResult<AssignmentColumns> {
    match assignment {
        ObjectiveAssignmentInput::Campaign { campaign_id } if !campaign_id.is_nil() => Ok((
            "campaign",
            Some(*campaign_id),
            None,
            None,
            None,
            None,
            None,
            None,
            None,
            None,
            None,
            serde_json::json!({"campaign_id": campaign_id}),
        )),
        ObjectiveAssignmentInput::AlreadySatisfied {
            objective_outcome_receipt_id,
            outcome_receipt_sha256,
            semantic_evidence_set_sha256,
        } if !objective_outcome_receipt_id.is_nil() => {
            validate_sha256(outcome_receipt_sha256)?;
            validate_sha256(semantic_evidence_set_sha256)?;
            Ok((
                "already_satisfied",
                None,
                Some(*objective_outcome_receipt_id),
                Some(outcome_receipt_sha256.clone()),
                Some(semantic_evidence_set_sha256.clone()),
                None,
                None,
                None,
                None,
                None,
                None,
                serde_json::json!({
                    "objective_outcome_receipt_id": objective_outcome_receipt_id,
                    "outcome_receipt_sha256": outcome_receipt_sha256,
                    "semantic_evidence_set_sha256": semantic_evidence_set_sha256,
                }),
            ))
        }
        ObjectiveAssignmentInput::Residual {
            residual_kind,
            reason_code,
            owner,
            next_action,
            residual_receipt_id,
            residual_receipt_sha256,
        } if !residual_receipt_id.is_nil()
            && [reason_code, owner, next_action]
                .iter()
                .all(|value| !value.trim().is_empty()) =>
        {
            validate_sha256(residual_receipt_sha256)?;
            let kind = residual_kind_str(*residual_kind);
            Ok((
                "residual",
                None,
                None,
                None,
                None,
                Some(kind),
                Some(reason_code.clone()),
                Some(owner.clone()),
                Some(next_action.clone()),
                Some(*residual_receipt_id),
                Some(residual_receipt_sha256.clone()),
                serde_json::json!({
                    "residual_kind": kind,
                    "reason_code": reason_code,
                    "owner": owner,
                    "next_action": next_action,
                    "residual_receipt_id": residual_receipt_id,
                    "residual_receipt_sha256": residual_receipt_sha256,
                }),
            ))
        }
        _ => Err(HypothesisVerificationTaskStoreError::InvalidInput(
            "objective_assignment",
        )),
    }
}

fn admission_disposition_str(value: VerificationAdmissionDispositionV1) -> &'static str {
    match value {
        VerificationAdmissionDispositionV1::Scheduled => "scheduled",
        VerificationAdmissionDispositionV1::NeedsEnrichment => "needs_enrichment",
        VerificationAdmissionDispositionV1::Deferred => "deferred",
        VerificationAdmissionDispositionV1::OutOfScope => "out_of_scope",
        VerificationAdmissionDispositionV1::Unsafe => "unsafe",
        VerificationAdmissionDispositionV1::AlreadyTerminal => "already_terminal",
        VerificationAdmissionDispositionV1::NoNewObligation => "no_new_obligation",
    }
}

fn residual_kind_str(value: TaskObjectiveResidualKindV1) -> &'static str {
    match value {
        TaskObjectiveResidualKindV1::NoKnownCapability => "no_known_capability",
        TaskObjectiveResidualKindV1::NeedsEnrichment => "needs_enrichment",
        TaskObjectiveResidualKindV1::Deferred => "deferred",
        TaskObjectiveResidualKindV1::OutOfScope => "out_of_scope",
        TaskObjectiveResidualKindV1::Unsafe => "unsafe",
        TaskObjectiveResidualKindV1::Blocked => "blocked",
    }
}

fn task_state_str(value: HypothesisVerificationTaskStateV1) -> &'static str {
    match value {
        HypothesisVerificationTaskStateV1::Admitted => "admitted",
        HypothesisVerificationTaskStateV1::Queued => "queued",
        HypothesisVerificationTaskStateV1::Planning => "planning",
        HypothesisVerificationTaskStateV1::Running => "running",
        HypothesisVerificationTaskStateV1::AwaitingAuthorization => "awaiting_authorization",
        HypothesisVerificationTaskStateV1::Consolidating => "consolidating",
        HypothesisVerificationTaskStateV1::StopPending => "stop_pending",
        HypothesisVerificationTaskStateV1::Draining => "draining",
        HypothesisVerificationTaskStateV1::Cancelled => "cancelled",
        HypothesisVerificationTaskStateV1::Blocked => "blocked",
        HypothesisVerificationTaskStateV1::RecoveryRequired => "recovery_required",
        HypothesisVerificationTaskStateV1::Terminal => "terminal",
    }
}

fn parse_task_state(
    value: &str,
) -> HypothesisVerificationTaskStoreResult<HypothesisVerificationTaskStateV1> {
    Ok(match value {
        "admitted" => HypothesisVerificationTaskStateV1::Admitted,
        "queued" => HypothesisVerificationTaskStateV1::Queued,
        "planning" => HypothesisVerificationTaskStateV1::Planning,
        "running" => HypothesisVerificationTaskStateV1::Running,
        "awaiting_authorization" => HypothesisVerificationTaskStateV1::AwaitingAuthorization,
        "consolidating" => HypothesisVerificationTaskStateV1::Consolidating,
        "stop_pending" => HypothesisVerificationTaskStateV1::StopPending,
        "draining" => HypothesisVerificationTaskStateV1::Draining,
        "cancelled" => HypothesisVerificationTaskStateV1::Cancelled,
        "blocked" => HypothesisVerificationTaskStateV1::Blocked,
        "recovery_required" => HypothesisVerificationTaskStateV1::RecoveryRequired,
        "terminal" => HypothesisVerificationTaskStateV1::Terminal,
        _ => {
            return Err(HypothesisVerificationTaskStoreError::IdentityConflict(
                "unknown_task_state",
            ))
        }
    })
}

fn validate_ids(values: &[Uuid]) -> HypothesisVerificationTaskStoreResult<()> {
    if values.iter().any(Uuid::is_nil) {
        return Err(HypothesisVerificationTaskStoreError::InvalidInput("uuid"));
    }
    Ok(())
}

fn validate_sha256(value: &str) -> HypothesisVerificationTaskStoreResult<()> {
    let valid = value.strip_prefix("sha256:").is_some_and(|hex| {
        hex.len() == 64
            && hex
                .bytes()
                .all(|byte| byte.is_ascii_hexdigit() && !byte.is_ascii_uppercase())
    });
    if !valid {
        return Err(HypothesisVerificationTaskStoreError::InvalidInput("sha256"));
    }
    Ok(())
}

fn sha256_json(value: &impl Serialize) -> String {
    let digest = Sha256::digest(
        serde_json::to_vec(value).expect("hypothesis task hash material is serializable"),
    );
    format!(
        "sha256:{}",
        digest
            .iter()
            .map(|byte| format!("{byte:02x}"))
            .collect::<String>()
    )
}
