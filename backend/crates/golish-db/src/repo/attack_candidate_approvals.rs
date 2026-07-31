//! Durable, exact-scope Candidate review and DB-backed resume barrier.

use chrono::{DateTime, Duration, Utc};
use sqlx::{PgPool, Postgres, Transaction};
use std::collections::{BTreeMap, BTreeSet};
use uuid::Uuid;

pub const ATTACK_CANDIDATE_PLAN_CHANGED: &str = "ATTACK_CANDIDATE_PLAN_CHANGED";
pub const ATTACK_REVIEW_SCOPE_MISMATCH: &str = "ATTACK_REVIEW_SCOPE_MISMATCH";
pub const ATTACK_APPROVAL_EXPIRED: &str = "ATTACK_APPROVAL_EXPIRED";
pub const ATTACK_REVIEW_ALREADY_CLOSED: &str = "ATTACK_REVIEW_ALREADY_CLOSED";
pub const ATTACK_RESUME_NOT_READY: &str = "ATTACK_RESUME_NOT_READY";
pub const ATTACK_ATTEMPT_FUEL_EXHAUSTED: &str = "ATTACK_ATTEMPT_FUEL_EXHAUSTED";
pub const DEFAULT_REVIEW_DISPATCH_STALE_SECONDS: i64 = 300;

#[derive(Debug, Clone, sqlx::FromRow, PartialEq)]
pub struct AttackCandidateApprovalRow {
    pub id: Uuid,
    pub candidate_id: Uuid,
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
    pub source_work_item_id: Uuid,
    pub execution_plan: serde_json::Value,
    pub allowed_capability_ids: Vec<String>,
    pub allowed_action_kinds: Vec<String>,
    pub budget: serde_json::Value,
    /// Latest instant at which a new Candidate action may begin.
    pub start_before: DateTime<Utc>,
    pub expires_at: DateTime<Utc>,
    pub decision_version: i64,
    pub status: String,
    pub decided_by: Uuid,
    pub decided_at: DateTime<Utc>,
    pub row_version: i64,
}

#[derive(Debug, Clone)]
pub struct CandidateReviewDecision {
    pub candidate_id: Uuid,
    pub expected_candidate_plan_hash: String,
    pub expected_candidate_row_version: i64,
    pub approve: bool,
    pub start_before: Option<DateTime<Utc>>,
    pub expires_at: Option<DateTime<Utc>>,
}

#[derive(Debug, Clone)]
pub struct ReviewCandidateBatch {
    pub operation_id: Uuid,
    pub wave_run_id: Uuid,
    pub decisions: Vec<CandidateReviewDecision>,
}

#[derive(Debug, Clone)]
pub struct CandidateReviewItemRow {
    pub candidate_id: Uuid,
    pub wave_unit_id: Uuid,
    pub organization_id: Uuid,
    pub target_live_id: Option<Uuid>,
    pub live_target_present: bool,
    pub target_type_at_time: String,
    pub target_value_at_time: String,
    pub target_identity_hash: String,
    pub hypothesis: String,
    pub technique: Option<String>,
    pub rationale: String,
    pub risk_class: String,
    pub execution_plan: serde_json::Value,
    pub candidate_plan_hash: String,
    pub disposition: String,
    pub row_version: i64,
    pub latest_approval: Option<AttackCandidateApprovalRow>,
}

#[derive(Debug, Clone, sqlx::FromRow, PartialEq, Eq)]
pub struct CandidateReviewBarrierRow {
    pub wave_run_id: Uuid,
    pub operation_id: Uuid,
    pub scope_snapshot_id: Uuid,
    pub status: String,
    pub resume_version: i64,
    pub last_error: Option<String>,
    pub dispatch_started_at: Option<DateTime<Utc>>,
    pub created_at: DateTime<Utc>,
    pub updated_at: DateTime<Utc>,
}

#[derive(Debug, Clone)]
pub struct CandidateReviewStateRow {
    pub operation_id: Uuid,
    pub project_scope_id: Uuid,
    pub scope_snapshot_id: Uuid,
    pub wave_run_id: Uuid,
    pub profile: String,
    pub review_closed: bool,
    pub wave_unit_count: i64,
    pub review_closed_unit_count: i64,
    pub candidate_count: i64,
    pub proposed_candidate_count: i64,
    pub barrier: CandidateReviewBarrierRow,
    pub candidates: Vec<CandidateReviewItemRow>,
}

#[derive(Debug, Clone)]
pub struct WaveReviewResult {
    pub state: CandidateReviewStateRow,
    pub approvals: Vec<AttackCandidateApprovalRow>,
    pub reopened_candidate_ids: Vec<Uuid>,
    pub replayed: bool,
}

#[derive(Debug, Clone)]
pub struct ReviewResumeClaim {
    pub operation_id: Uuid,
    pub project_scope_id: Uuid,
    pub scope_snapshot_id: Uuid,
    pub wave_run_id: Uuid,
    pub profile: String,
    pub session_id: Uuid,
    pub chat_session_key: String,
    pub dispatch_resume_version: i64,
    pub dispatch_required: bool,
    pub replayed: bool,
}

#[derive(Debug, Clone, sqlx::FromRow)]
struct ReviewAuthority {
    operation_id: Uuid,
    project_scope_id: Uuid,
    scope_snapshot_id: Uuid,
    wave_run_id: Uuid,
    profile: String,
    project_path_at_freeze: String,
    runtime_memory_contract: String,
    attack_execution_contract: String,
    investigation_rollout_mode: String,
    max_attempts_total: i32,
}

impl ReviewAuthority {
    fn investigation_mode(&self) -> crate::Result<golish_core::InvestigationRolloutMode> {
        golish_core::InvestigationRolloutMode::try_from(self.investigation_rollout_mode.as_str())
            .map_err(|error| review_error(ATTACK_REVIEW_SCOPE_MISMATCH, error.to_string()))
    }
}

#[derive(Debug, Clone, sqlx::FromRow)]
struct CandidateForReview {
    candidate_id: Uuid,
    wave_unit_id: Uuid,
    organization_id: Uuid,
    target_live_id: Option<Uuid>,
    target_type_at_time: String,
    target_value_at_time: String,
    target_identity_hash: String,
    hypothesis: String,
    technique: Option<String>,
    rationale: String,
    risk_class: String,
    candidate_plan_hash: String,
    source_work_item_id: Uuid,
    execution_plan: serde_json::Value,
    disposition: String,
    row_version: i64,
}

#[derive(Debug, Clone, sqlx::FromRow)]
struct ExpiringCandidateApproval {
    approval_id: Uuid,
    candidate_id: Uuid,
    wave_unit_id: Uuid,
    organization_id: Uuid,
    start_before: DateTime<Utc>,
    target_type_at_time: String,
    target_value_at_time: String,
    target_identity_hash: String,
    hypothesis: String,
    technique: Option<String>,
    candidate_plan_hash: String,
    source_work_item_id: Uuid,
}

