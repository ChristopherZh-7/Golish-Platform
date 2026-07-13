//! Durable Candidate review command surface.
//!
//! Requests intentionally omit actor, project, scope snapshot, organization,
//! WaveUnit, execution plan, capability, action and budget authority. The DB
//! derives and verifies those identities from `operation_id + wave_run_id` and
//! the server resolves the opaque local operator principal.

use chrono::{DateTime, Utc};
use golish_app_core::domain::operator::OperatorChannel;
use golish_db::repo::attack_candidate_approvals as review_repo;
use serde::{Deserialize, Serialize};
use tauri::State;
use ts_rs::TS;
use uuid::Uuid;

use crate::state::AgentState;

#[derive(Debug, Clone, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct AttackReviewCommandError {
    pub code: String,
    pub message: String,
}

impl AttackReviewCommandError {
    fn new(code: impl Into<String>, message: impl Into<String>) -> Self {
        Self {
            code: code.into(),
            message: message.into(),
        }
    }

    fn from_db(error: golish_db::DbError) -> Self {
        let code = review_repo::stable_review_error_code(&error).unwrap_or(match &error {
            golish_db::DbError::Sqlx(_) => "DATABASE",
            _ => "INTERNAL",
        });
        Self::new(code, error.to_string())
    }
}

impl From<crate::error::GolishError> for AttackReviewCommandError {
    fn from(error: crate::error::GolishError) -> Self {
        Self::new(error.code(), error.to_string())
    }
}

#[derive(Debug, Clone, Serialize, Deserialize, TS)]
#[serde(rename_all = "camelCase", deny_unknown_fields)]
#[ts(export, export_to = "../../../../frontend/lib/generated/")]
pub struct AttackCandidateReviewScopeRequest {
    pub operation_id: String,
    pub wave_run_id: String,
}

#[derive(Debug, Clone, Serialize, Deserialize, TS)]
#[serde(rename_all = "camelCase", deny_unknown_fields)]
#[ts(export, export_to = "../../../../frontend/lib/generated/")]
pub struct AttackCandidateReviewDecisionRequest {
    pub candidate_id: String,
    pub candidate_plan_hash: String,
    #[ts(type = "number")]
    pub expected_row_version: i64,
    // `approve` | `reject`.
    pub decision: String,
    // Required and future-dated for `approve`; omitted for `reject`.
    pub expires_at: Option<String>,
}

#[derive(Debug, Clone, Serialize, Deserialize, TS)]
#[serde(rename_all = "camelCase", deny_unknown_fields)]
#[ts(export, export_to = "../../../../frontend/lib/generated/")]
pub struct AttackCandidateReviewRequest {
    pub operation_id: String,
    pub wave_run_id: String,
    pub decisions: Vec<AttackCandidateReviewDecisionRequest>,
}

#[derive(Debug, Clone, Serialize, Deserialize, TS)]
#[serde(rename_all = "camelCase", deny_unknown_fields)]
#[ts(export, export_to = "../../../../frontend/lib/generated/")]
pub struct AttackCandidateResumeRequest {
    pub operation_id: String,
    pub wave_run_id: String,
    #[ts(type = "number")]
    pub expected_resume_version: i64,
}

#[derive(Debug, Clone, Serialize, TS)]
#[serde(rename_all = "camelCase")]
#[ts(export, export_to = "../../../../frontend/lib/generated/")]
pub struct AttackCandidateApprovalView {
    pub approval_id: String,
    pub status: String,
    pub expires_at: String,
    #[ts(type = "number")]
    pub decision_version: i64,
    #[ts(type = "number")]
    pub row_version: i64,
    pub decided_at: String,
}

#[derive(Debug, Clone, Serialize, TS)]
#[serde(rename_all = "camelCase")]
#[ts(export, export_to = "../../../../frontend/lib/generated/")]
pub struct AttackCandidateReviewItem {
    pub candidate_id: String,
    pub wave_unit_id: String,
    pub organization_id: String,
    pub target_live_id: Option<String>,
    pub live_target_present: bool,
    pub target_type_at_time: String,
    pub target_value_at_time: String,
    pub target_identity_hash: String,
    pub hypothesis: String,
    pub technique: Option<String>,
    pub rationale: String,
    pub risk_class: String,
    #[ts(type = "unknown")]
    pub execution_plan: serde_json::Value,
    pub candidate_plan_hash: String,
    pub disposition: String,
    #[ts(type = "number")]
    pub row_version: i64,
    pub latest_approval: Option<AttackCandidateApprovalView>,
}

