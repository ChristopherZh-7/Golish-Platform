//! Durable Candidate review command surface.
//!
//! Requests intentionally omit actor, project, scope snapshot, organization,
//! WaveUnit, execution plan, capability, action and budget authority. The DB
//! derives and verifies those identities from `operation_id + wave_run_id` and
//! the server resolves the opaque local operator principal.

use chrono::{DateTime, Utc};
use golish_app_core::domain::operator::{OperatorChannel, TrustedOperatorPrincipal};
use golish_db::repo::attack_candidate_approvals as review_repo;
use golish_db::repo::candidate_recovery as recovery_repo;
use serde::{Deserialize, Serialize};
use tauri::State;
use ts_rs::TS;
use uuid::Uuid;

use crate::state::AgentState;

const ATTACK_RECOVERY_CONFLICT: &str = "ATTACK_RECOVERY_CONFLICT";

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

    fn from_recovery_db(error: golish_db::DbError) -> Self {
        let code = match &error {
            golish_db::DbError::Sqlx(sqlx::Error::Database(database_error))
                if matches!(
                    database_error.code().as_deref(),
                    Some("23503" | "23505" | "23514")
                ) =>
            {
                ATTACK_RECOVERY_CONFLICT
            }
            golish_db::DbError::Sqlx(_) => "DATABASE",
            golish_db::DbError::NotFound(_) => review_repo::ATTACK_REVIEW_SCOPE_MISMATCH,
            _ => ATTACK_RECOVERY_CONFLICT,
        };
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
    // Required and future-dated for `approve`; no action may start at/after it.
    #[serde(default)]
    pub start_before: Option<String>,
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
    pub start_before: String,
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

#[derive(Debug, Clone, Serialize, TS)]
#[serde(rename_all = "camelCase")]
#[ts(export, export_to = "../../../../frontend/lib/generated/")]
pub struct AttackVerificationEvidenceView {
    #[ts(type = "number")]
    pub evidence_id: i64,
    pub role: String,
}

#[derive(Debug, Clone, Serialize, TS)]
#[serde(rename_all = "camelCase")]
#[ts(export, export_to = "../../../../frontend/lib/generated/")]
pub struct AttackVerificationActionView {
    pub action_id: String,
    #[ts(type = "number")]
    pub action_ordinal: i32,
    pub capability_id: String,
    pub action_kind: String,
    pub status: String,
    pub outcome_hash: Option<String>,
    pub error_code: Option<String>,
    pub started_at: Option<String>,
    pub completed_at: Option<String>,
    pub authorization_receipt_id: Option<String>,
    pub authorization_request_id: Option<String>,
    pub authorization_receipt_hash: Option<String>,
    pub authorized_at: Option<String>,
    pub start_before: Option<String>,
    pub execution_deadline: Option<String>,
}

#[derive(Debug, Clone, Serialize, TS)]
#[serde(rename_all = "camelCase")]
#[ts(export, export_to = "../../../../frontend/lib/generated/")]
pub struct AttackVerificationTerminalIntentView {
    pub intent_id: String,
    pub request_id: String,
    pub tool_call_record_id: String,
    pub disposition: String,
    pub result_hash: String,
    pub evidence_manifest_hash: String,
    #[ts(type = "number")]
    pub evidence_count: i32,
    pub intent_hash: String,
    pub created_at: String,
}

#[derive(Debug, Clone, Serialize, TS)]
#[serde(rename_all = "camelCase")]
#[ts(export, export_to = "../../../../frontend/lib/generated/")]
pub struct AttackVerificationTerminalBarrierView {
    pub barrier_id: String,
    pub intent_id: String,
    pub request_id: String,
    pub tool_call_record_id: String,
    pub created_at: String,
}

#[derive(Debug, Clone, Serialize, TS)]
#[serde(rename_all = "camelCase")]
#[ts(export, export_to = "../../../../frontend/lib/generated/")]
pub struct AttackVerificationTerminalReceiptView {
    pub receipt_id: String,
    pub intent_id: String,
    pub barrier_id: String,
    pub request_id: String,
    pub disposition: String,
    #[ts(type = "number")]
    pub terminal_attempt_row_version: i64,
    pub finding_id: Option<String>,
    #[ts(type = "number")]
    pub fact_delta_count: i32,
    pub terminal_event_id: String,
    pub receipt_hash: String,
    pub created_at: String,
}

#[derive(Debug, Clone, Serialize, TS)]
#[serde(rename_all = "camelCase")]
#[ts(export, export_to = "../../../../frontend/lib/generated/")]
pub struct AttackCandidateRecoveryCaseView {
    pub recovery_case_id: String,
    pub request_id: String,
    pub attempt_id: String,
    pub action_id: Option<String>,
    pub intent_id: Option<String>,
    pub case_kind: String,
    pub reason_code: String,
    #[ts(type = "number")]
    pub attempt_row_version: i64,
    pub status: String,
    pub resolution_kind: Option<String>,
    pub resolution_request_id: Option<String>,
    #[ts(type = "number")]
    pub row_version: i64,
    #[ts(type = "Array<number>")]
    pub evidence_ids: Vec<i64>,
    pub decided_at: Option<String>,
    pub completed_at: Option<String>,
    pub created_at: String,
    pub updated_at: String,
}

#[derive(Debug, Clone, Serialize, TS)]
#[serde(rename_all = "camelCase")]
#[ts(export, export_to = "../../../../frontend/lib/generated/")]
pub struct AttackVerificationQueueItem {
    pub attempt_id: String,
    pub candidate_id: String,
    pub approval_id: String,
    pub wave_unit_id: String,
    pub organization_id: String,
    pub target_live_id: Option<String>,
    pub target_type_at_time: String,
    pub target_value_at_time: String,
    pub target_identity_hash: String,
    pub candidate_plan_hash: String,
    pub hypothesis: String,
    pub technique: Option<String>,
    pub plan_schema_version: String,
    pub recipe_version: String,
    pub executor_contract_version: String,
    #[ts(type = "number")]
    pub budget_max_actions: i64,
    #[ts(type = "number")]
    pub budget_max_requests: i64,
    #[ts(type = "number")]
    pub budget_max_runtime_ms: i64,
    pub approval_start_before: String,
    pub approval_expires_at: String,
    #[ts(type = "number")]
    pub ordinal: i32,
    pub status: String,
    pub worker_run_id: Option<String>,
    pub worker_status: Option<String>,
    #[ts(type = "number")]
    pub row_version: i64,
    pub created_at: String,
    pub updated_at: String,
    pub terminal_at: Option<String>,
    pub observation_evidence: Vec<AttackVerificationEvidenceView>,
    pub attempt_evidence: Vec<AttackVerificationEvidenceView>,
    pub actions: Vec<AttackVerificationActionView>,
    pub terminal_intent: Option<AttackVerificationTerminalIntentView>,
    pub terminal_barrier: Option<AttackVerificationTerminalBarrierView>,
    pub terminal_receipt: Option<AttackVerificationTerminalReceiptView>,
    pub recovery_cases: Vec<AttackCandidateRecoveryCaseView>,
}

#[derive(Debug, Clone, Serialize, TS)]
#[serde(rename_all = "camelCase")]
#[ts(export, export_to = "../../../../frontend/lib/generated/")]
pub struct AttackVerificationWaveUnitView {
    pub wave_unit_id: String,
    pub organization_id: String,
    #[ts(type = "number")]
    pub ordinal: i32,
    pub status: String,
    pub review_closed: bool,
    pub verification_closed: bool,
    pub consolidation_status: String,
    #[ts(type = "number")]
    pub row_version: i64,
}

#[derive(Debug, Clone, Serialize, TS)]
#[serde(rename_all = "camelCase")]
#[ts(export, export_to = "../../../../frontend/lib/generated/")]
pub struct AttackVerificationConsolidationView {
    pub consolidation_id: String,
    pub decision_kind: String,
    pub target_wave_run_id: Option<String>,
    #[ts(type = "number")]
    pub fact_delta_count: i32,
    pub reason_code: String,
    #[ts(type = "number")]
    pub row_version: i64,
    pub decided_at: String,
}

#[derive(Debug, Clone, Serialize, TS)]
#[serde(rename_all = "camelCase")]
#[ts(export, export_to = "../../../../frontend/lib/generated/")]
pub struct AttackVerificationPendingEnrichmentView {
    pub enrichment_id: String,
    pub fact_delta_id: String,
    pub source_attempt_id: String,
    pub candidate_id: String,
    pub wave_unit_id: String,
    pub organization_id: String,
    pub subject_kind: String,
    pub subject_id: String,
    pub target_type_at_time: String,
    pub target_value_at_time: String,
    pub delta_kind: String,
    pub observation_kind: String,
    pub allowed_techniques: Vec<String>,
    pub enrichment_required: bool,
    pub reason_code: String,
    pub status: String,
    pub created_at: String,
}

#[derive(Debug, Clone, Serialize, TS)]
#[serde(rename_all = "camelCase")]
#[ts(export, export_to = "../../../../frontend/lib/generated/")]
pub struct AttackVerificationQueueState {
    pub operation_id: String,
    pub scope_snapshot_id: String,
    pub wave_run_id: String,
    #[ts(type = "number")]
    pub generation: i32,
    pub wave_status: String,
    #[ts(type = "number")]
    pub wave_row_version: i64,
    pub wave_units: Vec<AttackVerificationWaveUnitView>,
    pub consolidation: Option<AttackVerificationConsolidationView>,
    #[ts(type = "number")]
    pub pending_enrichment_count: usize,
    pub pending_enrichments: Vec<AttackVerificationPendingEnrichmentView>,
    pub items: Vec<AttackVerificationQueueItem>,
}

#[derive(Debug, Clone, Serialize, Deserialize, TS)]
#[serde(rename_all = "camelCase", deny_unknown_fields)]
#[ts(export, export_to = "../../../../frontend/lib/generated/")]
pub struct AttackCandidateRecoveryResolveRequest {
    pub operation_id: String,
    pub wave_run_id: String,
    pub recovery_case_id: String,
    pub request_id: String,
    #[ts(type = "number")]
    pub expected_row_version: i64,
    #[ts(type = "number")]
    pub expected_attempt_row_version: i64,
    pub decision: String,
    #[serde(default)]
    #[ts(type = "Array<number>")]
    pub evidence_ids: Vec<i64>,
}

#[derive(Debug, Clone, Serialize, TS)]
#[serde(rename_all = "camelCase")]
#[ts(export, export_to = "../../../../frontend/lib/generated/")]
pub struct AttackCandidateRecoveryResolveResponse {
    pub recovery_case_id: String,
    pub decision_request_id: String,
    pub decision: String,
    pub status: String,
    #[ts(type = "number")]
    pub row_version: i64,
    pub replayed: bool,
    pub pending_server_convergence: bool,
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
    local_operator(state).await.map(|_| ())
}

async fn local_operator(
    state: &AgentState,
) -> Result<TrustedOperatorPrincipal, AttackReviewCommandError> {
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
    Ok(principal)
}

fn verification_queue_view(
    queue: recovery_repo::CandidateVerificationQueueReadModel,
) -> AttackVerificationQueueState {
    AttackVerificationQueueState {
        operation_id: queue.operation_id.to_string(),
        scope_snapshot_id: queue.scope_snapshot_id.to_string(),
        wave_run_id: queue.wave_run_id.to_string(),
        generation: queue.generation,
        wave_status: queue.wave_status,
        wave_row_version: queue.wave_row_version,
        wave_units: queue
            .wave_units
            .into_iter()
            .map(|unit| AttackVerificationWaveUnitView {
                wave_unit_id: unit.wave_unit_id.to_string(),
                organization_id: unit.organization_id.to_string(),
                ordinal: unit.ordinal,
                status: unit.status,
                review_closed: unit.review_closed,
                verification_closed: unit.verification_closed,
                consolidation_status: unit.consolidation_status,
                row_version: unit.row_version,
            })
            .collect(),
        consolidation: queue.consolidation.map(|consolidation| {
            AttackVerificationConsolidationView {
                consolidation_id: consolidation.consolidation_id.to_string(),
                decision_kind: consolidation.decision_kind,
                target_wave_run_id: consolidation.target_wave_run_id.map(|id| id.to_string()),
                fact_delta_count: consolidation.fact_delta_count,
                reason_code: consolidation.reason_code,
                row_version: consolidation.row_version,
                decided_at: consolidation.decided_at.to_rfc3339(),
            }
        }),
        pending_enrichment_count: queue.pending_enrichment_count,
        pending_enrichments: queue
            .pending_enrichments
            .into_iter()
            .map(|item| AttackVerificationPendingEnrichmentView {
                enrichment_id: item.enrichment_id.to_string(),
                fact_delta_id: item.fact_delta_id.to_string(),
                source_attempt_id: item.source_attempt_id.to_string(),
                candidate_id: item.candidate_id.to_string(),
                wave_unit_id: item.wave_unit_id.to_string(),
                organization_id: item.organization_id.to_string(),
                subject_kind: item.subject_kind,
                subject_id: item.subject_id.to_string(),
                target_type_at_time: item.target_type_at_time,
                target_value_at_time: item.target_value_at_time,
                delta_kind: item.delta_kind,
                observation_kind: item.observation_kind,
                allowed_techniques: item.allowed_techniques,
                enrichment_required: item.enrichment_required,
                reason_code: item.reason_code,
                status: item.status,
                created_at: item.created_at.to_rfc3339(),
            })
            .collect(),
        items: queue
            .items
            .into_iter()
            .map(|item| AttackVerificationQueueItem {
                attempt_id: item.attempt_id.to_string(),
                candidate_id: item.candidate_id.to_string(),
                approval_id: item.approval_id.to_string(),
                wave_unit_id: item.wave_unit_id.to_string(),
                organization_id: item.organization_id.to_string(),
                target_live_id: item.target_live_id.map(|id| id.to_string()),
                target_type_at_time: item.target_type_at_time,
                target_value_at_time: item.target_value_at_time,
                target_identity_hash: item.target_identity_hash,
                candidate_plan_hash: item.candidate_plan_hash,
                hypothesis: item.hypothesis,
                technique: item.technique,
                plan_schema_version: item.plan_schema_version,
                recipe_version: item.recipe_version,
                executor_contract_version: item.executor_contract_version,
                budget_max_actions: item.budget_max_actions,
                budget_max_requests: item.budget_max_requests,
                budget_max_runtime_ms: item.budget_max_runtime_ms,
                approval_start_before: item.approval_start_before.to_rfc3339(),
                approval_expires_at: item.approval_expires_at.to_rfc3339(),
                ordinal: item.ordinal,
                status: item.status,
                worker_run_id: item.worker_run_id.map(|id| id.to_string()),
                worker_status: item.worker_status,
                row_version: item.row_version,
                created_at: item.created_at.to_rfc3339(),
                updated_at: item.updated_at.to_rfc3339(),
                terminal_at: item.terminal_at.map(|value| value.to_rfc3339()),
                observation_evidence: item
                    .observation_evidence
                    .into_iter()
                    .map(|evidence| AttackVerificationEvidenceView {
                        evidence_id: evidence.evidence_id,
                        role: evidence.role,
                    })
                    .collect(),
                attempt_evidence: item
                    .attempt_evidence
                    .into_iter()
                    .map(|evidence| AttackVerificationEvidenceView {
                        evidence_id: evidence.evidence_id,
                        role: evidence.role,
                    })
                    .collect(),
                actions: item
                    .actions
                    .into_iter()
                    .map(|action| AttackVerificationActionView {
                        action_id: action.action_id.to_string(),
                        action_ordinal: action.action_ordinal,
                        capability_id: action.capability_id,
                        action_kind: action.action_kind,
                        status: action.status,
                        outcome_hash: action.outcome_hash,
                        error_code: action.error_code,
                        started_at: action.started_at.map(|value| value.to_rfc3339()),
                        completed_at: action.completed_at.map(|value| value.to_rfc3339()),
                        authorization_receipt_id: action
                            .authorization_receipt_id
                            .map(|id| id.to_string()),
                        authorization_request_id: action.authorization_request_id,
                        authorization_receipt_hash: action.authorization_receipt_hash,
                        authorized_at: action.authorized_at.map(|value| value.to_rfc3339()),
                        start_before: action.start_before.map(|value| value.to_rfc3339()),
                        execution_deadline: action
                            .execution_deadline
                            .map(|value| value.to_rfc3339()),
                    })
                    .collect(),
                terminal_intent: item.terminal_intent.map(|intent| {
                    AttackVerificationTerminalIntentView {
                        intent_id: intent.intent_id.to_string(),
                        request_id: intent.request_id,
                        tool_call_record_id: intent.tool_call_record_id.to_string(),
                        disposition: intent.disposition,
                        result_hash: intent.result_hash,
                        evidence_manifest_hash: intent.evidence_manifest_hash,
                        evidence_count: intent.evidence_count,
                        intent_hash: intent.intent_hash,
                        created_at: intent.created_at.to_rfc3339(),
                    }
                }),
                terminal_barrier: item.terminal_barrier.map(|barrier| {
                    AttackVerificationTerminalBarrierView {
                        barrier_id: barrier.barrier_id.to_string(),
                        intent_id: barrier.intent_id.to_string(),
                        request_id: barrier.request_id,
                        tool_call_record_id: barrier.tool_call_record_id.to_string(),
                        created_at: barrier.created_at.to_rfc3339(),
                    }
                }),
                terminal_receipt: item.terminal_receipt.map(|receipt| {
                    AttackVerificationTerminalReceiptView {
                        receipt_id: receipt.receipt_id.to_string(),
                        intent_id: receipt.intent_id.to_string(),
                        barrier_id: receipt.barrier_id.to_string(),
                        request_id: receipt.request_id,
                        disposition: receipt.disposition,
                        terminal_attempt_row_version: receipt.terminal_attempt_row_version,
                        finding_id: receipt.finding_id.map(|id| id.to_string()),
                        fact_delta_count: receipt.fact_delta_count,
                        terminal_event_id: receipt.terminal_event_id.to_string(),
                        receipt_hash: receipt.receipt_hash,
                        created_at: receipt.created_at.to_rfc3339(),
                    }
                }),
                recovery_cases: item
                    .recovery_cases
                    .into_iter()
                    .map(|recovery| AttackCandidateRecoveryCaseView {
                        recovery_case_id: recovery.recovery_case_id.to_string(),
                        request_id: recovery.request_id,
                        attempt_id: recovery.attempt_id.to_string(),
                        action_id: recovery.action_id.map(|id| id.to_string()),
                        intent_id: recovery.intent_id.map(|id| id.to_string()),
                        case_kind: recovery.case_kind,
                        reason_code: recovery.reason_code,
                        attempt_row_version: recovery.attempt_row_version,
                        status: recovery.status,
                        resolution_kind: recovery.resolution_kind,
                        resolution_request_id: recovery.resolution_request_id,
                        row_version: recovery.row_version,
                        evidence_ids: recovery.evidence_ids,
                        decided_at: recovery.decided_at.map(|value| value.to_rfc3339()),
                        completed_at: recovery.completed_at.map(|value| value.to_rfc3339()),
                        created_at: recovery.created_at.to_rfc3339(),
                        updated_at: recovery.updated_at.to_rfc3339(),
                    })
                    .collect(),
            })
            .collect(),
    }
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
                            start_before: approval.start_before.to_rfc3339(),
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
                start_before: parse_expiry(decision.start_before.as_deref())?,
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

#[tauri::command]
pub async fn attack_list_verification_queue(
    request: AttackCandidateReviewScopeRequest,
    state: State<'_, AgentState>,
) -> Result<AttackVerificationQueueState, AttackReviewCommandError> {
    authorize_local_operator(&state).await?;
    let operation_id = parse_uuid(&request.operation_id, "operationId")?;
    let wave_run_id = parse_uuid(&request.wave_run_id, "waveRunId")?;
    recovery_repo::list_verification_queue(&state.db_pool, operation_id, wave_run_id)
        .await
        .map(verification_queue_view)
        .map_err(AttackReviewCommandError::from_recovery_db)
}

#[tauri::command]
pub async fn attack_resolve_candidate_recovery(
    request: AttackCandidateRecoveryResolveRequest,
    state: State<'_, AgentState>,
) -> Result<AttackCandidateRecoveryResolveResponse, AttackReviewCommandError> {
    let principal = local_operator(&state).await?;
    let operation_id = parse_uuid(&request.operation_id, "operationId")?;
    let wave_run_id = parse_uuid(&request.wave_run_id, "waveRunId")?;
    let recovery_case_id = parse_uuid(&request.recovery_case_id, "recoveryCaseId")?;
    if request.request_id.is_empty()
        || request.request_id != request.request_id.trim()
        || request.request_id.len() > 256
        || request.request_id.chars().any(char::is_control)
        || request.expected_row_version < 0
        || request.expected_attempt_row_version < 0
    {
        return Err(AttackReviewCommandError::new(
            ATTACK_RECOVERY_CONFLICT,
            "invalid Candidate recovery CAS request",
        ));
    }
    let mut evidence_ids = request.evidence_ids;
    evidence_ids.sort_unstable();
    let unique_count = evidence_ids.len();
    evidence_ids.dedup();
    if evidence_ids.len() != unique_count || evidence_ids.iter().any(|id| *id <= 0) {
        return Err(AttackReviewCommandError::new(
            ATTACK_RECOVERY_CONFLICT,
            "Candidate recovery evidence ids must be positive and unique",
        ));
    }
    let resolution = match request.decision.as_str() {
        "terminalize_blocked_outcome_unknown" => {
            recovery_repo::CandidateRecoveryResolution::TerminalizeBlockedOutcomeUnknown
        }
        "abandon_before_side_effect" => {
            recovery_repo::CandidateRecoveryResolution::AbandonBeforeSideEffect
        }
        "accept_external_result_with_exact_evidence" => {
            recovery_repo::CandidateRecoveryResolution::AcceptExternalResultWithExactEvidence
        }
        _ => {
            return Err(AttackReviewCommandError::new(
                ATTACK_RECOVERY_CONFLICT,
                "unknown Candidate recovery decision",
            ));
        }
    };

    // The recovery writer's frozen identity is immutable, so proving this id
    // belongs to the exact operation + Wave before its CAS cannot be raced into
    // a sibling Wave. The mutation itself accepts no org/target/plan authority.
    let queue = recovery_repo::list_verification_queue(&state.db_pool, operation_id, wave_run_id)
        .await
        .map_err(AttackReviewCommandError::from_recovery_db)?;
    let case_in_exact_wave = queue.items.iter().any(|item| {
        item.recovery_cases
            .iter()
            .any(|recovery| recovery.recovery_case_id == recovery_case_id)
    });
    if !case_in_exact_wave {
        return Err(AttackReviewCommandError::new(
            review_repo::ATTACK_REVIEW_SCOPE_MISMATCH,
            "Candidate recovery case is outside this operation Wave",
        ));
    }

    let resolved = recovery_repo::resolve_candidate_recovery(
        &state.db_pool,
        recovery_repo::ResolveCandidateRecovery {
            request_id: request.request_id.clone(),
            operation_id,
            recovery_case_id,
            expected_row_version: request.expected_row_version,
            expected_attempt_row_version: request.expected_attempt_row_version,
            resolved_by: principal.id().as_uuid(),
            resolution,
            evidence_ids,
        },
    )
    .await
    .map_err(AttackReviewCommandError::from_recovery_db)?;
    let decision_replayed = resolved.replayed;
    let recorded_case = resolved.recovery_case;
    let converged = match recovery_repo::converge_candidate_recovery(
        &state.db_pool,
        operation_id,
        recovery_case_id,
    )
    .await
    {
        Ok(converged) => Some(converged),
        Err(error) => {
            // The operator decision is already durable. A transient lane or
            // scheduler race must not pretend the decision was rolled back;
            // the Verification scheduler will retry this same immutable case.
            tracing::warn!(
                recovery_case_id = %recovery_case_id,
                operation_id = %operation_id,
                error = %error,
                "Candidate recovery decision recorded; server convergence remains pending"
            );
            None
        }
    };
    let recovery_case = converged
        .as_ref()
        .map(|value| &value.recovery_case)
        .unwrap_or(&recorded_case);
    Ok(AttackCandidateRecoveryResolveResponse {
        recovery_case_id: recovery_case.id.to_string(),
        decision_request_id: request.request_id,
        decision: resolution.as_str().to_string(),
        status: recovery_case.status.clone(),
        row_version: recovery_case.row_version,
        replayed: decision_replayed
            || converged
                .as_ref()
                .is_some_and(|converged| converged.replayed),
        pending_server_convergence: recovery_case.status != "resolved",
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
                start_before: None,
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

    #[test]
    fn attack_recovery_wire_has_exact_scope_request_and_versions_but_no_authority() {
        let value = serde_json::to_value(AttackCandidateRecoveryResolveRequest {
            operation_id: Uuid::from_u128(1).to_string(),
            wave_run_id: Uuid::from_u128(2).to_string(),
            recovery_case_id: Uuid::from_u128(3).to_string(),
            request_id: "recovery-request-1".to_string(),
            expected_row_version: 4,
            expected_attempt_row_version: 9,
            decision: "accept_external_result_with_exact_evidence".to_string(),
            evidence_ids: vec![41, 42],
        })
        .unwrap();
        assert_eq!(value["operationId"], Uuid::from_u128(1).to_string());
        assert_eq!(value["waveRunId"], Uuid::from_u128(2).to_string());
        assert_eq!(value["requestId"], "recovery-request-1");
        assert_eq!(value["expectedRowVersion"], 4);
        assert_eq!(value["expectedAttemptRowVersion"], 9);
        assert_eq!(value["evidenceIds"], serde_json::json!([41, 42]));
        let serialized = value.to_string();
        for forbidden in [
            "actorId",
            "operatorId",
            "organizationId",
            "targetId",
            "candidatePlanHash",
            "canonicalArgs",
            "budget",
            "leaseToken",
            "checkpoint",
        ] {
            assert!(
                !serialized.contains(forbidden),
                "forbidden recovery authority field: {forbidden}"
            );
        }
    }
}