fn candidate_shadow_source(
    candidate: &CandidateForReview,
    entity_version: i64,
    disposition: &str,
) -> super::hypothesis_legacy_projection::LegacyCandidateShadowSourceV1 {
    super::hypothesis_legacy_projection::LegacyCandidateShadowSourceV1 {
        entity_id: candidate.candidate_id,
        entity_version,
        organization_id: candidate.organization_id,
        source_work_item_id: candidate.source_work_item_id,
        target_type_at_time: candidate.target_type_at_time.clone(),
        target_value_at_time: candidate.target_value_at_time.clone(),
        target_identity_hash: candidate.target_identity_hash.clone(),
        hypothesis_hash: Some(super::attack_candidates::hypothesis_hash(
            &candidate.target_value_at_time,
            candidate.technique.as_deref(),
            &candidate.hypothesis,
        )),
        technique: candidate.technique.clone(),
        candidate_plan_hash: Some(candidate.candidate_plan_hash.clone()),
        disposition: disposition.to_owned(),
    }
}

fn expired_candidate_shadow_source(
    candidate: &ExpiringCandidateApproval,
    entity_version: i64,
) -> super::hypothesis_legacy_projection::LegacyCandidateShadowSourceV1 {
    super::hypothesis_legacy_projection::LegacyCandidateShadowSourceV1 {
        entity_id: candidate.candidate_id,
        entity_version,
        organization_id: candidate.organization_id,
        source_work_item_id: candidate.source_work_item_id,
        target_type_at_time: candidate.target_type_at_time.clone(),
        target_value_at_time: candidate.target_value_at_time.clone(),
        target_identity_hash: candidate.target_identity_hash.clone(),
        hypothesis_hash: Some(super::attack_candidates::hypothesis_hash(
            &candidate.target_value_at_time,
            candidate.technique.as_deref(),
            &candidate.hypothesis,
        )),
        technique: candidate.technique.clone(),
        candidate_plan_hash: Some(candidate.candidate_plan_hash.clone()),
        disposition: "proposed".to_owned(),
    }
}

#[derive(Debug, Clone, Copy)]
struct ReviewCounts {
    wave_unit_count: i64,
    review_ready_unit_count: i64,
    review_closed_unit_count: i64,
    candidate_count: i64,
    proposed_candidate_count: i64,
}

const APPROVAL_COLUMNS: &str = "id,candidate_id,operation_id,scope_snapshot_id,wave_run_id,\
    wave_unit_id,organization_id,target_live_id,target_type_at_time,target_value_at_time,\
    target_identity_hash,candidate_plan_hash,source_work_item_id,execution_plan,\
    allowed_capability_ids,allowed_action_kinds,budget,start_before,expires_at,decision_version,status,\
    decided_by,decided_at,row_version";
const BARRIER_COLUMNS: &str = "wave_run_id,operation_id,scope_snapshot_id,status,resume_version,\
    last_error,dispatch_started_at,created_at,updated_at";

fn review_error(code: &'static str, message: impl Into<String>) -> crate::DbError {
    crate::DbError::Other(anyhow::anyhow!(format!("{code}: {}", message.into())))
}

fn review_reservation_batch_fits(
    current: i64,
    fresh_approvals: usize,
    retryable_backlog: i64,
    cap: i32,
) -> bool {
    i64::try_from(fresh_approvals)
        .ok()
        .and_then(|fresh| current.checked_add(fresh))
        .and_then(|projected| projected.checked_add(retryable_backlog))
        .is_some_and(|projected| projected <= i64::from(cap))
}

pub fn stable_review_error_code(error: &crate::DbError) -> Option<&'static str> {
    let message = error.to_string();
    [
        ATTACK_CANDIDATE_PLAN_CHANGED,
        ATTACK_REVIEW_SCOPE_MISMATCH,
        ATTACK_APPROVAL_EXPIRED,
        ATTACK_REVIEW_ALREADY_CLOSED,
        ATTACK_RESUME_NOT_READY,
        ATTACK_ATTEMPT_FUEL_EXHAUSTED,
        super::attack_candidates::ATTACK_LEGACY_MUTATION_FORBIDDEN_BY_INVESTIGATION_CONTRACT,
    ]
    .into_iter()
    .find(|code| message.starts_with(code))
}

async fn lock_authority(
    tx: &mut Transaction<'_, Postgres>,
    operation_id: Uuid,
    wave_run_id: Uuid,
) -> crate::Result<ReviewAuthority> {
    super::attack_candidates::lock_and_require_legacy_candidate_mutation(tx, operation_id).await?;
    let authority = sqlx::query_as::<_, ReviewAuthority>(
        r#"SELECT operation.operation_id,
                  operation.project_scope_id,
                  snapshot.id AS scope_snapshot_id,
                  wave.id AS wave_run_id,
                  operation.profile,
                  snapshot.project_path_at_freeze,
                  operation.runtime_memory_contract,
                  operation.attack_execution_contract,
                  operation.investigation_rollout_mode,
                  wave.max_attempts_total
             FROM attack_wave_runs wave
             JOIN operation_state operation
               ON operation.operation_id=wave.operation_id
              AND operation.project_scope_id IS NOT NULL
             JOIN project_scopes project
               ON project.project_scope_id=operation.project_scope_id
              AND project.retired_at IS NULL
             JOIN operation_org_scope_snapshots snapshot
               ON snapshot.id=wave.scope_snapshot_id
              AND snapshot.operation_id=operation.operation_id
              AND snapshot.project_scope_id=operation.project_scope_id
              AND snapshot.sealed_at IS NOT NULL
            WHERE wave.id=$1 AND wave.operation_id=$2
            FOR UPDATE OF wave"#,
    )
    .bind(wave_run_id)
    .bind(operation_id)
    .fetch_optional(&mut **tx)
    .await?
    .ok_or_else(|| {
        review_error(
            ATTACK_REVIEW_SCOPE_MISMATCH,
            "operation/project/snapshot/wave authority did not match",
        )
    })?;
    if authority.attack_execution_contract == "legacy"
        || (authority.attack_execution_contract == "v2_only"
            && authority.runtime_memory_contract != "v2_only")
    {
        return Err(review_error(
            ATTACK_REVIEW_SCOPE_MISMATCH,
            "operation contracts do not authorize Candidate V2 review",
        ));
    }
    Ok(authority)
}