#[derive(Debug, Clone, Serialize, TS)]
#[serde(rename_all = "camelCase")]
#[ts(export, export_to = "../../../../frontend/lib/generated/")]
pub struct AttackCandidateReviewState {
    pub operation_id: String,
    pub project_scope_id: String,
    pub scope_snapshot_id: String,
    pub wave_run_id: String,
    pub profile: String,
    pub review_closed: bool,
    pub status: String,
    #[ts(type = "number")]
    pub resume_version: i64,
    pub last_error: Option<String>,
    #[ts(type = "number")]
    pub wave_unit_count: i64,
    #[ts(type = "number")]
    pub review_closed_unit_count: i64,
    #[ts(type = "number")]
    pub candidate_count: i64,
    #[ts(type = "number")]
    pub proposed_candidate_count: i64,
    pub candidates: Vec<AttackCandidateReviewItem>,
}

#[derive(Debug, Clone, Serialize, TS)]
#[serde(rename_all = "camelCase")]
#[ts(export, export_to = "../../../../frontend/lib/generated/")]
pub struct AttackCandidateReviewResponse {
    pub state: AttackCandidateReviewState,
    pub replayed: bool,
    #[ts(type = "number")]
    pub approvals_written: i64,
}

#[derive(Debug, Clone, Serialize, TS)]
#[serde(rename_all = "camelCase")]
#[ts(export, export_to = "../../../../frontend/lib/generated/")]
pub struct CandidateAttemptRow {
    pub attempt_id: String,
    pub candidate_id: String,
    pub approval_id: String,
    pub organization_id: String,
    pub target_live_id: Option<String>,
    pub target_type_at_time: String,
    pub target_value_at_time: String,
    pub target_identity_hash: String,
    pub candidate_plan_hash: String,
    #[ts(type = "number")]
    pub ordinal: i32,
    pub status: String,
    pub stage_worker_run_id: Option<String>,
    #[ts(type = "unknown | null")]
    pub result: Option<serde_json::Value>,
    pub result_hash: Option<String>,
    #[ts(type = "number")]
    pub row_version: i64,
    pub created_at: String,
    pub updated_at: String,
    pub terminal_at: Option<String>,
}

fn parse_uuid(value: &str, field: &str) -> Result<Uuid, AttackReviewCommandError> {
    Uuid::parse_str(value).map_err(|_| {
        AttackReviewCommandError::new(
            review_repo::ATTACK_REVIEW_SCOPE_MISMATCH,
            format!("invalid {field}"),
        )
    })
}

fn parse_expiry(value: Option<&str>) -> Result<Option<DateTime<Utc>>, AttackReviewCommandError> {
    value
        .map(|value| {
            DateTime::parse_from_rfc3339(value)
                .map(|value| value.with_timezone(&Utc))
                .map_err(|_| {
                    AttackReviewCommandError::new(
                        review_repo::ATTACK_APPROVAL_EXPIRED,
                        "invalid Candidate approval expiry",
                    )
                })
        })
        .transpose()
}

async fn authorize_local_operator(state: &AgentState) -> Result<(), AttackReviewCommandError> {
    let principal = state
        .operator_principal_provider
        .current(OperatorChannel::LocalDesktop)
        .await?;
    if principal.channel() != OperatorChannel::LocalDesktop {
        return Err(AttackReviewCommandError::new(
            review_repo::ATTACK_REVIEW_SCOPE_MISMATCH,
            "local operator principal is required",
        ));
    }
    Ok(())
}

impl From<review_repo::CandidateReviewStateRow> for AttackCandidateReviewState {
    fn from(state: review_repo::CandidateReviewStateRow) -> Self {
        Self {
            operation_id: state.operation_id.to_string(),
            project_scope_id: state.project_scope_id.to_string(),
            scope_snapshot_id: state.scope_snapshot_id.to_string(),
            wave_run_id: state.wave_run_id.to_string(),
            profile: state.profile,
            review_closed: state.review_closed,
            status: state.barrier.status,
            resume_version: state.barrier.resume_version,
            last_error: state.barrier.last_error,
            wave_unit_count: state.wave_unit_count,
            review_closed_unit_count: state.review_closed_unit_count,
            candidate_count: state.candidate_count,
            proposed_candidate_count: state.proposed_candidate_count,
            candidates: state
                .candidates
                .into_iter()
                .map(|candidate| AttackCandidateReviewItem {
                    candidate_id: candidate.candidate_id.to_string(),
                    wave_unit_id: candidate.wave_unit_id.to_string(),
                    organization_id: candidate.organization_id.to_string(),
                    target_live_id: candidate.target_live_id.map(|id| id.to_string()),
                    live_target_present: candidate.live_target_present,
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
                    latest_approval: candidate.latest_approval.map(|approval| {
                        AttackCandidateApprovalView {
                            approval_id: approval.id.to_string(),
                            status: approval.status,
                            expires_at: approval.expires_at.to_rfc3339(),
                            decision_version: approval.decision_version,
                            row_version: approval.row_version,
                            decided_at: approval.decided_at.to_rfc3339(),
                        }
                    }),
                })
                .collect(),
        }
    }
}