async fn lock_candidates(
    tx: &mut Transaction<'_, Postgres>,
    authority: &ReviewAuthority,
) -> crate::Result<Vec<CandidateForReview>> {
    sqlx::query_as::<_, CandidateForReview>(
        r#"SELECT candidate.candidate_id,candidate.wave_unit_id,
                  candidate.organization_id,candidate.target_live_id,
                  candidate.target_type_at_time,candidate.target_value_at_time,
                  candidate.target_identity_hash,candidate.hypothesis,
                  candidate.technique,candidate.rationale,candidate.risk_class,
                  candidate.candidate_plan_hash,candidate.source_work_item_id,
                  candidate.execution_plan,candidate.disposition,candidate.row_version
             FROM attack_candidates candidate
             JOIN attack_wave_units wave_unit
               ON wave_unit.id=candidate.wave_unit_id
              AND wave_unit.wave_run_id=candidate.wave_run_id
              AND wave_unit.operation_id=candidate.operation_uuid
              AND wave_unit.scope_snapshot_id=candidate.scope_snapshot_id
              AND wave_unit.organization_id=candidate.organization_id
             JOIN operation_org_scope_units scope_unit
               ON scope_unit.snapshot_id=candidate.scope_snapshot_id
              AND scope_unit.organization_id=candidate.organization_id
            WHERE candidate.operation_uuid=$1
              AND candidate.scope_snapshot_id=$2
              AND candidate.wave_run_id=$3
            ORDER BY candidate.organization_id,candidate.candidate_id
            FOR UPDATE OF candidate"#,
    )
    .bind(authority.operation_id)
    .bind(authority.scope_snapshot_id)
    .bind(authority.wave_run_id)
    .fetch_all(&mut **tx)
    .await
    .map_err(Into::into)
}

async fn latest_approval(
    tx: &mut Transaction<'_, Postgres>,
    candidate_id: Uuid,
) -> crate::Result<Option<AttackCandidateApprovalRow>> {
    let sql = format!(
        "SELECT {APPROVAL_COLUMNS} FROM attack_candidate_approvals
         WHERE candidate_id=$1 ORDER BY decision_version DESC LIMIT 1 FOR UPDATE"
    );
    sqlx::query_as(&sql)
        .bind(candidate_id)
        .fetch_optional(&mut **tx)
        .await
        .map_err(Into::into)
}

fn plan_review_fields(
    plan: &serde_json::Value,
) -> crate::Result<(Vec<String>, Vec<String>, serde_json::Value)> {
    if !plan
        .get("foreground_only")
        .and_then(serde_json::Value::as_bool)
        .unwrap_or(false)
    {
        return Err(review_error(
            ATTACK_CANDIDATE_PLAN_CHANGED,
            "Candidate plan is not foreground-only",
        ));
    }
    let actions = plan
        .get("actions")
        .and_then(serde_json::Value::as_array)
        .filter(|actions| !actions.is_empty())
        .ok_or_else(|| {
            review_error(
                ATTACK_CANDIDATE_PLAN_CHANGED,
                "Candidate plan actions are missing",
            )
        })?;
    let mut capability_ids = BTreeSet::new();
    let mut action_kinds = BTreeSet::new();
    for action in actions {
        let capability_id = action
            .get("capability_id")
            .and_then(serde_json::Value::as_str)
            .map(str::trim)
            .filter(|value| !value.is_empty())
            .ok_or_else(|| {
                review_error(
                    ATTACK_CANDIDATE_PLAN_CHANGED,
                    "Candidate capability id is missing",
                )
            })?;
        let action_kind = action
            .get("action_kind")
            .and_then(serde_json::Value::as_str)
            .map(str::trim)
            .filter(|value| !value.is_empty())
            .ok_or_else(|| {
                review_error(
                    ATTACK_CANDIDATE_PLAN_CHANGED,
                    "Candidate action kind is missing",
                )
            })?;
        capability_ids.insert(capability_id.to_string());
        action_kinds.insert(action_kind.to_string());
    }
    let budget = plan
        .get("budget")
        .filter(|budget| budget.is_object())
        .cloned()
        .ok_or_else(|| {
            review_error(
                ATTACK_CANDIDATE_PLAN_CHANGED,
                "Candidate plan budget is missing",
            )
        })?;
    Ok((
        capability_ids.into_iter().collect(),
        action_kinds.into_iter().collect(),
        budget,
    ))
}

async fn expire_unstarted_approvals(
    tx: &mut Transaction<'_, Postgres>,
    authority: &ReviewAuthority,
) -> crate::Result<Vec<Uuid>> {
    let expired = sqlx::query_as::<_, ExpiringCandidateApproval>(
        r#"SELECT approval.id AS approval_id,approval.candidate_id,
                  approval.wave_unit_id,approval.organization_id,approval.start_before,
                  candidate.target_type_at_time,candidate.target_value_at_time,
                  candidate.target_identity_hash,candidate.hypothesis,candidate.technique,
                  candidate.candidate_plan_hash,candidate.source_work_item_id
             FROM attack_candidate_approvals approval
             JOIN attack_candidates candidate
               ON candidate.candidate_id=approval.candidate_id
              AND candidate.operation_uuid=approval.operation_id
              AND candidate.scope_snapshot_id=approval.scope_snapshot_id
              AND candidate.wave_run_id=approval.wave_run_id
              AND candidate.wave_unit_id=approval.wave_unit_id
              AND candidate.organization_id=approval.organization_id
            WHERE approval.operation_id=$1 AND approval.scope_snapshot_id=$2
              AND approval.wave_run_id=$3 AND approval.status='approved'
              AND approval.start_before <= NOW()
              AND NOT EXISTS(
                    SELECT 1 FROM candidate_attempts attempt
                     WHERE attempt.approval_id=approval.id
                       AND attempt.status IN (
                           'queued','running','submitted','terminalization_pending'
                       )
                  )
            ORDER BY approval.candidate_id
            FOR UPDATE OF approval,candidate"#,
    )
    .bind(authority.operation_id)
    .bind(authority.scope_snapshot_id)
    .bind(authority.wave_run_id)
    .fetch_all(&mut **tx)
    .await?;
    let mut reopened = Vec::new();
    for candidate in expired {
        sqlx::query(
            "UPDATE attack_candidate_approvals
             SET status='expired',row_version=row_version+1
             WHERE id=$1 AND status='approved'",
        )
        .bind(candidate.approval_id)
        .execute(&mut **tx)
        .await?;
        let updated_version: Option<i64> = sqlx::query_scalar(
            r#"UPDATE attack_candidates
                  SET disposition='proposed',row_version=row_version+1,updated_at=NOW()
                WHERE candidate_id=$1 AND operation_uuid=$2 AND scope_snapshot_id=$3
                  AND wave_run_id=$4 AND wave_unit_id=$5 AND organization_id=$6
                  AND disposition='approved'
                RETURNING row_version"#,
        )
        .bind(candidate.candidate_id)
        .bind(authority.operation_id)
        .bind(authority.scope_snapshot_id)
        .bind(authority.wave_run_id)
        .bind(candidate.wave_unit_id)
        .bind(candidate.organization_id)
        .fetch_optional(&mut **tx)
        .await?;
        if let Some(entity_version) = updated_version {
            let stable_source_id = Uuid::new_v5(
                &candidate.approval_id,
                b"legacy-candidate-approval-expired.v1",
            );
            super::hypothesis_legacy_projection::append_legacy_candidate_shadow_batch_with_connection(
                tx,
                authority.investigation_mode()?,
                authority.operation_id,
                stable_source_id,
                candidate.start_before,
                vec![expired_candidate_shadow_source(
                    &candidate,
                    entity_version.saturating_add(1),
                )],
            )
            .await?;
            reopened.push(candidate.candidate_id);
        }
    }
    Ok(reopened)
}

async fn recompute_units_and_counts(
    tx: &mut Transaction<'_, Postgres>,
    authority: &ReviewAuthority,
) -> crate::Result<ReviewCounts> {
    sqlx::query(
        r#"UPDATE attack_wave_units wave_unit
              SET review_closed=NOT EXISTS(
                      SELECT 1 FROM attack_candidates candidate
                       WHERE candidate.operation_uuid=wave_unit.operation_id
                         AND candidate.scope_snapshot_id=wave_unit.scope_snapshot_id
                         AND candidate.wave_run_id=wave_unit.wave_run_id
                         AND candidate.wave_unit_id=wave_unit.id
                         AND candidate.organization_id=wave_unit.organization_id
                         AND candidate.disposition='proposed'
                  ),
                  status=CASE WHEN EXISTS(
                      SELECT 1 FROM attack_candidates candidate
                       WHERE candidate.operation_uuid=wave_unit.operation_id
                         AND candidate.scope_snapshot_id=wave_unit.scope_snapshot_id
                         AND candidate.wave_run_id=wave_unit.wave_run_id
                         AND candidate.wave_unit_id=wave_unit.id
                         AND candidate.organization_id=wave_unit.organization_id
                         AND candidate.disposition='proposed'
                  ) THEN 'review' ELSE 'verification' END,
                  row_version=row_version+1,updated_at=NOW()
            WHERE wave_unit.operation_id=$1 AND wave_unit.scope_snapshot_id=$2
              AND wave_unit.wave_run_id=$3 AND wave_unit.terminal_at IS NULL
              AND wave_unit.status IN ('review','verification')
              AND wave_unit.review_closed IS DISTINCT FROM NOT EXISTS(
                  SELECT 1 FROM attack_candidates candidate
                   WHERE candidate.operation_uuid=wave_unit.operation_id
                     AND candidate.scope_snapshot_id=wave_unit.scope_snapshot_id
                     AND candidate.wave_run_id=wave_unit.wave_run_id
                     AND candidate.wave_unit_id=wave_unit.id
                     AND candidate.organization_id=wave_unit.organization_id
                     AND candidate.disposition='proposed'
              )"#,
    )
    .bind(authority.operation_id)
    .bind(authority.scope_snapshot_id)
    .bind(authority.wave_run_id)
    .execute(&mut **tx)
    .await?;

    let (wave_unit_count, review_ready_unit_count, review_closed_unit_count): (i64, i64, i64) =
        sqlx::query_as(
            r#"SELECT COUNT(*),
                      COUNT(*) FILTER (WHERE status IN ('review','verification','terminal')),
                      COUNT(*) FILTER (WHERE review_closed)
                 FROM attack_wave_units
                WHERE operation_id=$1 AND scope_snapshot_id=$2 AND wave_run_id=$3"#,
        )
        .bind(authority.operation_id)
        .bind(authority.scope_snapshot_id)
        .bind(authority.wave_run_id)
        .fetch_one(&mut **tx)
        .await?;
    let (candidate_count, proposed_candidate_count): (i64, i64) = sqlx::query_as(
        r#"SELECT COUNT(*),COUNT(*) FILTER (WHERE disposition='proposed')
             FROM attack_candidates
            WHERE operation_uuid=$1 AND scope_snapshot_id=$2 AND wave_run_id=$3"#,
    )
    .bind(authority.operation_id)
    .bind(authority.scope_snapshot_id)
    .bind(authority.wave_run_id)
    .fetch_one(&mut **tx)
    .await?;
    Ok(ReviewCounts {
        wave_unit_count,
        review_ready_unit_count,
        review_closed_unit_count,
        candidate_count,
        proposed_candidate_count,
    })
}

async fn refresh_barrier(
    tx: &mut Transaction<'_, Postgres>,
    authority: &ReviewAuthority,
    stale_after: Duration,
) -> crate::Result<(CandidateReviewBarrierRow, ReviewCounts, Vec<Uuid>)> {
    let reopened = expire_unstarted_approvals(tx, authority).await?;
    let counts = recompute_units_and_counts(tx, authority).await?;
    if counts.wave_unit_count == 0 {
        return Err(review_error(
            ATTACK_REVIEW_SCOPE_MISMATCH,
            "Candidate wave has no scoped units",
        ));
    }
    sqlx::query(
        r#"INSERT INTO candidate_review_barriers(
               wave_run_id,operation_id,scope_snapshot_id,status)
           VALUES($1,$2,$3,'open') ON CONFLICT(wave_run_id) DO NOTHING"#,
    )
    .bind(authority.wave_run_id)
    .bind(authority.operation_id)
    .bind(authority.scope_snapshot_id)
    .execute(&mut **tx)
    .await?;
    let sql = format!(
        "SELECT {BARRIER_COLUMNS} FROM candidate_review_barriers
         WHERE wave_run_id=$1 AND operation_id=$2 AND scope_snapshot_id=$3 FOR UPDATE"
    );
    let mut barrier = sqlx::query_as::<_, CandidateReviewBarrierRow>(&sql)
        .bind(authority.wave_run_id)
        .bind(authority.operation_id)
        .bind(authority.scope_snapshot_id)
        .fetch_optional(&mut **tx)
        .await?
        .ok_or_else(|| {
            review_error(
                ATTACK_REVIEW_SCOPE_MISMATCH,
                "Candidate review barrier ownership drifted",
            )
        })?;
    let fully_reviewed = counts.review_ready_unit_count == counts.wave_unit_count
        && counts.review_closed_unit_count == counts.wave_unit_count
        && counts.proposed_candidate_count == 0;
    let stale_dispatch = barrier.status == "dispatching"
        && barrier
            .dispatch_started_at
            .map(|started| started <= Utc::now() - stale_after)
            .unwrap_or(true);
    let desired_status = if !fully_reviewed {
        Some("open")
    } else if stale_dispatch || barrier.status == "open" {
        Some("resume_pending")
    } else {
        None
    };
    if let Some(status) = desired_status.filter(|status| *status != barrier.status) {
        let update = format!(
            "UPDATE candidate_review_barriers
             SET status=$2,resume_version=resume_version+1,last_error=NULL,
                 dispatch_started_at=NULL,updated_at=NOW()
             WHERE wave_run_id=$1 AND operation_id=$3 AND scope_snapshot_id=$4
             RETURNING {BARRIER_COLUMNS}"
        );
        barrier = sqlx::query_as(&update)
            .bind(authority.wave_run_id)
            .bind(status)
            .bind(authority.operation_id)
            .bind(authority.scope_snapshot_id)
            .fetch_one(&mut **tx)
            .await?;
    }
    let wave_status = if counts.review_ready_unit_count != counts.wave_unit_count {
        "open"
    } else if fully_reviewed {
        "verification"
    } else {
        "review"
    };
    sqlx::query(
        r#"UPDATE attack_wave_runs SET status=$2,row_version=row_version+1,updated_at=NOW()
            WHERE id=$1 AND operation_id=$3 AND scope_snapshot_id=$4
              AND terminal_at IS NULL AND status<>$2"#,
    )
    .bind(authority.wave_run_id)
    .bind(wave_status)
    .bind(authority.operation_id)
    .bind(authority.scope_snapshot_id)
    .execute(&mut **tx)
    .await?;
    Ok((barrier, counts, reopened))
}