#[tauri::command]
pub async fn attack_list_candidate_reviews(
    request: AttackCandidateReviewScopeRequest,
    state: State<'_, AgentState>,
) -> Result<AttackCandidateReviewState, AttackReviewCommandError> {
    authorize_local_operator(&state).await?;
    let operation_id = parse_uuid(&request.operation_id, "operationId")?;
    let wave_run_id = parse_uuid(&request.wave_run_id, "waveRunId")?;
    review_repo::list_candidate_reviews(&state.db_pool, operation_id, wave_run_id)
        .await
        .map(Into::into)
        .map_err(AttackReviewCommandError::from_db)
}

#[tauri::command]
pub async fn attack_review_candidates(
    request: AttackCandidateReviewRequest,
    state: State<'_, AgentState>,
) -> Result<AttackCandidateReviewResponse, AttackReviewCommandError> {
    authorize_local_operator(&state).await?;
    let operation_id = parse_uuid(&request.operation_id, "operationId")?;
    let wave_run_id = parse_uuid(&request.wave_run_id, "waveRunId")?;
    let decisions = request
        .decisions
        .into_iter()
        .map(|decision| {
            let approve = match decision.decision.as_str() {
                "approve" => true,
                "reject" => false,
                _ => {
                    return Err(AttackReviewCommandError::new(
                        review_repo::ATTACK_REVIEW_SCOPE_MISMATCH,
                        "decision must be approve or reject",
                    ));
                }
            };
            if decision.candidate_plan_hash.trim().is_empty() || decision.expected_row_version < 0 {
                return Err(AttackReviewCommandError::new(
                    review_repo::ATTACK_CANDIDATE_PLAN_CHANGED,
                    "Candidate plan hash and row version are required",
                ));
            }
            Ok(review_repo::CandidateReviewDecision {
                candidate_id: parse_uuid(&decision.candidate_id, "candidateId")?,
                expected_candidate_plan_hash: decision.candidate_plan_hash,
                expected_candidate_row_version: decision.expected_row_version,
                approve,
                expires_at: parse_expiry(decision.expires_at.as_deref())?,
            })
        })
        .collect::<Result<Vec<_>, AttackReviewCommandError>>()?;
    let reviewed = review_repo::review_wave_candidates(
        &state.db_pool,
        review_repo::ReviewCandidateBatch {
            operation_id,
            wave_run_id,
            decisions,
        },
    )
    .await
    .map_err(AttackReviewCommandError::from_db)?;
    Ok(AttackCandidateReviewResponse {
        state: reviewed.state.into(),
        replayed: reviewed.replayed,
        approvals_written: i64::try_from(reviewed.approvals.len()).unwrap_or(i64::MAX),
    })
}

#[tauri::command]
pub async fn attack_resume_candidate_review(
    request: AttackCandidateResumeRequest,
    state: State<'_, AgentState>,
) -> Result<AttackCandidateReviewResponse, AttackReviewCommandError> {
    authorize_local_operator(&state).await?;
    let operation_id = parse_uuid(&request.operation_id, "operationId")?;
    let wave_run_id = parse_uuid(&request.wave_run_id, "waveRunId")?;
    let claim = review_repo::claim_candidate_review_resume(
        &state.db_pool,
        operation_id,
        wave_run_id,
        request.expected_resume_version,
    )
    .await
    .map_err(AttackReviewCommandError::from_db)?;
    if claim.dispatch_required {
        let bridge = match super::core::operation_resume::start_trusted_candidate_review_resume(
            &state, &claim,
        )
        .await
        {
            Ok(bridge) => bridge,
            Err(error) => {
                let _ = review_repo::mark_candidate_review_resume_failed(
                    &state.db_pool,
                    &claim,
                    &format!("{error:#}"),
                )
                .await;
                return Err(AttackReviewCommandError::new(
                    review_repo::ATTACK_RESUME_NOT_READY,
                    format!("trusted operation resume could not start: {error:#}"),
                ));
            }
        };
        let resumed = review_repo::mark_candidate_review_resumed(&state.db_pool, &claim)
            .await
            .map_err(AttackReviewCommandError::from_db)?;
        bridge.emit_event(golish_core::events::AiEvent::HarnessTrace {
            operation_id: operation_id.to_string(),
            stage: "attack_candidate".to_string(),
            agent_path: "main".to_string(),
            trace: golish_core::events::HarnessTraceKind::CandidateReviewResumed {
                wave_run_id: wave_run_id.to_string(),
                resume_version: resumed.resume_version,
            },
        });
    }
    let review = review_repo::list_candidate_reviews(&state.db_pool, operation_id, wave_run_id)
        .await
        .map_err(AttackReviewCommandError::from_db)?;
    Ok(AttackCandidateReviewResponse {
        state: review.into(),
        replayed: claim.replayed,
        approvals_written: 0,
    })
}