/// Recompute the durable review barrier after a recovery transaction safely
/// abandons an unstarted Attempt and moves its Candidate back to `proposed`.
/// Keeping this wrapper in the review repository prevents recovery code from
/// duplicating Wave/barrier state-machine rules.
pub(super) async fn reopen_review_after_candidate_abandon(
    tx: &mut Transaction<'_, Postgres>,
    operation_id: Uuid,
    scope_snapshot_id: Uuid,
    wave_run_id: Uuid,
) -> crate::Result<()> {
    let authority = lock_authority(tx, operation_id, wave_run_id).await?;
    if authority.scope_snapshot_id != scope_snapshot_id {
        return Err(review_error(
            ATTACK_REVIEW_SCOPE_MISMATCH,
            "Candidate recovery snapshot authority drifted",
        ));
    }
    refresh_barrier(
        tx,
        &authority,
        Duration::seconds(DEFAULT_REVIEW_DISPATCH_STALE_SECONDS),
    )
    .await?;
    Ok(())
}

fn exact_decision_replay(
    candidate: &CandidateForReview,
    decision: &CandidateReviewDecision,
    approval: &AttackCandidateApprovalRow,
    allowed_capability_ids: &[String],
    allowed_action_kinds: &[String],
    budget: &serde_json::Value,
) -> bool {
    let expected_status = if decision.approve {
        "approved"
    } else {
        "rejected"
    };
    let expected_start_before = decision
        .start_before
        .as_ref()
        .or(decision.expires_at.as_ref());
    let temporal_authority_matches = !decision.approve
        || (expected_start_before == Some(&approval.start_before)
            && decision.expires_at.as_ref() == Some(&approval.expires_at));
    candidate.row_version == decision.expected_candidate_row_version.saturating_add(1)
        && candidate.disposition == expected_status
        && candidate.candidate_plan_hash == decision.expected_candidate_plan_hash
        && approval.status == expected_status
        && approval.candidate_plan_hash == decision.expected_candidate_plan_hash
        && approval.execution_plan == candidate.execution_plan
        && approval.allowed_capability_ids == allowed_capability_ids
        && approval.allowed_action_kinds == allowed_action_kinds
        && approval.budget == *budget
        && temporal_authority_matches
}

pub async fn review_wave_candidates(
    pool: &PgPool,
    command: ReviewCandidateBatch,
) -> crate::Result<WaveReviewResult> {
    if command.decisions.is_empty() || command.decisions.len() > 100 {
        return Err(review_error(
            ATTACK_REVIEW_SCOPE_MISMATCH,
            "Candidate review requires one bounded exact decision batch",
        ));
    }
    let mut tx = pool.begin().await?;
    let authority = lock_authority(&mut tx, command.operation_id, command.wave_run_id).await?;
    let (_before_barrier, before_counts, reopened_candidate_ids) = refresh_barrier(
        &mut tx,
        &authority,
        Duration::seconds(DEFAULT_REVIEW_DISPATCH_STALE_SECONDS),
    )
    .await?;
    if before_counts.review_ready_unit_count != before_counts.wave_unit_count {
        return Err(review_error(
            ATTACK_RESUME_NOT_READY,
            "Candidate reasoning has not reached review for every scoped WaveUnit",
        ));
    }
    let candidates = lock_candidates(&mut tx, &authority).await?;
    let by_id = candidates
        .iter()
        .map(|candidate| (candidate.candidate_id, candidate))
        .collect::<BTreeMap<_, _>>();
    let proposed_ids = candidates
        .iter()
        .filter(|candidate| candidate.disposition == "proposed")
        .map(|candidate| candidate.candidate_id)
        .collect::<BTreeSet<_>>();
    let provided_ids = command
        .decisions
        .iter()
        .map(|decision| decision.candidate_id)
        .collect::<BTreeSet<_>>();
    if provided_ids.len() != command.decisions.len()
        || (!proposed_ids.is_empty() && proposed_ids != provided_ids)
    {
        return Err(review_error(
            ATTACK_REVIEW_SCOPE_MISMATCH,
            "review decisions must exactly cover the current proposed Candidate snapshot",
        ));
    }
    let fresh_approval_count = if proposed_ids.is_empty() {
        0
    } else {
        command
            .decisions
            .iter()
            .filter(|decision| decision.approve)
            .count()
    };
    let (effective_attempt_fuel, retryable_backlog): (i64, i64) = sqlx::query_as(
        r#"SELECT
               (SELECT COUNT(*) FROM candidate_attempts
                 WHERE operation_id=$1)
               +
               (SELECT COUNT(*) FROM attack_candidates AS candidate
                 WHERE candidate.operation_uuid=$1
                   AND candidate.disposition='approved'
                   AND NOT EXISTS (
                       SELECT 1 FROM candidate_attempts AS attempt
                        WHERE attempt.candidate_id=candidate.candidate_id
                   )),
               (SELECT COUNT(*) FROM attack_candidates AS candidate
                 WHERE candidate.operation_uuid=$1
                   AND candidate.disposition='approved'
                   AND (
                       SELECT latest.status
                         FROM candidate_attempts AS latest
                        WHERE latest.candidate_id=candidate.candidate_id
                        ORDER BY latest.ordinal DESC
                        LIMIT 1
                   )='retryable_failed')"#,
    )
    .bind(authority.operation_id)
    .fetch_one(&mut *tx)
    .await?;
    if !review_reservation_batch_fits(
        effective_attempt_fuel,
        fresh_approval_count,
        retryable_backlog,
        authority.max_attempts_total,
    ) {
        return Err(review_error(
            ATTACK_ATTEMPT_FUEL_EXHAUSTED,
            "review batch cannot reserve every approved Candidate first Attempt",
        ));
    }
    let operator_id: Uuid = sqlx::query_scalar(
        "SELECT id FROM operator_principals
         WHERE principal_kind='local_operator' AND active FOR SHARE",
    )
    .fetch_optional(&mut *tx)
    .await?
    .ok_or_else(|| {
        review_error(
            ATTACK_REVIEW_SCOPE_MISMATCH,
            "server-owned local operator principal is unavailable",
        )
    })?;
    let mut approvals = Vec::with_capacity(command.decisions.len());
    let replay_only = proposed_ids.is_empty();
    let mut replayed_count = 0usize;
    for decision in &command.decisions {
        let candidate = by_id.get(&decision.candidate_id).copied().ok_or_else(|| {
            review_error(
                ATTACK_REVIEW_SCOPE_MISMATCH,
                "Candidate does not belong to the exact operation/project/snapshot/wave",
            )
        })?;
        let (allowed_capability_ids, allowed_action_kinds, budget) =
            plan_review_fields(&candidate.execution_plan)?;
        let latest = latest_approval(&mut tx, candidate.candidate_id).await?;
        if let Some(approval) = latest.as_ref().filter(|approval| {
            exact_decision_replay(
                candidate,
                decision,
                approval,
                &allowed_capability_ids,
                &allowed_action_kinds,
                &budget,
            )
        }) {
            super::hypothesis_legacy_projection::append_legacy_candidate_shadow_batch_with_connection(
                &mut tx,
                authority.investigation_mode()?,
                authority.operation_id,
                approval.id,
                approval.decided_at,
                vec![candidate_shadow_source(
                    candidate,
                    candidate.row_version.saturating_add(1),
                    &candidate.disposition,
                )],
            )
            .await?;
            approvals.push(approval.clone());
            replayed_count += 1;
            continue;
        }
        if replay_only {
            return Err(review_error(
                ATTACK_REVIEW_ALREADY_CLOSED,
                "review is already closed and the request is not an exact replay",
            ));
        }
        if candidate.candidate_plan_hash != decision.expected_candidate_plan_hash
            || candidate.row_version != decision.expected_candidate_row_version
        {
            return Err(review_error(
                ATTACK_CANDIDATE_PLAN_CHANGED,
                "Candidate plan hash or row version changed",
            ));
        }
        if candidate.disposition != "proposed" {
            return Err(review_error(
                ATTACK_REVIEW_ALREADY_CLOSED,
                "Candidate is no longer proposed",
            ));
        }
        let now = Utc::now();
        let (start_before, expires_at) = if decision.approve {
            let expires_at = decision
                .expires_at
                .filter(|expires| *expires > now)
                .ok_or_else(|| {
                    review_error(
                        ATTACK_APPROVAL_EXPIRED,
                        "approved Candidate requires a future approval expiry",
                    )
                })?;
            let start_before = decision
                .start_before
                .or(Some(expires_at))
                .filter(|start_before| *start_before > now && *start_before <= expires_at)
                .ok_or_else(|| {
                    review_error(
                        ATTACK_APPROVAL_EXPIRED,
                        "approved Candidate requires a future start-before no later than expiry",
                    )
                })?;
            (start_before, expires_at)
        } else {
            let expires_at = decision.expires_at.unwrap_or(now);
            let start_before = decision.start_before.unwrap_or(expires_at);
            (start_before.min(expires_at), expires_at)
        };
        let decision_version: i64 = sqlx::query_scalar(
            "SELECT COALESCE(MAX(decision_version),0)+1
             FROM attack_candidate_approvals WHERE candidate_id=$1",
        )
        .bind(candidate.candidate_id)
        .fetch_one(&mut *tx)
        .await?;
        let status = if decision.approve {
            "approved"
        } else {
            "rejected"
        };
        let sql = format!(
            "INSERT INTO attack_candidate_approvals(
                 id,candidate_id,operation_id,scope_snapshot_id,wave_run_id,wave_unit_id,
                 organization_id,target_live_id,target_type_at_time,target_value_at_time,
                 target_identity_hash,candidate_plan_hash,source_work_item_id,execution_plan,
                 allowed_capability_ids,allowed_action_kinds,budget,start_before,expires_at,decision_version,
                 status,decided_by)
             VALUES($1,$2,$3,$4,$5,$6,$7,$8,$9,$10,$11,$12,$13,$14,$15,$16,$17,$18,$19,$20,$21,$22)
             RETURNING {APPROVAL_COLUMNS}"
        );
        let approval = sqlx::query_as::<_, AttackCandidateApprovalRow>(&sql)
            .bind(Uuid::new_v4())
            .bind(candidate.candidate_id)
            .bind(authority.operation_id)
            .bind(authority.scope_snapshot_id)
            .bind(authority.wave_run_id)
            .bind(candidate.wave_unit_id)
            .bind(candidate.organization_id)
            .bind(candidate.target_live_id)
            .bind(&candidate.target_type_at_time)
            .bind(&candidate.target_value_at_time)
            .bind(&candidate.target_identity_hash)
            .bind(&candidate.candidate_plan_hash)
            .bind(candidate.source_work_item_id)
            .bind(&candidate.execution_plan)
            .bind(&allowed_capability_ids)
            .bind(&allowed_action_kinds)
            .bind(&budget)
            .bind(start_before)
            .bind(expires_at)
            .bind(decision_version)
            .bind(status)
            .bind(operator_id)
            .fetch_one(&mut *tx)
            .await?;
        let updated_version: Option<i64> = sqlx::query_scalar(
            r#"UPDATE attack_candidates SET disposition=$2,row_version=row_version+1,
                       updated_at=NOW()
                WHERE candidate_id=$1 AND operation_uuid=$3 AND scope_snapshot_id=$4
                  AND wave_run_id=$5 AND wave_unit_id=$6 AND organization_id=$7
                  AND candidate_plan_hash=$8 AND row_version=$9 AND disposition='proposed'
                RETURNING row_version"#,
        )
        .bind(candidate.candidate_id)
        .bind(status)
        .bind(authority.operation_id)
        .bind(authority.scope_snapshot_id)
        .bind(authority.wave_run_id)
        .bind(candidate.wave_unit_id)
        .bind(candidate.organization_id)
        .bind(&candidate.candidate_plan_hash)
        .bind(decision.expected_candidate_row_version)
        .fetch_optional(&mut *tx)
        .await?;
        let Some(entity_version) = updated_version else {
            return Err(review_error(
                ATTACK_CANDIDATE_PLAN_CHANGED,
                "Candidate review row-version CAS was lost",
            ));
        };
        super::hypothesis_legacy_projection::append_legacy_candidate_shadow_batch_with_connection(
            &mut tx,
            authority.investigation_mode()?,
            authority.operation_id,
            approval.id,
            approval.decided_at,
            vec![candidate_shadow_source(
                candidate,
                entity_version.saturating_add(1),
                status,
            )],
        )
        .await?;
        approvals.push(approval);
    }
    let (barrier, counts, additionally_reopened) = refresh_barrier(
        &mut tx,
        &authority,
        Duration::seconds(DEFAULT_REVIEW_DISPATCH_STALE_SECONDS),
    )
    .await?;
    let mut reopened_candidate_ids = reopened_candidate_ids;
    reopened_candidate_ids.extend(additionally_reopened);
    reopened_candidate_ids.sort_unstable();
    reopened_candidate_ids.dedup();
    let candidates = load_review_items(&mut tx, &authority).await?;
    let state = state_from_parts(&authority, counts, barrier, candidates);
    tx.commit().await?;
    Ok(WaveReviewResult {
        state,
        approvals,
        reopened_candidate_ids,
        replayed: replayed_count == command.decisions.len(),
    })
}