#[tauri::command]
pub async fn attack_list_candidate_attempts(
    request: AttackCandidateReviewScopeRequest,
    state: State<'_, AgentState>,
) -> Result<Vec<CandidateAttemptRow>, AttackReviewCommandError> {
    authorize_local_operator(&state).await?;
    let operation_id = parse_uuid(&request.operation_id, "operationId")?;
    let wave_run_id = parse_uuid(&request.wave_run_id, "waveRunId")?;
    // Authoritative scope refresh fails before any Attempt row is returned.
    review_repo::list_candidate_reviews(&state.db_pool, operation_id, wave_run_id)
        .await
        .map_err(AttackReviewCommandError::from_db)?;
    golish_db::repo::candidate_attempts::list_for_review_wave(
        &state.db_pool,
        operation_id,
        wave_run_id,
    )
    .await
    .map_err(AttackReviewCommandError::from_db)
    .map(|attempts| {
        attempts
            .into_iter()
            .map(|attempt| CandidateAttemptRow {
                attempt_id: attempt.id.to_string(),
                candidate_id: attempt.candidate_id.to_string(),
                approval_id: attempt.approval_id.to_string(),
                organization_id: attempt.organization_id.to_string(),
                target_live_id: attempt.target_live_id.map(|id| id.to_string()),
                target_type_at_time: attempt.target_type_at_time,
                target_value_at_time: attempt.target_value_at_time,
                target_identity_hash: attempt.target_identity_hash,
                candidate_plan_hash: attempt.candidate_plan_hash,
                ordinal: attempt.ordinal,
                status: attempt.status,
                stage_worker_run_id: attempt.stage_worker_run_id.map(|id| id.to_string()),
                result: attempt.result_json,
                result_hash: attempt.result_hash,
                row_version: attempt.row_version,
                created_at: attempt.created_at.to_rfc3339(),
                updated_at: attempt.updated_at.to_rfc3339(),
                terminal_at: attempt.terminal_at.map(|value| value.to_rfc3339()),
            })
            .collect()
    })
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn attack_review_wire_has_no_actor_project_snapshot_org_or_plan_authority() {
        let value = serde_json::to_value(AttackCandidateReviewRequest {
            operation_id: Uuid::from_u128(1).to_string(),
            wave_run_id: Uuid::from_u128(2).to_string(),
            decisions: vec![AttackCandidateReviewDecisionRequest {
                candidate_id: Uuid::from_u128(3).to_string(),
                candidate_plan_hash: "sha256:plan".to_string(),
                expected_row_version: 0,
                decision: "reject".to_string(),
                expires_at: None,
            }],
        })
        .unwrap();
        let serialized = value.to_string();
        for forbidden in [
            "actorId",
            "operatorId",
            "projectScopeId",
            "scopeSnapshotId",
            "organizationId",
            "waveUnitId",
            "executionPlan",
            "allowedCapabilityIds",
            "budget",
        ] {
            assert!(
                !serialized.contains(forbidden),
                "forbidden field: {forbidden}"
            );
        }
    }

    #[test]
    fn attack_review_errors_keep_stable_ipc_codes() {
        let error =
            AttackReviewCommandError::new(review_repo::ATTACK_CANDIDATE_PLAN_CHANGED, "stale plan");
        assert_eq!(
            serde_json::to_value(error).unwrap()["code"],
            review_repo::ATTACK_CANDIDATE_PLAN_CHANGED
        );
    }
}