async fn load_review_items(
    tx: &mut Transaction<'_, Postgres>,
    authority: &ReviewAuthority,
) -> crate::Result<Vec<CandidateReviewItemRow>> {
    let candidates = lock_candidates(tx, authority).await?;
    let mut items = Vec::with_capacity(candidates.len());
    for candidate in candidates {
        let latest_approval = latest_approval(tx, candidate.candidate_id).await?;
        let live_target_present: bool = if let Some(target_id) = candidate.target_live_id {
            sqlx::query_scalar(
                r#"SELECT EXISTS(
                       SELECT 1 FROM targets target
                        WHERE target.id=$1 AND target.organization_id=$2
                          AND target.scope='in' AND target.project_path=$3
                          AND target.value=$4 AND LOWER(target.target_type::TEXT)=LOWER($5)
                   )"#,
            )
            .bind(target_id)
            .bind(candidate.organization_id)
            .bind(&authority.project_path_at_freeze)
            .bind(&candidate.target_value_at_time)
            .bind(&candidate.target_type_at_time)
            .fetch_one(&mut **tx)
            .await?
        } else {
            false
        };
        items.push(CandidateReviewItemRow {
            candidate_id: candidate.candidate_id,
            wave_unit_id: candidate.wave_unit_id,
            organization_id: candidate.organization_id,
            target_live_id: candidate.target_live_id,
            live_target_present,
            target_type_at_time: candidate.target_type_at_time,
            target_value_at_time: candidate.target_value_at_time,
            target_identity_hash: candidate.target_identity_hash,
            hypothesis: candidate.hypothesis,
            technique: candidate.technique,
            rationale: candidate.rationale,
            risk_class: candidate.risk_class,
            execution_plan: candidate.execution_plan,
            candidate_plan_hash: candidate.candidate_plan_hash,
            disposition: candidate.disposition,
            row_version: candidate.row_version,
            latest_approval,
        });
    }
    Ok(items)
}

fn state_from_parts(
    authority: &ReviewAuthority,
    counts: ReviewCounts,
    barrier: CandidateReviewBarrierRow,
    candidates: Vec<CandidateReviewItemRow>,
) -> CandidateReviewStateRow {
    CandidateReviewStateRow {
        operation_id: authority.operation_id,
        project_scope_id: authority.project_scope_id,
        scope_snapshot_id: authority.scope_snapshot_id,
        wave_run_id: authority.wave_run_id,
        profile: authority.profile.clone(),
        review_closed: counts.review_ready_unit_count == counts.wave_unit_count
            && counts.review_closed_unit_count == counts.wave_unit_count
            && counts.proposed_candidate_count == 0,
        wave_unit_count: counts.wave_unit_count,
        review_closed_unit_count: counts.review_closed_unit_count,
        candidate_count: counts.candidate_count,
        proposed_candidate_count: counts.proposed_candidate_count,
        barrier,
        candidates,
    }
}

pub async fn list_candidate_reviews(
    pool: &PgPool,
    operation_id: Uuid,
    wave_run_id: Uuid,
) -> crate::Result<CandidateReviewStateRow> {
    let mut tx = pool.begin().await?;
    let authority = lock_authority(&mut tx, operation_id, wave_run_id).await?;
    let (barrier, counts, _) = refresh_barrier(
        &mut tx,
        &authority,
        Duration::seconds(DEFAULT_REVIEW_DISPATCH_STALE_SECONDS),
    )
    .await?;
    let candidates = load_review_items(&mut tx, &authority).await?;
    let state = state_from_parts(&authority, counts, barrier, candidates);
    tx.commit().await?;
    Ok(state)
}

pub async fn review_barrier_for_operation(
    pool: &PgPool,
    operation_id: Uuid,
) -> crate::Result<CandidateReviewStateRow> {
    let wave_run_id: Uuid = sqlx::query_scalar(
        r#"SELECT id FROM attack_wave_runs
            WHERE operation_id=$1 AND status<>'terminal'
            ORDER BY generation DESC LIMIT 1"#,
    )
    .bind(operation_id)
    .fetch_optional(pool)
    .await?
    .ok_or_else(|| {
        review_error(
            ATTACK_REVIEW_SCOPE_MISMATCH,
            "active Candidate wave is missing",
        )
    })?;
    list_candidate_reviews(pool, operation_id, wave_run_id).await
}

pub async fn claim_candidate_review_resume(
    pool: &PgPool,
    operation_id: Uuid,
    wave_run_id: Uuid,
    expected_resume_version: i64,
) -> crate::Result<ReviewResumeClaim> {
    let mut tx = pool.begin().await?;
    let authority = lock_authority(&mut tx, operation_id, wave_run_id).await?;
    let (barrier, counts, _) = refresh_barrier(
        &mut tx,
        &authority,
        Duration::seconds(DEFAULT_REVIEW_DISPATCH_STALE_SECONDS),
    )
    .await?;
    let (session_id, chat_session_key): (Uuid, Option<String>) = sqlx::query_as(
        r#"SELECT task.session_id,session.chat_session_key
             FROM tasks task JOIN sessions session ON session.id=task.session_id
            WHERE task.id=$1"#,
    )
    .bind(operation_id)
    .fetch_optional(&mut *tx)
    .await?
    .ok_or_else(|| {
        review_error(
            ATTACK_REVIEW_SCOPE_MISMATCH,
            "operation task/session ownership is missing",
        )
    })?;
    let chat_session_key = chat_session_key
        .filter(|key| !key.trim().is_empty())
        .ok_or_else(|| {
            review_error(
                ATTACK_RESUME_NOT_READY,
                "operation has no durable chat-session key",
            )
        })?;
    let fully_reviewed = counts.review_ready_unit_count == counts.wave_unit_count
        && counts.review_closed_unit_count == counts.wave_unit_count
        && counts.proposed_candidate_count == 0;
    if !fully_reviewed {
        return Err(review_error(
            ATTACK_RESUME_NOT_READY,
            "Candidate review DB snapshot is still open",
        ));
    }
    if matches!(barrier.status.as_str(), "resumed" | "terminal") {
        tx.commit().await?;
        return Ok(ReviewResumeClaim {
            operation_id,
            project_scope_id: authority.project_scope_id,
            scope_snapshot_id: authority.scope_snapshot_id,
            wave_run_id,
            profile: authority.profile,
            session_id,
            chat_session_key,
            dispatch_resume_version: barrier.resume_version,
            dispatch_required: false,
            replayed: true,
        });
    }
    if barrier.status != "resume_pending" || barrier.resume_version != expected_resume_version {
        return Err(review_error(
            ATTACK_RESUME_NOT_READY,
            "resume barrier status or version is not ready",
        ));
    }
    let sql = format!(
        "UPDATE candidate_review_barriers
         SET status='dispatching',resume_version=resume_version+1,
             dispatch_started_at=NOW(),last_error=NULL,updated_at=NOW()
         WHERE wave_run_id=$1 AND operation_id=$2 AND scope_snapshot_id=$3
           AND status='resume_pending' AND resume_version=$4
         RETURNING {BARRIER_COLUMNS}"
    );
    let dispatch = sqlx::query_as::<_, CandidateReviewBarrierRow>(&sql)
        .bind(wave_run_id)
        .bind(operation_id)
        .bind(authority.scope_snapshot_id)
        .bind(expected_resume_version)
        .fetch_optional(&mut *tx)
        .await?
        .ok_or_else(|| review_error(ATTACK_RESUME_NOT_READY, "resume dispatch CAS was lost"))?;
    tx.commit().await?;
    Ok(ReviewResumeClaim {
        operation_id,
        project_scope_id: authority.project_scope_id,
        scope_snapshot_id: authority.scope_snapshot_id,
        wave_run_id,
        profile: authority.profile,
        session_id,
        chat_session_key,
        dispatch_resume_version: dispatch.resume_version,
        dispatch_required: true,
        replayed: false,
    })
}

pub async fn mark_candidate_review_resumed(
    pool: &PgPool,
    claim: &ReviewResumeClaim,
) -> crate::Result<CandidateReviewBarrierRow> {
    let mut tx = pool.begin().await?;
    super::attack_candidates::lock_and_require_legacy_candidate_mutation(
        &mut tx,
        claim.operation_id,
    )
    .await?;
    let sql = format!(
        "UPDATE candidate_review_barriers
         SET status='resumed',resume_version=resume_version+1,
             dispatch_started_at=NULL,last_error=NULL,updated_at=NOW()
         WHERE wave_run_id=$1 AND operation_id=$2 AND scope_snapshot_id=$3
           AND status='dispatching' AND resume_version=$4
         RETURNING {BARRIER_COLUMNS}"
    );
    let barrier = sqlx::query_as(&sql)
        .bind(claim.wave_run_id)
        .bind(claim.operation_id)
        .bind(claim.scope_snapshot_id)
        .bind(claim.dispatch_resume_version)
        .fetch_optional(&mut *tx)
        .await?
        .ok_or_else(|| review_error(ATTACK_RESUME_NOT_READY, "resume completion CAS was lost"))?;
    tx.commit().await?;
    Ok(barrier)
}

pub async fn mark_candidate_review_resume_failed(
    pool: &PgPool,
    claim: &ReviewResumeClaim,
    error: &str,
) -> crate::Result<CandidateReviewBarrierRow> {
    let mut tx = pool.begin().await?;
    super::attack_candidates::lock_and_require_legacy_candidate_mutation(
        &mut tx,
        claim.operation_id,
    )
    .await?;
    let last_error = error.chars().take(2048).collect::<String>();
    let sql = format!(
        "UPDATE candidate_review_barriers
         SET status='resume_pending',resume_version=resume_version+1,
             dispatch_started_at=NULL,last_error=$5,updated_at=NOW()
         WHERE wave_run_id=$1 AND operation_id=$2 AND scope_snapshot_id=$3
           AND status='dispatching' AND resume_version=$4
         RETURNING {BARRIER_COLUMNS}"
    );
    let barrier = sqlx::query_as(&sql)
        .bind(claim.wave_run_id)
        .bind(claim.operation_id)
        .bind(claim.scope_snapshot_id)
        .bind(claim.dispatch_resume_version)
        .bind(last_error)
        .fetch_optional(&mut *tx)
        .await?
        .ok_or_else(|| review_error(ATTACK_RESUME_NOT_READY, "resume failure CAS was lost"))?;
    tx.commit().await?;
    Ok(barrier)
}

pub async fn reap_stale_candidate_review_dispatches(
    pool: &PgPool,
    stale_after: Duration,
) -> crate::Result<u64> {
    let stale_interval = format!("{} seconds", stale_after.num_seconds().max(1));
    let operation_ids: Vec<Uuid> = sqlx::query_scalar(
        r#"SELECT DISTINCT operation_id
             FROM candidate_review_barriers
            WHERE status='dispatching'
              AND (dispatch_started_at IS NULL OR dispatch_started_at <= NOW()-$1::interval)
            ORDER BY operation_id"#,
    )
    .bind(&stale_interval)
    .fetch_all(pool)
    .await?;

    let mut reaped = 0_u64;
    for operation_id in operation_ids {
        let mut tx = pool.begin().await?;
        let mode =
            super::attack_candidates::lock_frozen_investigation_rollout_mode(&mut tx, operation_id)
                .await?;
        if !stale_dispatch_reaper_allows(mode) {
            tx.rollback().await?;
            continue;
        }
        let result = sqlx::query(
            r#"UPDATE candidate_review_barriers
                  SET status='resume_pending',resume_version=resume_version+1,
                      dispatch_started_at=NULL,
                      last_error='stale dispatch reclaimed after process restart',updated_at=NOW()
                WHERE operation_id=$1 AND status='dispatching'
                  AND (dispatch_started_at IS NULL OR dispatch_started_at <= NOW()-$2::interval)"#,
        )
        .bind(operation_id)
        .bind(&stale_interval)
        .execute(&mut *tx)
        .await?;
        tx.commit().await?;
        reaped = reaped.saturating_add(result.rows_affected());
    }
    Ok(reaped)
}

fn stale_dispatch_reaper_allows(mode: golish_core::InvestigationRolloutMode) -> bool {
    mode.policy().allow_legacy_mutation
}

#[cfg(test)]
mod fuel_tests {
    use super::{review_reservation_batch_fits, stale_dispatch_reaper_allows};
    use golish_core::InvestigationRolloutMode;

    #[test]
    fn review_approval_batch_reserves_every_first_attempt_atomically() {
        assert!(review_reservation_batch_fits(1, 1, 1, 3));
        assert!(!review_reservation_batch_fits(1, 2, 1, 3));
        assert!(review_reservation_batch_fits(3, 0, 0, 3));
        assert!(!review_reservation_batch_fits(2, 0, 2, 3));
        assert!(!review_reservation_batch_fits(i64::MAX, 1, 0, i32::MAX));
    }

    #[test]
    fn startup_stale_dispatch_reaper_uses_the_frozen_five_mode_policy() {
        for mode in InvestigationRolloutMode::ALL {
            assert_eq!(
                stale_dispatch_reaper_allows(mode),
                mode.policy().allow_legacy_mutation,
                "reaper policy drift for {}",
                mode.as_str()
            );
        }
    }
}
