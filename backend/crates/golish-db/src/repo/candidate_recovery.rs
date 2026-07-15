//! Durable Candidate terminal-intent, checkpoint-barrier, and recovery CAS.
//!
//! The model-facing submit tool may persist only an immutable terminal intent.
//! A server scheduler later consumes a durable finished-tool/checkpoint barrier
//! and writes the canonical terminal state plus its receipt in one transaction.

use chrono::{DateTime, Utc};
use sqlx::{PgPool, Postgres, Transaction};
use std::collections::{BTreeMap, BTreeSet};
use uuid::Uuid;

use super::candidate_attempts::{self, AttemptEvidenceLink, CandidateAttemptRow, ATTEMPT_COLUMNS};

const MAX_REQUEST_ID_BYTES: usize = 256;
const MAX_TOOL_RESULT_BYTES: usize = 64 * 1024;

#[derive(Debug, Clone)]
pub struct RecordCandidateTerminalIntent {
    pub request_id: String,
    pub operation_id: Uuid,
    pub organization_id: Uuid,
    pub candidate_id: Uuid,
    pub approval_id: Uuid,
    pub attempt_id: Uuid,
    pub candidate_plan_hash: String,
    pub worker_run_id: Uuid,
    pub lease_token: Uuid,
    pub attempt_epoch: i64,
    pub tool_call_record_id: Uuid,
    pub disposition: String,
    pub submitted_result: serde_json::Value,
    pub evidence: Vec<AttemptEvidenceLink>,
    pub tool_result_text: String,
}

#[derive(Debug, Clone, sqlx::FromRow, PartialEq)]
pub struct CandidateTerminalIntentRow {
    pub id: Uuid,
    pub request_id: String,
    pub attempt_id: Uuid,
    pub approval_id: Uuid,
    pub candidate_id: Uuid,
    pub operation_id: Uuid,
    pub scope_snapshot_id: Uuid,
    pub wave_run_id: Uuid,
    pub wave_unit_id: Uuid,
    pub organization_id: Uuid,
    pub target_identity_hash: String,
    pub candidate_plan_hash: String,
    pub worker_run_id: Uuid,
    pub attempt_epoch: i64,
    pub lease_token: Uuid,
    pub tool_call_record_id: Uuid,
    pub disposition: String,
    pub submitted_result: serde_json::Value,
    pub result_hash: String,
    pub evidence_manifest_hash: String,
    pub evidence_count: i32,
    pub tool_result_text: String,
    pub tool_result_hash: String,
    pub intent_hash: String,
    pub created_at: DateTime<Utc>,
}

#[derive(Debug, Clone, PartialEq)]
pub struct RecordedCandidateTerminalIntent {
    pub intent: CandidateTerminalIntentRow,
    pub attempt: CandidateAttemptRow,
    pub replayed: bool,
}

#[derive(Debug, Clone)]
pub struct RecordCandidateTerminalBarrier {
    pub request_id: String,
    pub intent_id: Uuid,
    pub attempt_id: Uuid,
    pub worker_run_id: Uuid,
    pub tool_call_record_id: Uuid,
    pub attempt_epoch: i64,
    pub checkpoint_version: i64,
}

#[derive(Debug, Clone, sqlx::FromRow, PartialEq, Eq)]
pub struct CandidateTerminalBarrierRow {
    pub id: Uuid,
    pub request_id: String,
    pub intent_id: Uuid,
    pub attempt_id: Uuid,
    pub worker_run_id: Uuid,
    pub tool_call_record_id: Uuid,
    pub message_chain_id: Uuid,
    pub attempt_epoch: i64,
    pub checkpoint_version: i64,
    pub checkpoint_hash: String,
    pub tool_result_hash: String,
    pub barrier_hash: String,
    pub created_at: DateTime<Utc>,
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct RecordedCandidateTerminalBarrier {
    pub barrier: CandidateTerminalBarrierRow,
    pub replayed: bool,
}

#[derive(Debug, Clone)]
pub struct CheckpointCandidateTerminalBarrier {
    pub request_id: String,
    pub intent_id: Uuid,
    pub expected_intent_hash: String,
    pub checkpoint: super::runtime_memory_tx::CheckpointBoundWorkerChainRow,
}

/// Server-owned recovery command for an immutable TerminalIntent whose normal
/// tool-finish/checkpoint path was interrupted. It deliberately carries no
/// lease, lane, tool result, chain body, or terminal disposition: all of those
/// are reloaded from the intent and its exact relational owners.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct RecoverCandidateTerminalIntent {
    pub operation_id: Uuid,
    pub intent_id: Uuid,
    pub expected_intent_hash: String,
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct RecoveredCandidateTerminalBarrier {
    pub barrier: CandidateTerminalBarrierRow,
    /// `true` means the immutable barrier already existed. No mutable tool,
    /// Worker, or chain row was rewritten on this replay path.
    pub replayed: bool,
    pub tool_reconciled: bool,
    pub worker_reconciled: bool,
}

#[derive(Debug, Clone, sqlx::FromRow, PartialEq, Eq)]
pub struct CandidateTerminalIntentQueueRow {
    pub operation_id: Uuid,
    pub intent_id: Uuid,
    pub request_id: String,
    pub organization_id: Uuid,
    pub candidate_id: Uuid,
    pub attempt_id: Uuid,
    pub worker_run_id: Uuid,
    pub tool_call_record_id: Uuid,
    pub candidate_plan_hash: String,
    pub result_hash: String,
    pub evidence_manifest_hash: String,
    pub tool_result_hash: String,
    pub intent_hash: String,
    pub barrier_id: Option<Uuid>,
    pub barrier_hash: Option<String>,
    pub receipt_id: Option<Uuid>,
    pub created_at: DateTime<Utc>,
}

#[derive(Debug, Clone)]
pub struct TerminalizeCandidateTerminalIntent {
    pub request_id: String,
    pub operation_id: Uuid,
    pub intent_id: Uuid,
    pub barrier_id: Uuid,
}

#[derive(Debug, Clone, sqlx::FromRow, PartialEq)]
pub struct CandidateTerminalReceiptRow {
    pub id: Uuid,
    pub request_id: String,
    pub intent_id: Uuid,
    pub barrier_id: Uuid,
    pub attempt_id: Uuid,
    pub candidate_id: Uuid,
    pub operation_id: Uuid,
    pub scope_snapshot_id: Uuid,
    pub wave_run_id: Uuid,
    pub wave_unit_id: Uuid,
    pub organization_id: Uuid,
    pub worker_run_id: Uuid,
    pub disposition: String,
    pub result_hash: String,
    pub terminal_attempt_row_version: i64,
    pub finding_id: Option<Uuid>,
    pub fact_delta_count: i32,
    pub terminal_event_id: Uuid,
    pub receipt_payload: serde_json::Value,
    pub receipt_hash: String,
    pub created_at: DateTime<Utc>,
}

#[derive(Debug, Clone, PartialEq)]
pub struct TerminalizedCandidateTerminalIntent {
    pub receipt: CandidateTerminalReceiptRow,
    pub terminalized: super::finding_lineage::TerminalizedCandidateAttempt,
    pub replayed: bool,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum CandidateRecoveryResolution {
    TerminalizeBlockedOutcomeUnknown,
    AbandonBeforeSideEffect,
    AcceptExternalResultWithExactEvidence,
}

impl CandidateRecoveryResolution {
    pub const fn as_str(self) -> &'static str {
        match self {
            Self::TerminalizeBlockedOutcomeUnknown => "terminalize_blocked_outcome_unknown",
            Self::AbandonBeforeSideEffect => "abandon_before_side_effect",
            Self::AcceptExternalResultWithExactEvidence => {
                "accept_external_result_with_exact_evidence"
            }
        }
    }
}

#[derive(Debug, Clone)]
pub struct ResolveCandidateRecovery {
    pub request_id: String,
    pub operation_id: Uuid,
    pub recovery_case_id: Uuid,
    pub expected_row_version: i64,
    pub expected_attempt_row_version: i64,
    pub resolved_by: Uuid,
    pub resolution: CandidateRecoveryResolution,
    pub evidence_ids: Vec<i64>,
}

#[derive(Debug, Clone, sqlx::FromRow, PartialEq)]
pub struct CandidateRecoveryCaseRow {
    pub id: Uuid,
    pub request_id: String,
    pub attempt_id: Uuid,
    pub action_id: Option<Uuid>,
    pub intent_id: Option<Uuid>,
    pub operation_id: Uuid,
    pub organization_id: Uuid,
    pub candidate_id: Uuid,
    pub worker_run_id: Uuid,
    pub reason_code: String,
    pub attempt_row_version: i64,
    pub status: String,
    pub resolution_kind: Option<String>,
    pub resolution_request_id: Option<String>,
    pub resolution_payload: Option<serde_json::Value>,
    pub resolved_by: Option<Uuid>,
    pub decided_at: Option<DateTime<Utc>>,
    pub completed_at: Option<DateTime<Utc>>,
    pub row_version: i64,
    pub created_at: DateTime<Utc>,
    pub updated_at: DateTime<Utc>,
}

#[derive(Debug, Clone, PartialEq)]
pub struct ResolvedCandidateRecovery {
    pub recovery_case: CandidateRecoveryCaseRow,
    pub replayed: bool,
}

/// Server-owned convergence result for one durable operator recovery decision.
/// The operator chooses only a closed decision kind; every identity and state
/// transition is reloaded and fenced by the database transaction below.
#[derive(Debug, Clone, PartialEq)]
pub struct ConvergedCandidateRecovery {
    pub recovery_case: CandidateRecoveryCaseRow,
    pub terminalized: Option<super::finding_lineage::TerminalizedCandidateAttempt>,
    pub candidate_reopened: bool,
    pub replayed: bool,
}

#[derive(Debug, Clone, sqlx::FromRow)]
struct AbandonBeforeSideEffectAuthority {
    attempt_id: Uuid,
    approval_id: Uuid,
    candidate_id: Uuid,
    operation_id: Uuid,
    scope_snapshot_id: Uuid,
    wave_run_id: Uuid,
    wave_unit_id: Uuid,
    organization_id: Uuid,
    worker_run_id: Uuid,
    attempt_status: String,
    attempt_row_version: i64,
    approval_status: String,
    approval_start_before: DateTime<Utc>,
    candidate_disposition: String,
    terminal_attempt_id: Option<Uuid>,
    terminal_finding_id: Option<Uuid>,
    worker_status: String,
    has_started_action: bool,
    has_terminal_intent: bool,
}

/// Safe, exact-scope Verification queue projection. This deliberately omits
/// lease tokens, checkpoint bodies/hashes, action arguments, raw tool output,
/// submitted result bodies, and operator-selected authority.
#[derive(Debug, Clone, PartialEq)]
pub struct CandidateVerificationQueueReadModel {
    pub operation_id: Uuid,
    pub scope_snapshot_id: Uuid,
    pub wave_run_id: Uuid,
    pub generation: i32,
    pub wave_status: String,
    pub wave_row_version: i64,
    pub wave_units: Vec<CandidateVerificationWaveUnitView>,
    pub consolidation: Option<CandidateVerificationConsolidationView>,
    pub pending_enrichment_count: usize,
    pub pending_enrichments: Vec<CandidateVerificationPendingEnrichmentView>,
    pub items: Vec<CandidateVerificationQueueItem>,
}

#[derive(Debug, Clone, PartialEq, Eq, sqlx::FromRow)]
pub struct CandidateVerificationWaveUnitView {
    pub wave_unit_id: Uuid,
    pub organization_id: Uuid,
    pub ordinal: i32,
    pub status: String,
    pub review_closed: bool,
    pub verification_closed: bool,
    pub consolidation_status: String,
    pub row_version: i64,
}

#[derive(Debug, Clone, PartialEq, Eq, sqlx::FromRow)]
pub struct CandidateVerificationConsolidationView {
    pub consolidation_id: Uuid,
    pub decision_kind: String,
    pub target_wave_run_id: Option<Uuid>,
    pub fact_delta_count: i32,
    pub reason_code: String,
    pub row_version: i64,
    pub decided_at: DateTime<Utc>,
}

/// Safe projection of an immutable FactDelta enrichment authority. The raw
/// request/evidence/output and all execution authority stay server-side.
#[derive(Debug, Clone, PartialEq, Eq, sqlx::FromRow)]
pub struct CandidateVerificationPendingEnrichmentView {
    pub enrichment_id: Uuid,
    pub fact_delta_id: Uuid,
    pub source_attempt_id: Uuid,
    pub candidate_id: Uuid,
    pub wave_unit_id: Uuid,
    pub organization_id: Uuid,
    pub subject_kind: String,
    pub subject_id: Uuid,
    pub target_type_at_time: String,
    pub target_value_at_time: String,
    pub delta_kind: String,
    pub observation_kind: String,
    pub allowed_techniques: Vec<String>,
    pub enrichment_required: bool,
    pub reason_code: String,
    pub status: String,
    pub created_at: DateTime<Utc>,
}

#[derive(Debug, Clone, PartialEq)]
pub struct CandidateVerificationQueueItem {
    pub attempt_id: Uuid,
    pub candidate_id: Uuid,
    pub approval_id: Uuid,
    pub wave_unit_id: Uuid,
    pub organization_id: Uuid,
    pub target_live_id: Option<Uuid>,
    pub target_type_at_time: String,
    pub target_value_at_time: String,
    pub target_identity_hash: String,
    pub candidate_plan_hash: String,
    pub hypothesis: String,
    pub technique: Option<String>,
    pub plan_schema_version: String,
    pub recipe_version: String,
    pub executor_contract_version: String,
    pub budget_max_actions: i64,
    pub budget_max_requests: i64,
    pub budget_max_runtime_ms: i64,
    pub approval_start_before: DateTime<Utc>,
    pub approval_expires_at: DateTime<Utc>,
    pub ordinal: i32,
    pub status: String,
    pub worker_run_id: Option<Uuid>,
    pub worker_status: Option<String>,
    pub row_version: i64,
    pub created_at: DateTime<Utc>,
    pub updated_at: DateTime<Utc>,
    pub terminal_at: Option<DateTime<Utc>>,
    pub observation_evidence: Vec<CandidateVerificationEvidenceView>,
    pub attempt_evidence: Vec<CandidateVerificationEvidenceView>,
    pub actions: Vec<CandidateVerificationActionView>,
    pub terminal_intent: Option<CandidateVerificationTerminalIntentView>,
    pub terminal_barrier: Option<CandidateVerificationTerminalBarrierView>,
    pub terminal_receipt: Option<CandidateVerificationTerminalReceiptView>,
    pub recovery_cases: Vec<CandidateVerificationRecoveryView>,
}

#[derive(Debug, Clone, PartialEq, Eq, sqlx::FromRow)]
pub struct CandidateVerificationEvidenceView {
    pub evidence_id: i64,
    pub role: String,
}

#[derive(Debug, Clone, PartialEq, Eq, sqlx::FromRow)]
pub struct CandidateVerificationActionView {
    pub attempt_id: Uuid,
    pub action_id: Uuid,
    pub action_ordinal: i32,
    pub capability_id: String,
    pub action_kind: String,
    pub status: String,
    pub outcome_hash: Option<String>,
    pub error_code: Option<String>,
    pub started_at: Option<DateTime<Utc>>,
    pub completed_at: Option<DateTime<Utc>>,
    pub authorization_receipt_id: Option<Uuid>,
    pub authorization_request_id: Option<String>,
    pub authorization_receipt_hash: Option<String>,
    pub authorized_at: Option<DateTime<Utc>>,
    pub start_before: Option<DateTime<Utc>>,
    pub execution_deadline: Option<DateTime<Utc>>,
}

#[derive(Debug, Clone, PartialEq, Eq, sqlx::FromRow)]
pub struct CandidateVerificationTerminalIntentView {
    pub attempt_id: Uuid,
    pub intent_id: Uuid,
    pub request_id: String,
    pub tool_call_record_id: Uuid,
    pub disposition: String,
    pub result_hash: String,
    pub evidence_manifest_hash: String,
    pub evidence_count: i32,
    pub intent_hash: String,
    pub created_at: DateTime<Utc>,
}

#[derive(Debug, Clone, PartialEq, Eq, sqlx::FromRow)]
pub struct CandidateVerificationTerminalBarrierView {
    pub attempt_id: Uuid,
    pub barrier_id: Uuid,
    pub intent_id: Uuid,
    pub request_id: String,
    pub tool_call_record_id: Uuid,
    pub created_at: DateTime<Utc>,
}

#[derive(Debug, Clone, PartialEq, Eq, sqlx::FromRow)]
pub struct CandidateVerificationTerminalReceiptView {
    pub attempt_id: Uuid,
    pub receipt_id: Uuid,
    pub intent_id: Uuid,
    pub barrier_id: Uuid,
    pub request_id: String,
    pub disposition: String,
    pub terminal_attempt_row_version: i64,
    pub finding_id: Option<Uuid>,
    pub fact_delta_count: i32,
    pub terminal_event_id: Uuid,
    pub receipt_hash: String,
    pub created_at: DateTime<Utc>,
}

#[derive(Debug, Clone, PartialEq)]
pub struct CandidateVerificationRecoveryView {
    pub recovery_case_id: Uuid,
    pub request_id: String,
    pub attempt_id: Uuid,
    pub action_id: Option<Uuid>,
    pub intent_id: Option<Uuid>,
    pub case_kind: String,
    pub reason_code: String,
    pub attempt_row_version: i64,
    pub status: String,
    pub resolution_kind: Option<String>,
    pub resolution_request_id: Option<String>,
    pub row_version: i64,
    pub evidence_ids: Vec<i64>,
    pub decided_at: Option<DateTime<Utc>>,
    pub completed_at: Option<DateTime<Utc>>,
    pub created_at: DateTime<Utc>,
    pub updated_at: DateTime<Utc>,
}

#[derive(Debug, sqlx::FromRow)]
struct CandidateVerificationWaveRow {
    scope_snapshot_id: Uuid,
    generation: i32,
    wave_status: String,
    wave_row_version: i64,
}

#[derive(Debug, sqlx::FromRow)]
struct CandidateVerificationQueueItemRow {
    attempt_id: Uuid,
    candidate_id: Uuid,
    approval_id: Uuid,
    wave_unit_id: Uuid,
    organization_id: Uuid,
    target_live_id: Option<Uuid>,
    target_type_at_time: String,
    target_value_at_time: String,
    target_identity_hash: String,
    candidate_plan_hash: String,
    hypothesis: String,
    technique: Option<String>,
    plan_schema_version: String,
    recipe_version: String,
    executor_contract_version: String,
    budget_max_actions: i64,
    budget_max_requests: i64,
    budget_max_runtime_ms: i64,
    approval_start_before: DateTime<Utc>,
    approval_expires_at: DateTime<Utc>,
    ordinal: i32,
    status: String,
    worker_run_id: Option<Uuid>,
    worker_status: Option<String>,
    row_version: i64,
    created_at: DateTime<Utc>,
    updated_at: DateTime<Utc>,
    terminal_at: Option<DateTime<Utc>>,
}

#[derive(Debug, sqlx::FromRow)]
struct CandidateVerificationEvidenceOwnerRow {
    attempt_id: Uuid,
    evidence_id: i64,
    role: String,
}

#[derive(Debug, sqlx::FromRow)]
struct CandidateVerificationRecoveryRow {
    recovery_case_id: Uuid,
    request_id: String,
    attempt_id: Uuid,
    action_id: Option<Uuid>,
    intent_id: Option<Uuid>,
    case_kind: String,
    reason_code: String,
    attempt_row_version: i64,
    status: String,
    resolution_kind: Option<String>,
    resolution_request_id: Option<String>,
    row_version: i64,
    decided_at: Option<DateTime<Utc>>,
    completed_at: Option<DateTime<Utc>>,
    created_at: DateTime<Utc>,
    updated_at: DateTime<Utc>,
}

#[derive(Debug, sqlx::FromRow)]
struct PendingIntentRecoveryWorkerRow {
    id: Uuid,
    stage_execution_id: Uuid,
    stage_run_unit_id: Uuid,
    organization_id: Uuid,
    message_chain_id: Option<Uuid>,
    status: String,
    checkpoint_version: i64,
    checkpoint: serde_json::Value,
    lease_token: Option<Uuid>,
    lease_owner: Option<String>,
    lease_acquired_at: Option<DateTime<Utc>>,
    lease_expires_at: Option<DateTime<Utc>>,
    heartbeat_at: Option<DateTime<Utc>>,
    attempt_epoch: i64,
    active_tool_call_id: Option<Uuid>,
}

#[derive(Debug, sqlx::FromRow)]
struct PendingIntentRecoveryToolRow {
    call_id: String,
    name: String,
    args: serde_json::Value,
    status: String,
    result: Option<String>,
}

#[derive(Debug, sqlx::FromRow)]
struct PendingIntentRecoveryChainRow {
    chain: Option<serde_json::Value>,
}

#[derive(Debug, sqlx::FromRow)]
struct TerminalAuthorityRow {
    attempt_id: Uuid,
    approval_id: Uuid,
    candidate_id: Uuid,
    operation_id: Uuid,
    scope_snapshot_id: Uuid,
    wave_run_id: Uuid,
    wave_unit_id: Uuid,
    organization_id: Uuid,
    candidate_plan_hash: String,
    worker_run_id: Uuid,
    stage_execution_id: Uuid,
    stage_run_unit_id: Uuid,
    lease_token: Uuid,
    lease_owner: Option<String>,
    attempt_epoch: i64,
    checkpoint_version: i64,
    submitted_result: serde_json::Value,
    result_hash: String,
}

const INTENT_COLUMNS: &str = "id,request_id,attempt_id,approval_id,candidate_id,operation_id,\
    scope_snapshot_id,wave_run_id,wave_unit_id,organization_id,target_identity_hash,\
    candidate_plan_hash,worker_run_id,attempt_epoch,lease_token,tool_call_record_id,\
    disposition,submitted_result,result_hash,evidence_manifest_hash,evidence_count,\
    tool_result_text,tool_result_hash,intent_hash,created_at";
const BARRIER_COLUMNS: &str = "id,request_id,intent_id,attempt_id,worker_run_id,\
    tool_call_record_id,message_chain_id,attempt_epoch,checkpoint_version,checkpoint_hash,\
    tool_result_hash,barrier_hash,created_at";
const RECEIPT_COLUMNS: &str = "id,request_id,intent_id,barrier_id,attempt_id,candidate_id,\
    operation_id,scope_snapshot_id,wave_run_id,wave_unit_id,organization_id,worker_run_id,\
    disposition,result_hash,terminal_attempt_row_version,finding_id,fact_delta_count,\
    terminal_event_id,receipt_payload,receipt_hash,created_at";
const RECOVERY_CASE_COLUMNS: &str = "id,request_id,attempt_id,action_id,intent_id,operation_id,\
    organization_id,candidate_id,worker_run_id,reason_code,attempt_row_version,status,\
    resolution_kind,resolution_request_id,resolution_payload,resolved_by,decided_at,\
    completed_at,row_version,created_at,updated_at";

fn conflict(message: impl Into<String>) -> crate::DbError {
    crate::DbError::Other(anyhow::anyhow!(message.into()))
}

fn valid_request_id(value: &str) -> bool {
    !value.is_empty()
        && value == value.trim()
        && value.len() <= MAX_REQUEST_ID_BYTES
        && !value.chars().any(char::is_control)
}

fn validate_evidence(evidence: &[AttemptEvidenceLink]) -> crate::Result<Vec<(i64, String)>> {
    let mut unique = BTreeSet::new();
    for link in evidence {
        if link.evidence_id <= 0
            || !matches!(
                link.role.as_str(),
                "proof" | "refutation" | "blocker" | "fact_delta"
            )
            || !unique.insert((link.evidence_id, link.role.clone()))
        {
            return Err(conflict("invalid or duplicate Candidate terminal evidence"));
        }
    }
    Ok(unique.into_iter().collect())
}

fn intent_id(attempt_id: Uuid, request_id: &str) -> Uuid {
    Uuid::new_v5(
        &attempt_id,
        format!("terminal-intent:{request_id}").as_bytes(),
    )
}

fn barrier_id(intent_id: Uuid, request_id: &str) -> Uuid {
    Uuid::new_v5(
        &intent_id,
        format!("terminal-barrier:{request_id}").as_bytes(),
    )
}

fn receipt_id(intent_id: Uuid) -> Uuid {
    Uuid::new_v5(&intent_id, b"terminal-receipt:v1")
}

async fn load_attempt(
    tx: &mut Transaction<'_, Postgres>,
    attempt_id: Uuid,
) -> crate::Result<CandidateAttemptRow> {
    let sql = format!("SELECT {ATTEMPT_COLUMNS} FROM candidate_attempts WHERE id=$1 FOR UPDATE");
    sqlx::query_as::<_, CandidateAttemptRow>(&sql)
        .bind(attempt_id)
        .fetch_optional(&mut **tx)
        .await?
        .ok_or_else(|| crate::DbError::NotFound("candidate_attempt".to_string()))
}

async fn load_intent_for_attempt(
    tx: &mut Transaction<'_, Postgres>,
    attempt_id: Uuid,
) -> crate::Result<Option<CandidateTerminalIntentRow>> {
    let sql = format!(
        "SELECT {INTENT_COLUMNS} FROM candidate_attempt_terminal_intents \
         WHERE attempt_id=$1 FOR UPDATE"
    );
    sqlx::query_as::<_, CandidateTerminalIntentRow>(&sql)
        .bind(attempt_id)
        .fetch_optional(&mut **tx)
        .await
        .map_err(Into::into)
}

fn intent_replay_matches(
    intent: &CandidateTerminalIntentRow,
    command: &RecordCandidateTerminalIntent,
) -> bool {
    intent.request_id == command.request_id
        && intent.operation_id == command.operation_id
        && intent.organization_id == command.organization_id
        && intent.candidate_id == command.candidate_id
        && intent.approval_id == command.approval_id
        && intent.attempt_id == command.attempt_id
        && intent.candidate_plan_hash == command.candidate_plan_hash
        && intent.worker_run_id == command.worker_run_id
        && intent.lease_token == command.lease_token
        && intent.attempt_epoch == command.attempt_epoch
        && intent.tool_call_record_id == command.tool_call_record_id
        && intent.disposition == command.disposition
        && intent.submitted_result == command.submitted_result
        && intent.tool_result_text == command.tool_result_text
}

/// Persist an immutable terminal intent and move the Attempt to the explicit
/// `terminalization_pending` state without exposing the result on the Attempt.
pub async fn record_candidate_terminal_intent(
    tx: &mut Transaction<'_, Postgres>,
    command: RecordCandidateTerminalIntent,
) -> crate::Result<RecordedCandidateTerminalIntent> {
    if !valid_request_id(&command.request_id)
        || command.operation_id.is_nil()
        || command.organization_id.is_nil()
        || command.candidate_id.is_nil()
        || command.approval_id.is_nil()
        || command.attempt_id.is_nil()
        || command.worker_run_id.is_nil()
        || command.lease_token.is_nil()
        || command.tool_call_record_id.is_nil()
        || command.attempt_epoch < 0
        || command.candidate_plan_hash.trim().is_empty()
        || !matches!(
            command.disposition.as_str(),
            "verified" | "refuted" | "blocked"
        )
        || !command.submitted_result.is_object()
        || command
            .submitted_result
            .get("disposition")
            .and_then(serde_json::Value::as_str)
            != Some(command.disposition.as_str())
        || command.tool_result_text.is_empty()
        || command.tool_result_text.len() > MAX_TOOL_RESULT_BYTES
    {
        return Err(conflict("invalid Candidate terminal intent"));
    }
    let evidence = validate_evidence(&command.evidence)?;
    candidate_attempts::lock_v2_operation(tx, command.operation_id).await?;
    let attempt = load_attempt(tx, command.attempt_id).await?;
    if let Some(intent) = load_intent_for_attempt(tx, command.attempt_id).await? {
        if !intent_replay_matches(&intent, &command) {
            return Err(conflict("Candidate terminal intent replay drift"));
        }
        let actual_evidence: Vec<(i64, String)> = sqlx::query_as(
            "SELECT evidence_id,role FROM candidate_attempt_evidence \
             WHERE attempt_id=$1 ORDER BY evidence_id,role",
        )
        .bind(command.attempt_id)
        .fetch_all(&mut **tx)
        .await?;
        if actual_evidence != evidence || attempt.status != "terminalization_pending" {
            return Err(conflict("Candidate terminal intent replay state drift"));
        }
        return Ok(RecordedCandidateTerminalIntent {
            intent,
            attempt,
            replayed: true,
        });
    }
    if attempt.operation_id != command.operation_id
        || attempt.organization_id != command.organization_id
        || attempt.candidate_id != command.candidate_id
        || attempt.approval_id != command.approval_id
        || attempt.candidate_plan_hash != command.candidate_plan_hash
        || attempt.stage_worker_run_id != Some(command.worker_run_id)
        || attempt.status != "running"
        || attempt.result_json.is_some()
        || attempt.result_hash.is_some()
    {
        return Err(conflict("Candidate terminal intent authority drift"));
    }
    candidate_attempts::lock_claim_scope(
        tx,
        attempt.operation_id,
        attempt.scope_snapshot_id,
        attempt.wave_run_id,
        attempt.wave_unit_id,
        attempt.organization_id,
    )
    .await?;
    let lane = super::attack_execution_lanes::lock_global(tx).await?;
    if lane.stage_worker_run_id != Some(command.worker_run_id)
        || lane.lease_token != Some(command.lease_token)
        || lane
            .lease_owner
            .as_deref()
            .is_none_or(|owner| owner.trim().is_empty())
        || lane
            .lease_expires_at
            .is_none_or(|expires_at| expires_at <= Utc::now())
    {
        return Err(conflict("Candidate terminal intent lane fence lost"));
    }
    let execution_plan: serde_json::Value = sqlx::query_scalar(
        "SELECT candidate.execution_plan FROM attack_candidates candidate \
         JOIN attack_candidate_approvals approval \
           ON approval.id=$8 AND approval.candidate_id=candidate.candidate_id \
          AND approval.operation_id=candidate.operation_uuid \
          AND approval.scope_snapshot_id=candidate.scope_snapshot_id \
          AND approval.wave_run_id=candidate.wave_run_id \
          AND approval.wave_unit_id=candidate.wave_unit_id \
          AND approval.organization_id=candidate.organization_id \
          AND approval.candidate_plan_hash=candidate.candidate_plan_hash \
         WHERE candidate.candidate_id=$1 AND candidate.operation_uuid=$2 \
           AND candidate.scope_snapshot_id=$3 AND candidate.wave_run_id=$4 \
           AND candidate.wave_unit_id=$5 AND candidate.organization_id=$6 \
           AND candidate.candidate_plan_hash=$7 AND candidate.disposition='approved' \
           AND approval.status IN ('approved','expired') FOR UPDATE OF candidate,approval",
    )
    .bind(command.candidate_id)
    .bind(command.operation_id)
    .bind(attempt.scope_snapshot_id)
    .bind(attempt.wave_run_id)
    .bind(attempt.wave_unit_id)
    .bind(command.organization_id)
    .bind(&command.candidate_plan_hash)
    .bind(command.approval_id)
    .fetch_optional(&mut **tx)
    .await?
    .ok_or_else(|| conflict("Candidate terminal intent plan authority missing"))?;
    let server_evidence = candidate_attempts::validate_terminal_action_journal(
        tx,
        command.attempt_id,
        &execution_plan,
    )
    .await?;
    candidate_attempts::validate_exact_replay_evidence_pairs(
        server_evidence.as_ref(),
        &command.evidence,
    )?;
    candidate_attempts::validate_exact_replay_result_evidence_pairs(
        server_evidence.as_ref(),
        &command.submitted_result,
    )?;
    for (evidence_id, role) in &evidence {
        sqlx::query(
            "INSERT INTO candidate_attempt_evidence(attempt_id,evidence_id,role) \
             VALUES($1,$2,$3)",
        )
        .bind(command.attempt_id)
        .bind(evidence_id)
        .bind(role)
        .execute(&mut **tx)
        .await?;
    }
    let id = intent_id(command.attempt_id, &command.request_id);
    let sql = format!(
        "INSERT INTO candidate_attempt_terminal_intents(\
             id,request_id,attempt_id,tool_call_record_id,lease_token,disposition,\
             submitted_result,tool_result_text) \
         VALUES($1,$2,$3,$4,$5,$6,$7,$8) RETURNING {INTENT_COLUMNS}"
    );
    let intent = sqlx::query_as::<_, CandidateTerminalIntentRow>(&sql)
        .bind(id)
        .bind(&command.request_id)
        .bind(command.attempt_id)
        .bind(command.tool_call_record_id)
        .bind(command.lease_token)
        .bind(&command.disposition)
        .bind(&command.submitted_result)
        .bind(&command.tool_result_text)
        .fetch_one(&mut **tx)
        .await?;
    let update_sql = format!(
        "UPDATE candidate_attempts \
         SET status='terminalization_pending',row_version=row_version+1,updated_at=NOW() \
         WHERE id=$1 AND status='running' AND result_json IS NULL AND result_hash IS NULL \
         RETURNING {ATTEMPT_COLUMNS}"
    );
    let attempt = sqlx::query_as::<_, CandidateAttemptRow>(&update_sql)
        .bind(command.attempt_id)
        .fetch_optional(&mut **tx)
        .await?
        .ok_or_else(|| conflict("Candidate terminal intent Attempt CAS lost"))?;
    Ok(RecordedCandidateTerminalIntent {
        intent,
        attempt,
        replayed: false,
    })
}

fn barrier_replay_matches(
    barrier: &CandidateTerminalBarrierRow,
    command: &RecordCandidateTerminalBarrier,
) -> bool {
    barrier.request_id == command.request_id
        && barrier.intent_id == command.intent_id
        && barrier.attempt_id == command.attempt_id
        && barrier.worker_run_id == command.worker_run_id
        && barrier.tool_call_record_id == command.tool_call_record_id
        && barrier.attempt_epoch == command.attempt_epoch
        && barrier.checkpoint_version == command.checkpoint_version
}

/// Persist the exact finished ToolResult/checkpoint fence. Callers should use
/// the same transaction that advances the Worker checkpoint.
pub async fn record_candidate_terminal_barrier(
    tx: &mut Transaction<'_, Postgres>,
    command: RecordCandidateTerminalBarrier,
) -> crate::Result<RecordedCandidateTerminalBarrier> {
    if !valid_request_id(&command.request_id)
        || command.intent_id.is_nil()
        || command.attempt_id.is_nil()
        || command.worker_run_id.is_nil()
        || command.tool_call_record_id.is_nil()
        || command.attempt_epoch < 0
        || command.checkpoint_version <= 0
    {
        return Err(conflict("invalid Candidate terminal barrier"));
    }
    let select_sql = format!(
        "SELECT {BARRIER_COLUMNS} FROM candidate_attempt_terminal_barriers \
         WHERE intent_id=$1 FOR UPDATE"
    );
    if let Some(barrier) = sqlx::query_as::<_, CandidateTerminalBarrierRow>(&select_sql)
        .bind(command.intent_id)
        .fetch_optional(&mut **tx)
        .await?
    {
        if !barrier_replay_matches(&barrier, &command) {
            return Err(conflict("Candidate terminal barrier replay drift"));
        }
        return Ok(RecordedCandidateTerminalBarrier {
            barrier,
            replayed: true,
        });
    }
    let id = barrier_id(command.intent_id, &command.request_id);
    let insert_sql = format!(
        "INSERT INTO candidate_attempt_terminal_barriers(\
             id,request_id,intent_id,checkpoint_version) \
         VALUES($1,$2,$3,$4) RETURNING {BARRIER_COLUMNS}"
    );
    let barrier = sqlx::query_as::<_, CandidateTerminalBarrierRow>(&insert_sql)
        .bind(id)
        .bind(&command.request_id)
        .bind(command.intent_id)
        .bind(command.checkpoint_version)
        .fetch_one(&mut **tx)
        .await?;
    if !barrier_replay_matches(&barrier, &command) {
        return Err(conflict("Candidate terminal barrier authority drift"));
    }
    Ok(RecordedCandidateTerminalBarrier {
        barrier,
        replayed: false,
    })
}

/// Atomically advance the exact bound chain/Worker checkpoint and persist the
/// Candidate terminal barrier. Exact response-loss replay compares the already
/// committed chain and checkpoint instead of incrementing them a second time.
pub async fn checkpoint_candidate_terminal_barrier(
    pool: &PgPool,
    command: CheckpointCandidateTerminalBarrier,
) -> crate::Result<RecordedCandidateTerminalBarrier> {
    if !valid_request_id(&command.request_id)
        || command.intent_id.is_nil()
        || command.expected_intent_hash.trim().is_empty()
        || !command.checkpoint.chain.is_array()
    {
        return Err(conflict("invalid Candidate terminal checkpoint barrier"));
    }
    let operation_id = command.checkpoint.fence.operation_id;
    let mut tx = pool.begin().await?;
    candidate_attempts::lock_v2_operation(&mut tx, operation_id).await?;
    let intent_sql = format!(
        "SELECT {INTENT_COLUMNS} FROM candidate_attempt_terminal_intents
         WHERE id=$1 AND operation_id=$2 FOR UPDATE"
    );
    let intent = sqlx::query_as::<_, CandidateTerminalIntentRow>(&intent_sql)
        .bind(command.intent_id)
        .bind(operation_id)
        .fetch_optional(&mut *tx)
        .await?
        .ok_or_else(|| crate::DbError::NotFound("candidate_terminal_intent".to_string()))?;
    if intent.intent_hash != command.expected_intent_hash
        || intent.worker_run_id != command.checkpoint.fence.worker_run_id
        || intent.lease_token != command.checkpoint.fence.lease_token
        || intent.attempt_epoch != command.checkpoint.fence.attempt_epoch
    {
        return Err(conflict("Candidate terminal checkpoint intent fence drift"));
    }
    let barrier_command = RecordCandidateTerminalBarrier {
        request_id: command.request_id,
        intent_id: intent.id,
        attempt_id: intent.attempt_id,
        worker_run_id: intent.worker_run_id,
        tool_call_record_id: intent.tool_call_record_id,
        attempt_epoch: intent.attempt_epoch,
        checkpoint_version: command
            .checkpoint
            .fence
            .expected_checkpoint_version
            .checked_add(1)
            .ok_or_else(|| conflict("Candidate terminal checkpoint version overflow"))?,
    };
    let select_sql = format!(
        "SELECT {BARRIER_COLUMNS} FROM candidate_attempt_terminal_barriers
         WHERE intent_id=$1 FOR UPDATE"
    );
    if let Some(existing) = sqlx::query_as::<_, CandidateTerminalBarrierRow>(&select_sql)
        .bind(intent.id)
        .fetch_optional(&mut *tx)
        .await?
    {
        if !barrier_replay_matches(&existing, &barrier_command)
            || existing.message_chain_id != command.checkpoint.message_chain_id
        {
            return Err(conflict(
                "Candidate terminal checkpoint barrier replay drift",
            ));
        }
        let persisted: Option<(serde_json::Value, serde_json::Value)> = sqlx::query_as(
            "SELECT worker.checkpoint,chain.chain
             FROM stage_worker_runs worker
             JOIN message_chains chain ON chain.id=worker.message_chain_id
             WHERE worker.id=$1 AND worker.operation_id=$2
               AND worker.stage_execution_id=$3 AND worker.stage_run_unit_id=$4
               AND worker.message_chain_id=$5 AND worker.checkpoint_version=$6
             FOR SHARE OF worker,chain",
        )
        .bind(command.checkpoint.fence.worker_run_id)
        .bind(operation_id)
        .bind(command.checkpoint.fence.stage_execution_id)
        .bind(command.checkpoint.fence.stage_run_unit_id)
        .bind(command.checkpoint.message_chain_id)
        .bind(existing.checkpoint_version)
        .fetch_optional(&mut *tx)
        .await?;
        if persisted.as_ref()
            != Some(&(
                command.checkpoint.checkpoint.clone(),
                command.checkpoint.chain.clone(),
            ))
        {
            return Err(conflict(
                "Candidate terminal checkpoint replay payload drift",
            ));
        }
        tx.commit().await?;
        return Ok(RecordedCandidateTerminalBarrier {
            barrier: existing,
            replayed: true,
        });
    }
    let (worker, _contract) =
        super::runtime_memory_tx::checkpoint_bound_worker_chain_in_transaction(
            &mut tx,
            &command.checkpoint,
        )
        .await
        .map_err(|error| conflict(format!("Candidate terminal checkpoint failed: {error}")))?;
    if worker.checkpoint_version != barrier_command.checkpoint_version
        || worker.message_chain_id != Some(command.checkpoint.message_chain_id)
    {
        return Err(conflict("Candidate terminal checkpoint authority drift"));
    }
    let recorded = record_candidate_terminal_barrier(&mut tx, barrier_command).await?;
    tx.commit().await?;
    Ok(recorded)
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
enum TerminalToolPairState {
    Missing,
    Exact,
}

fn terminal_tool_pair_state(
    chain: &serde_json::Value,
    intent: &CandidateTerminalIntentRow,
    tool: &PendingIntentRecoveryToolRow,
) -> crate::Result<TerminalToolPairState> {
    let messages = chain
        .as_array()
        .ok_or_else(|| conflict("Candidate terminal recovery chain is not an array"))?;
    let Some((assistant, user)) = messages
        .len()
        .checked_sub(2)
        .map(|index| (&messages[index], &messages[index + 1]))
    else {
        return Ok(TerminalToolPairState::Missing);
    };
    let assistant_calls = assistant
        .get("content")
        .and_then(serde_json::Value::as_array)
        .into_iter()
        .flatten()
        .filter(|content| {
            content
                .get("function")
                .and_then(|function| function.get("name"))
                .and_then(serde_json::Value::as_str)
                == Some("submit_candidate_attempt")
        })
        .collect::<Vec<_>>();
    let user_results = user
        .get("content")
        .and_then(serde_json::Value::as_array)
        .into_iter()
        .flatten()
        .filter(|content| {
            content.get("type").and_then(serde_json::Value::as_str) == Some("toolresult")
        })
        .collect::<Vec<_>>();
    if assistant_calls.is_empty() {
        let carries_terminal_result = user_results.iter().any(|result| {
            result
                .get("content")
                .and_then(serde_json::Value::as_array)
                .into_iter()
                .flatten()
                .any(|content| {
                    content.get("text").and_then(serde_json::Value::as_str)
                        == Some(intent.tool_result_text.as_str())
                })
        });
        return if carries_terminal_result {
            Err(conflict(
                "Candidate terminal recovery chain has an orphan ToolResult",
            ))
        } else {
            Ok(TerminalToolPairState::Missing)
        };
    }
    if assistant.get("role").and_then(serde_json::Value::as_str) != Some("assistant")
        || user.get("role").and_then(serde_json::Value::as_str) != Some("user")
        || assistant_calls.len() != 1
    {
        return Err(conflict(
            "Candidate terminal recovery chain has ambiguous submit ToolCall history",
        ));
    }
    let call = assistant_calls[0];
    if call
        .get("function")
        .and_then(|function| function.get("arguments"))
        != Some(&tool.args)
    {
        return Err(conflict(
            "Candidate terminal recovery chain submit arguments drift",
        ));
    }
    let call_key = call
        .get("call_id")
        .and_then(serde_json::Value::as_str)
        .or_else(|| call.get("id").and_then(serde_json::Value::as_str))
        .filter(|value| !value.trim().is_empty())
        .ok_or_else(|| conflict("Candidate terminal recovery chain ToolCall id missing"))?;
    let matching_results = user_results
        .iter()
        .filter(|result| {
            result
                .get("call_id")
                .and_then(serde_json::Value::as_str)
                .or_else(|| result.get("id").and_then(serde_json::Value::as_str))
                == Some(call_key)
        })
        .collect::<Vec<_>>();
    if matching_results.len() != 1 {
        return Err(conflict(
            "Candidate terminal recovery chain ToolResult identity drift",
        ));
    }
    let result_texts = matching_results[0]
        .get("content")
        .and_then(serde_json::Value::as_array)
        .into_iter()
        .flatten()
        .filter_map(|content| content.get("text").and_then(serde_json::Value::as_str))
        .collect::<Vec<_>>();
    if result_texts.as_slice() != [intent.tool_result_text.as_str()] {
        return Err(conflict(
            "Candidate terminal recovery chain ToolResult payload drift",
        ));
    }
    Ok(TerminalToolPairState::Exact)
}

fn append_recovered_terminal_tool_pair(
    mut chain: serde_json::Value,
    intent: &CandidateTerminalIntentRow,
    tool: &PendingIntentRecoveryToolRow,
) -> crate::Result<serde_json::Value> {
    let messages = chain
        .as_array_mut()
        .ok_or_else(|| conflict("Candidate terminal recovery chain is not an array"))?;
    if tool.name != "submit_candidate_attempt"
        || !tool.args.is_object()
        || tool.call_id.trim().is_empty()
    {
        return Err(conflict(
            "Candidate terminal recovery tool identity is not provider-safe",
        ));
    }

    // The generic tool ledger does not retain the provider's optional internal
    // ToolCall id. Recovery therefore creates a stable server id while keeping
    // the durable event correlation id as `call_id` on both sides of the pair.
    // This is a complete rig-compatible assistant ToolCall + user ToolResult
    // turn, but it is terminal audit history only: no provider or external
    // action is invoked during recovery.
    let recovered_tool_id = format!("candidate-terminal-recovery-{}", intent.tool_call_record_id);
    messages.push(serde_json::json!({
        "role": "assistant",
        "id": null,
        "content": [{
            "id": recovered_tool_id,
            "call_id": tool.call_id,
            "function": {
                "name": tool.name,
                "arguments": tool.args,
            },
            "signature": null,
            "additional_params": null,
        }],
    }));
    messages.push(serde_json::json!({
        "role": "user",
        "content": [{
            "type": "toolresult",
            "id": recovered_tool_id,
            "call_id": tool.call_id,
            "content": [{
                "type": "text",
                "text": intent.tool_result_text,
            }],
        }],
    }));
    Ok(chain)
}

/// Recover the post-submit protocol without replaying the verifier Action.
///
/// A committed TerminalIntent proves that `submit_candidate_attempt` already
/// passed the original active-tool/lease fence. This transaction may therefore
/// finish only that exact telemetry row, clear only that exact active-tool
/// marker, append a deterministic terminal ToolCall/ToolResult pair, advance
/// the existing chain checkpoint once, and create the immutable barrier. It
/// never calls an adapter, creates an Action, or accepts caller-provided result
/// data. An existing barrier is returned as an exact no-write replay.
pub async fn recover_candidate_terminal_intent_barrier(
    pool: &PgPool,
    command: RecoverCandidateTerminalIntent,
) -> crate::Result<RecoveredCandidateTerminalBarrier> {
    if command.operation_id.is_nil()
        || command.intent_id.is_nil()
        || command.expected_intent_hash.trim().is_empty()
    {
        return Err(conflict("invalid Candidate terminal recovery command"));
    }

    let mut tx = pool.begin().await?;
    candidate_attempts::lock_v2_operation(&mut tx, command.operation_id).await?;
    let intent_sql = format!(
        "SELECT {INTENT_COLUMNS} FROM candidate_attempt_terminal_intents
         WHERE id=$1 AND operation_id=$2 FOR UPDATE"
    );
    let intent = sqlx::query_as::<_, CandidateTerminalIntentRow>(&intent_sql)
        .bind(command.intent_id)
        .bind(command.operation_id)
        .fetch_optional(&mut *tx)
        .await?
        .ok_or_else(|| crate::DbError::NotFound("candidate_terminal_intent".to_string()))?;
    if intent.intent_hash != command.expected_intent_hash {
        return Err(conflict("Candidate terminal recovery intent hash drift"));
    }

    let barrier_sql = format!(
        "SELECT {BARRIER_COLUMNS} FROM candidate_attempt_terminal_barriers
         WHERE intent_id=$1 FOR UPDATE"
    );
    if let Some(existing) = sqlx::query_as::<_, CandidateTerminalBarrierRow>(&barrier_sql)
        .bind(intent.id)
        .fetch_optional(&mut *tx)
        .await?
    {
        if existing.attempt_id != intent.attempt_id
            || existing.worker_run_id != intent.worker_run_id
            || existing.tool_call_record_id != intent.tool_call_record_id
            || existing.attempt_epoch != intent.attempt_epoch
            || existing.tool_result_hash != intent.tool_result_hash
        {
            return Err(conflict("Candidate terminal recovery barrier drift"));
        }
        tx.commit().await?;
        return Ok(RecoveredCandidateTerminalBarrier {
            barrier: existing,
            replayed: true,
            tool_reconciled: false,
            worker_reconciled: false,
        });
    }

    let attempt = load_attempt(&mut tx, intent.attempt_id).await?;
    if attempt.operation_id != intent.operation_id
        || attempt.organization_id != intent.organization_id
        || attempt.candidate_id != intent.candidate_id
        || attempt.approval_id != intent.approval_id
        || attempt.stage_worker_run_id != Some(intent.worker_run_id)
        || attempt.status != "terminalization_pending"
        || attempt.result_json.is_some()
        || attempt.result_hash.is_some()
        || attempt.terminal_at.is_some()
    {
        return Err(conflict(
            "Candidate terminal recovery Attempt authority drift",
        ));
    }

    let worker = sqlx::query_as::<_, PendingIntentRecoveryWorkerRow>(
        r#"SELECT worker.id,worker.stage_execution_id,worker.stage_run_unit_id,
                  worker.organization_id,worker.message_chain_id,worker.status,
                  worker.checkpoint_version,worker.checkpoint,
                  worker.lease_token,worker.lease_owner,
                  worker.lease_acquired_at,worker.lease_expires_at,worker.heartbeat_at,
                  worker.attempt_epoch,worker.active_tool_call_id
             FROM stage_worker_runs worker
             JOIN stage_runs execution
               ON execution.id=worker.stage_execution_id
              AND execution.operation_id=worker.operation_id
              AND execution.stage_kind='verification'
              AND execution.status='started'
             JOIN stage_run_units unit
               ON unit.id=worker.stage_run_unit_id
              AND unit.operation_id=worker.operation_id
              AND unit.stage_execution_id=worker.stage_execution_id
              AND unit.organization_id=worker.organization_id
              AND unit.stage_kind='verification'
              AND unit.specialist='candidate_verifier'
              AND unit.status IN ('queued','running')
            WHERE worker.id=$1 AND worker.operation_id=$2
              AND worker.organization_id=$3
            FOR UPDATE OF worker"#,
    )
    .bind(intent.worker_run_id)
    .bind(intent.operation_id)
    .bind(intent.organization_id)
    .fetch_optional(&mut *tx)
    .await?
    .ok_or_else(|| conflict("Candidate terminal recovery Worker scope drift"))?;
    let original_lease_retained = worker.lease_token == Some(intent.lease_token)
        && worker
            .lease_owner
            .as_deref()
            .is_some_and(|owner| !owner.trim().is_empty())
        && worker.lease_acquired_at.is_some()
        && worker.lease_expires_at.is_some();
    let expired_lease_cleared = worker.lease_token.is_none()
        && worker.lease_owner.is_none()
        && worker.lease_acquired_at.is_none()
        && worker.lease_expires_at.is_none()
        && worker.heartbeat_at.is_none();
    let valid_worker_state = match worker.status.as_str() {
        "running" => original_lease_retained,
        "recovery_required" => original_lease_retained || expired_lease_cleared,
        "queued" => expired_lease_cleared,
        _ => false,
    };
    if worker.id != intent.worker_run_id
        || worker.organization_id != intent.organization_id
        || worker.attempt_epoch != intent.attempt_epoch
        || worker.message_chain_id.is_none()
        || !valid_worker_state
        || worker
            .active_tool_call_id
            .is_some_and(|tool_id| tool_id != intent.tool_call_record_id)
    {
        return Err(conflict(
            "Candidate terminal recovery Worker fence is not recoverable",
        ));
    }

    let tool = sqlx::query_as::<_, PendingIntentRecoveryToolRow>(
        r#"SELECT call_id,name,args,status::TEXT AS status,result
             FROM tool_calls
            WHERE id=$1 AND worker_run_id=$2 AND operation_id=$3
              AND stage_execution_id=$4 AND stage_run_unit_id=$5
              AND organization_id=$6 AND attempt_epoch=$7 AND lease_token=$8
            FOR UPDATE"#,
    )
    .bind(intent.tool_call_record_id)
    .bind(intent.worker_run_id)
    .bind(intent.operation_id)
    .bind(worker.stage_execution_id)
    .bind(worker.stage_run_unit_id)
    .bind(intent.organization_id)
    .bind(intent.attempt_epoch)
    .bind(intent.lease_token)
    .fetch_optional(&mut *tx)
    .await?
    .ok_or_else(|| conflict("Candidate terminal recovery tool identity drift"))?;
    let tool_reconciled = match (tool.status.as_str(), tool.result.as_deref()) {
        ("received" | "running", None) => {
            let updated = sqlx::query(
                "UPDATE tool_calls
                    SET status='finished',result=$2,duration_ms=COALESCE(duration_ms,0),
                        updated_at=NOW()
                  WHERE id=$1 AND status IN ('received','running') AND result IS NULL",
            )
            .bind(intent.tool_call_record_id)
            .bind(&intent.tool_result_text)
            .execute(&mut *tx)
            .await?;
            if updated.rows_affected() != 1 {
                return Err(conflict("Candidate terminal recovery tool CAS lost"));
            }
            true
        }
        ("finished", Some(result)) if result == intent.tool_result_text => false,
        _ => {
            return Err(conflict(
                "Candidate terminal recovery tool result is ambiguous",
            ));
        }
    };

    let message_chain_id = worker
        .message_chain_id
        .ok_or_else(|| conflict("Candidate terminal recovery chain missing"))?;
    let chain = sqlx::query_as::<_, PendingIntentRecoveryChainRow>(
        "SELECT chain FROM message_chains
         WHERE id=$1 AND task_id=$2 FOR UPDATE",
    )
    .bind(message_chain_id)
    .bind(intent.operation_id)
    .fetch_optional(&mut *tx)
    .await?
    .ok_or_else(|| conflict("Candidate terminal recovery chain owner drift"))?
    .chain
    .ok_or_else(|| conflict("Candidate terminal recovery chain body missing"))?;
    let pair_state = terminal_tool_pair_state(&chain, &intent, &tool)?;
    let (next_checkpoint_version, worker_reconciled) = match pair_state {
        TerminalToolPairState::Exact => {
            if worker.checkpoint_version <= 0 || worker.checkpoint != chain {
                return Err(conflict(
                    "Candidate terminal recovery chain/checkpoint pair drift",
                ));
            }
            if worker.active_tool_call_id == Some(intent.tool_call_record_id) {
                let cleared = sqlx::query(
                    "UPDATE stage_worker_runs
                        SET active_tool_call_id=NULL,active_tool_started_at=NULL,updated_at=NOW()
                      WHERE id=$1 AND checkpoint_version=$2 AND status=$3
                        AND active_tool_call_id=$4",
                )
                .bind(worker.id)
                .bind(worker.checkpoint_version)
                .bind(&worker.status)
                .bind(intent.tool_call_record_id)
                .execute(&mut *tx)
                .await?;
                if cleared.rows_affected() != 1 {
                    return Err(conflict(
                        "Candidate terminal recovery active-tool clear CAS lost",
                    ));
                }
                (worker.checkpoint_version, true)
            } else {
                (worker.checkpoint_version, false)
            }
        }
        TerminalToolPairState::Missing => {
            let recovered_chain = append_recovered_terminal_tool_pair(chain, &intent, &tool)?;
            let chain_updated = sqlx::query(
                "UPDATE message_chains SET chain=$3,updated_at=NOW()
                 WHERE id=$1 AND task_id=$2",
            )
            .bind(message_chain_id)
            .bind(intent.operation_id)
            .bind(&recovered_chain)
            .execute(&mut *tx)
            .await?;
            if chain_updated.rows_affected() != 1 {
                return Err(conflict("Candidate terminal recovery chain CAS lost"));
            }
            let next_checkpoint_version = worker
                .checkpoint_version
                .checked_add(1)
                .ok_or_else(|| conflict("Candidate terminal recovery checkpoint overflow"))?;
            let worker_updated = sqlx::query_scalar::<_, i64>(
                "UPDATE stage_worker_runs
                    SET active_tool_call_id=NULL,active_tool_started_at=NULL,
                        checkpoint=$3,checkpoint_version=checkpoint_version+1,updated_at=NOW()
                  WHERE id=$1 AND checkpoint_version=$2 AND status=$4
                    AND (active_tool_call_id IS NULL OR active_tool_call_id=$5)
                  RETURNING checkpoint_version",
            )
            .bind(worker.id)
            .bind(worker.checkpoint_version)
            .bind(&recovered_chain)
            .bind(&worker.status)
            .bind(intent.tool_call_record_id)
            .fetch_optional(&mut *tx)
            .await?
            .ok_or_else(|| conflict("Candidate terminal recovery Worker CAS lost"))?;
            if worker_updated != next_checkpoint_version {
                return Err(conflict(
                    "Candidate terminal recovery checkpoint authority drift",
                ));
            }
            (next_checkpoint_version, true)
        }
    };

    let barrier_request_id = format!("candidate-terminal-recovery:{}", intent.id);
    let recorded = record_candidate_terminal_barrier(
        &mut tx,
        RecordCandidateTerminalBarrier {
            request_id: barrier_request_id,
            intent_id: intent.id,
            attempt_id: intent.attempt_id,
            worker_run_id: intent.worker_run_id,
            tool_call_record_id: intent.tool_call_record_id,
            attempt_epoch: intent.attempt_epoch,
            checkpoint_version: next_checkpoint_version,
        },
    )
    .await?;
    if recorded.replayed {
        return Err(conflict(
            "Candidate terminal recovery barrier appeared during locked recovery",
        ));
    }
    tx.commit().await?;
    Ok(RecoveredCandidateTerminalBarrier {
        barrier: recorded.barrier,
        replayed: false,
        tool_reconciled,
        worker_reconciled,
    })
}

const TERMINAL_INTENT_QUEUE_COLUMNS: &str =
    "intent.operation_id,intent.id AS intent_id,intent.request_id,intent.organization_id,\
     intent.candidate_id,intent.attempt_id,intent.worker_run_id,intent.tool_call_record_id,\
     intent.candidate_plan_hash,intent.result_hash,intent.evidence_manifest_hash,\
     intent.tool_result_hash,intent.intent_hash,barrier.id AS barrier_id,\
     barrier.barrier_hash,receipt.id AS receipt_id,intent.created_at";

/// Return the oldest unconsumed intent, including the optional durable
/// checkpoint barrier. The scheduler must see `pending` intents too; silently
/// skipping one would allow a new claim past a response-loss recovery fence.
pub async fn next_candidate_terminal_intent(
    pool: &PgPool,
    operation_id: Uuid,
) -> crate::Result<Option<CandidateTerminalIntentQueueRow>> {
    if operation_id.is_nil() {
        return Err(conflict("invalid Candidate terminal intent operation"));
    }
    let sql = format!(
        "SELECT {TERMINAL_INTENT_QUEUE_COLUMNS} \
         FROM candidate_attempt_terminal_intents intent \
         LEFT JOIN candidate_attempt_terminal_barriers barrier ON barrier.intent_id=intent.id \
         JOIN candidate_attempts attempt ON attempt.id=intent.attempt_id \
         LEFT JOIN candidate_attempt_terminal_receipts receipt ON receipt.intent_id=intent.id \
         WHERE intent.operation_id=$1 AND receipt.id IS NULL \
           AND attempt.status='terminalization_pending' \
         ORDER BY intent.created_at,intent.id LIMIT 1"
    );
    sqlx::query_as::<_, CandidateTerminalIntentQueueRow>(&sql)
        .bind(operation_id)
        .fetch_optional(pool)
        .await
        .map_err(Into::into)
}

/// Load one exact intent and its barrier/receipt state for bridge-side
/// identity and hash mapping. No submitted result or lease secret is exposed.
pub async fn load_candidate_terminal_intent(
    pool: &PgPool,
    operation_id: Uuid,
    intent_id: Uuid,
) -> crate::Result<Option<CandidateTerminalIntentQueueRow>> {
    if operation_id.is_nil() || intent_id.is_nil() {
        return Err(conflict("invalid Candidate terminal intent identity"));
    }
    let sql = format!(
        "SELECT {TERMINAL_INTENT_QUEUE_COLUMNS} \
         FROM candidate_attempt_terminal_intents intent \
         LEFT JOIN candidate_attempt_terminal_barriers barrier ON barrier.intent_id=intent.id \
         LEFT JOIN candidate_attempt_terminal_receipts receipt ON receipt.intent_id=intent.id \
         WHERE intent.operation_id=$1 AND intent.id=$2"
    );
    sqlx::query_as::<_, CandidateTerminalIntentQueueRow>(&sql)
        .bind(operation_id)
        .bind(intent_id)
        .fetch_optional(pool)
        .await
        .map_err(Into::into)
}

async fn load_receipt_for_intent(
    tx: &mut Transaction<'_, Postgres>,
    intent_id: Uuid,
) -> crate::Result<Option<CandidateTerminalReceiptRow>> {
    let sql = format!(
        "SELECT {RECEIPT_COLUMNS} FROM candidate_attempt_terminal_receipts \
         WHERE intent_id=$1 FOR UPDATE"
    );
    sqlx::query_as::<_, CandidateTerminalReceiptRow>(&sql)
        .bind(intent_id)
        .fetch_optional(&mut **tx)
        .await
        .map_err(Into::into)
}

async fn terminalized_from_receipt(
    tx: &mut Transaction<'_, Postgres>,
    receipt: &CandidateTerminalReceiptRow,
    replayed: bool,
) -> crate::Result<super::finding_lineage::TerminalizedCandidateAttempt> {
    let evidence_count: i64 = sqlx::query_scalar(
        "SELECT COUNT(DISTINCT evidence_id) FROM candidate_attempt_evidence WHERE attempt_id=$1",
    )
    .bind(receipt.attempt_id)
    .fetch_one(&mut **tx)
    .await?;
    Ok(super::finding_lineage::TerminalizedCandidateAttempt {
        scope_snapshot_id: receipt.scope_snapshot_id,
        wave_run_id: receipt.wave_run_id,
        wave_unit_id: receipt.wave_unit_id,
        organization_id: receipt.organization_id,
        candidate_id: receipt.candidate_id,
        attempt_id: receipt.attempt_id,
        status: receipt.disposition.clone(),
        disposition: receipt.disposition.clone(),
        finding_id: receipt.finding_id,
        evidence_count: u32::try_from(evidence_count)
            .map_err(|_| conflict("Candidate terminal receipt evidence count overflow"))?,
        fact_delta_count: u32::try_from(receipt.fact_delta_count)
            .map_err(|_| conflict("Candidate terminal receipt FactDelta count overflow"))?,
        replayed,
    })
}

/// Consume one barrier-ready intent under server authority. The existing
/// canonical terminalizer and the immutable terminal receipt share this short
/// transaction, so response loss can only yield a replayable receipt.
pub async fn terminalize_candidate_terminal_intent(
    pool: &PgPool,
    command: TerminalizeCandidateTerminalIntent,
) -> crate::Result<TerminalizedCandidateTerminalIntent> {
    if !valid_request_id(&command.request_id)
        || command.operation_id.is_nil()
        || command.intent_id.is_nil()
        || command.barrier_id.is_nil()
    {
        return Err(conflict("invalid Candidate terminalization request"));
    }
    let mut tx = pool.begin().await?;
    if let Some(receipt) = load_receipt_for_intent(&mut tx, command.intent_id).await? {
        if receipt.request_id != command.request_id
            || receipt.operation_id != command.operation_id
            || receipt.barrier_id != command.barrier_id
        {
            return Err(conflict("Candidate terminal receipt replay drift"));
        }
        let terminalized = terminalized_from_receipt(&mut tx, &receipt, true).await?;
        tx.commit().await?;
        return Ok(TerminalizedCandidateTerminalIntent {
            receipt,
            terminalized,
            replayed: true,
        });
    }
    let authority = sqlx::query_as::<_, TerminalAuthorityRow>(
        r#"SELECT intent.attempt_id,intent.approval_id,intent.candidate_id,
                  intent.operation_id,intent.scope_snapshot_id,intent.wave_run_id,
                  intent.wave_unit_id,intent.organization_id,intent.candidate_plan_hash,
                  intent.worker_run_id,worker.stage_execution_id,worker.stage_run_unit_id,
                  intent.lease_token,
                  COALESCE(
                      worker.lease_owner,
                      (SELECT lane.lease_owner FROM attack_execution_lanes lane
                        WHERE lane.lane_key='global:exploit'
                          AND lane.stage_worker_run_id=intent.worker_run_id
                          AND lane.lease_token=intent.lease_token)
                  ) AS lease_owner,
                  intent.attempt_epoch,
                  barrier.checkpoint_version,intent.submitted_result,intent.result_hash
             FROM candidate_attempt_terminal_intents intent
             JOIN candidate_attempt_terminal_barriers barrier
               ON barrier.id=$2 AND barrier.intent_id=intent.id
             JOIN stage_worker_runs worker ON worker.id=intent.worker_run_id
            WHERE intent.id=$1 AND intent.operation_id=$3
            FOR UPDATE OF intent,barrier,worker"#,
    )
    .bind(command.intent_id)
    .bind(command.barrier_id)
    .bind(command.operation_id)
    .fetch_optional(&mut *tx)
    .await?
    .ok_or_else(|| crate::DbError::NotFound("candidate_terminal_intent".to_string()))?;
    // A generic expired-worker reaper may already have cleared both the Worker
    // and lane owner. Server terminalization does not resurrect that authority;
    // this deterministic label is used only when no retained exact lane needs
    // releasing. `finding_lineage` still rejects a retained lane whose owner
    // does not match the exact original identity loaded above.
    let lease_owner = authority
        .lease_owner
        .clone()
        .filter(|owner| !owner.trim().is_empty())
        .unwrap_or_else(|| format!("candidate-terminalizer:{}", authority.worker_run_id));
    let terminalized = super::finding_lineage::terminalize_candidate_attempt_from_intent(
        &mut tx,
        super::finding_lineage::TerminalizeCandidateAttempt {
            operation_id: authority.operation_id,
            scope_snapshot_id: authority.scope_snapshot_id,
            wave_run_id: authority.wave_run_id,
            wave_unit_id: authority.wave_unit_id,
            organization_id: authority.organization_id,
            candidate_id: authority.candidate_id,
            approval_id: authority.approval_id,
            attempt_id: authority.attempt_id,
            candidate_plan_hash: authority.candidate_plan_hash.clone(),
            expected_result_hash: authority.result_hash.clone(),
            worker_run_id: authority.worker_run_id,
            stage_execution_id: authority.stage_execution_id,
            stage_run_unit_id: authority.stage_run_unit_id,
            lease_token: authority.lease_token,
            lease_owner,
            attempt_epoch: authority.attempt_epoch,
            expected_checkpoint_version: authority.checkpoint_version,
        },
        authority.submitted_result,
    )
    .await?;
    let id = receipt_id(command.intent_id);
    let insert_sql = format!(
        "INSERT INTO candidate_attempt_terminal_receipts(\
             id,request_id,intent_id,barrier_id) VALUES($1,$2,$3,$4) \
         RETURNING {RECEIPT_COLUMNS}"
    );
    let receipt = sqlx::query_as::<_, CandidateTerminalReceiptRow>(&insert_sql)
        .bind(id)
        .bind(&command.request_id)
        .bind(command.intent_id)
        .bind(command.barrier_id)
        .fetch_one(&mut *tx)
        .await?;
    tx.commit().await?;
    Ok(TerminalizedCandidateTerminalIntent {
        receipt,
        terminalized,
        replayed: false,
    })
}

/// Load the complete safe protocol view for one immutable operation + Wave.
///
/// All component queries share one read-only repeatable-read snapshot. The
/// first query proves the active project and sealed scope binding; every child
/// then joins through the exact Attempt identity. No caller-selected org or
/// worker fence is accepted.
pub async fn list_verification_queue(
    pool: &PgPool,
    operation_id: Uuid,
    wave_run_id: Uuid,
) -> crate::Result<CandidateVerificationQueueReadModel> {
    if operation_id.is_nil() || wave_run_id.is_nil() {
        return Err(conflict("invalid Candidate Verification queue scope"));
    }
    let mut tx = pool.begin().await?;
    sqlx::query("SET TRANSACTION ISOLATION LEVEL REPEATABLE READ READ ONLY")
        .execute(&mut *tx)
        .await?;
    let wave = sqlx::query_as::<_, CandidateVerificationWaveRow>(
        r#"SELECT wave.scope_snapshot_id,wave.generation,
                  wave.status AS wave_status,wave.row_version AS wave_row_version
             FROM attack_wave_runs wave
             JOIN operation_state operation
               ON operation.operation_id=wave.operation_id
              AND operation.project_scope_id IS NOT NULL
             JOIN project_scopes project
               ON project.project_scope_id=operation.project_scope_id
              AND project.retired_at IS NULL
             JOIN operation_org_scope_snapshots snapshot
               ON snapshot.id=wave.scope_snapshot_id
              AND snapshot.operation_id=wave.operation_id
              AND snapshot.project_scope_id=operation.project_scope_id
              AND snapshot.sealed_at IS NOT NULL
            WHERE wave.id=$1 AND wave.operation_id=$2"#,
    )
    .bind(wave_run_id)
    .bind(operation_id)
    .fetch_optional(&mut *tx)
    .await?
    .ok_or_else(|| crate::DbError::NotFound("candidate_verification_wave".to_string()))?;

    let wave_units = sqlx::query_as::<_, CandidateVerificationWaveUnitView>(
        r#"SELECT unit.id AS wave_unit_id,unit.organization_id,unit.ordinal,
                  unit.status,unit.review_closed,unit.verification_closed,
                  unit.consolidation_status,unit.row_version
             FROM attack_wave_units unit
             JOIN operation_org_scope_units scope_unit
               ON scope_unit.snapshot_id=unit.scope_snapshot_id
              AND scope_unit.organization_id=unit.organization_id
            WHERE unit.operation_id=$1 AND unit.wave_run_id=$2
              AND unit.scope_snapshot_id=$3
            ORDER BY unit.ordinal,unit.organization_id"#,
    )
    .bind(operation_id)
    .bind(wave_run_id)
    .bind(wave.scope_snapshot_id)
    .fetch_all(&mut *tx)
    .await?;
    let consolidation = sqlx::query_as::<_, CandidateVerificationConsolidationView>(
        r#"SELECT consolidation.id AS consolidation_id,consolidation.decision_kind,
                  consolidation.target_wave_run_id,consolidation.fact_delta_count,
                  consolidation.reason_code,consolidation.row_version,
                  consolidation.decided_at
             FROM attack_wave_consolidations consolidation
            WHERE consolidation.operation_id=$1
              AND consolidation.scope_snapshot_id=$2
              AND consolidation.source_wave_run_id=$3"#,
    )
    .bind(operation_id)
    .bind(wave.scope_snapshot_id)
    .bind(wave_run_id)
    .fetch_optional(&mut *tx)
    .await?;

    let pending_enrichments = sqlx::query_as::<_, CandidateVerificationPendingEnrichmentView>(
        r#"SELECT item.id AS enrichment_id,item.fact_delta_id,
                      item.source_attempt_id,item.candidate_id,
                      item.source_wave_unit_id AS wave_unit_id,item.organization_id,
                      delta.canonical_ref_kind AS subject_kind,
                      delta.canonical_ref_id AS subject_id,
                      delta.target_type_at_time,delta.target_value_at_time,
                      item.delta_kind,item.observation_kind,item.allowed_techniques,
                      item.enrichment_required,
                      'typed_observation_required'::TEXT AS reason_code,
                      item.status,item.created_at
                 FROM attack_fact_delta_enrichment_items item
                 JOIN attack_fact_deltas delta
                   ON delta.id=item.fact_delta_id
                  AND delta.source_attempt_id=item.source_attempt_id
                  AND delta.candidate_id=item.candidate_id
                  AND delta.operation_id=item.operation_id
                  AND delta.scope_snapshot_id=item.scope_snapshot_id
                  AND delta.wave_run_id=item.source_wave_run_id
                  AND delta.wave_unit_id=item.source_wave_unit_id
                  AND delta.organization_id=item.organization_id
                WHERE item.operation_id=$1 AND item.scope_snapshot_id=$2
                  AND item.source_wave_run_id=$3 AND item.status='pending'
                ORDER BY item.organization_id,item.fact_delta_id"#,
    )
    .bind(operation_id)
    .bind(wave.scope_snapshot_id)
    .bind(wave_run_id)
    .fetch_all(&mut *tx)
    .await?;

    let item_rows = sqlx::query_as::<_, CandidateVerificationQueueItemRow>(
        r#"SELECT attempt.id AS attempt_id,attempt.candidate_id,attempt.approval_id,
                  attempt.wave_unit_id,attempt.organization_id,attempt.target_live_id,
                  attempt.target_type_at_time,attempt.target_value_at_time,
                  attempt.target_identity_hash,attempt.candidate_plan_hash,
                  candidate.hypothesis,candidate.technique,
                  COALESCE(NULLIF(candidate.execution_plan->>'schema_version',''),'unknown')
                      AS plan_schema_version,
                  COALESCE(NULLIF(candidate.execution_plan->>'recipe_version',''),
                      'candidate-recipe.legacy-generic-v1') AS recipe_version,
                  COALESCE(NULLIF(candidate.execution_plan->>'executor_contract_version',''),
                      'candidate-executor.legacy-generic-v1') AS executor_contract_version,
                  CASE WHEN jsonb_typeof(candidate.execution_plan->'budget'->'max_actions')='number'
                       THEN (candidate.execution_plan->'budget'->>'max_actions')::BIGINT ELSE 0 END
                      AS budget_max_actions,
                  CASE WHEN jsonb_typeof(candidate.execution_plan->'budget'->'max_requests')='number'
                       THEN (candidate.execution_plan->'budget'->>'max_requests')::BIGINT ELSE 0 END
                      AS budget_max_requests,
                  CASE WHEN jsonb_typeof(candidate.execution_plan->'budget'->'max_runtime_ms')='number'
                       THEN (candidate.execution_plan->'budget'->>'max_runtime_ms')::BIGINT ELSE 0 END
                      AS budget_max_runtime_ms,
                  approval.start_before AS approval_start_before,
                  approval.expires_at AS approval_expires_at,
                  attempt.ordinal,attempt.status,
                  attempt.stage_worker_run_id AS worker_run_id,worker.status AS worker_status,
                  attempt.row_version,attempt.created_at,attempt.updated_at,attempt.terminal_at
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
             JOIN operation_org_scope_units scope_unit
               ON scope_unit.snapshot_id=attempt.scope_snapshot_id
              AND scope_unit.organization_id=attempt.organization_id
             LEFT JOIN stage_worker_runs worker ON worker.id=attempt.stage_worker_run_id
            WHERE attempt.operation_id=$1 AND attempt.wave_run_id=$2
              AND attempt.scope_snapshot_id=$3
            ORDER BY attempt.organization_id,attempt.candidate_id,attempt.ordinal"#,
    )
    .bind(operation_id)
    .bind(wave_run_id)
    .bind(wave.scope_snapshot_id)
    .fetch_all(&mut *tx)
    .await?;

    let observation_evidence = sqlx::query_as::<_, CandidateVerificationEvidenceOwnerRow>(
        r#"SELECT attempt.id AS attempt_id,evidence.evidence_id,evidence.role
             FROM candidate_attempts attempt
             JOIN attack_candidates candidate
               ON candidate.candidate_id=attempt.candidate_id
              AND candidate.operation_uuid=attempt.operation_id
              AND candidate.scope_snapshot_id=attempt.scope_snapshot_id
              AND candidate.wave_run_id=attempt.wave_run_id
              AND candidate.wave_unit_id=attempt.wave_unit_id
              AND candidate.organization_id=attempt.organization_id
              AND candidate.candidate_plan_hash=attempt.candidate_plan_hash
             JOIN attack_candidate_work_item_evidence evidence
               ON evidence.work_item_id=candidate.source_work_item_id
            WHERE attempt.operation_id=$1 AND attempt.wave_run_id=$2
              AND attempt.scope_snapshot_id=$3
            ORDER BY attempt.id,evidence.evidence_id,evidence.role"#,
    )
    .bind(operation_id)
    .bind(wave_run_id)
    .bind(wave.scope_snapshot_id)
    .fetch_all(&mut *tx)
    .await?;
    let attempt_evidence = sqlx::query_as::<_, CandidateVerificationEvidenceOwnerRow>(
        r#"SELECT attempt.id AS attempt_id,evidence.evidence_id,evidence.role
             FROM candidate_attempts attempt
             JOIN candidate_attempt_evidence evidence ON evidence.attempt_id=attempt.id
            WHERE attempt.operation_id=$1 AND attempt.wave_run_id=$2
              AND attempt.scope_snapshot_id=$3
            ORDER BY attempt.id,evidence.evidence_id,evidence.role"#,
    )
    .bind(operation_id)
    .bind(wave_run_id)
    .bind(wave.scope_snapshot_id)
    .fetch_all(&mut *tx)
    .await?;
    let actions = sqlx::query_as::<_, CandidateVerificationActionView>(
        r#"SELECT action.attempt_id,action.id AS action_id,action.action_ordinal,
                  action.capability_id,action.action_kind,action.status,
                  action.outcome_hash,action.error_code,action.started_at,action.completed_at,
                  receipt.id AS authorization_receipt_id,
                  receipt.request_id AS authorization_request_id,
                  receipt.receipt_hash AS authorization_receipt_hash,
                  receipt.authorized_at,receipt.start_before,receipt.execution_deadline
             FROM candidate_attempt_actions action
             JOIN candidate_attempts attempt ON attempt.id=action.attempt_id
             LEFT JOIN candidate_action_authorization_receipts receipt
               ON receipt.id=action.authorization_receipt_id
              AND receipt.action_id=action.id AND receipt.attempt_id=action.attempt_id
            WHERE attempt.operation_id=$1 AND attempt.wave_run_id=$2
              AND attempt.scope_snapshot_id=$3
            ORDER BY action.attempt_id,action.action_ordinal,action.id"#,
    )
    .bind(operation_id)
    .bind(wave_run_id)
    .bind(wave.scope_snapshot_id)
    .fetch_all(&mut *tx)
    .await?;
    let intents = sqlx::query_as::<_, CandidateVerificationTerminalIntentView>(
        r#"SELECT intent.attempt_id,intent.id AS intent_id,intent.request_id,
                  intent.tool_call_record_id,intent.disposition,intent.result_hash,
                  intent.evidence_manifest_hash,intent.evidence_count,intent.intent_hash,
                  intent.created_at
             FROM candidate_attempt_terminal_intents intent
            WHERE intent.operation_id=$1 AND intent.wave_run_id=$2
              AND intent.scope_snapshot_id=$3
            ORDER BY intent.attempt_id"#,
    )
    .bind(operation_id)
    .bind(wave_run_id)
    .bind(wave.scope_snapshot_id)
    .fetch_all(&mut *tx)
    .await?;
    let barriers = sqlx::query_as::<_, CandidateVerificationTerminalBarrierView>(
        r#"SELECT barrier.attempt_id,barrier.id AS barrier_id,barrier.intent_id,
                  barrier.request_id,barrier.tool_call_record_id,barrier.created_at
             FROM candidate_attempt_terminal_barriers barrier
             JOIN candidate_attempt_terminal_intents intent ON intent.id=barrier.intent_id
            WHERE intent.operation_id=$1 AND intent.wave_run_id=$2
              AND intent.scope_snapshot_id=$3
            ORDER BY barrier.attempt_id"#,
    )
    .bind(operation_id)
    .bind(wave_run_id)
    .bind(wave.scope_snapshot_id)
    .fetch_all(&mut *tx)
    .await?;
    let receipts = sqlx::query_as::<_, CandidateVerificationTerminalReceiptView>(
        r#"SELECT receipt.attempt_id,receipt.id AS receipt_id,receipt.intent_id,
                  receipt.barrier_id,receipt.request_id,receipt.disposition,
                  receipt.terminal_attempt_row_version,receipt.finding_id,
                  receipt.fact_delta_count,receipt.terminal_event_id,
                  receipt.receipt_hash,receipt.created_at
             FROM candidate_attempt_terminal_receipts receipt
            WHERE receipt.operation_id=$1 AND receipt.wave_run_id=$2
              AND receipt.scope_snapshot_id=$3
            ORDER BY receipt.attempt_id"#,
    )
    .bind(operation_id)
    .bind(wave_run_id)
    .bind(wave.scope_snapshot_id)
    .fetch_all(&mut *tx)
    .await?;
    let recovery_rows = sqlx::query_as::<_, CandidateVerificationRecoveryRow>(
        r#"SELECT recovery.id AS recovery_case_id,recovery.request_id,
                  recovery.attempt_id,recovery.action_id,recovery.intent_id,
                  recovery.case_kind,recovery.reason_code,recovery.attempt_row_version,
                  recovery.status,recovery.resolution_kind,recovery.resolution_request_id,
                  recovery.row_version,recovery.decided_at,recovery.completed_at,
                  recovery.created_at,recovery.updated_at
             FROM candidate_recovery_cases recovery
            WHERE recovery.operation_id=$1 AND recovery.wave_run_id=$2
              AND recovery.scope_snapshot_id=$3
            ORDER BY recovery.attempt_id,recovery.created_at,recovery.id"#,
    )
    .bind(operation_id)
    .bind(wave_run_id)
    .bind(wave.scope_snapshot_id)
    .fetch_all(&mut *tx)
    .await?;
    let recovery_evidence = sqlx::query_as::<_, (Uuid, i64)>(
        r#"SELECT evidence.recovery_case_id,evidence.evidence_id
             FROM candidate_recovery_evidence evidence
             JOIN candidate_recovery_cases recovery ON recovery.id=evidence.recovery_case_id
            WHERE recovery.operation_id=$1 AND recovery.wave_run_id=$2
              AND recovery.scope_snapshot_id=$3
            ORDER BY evidence.recovery_case_id,evidence.evidence_id"#,
    )
    .bind(operation_id)
    .bind(wave_run_id)
    .bind(wave.scope_snapshot_id)
    .fetch_all(&mut *tx)
    .await?;

    let mut items = Vec::with_capacity(item_rows.len());
    let mut item_indexes = BTreeMap::new();
    for row in item_rows {
        item_indexes.insert(row.attempt_id, items.len());
        items.push(CandidateVerificationQueueItem {
            attempt_id: row.attempt_id,
            candidate_id: row.candidate_id,
            approval_id: row.approval_id,
            wave_unit_id: row.wave_unit_id,
            organization_id: row.organization_id,
            target_live_id: row.target_live_id,
            target_type_at_time: row.target_type_at_time,
            target_value_at_time: row.target_value_at_time,
            target_identity_hash: row.target_identity_hash,
            candidate_plan_hash: row.candidate_plan_hash,
            hypothesis: row.hypothesis,
            technique: row.technique,
            plan_schema_version: row.plan_schema_version,
            recipe_version: row.recipe_version,
            executor_contract_version: row.executor_contract_version,
            budget_max_actions: row.budget_max_actions,
            budget_max_requests: row.budget_max_requests,
            budget_max_runtime_ms: row.budget_max_runtime_ms,
            approval_start_before: row.approval_start_before,
            approval_expires_at: row.approval_expires_at,
            ordinal: row.ordinal,
            status: row.status,
            worker_run_id: row.worker_run_id,
            worker_status: row.worker_status,
            row_version: row.row_version,
            created_at: row.created_at,
            updated_at: row.updated_at,
            terminal_at: row.terminal_at,
            observation_evidence: Vec::new(),
            attempt_evidence: Vec::new(),
            actions: Vec::new(),
            terminal_intent: None,
            terminal_barrier: None,
            terminal_receipt: None,
            recovery_cases: Vec::new(),
        });
    }
    for row in observation_evidence {
        if let Some(index) = item_indexes.get(&row.attempt_id) {
            items[*index]
                .observation_evidence
                .push(CandidateVerificationEvidenceView {
                    evidence_id: row.evidence_id,
                    role: row.role,
                });
        }
    }
    for row in attempt_evidence {
        if let Some(index) = item_indexes.get(&row.attempt_id) {
            items[*index]
                .attempt_evidence
                .push(CandidateVerificationEvidenceView {
                    evidence_id: row.evidence_id,
                    role: row.role,
                });
        }
    }
    for action in actions {
        if let Some(index) = item_indexes.get(&action.attempt_id) {
            items[*index].actions.push(action);
        }
    }
    for intent in intents {
        if let Some(index) = item_indexes.get(&intent.attempt_id) {
            items[*index].terminal_intent = Some(intent);
        }
    }
    for barrier in barriers {
        if let Some(index) = item_indexes.get(&barrier.attempt_id) {
            items[*index].terminal_barrier = Some(barrier);
        }
    }
    for receipt in receipts {
        if let Some(index) = item_indexes.get(&receipt.attempt_id) {
            items[*index].terminal_receipt = Some(receipt);
        }
    }
    let mut recovery_evidence_by_case = BTreeMap::<Uuid, Vec<i64>>::new();
    for (recovery_case_id, evidence_id) in recovery_evidence {
        recovery_evidence_by_case
            .entry(recovery_case_id)
            .or_default()
            .push(evidence_id);
    }
    for recovery in recovery_rows {
        if let Some(index) = item_indexes.get(&recovery.attempt_id) {
            items[*index]
                .recovery_cases
                .push(CandidateVerificationRecoveryView {
                    recovery_case_id: recovery.recovery_case_id,
                    request_id: recovery.request_id,
                    attempt_id: recovery.attempt_id,
                    action_id: recovery.action_id,
                    intent_id: recovery.intent_id,
                    case_kind: recovery.case_kind,
                    reason_code: recovery.reason_code,
                    attempt_row_version: recovery.attempt_row_version,
                    status: recovery.status,
                    resolution_kind: recovery.resolution_kind,
                    resolution_request_id: recovery.resolution_request_id,
                    row_version: recovery.row_version,
                    evidence_ids: recovery_evidence_by_case
                        .remove(&recovery.recovery_case_id)
                        .unwrap_or_default(),
                    decided_at: recovery.decided_at,
                    completed_at: recovery.completed_at,
                    created_at: recovery.created_at,
                    updated_at: recovery.updated_at,
                });
        }
    }
    let pending_enrichment_count = pending_enrichments.len();
    tx.commit().await?;
    Ok(CandidateVerificationQueueReadModel {
        operation_id,
        scope_snapshot_id: wave.scope_snapshot_id,
        wave_run_id,
        generation: wave.generation,
        wave_status: wave.wave_status,
        wave_row_version: wave.wave_row_version,
        wave_units,
        consolidation,
        pending_enrichment_count,
        pending_enrichments,
        items,
    })
}

async fn load_abandon_before_side_effect_authority(
    tx: &mut Transaction<'_, Postgres>,
    operation_id: Uuid,
    attempt_id: Uuid,
) -> crate::Result<AbandonBeforeSideEffectAuthority> {
    sqlx::query_as::<_, AbandonBeforeSideEffectAuthority>(
        r#"SELECT attempt.id AS attempt_id,attempt.approval_id,
                  attempt.candidate_id,attempt.operation_id,
                  attempt.scope_snapshot_id,attempt.wave_run_id,
                  attempt.wave_unit_id,attempt.organization_id,
                  worker.id AS worker_run_id,attempt.status AS attempt_status,
                  attempt.row_version AS attempt_row_version,
                  approval.status AS approval_status,
                  approval.start_before AS approval_start_before,
                  candidate.disposition AS candidate_disposition,
                  candidate.terminal_attempt_id,candidate.terminal_finding_id,
                  worker.status AS worker_status,
                  EXISTS(
                      SELECT 1 FROM candidate_attempt_actions action
                       WHERE action.attempt_id=attempt.id
                         AND (
                             action.status<>'planned'
                             OR action.started_at IS NOT NULL
                             OR action.authorization_receipt_id IS NOT NULL
                         )
                  ) AS has_started_action,
                  EXISTS(
                      SELECT 1 FROM candidate_attempt_terminal_intents intent
                       WHERE intent.attempt_id=attempt.id
                  ) AS has_terminal_intent
             FROM candidate_attempts attempt
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
             JOIN attack_candidates candidate
               ON candidate.candidate_id=attempt.candidate_id
              AND candidate.operation_uuid=attempt.operation_id
              AND candidate.scope_snapshot_id=attempt.scope_snapshot_id
              AND candidate.wave_run_id=attempt.wave_run_id
              AND candidate.wave_unit_id=attempt.wave_unit_id
              AND candidate.organization_id=attempt.organization_id
              AND candidate.target_identity_hash=attempt.target_identity_hash
              AND candidate.candidate_plan_hash=attempt.candidate_plan_hash
             JOIN stage_worker_runs worker
               ON worker.id=attempt.stage_worker_run_id
              AND worker.operation_id=attempt.operation_id
              AND worker.organization_id=attempt.organization_id
            WHERE attempt.id=$1 AND attempt.operation_id=$2
            FOR UPDATE OF attempt,approval,candidate,worker"#,
    )
    .bind(attempt_id)
    .bind(operation_id)
    .fetch_optional(&mut **tx)
    .await?
    .ok_or_else(|| crate::DbError::NotFound("candidate_attempt".to_string()))
}

/// Revoke a verifier that never crossed the action-start boundary. This is
/// deliberately stricter than a normal retry: the old Attempt is abandoned,
/// its Worker is superseded, the approval is expired, and the frozen Candidate
/// returns to review. No action or terminal intent may exist.
async fn abandon_before_side_effect(
    tx: &mut Transaction<'_, Postgres>,
    operation_id: Uuid,
    attempt_id: Uuid,
    expected_attempt_row_version: Option<i64>,
) -> crate::Result<()> {
    let authority = load_abandon_before_side_effect_authority(tx, operation_id, attempt_id).await?;
    if authority.operation_id != operation_id
        || authority.attempt_id != attempt_id
        || !matches!(authority.attempt_status.as_str(), "queued" | "running")
        || !matches!(authority.approval_status.as_str(), "approved" | "expired")
        || authority.approval_start_before > Utc::now()
        || authority.candidate_disposition != "approved"
        || authority.terminal_attempt_id.is_some()
        || authority.terminal_finding_id.is_some()
        || authority.has_started_action
        || authority.has_terminal_intent
        || !matches!(
            authority.worker_status.as_str(),
            "running" | "queued" | "recovery_required"
        )
        || expected_attempt_row_version
            .is_some_and(|expected| expected != authority.attempt_row_version)
    {
        return Err(conflict(
            "Candidate Attempt is not safely abandonable before side effect",
        ));
    }

    let lane = super::attack_execution_lanes::lock_global(tx).await?;
    match lane.stage_worker_run_id {
        Some(worker_run_id) if worker_run_id == authority.worker_run_id => {
            let cleared = sqlx::query(
                "UPDATE attack_execution_lanes
                 SET stage_worker_run_id=NULL,lease_token=NULL,lease_owner=NULL,
                     lease_expires_at=NULL,updated_at=NOW()
                 WHERE lane_key=$1 AND stage_worker_run_id=$2",
            )
            .bind(super::attack_execution_lanes::GLOBAL_EXPLOIT_LANE)
            .bind(authority.worker_run_id)
            .execute(&mut **tx)
            .await?;
            if cleared.rows_affected() != 1 {
                return Err(conflict("Candidate expiry lane CAS lost"));
            }
        }
        Some(_) | None if authority.attempt_status == "running" => {
            return Err(conflict(
                "running Candidate Worker does not own the global execution lane",
            ));
        }
        _ => {}
    }

    let attempt_updated = sqlx::query(
        "UPDATE candidate_attempts
         SET status='abandoned',row_version=row_version+1,updated_at=NOW()
         WHERE id=$1 AND operation_id=$2 AND status=$4
           AND row_version=$3 AND result_json IS NULL AND result_hash IS NULL",
    )
    .bind(authority.attempt_id)
    .bind(authority.operation_id)
    .bind(authority.attempt_row_version)
    .bind(&authority.attempt_status)
    .execute(&mut **tx)
    .await?;
    if attempt_updated.rows_affected() != 1 {
        return Err(conflict("Candidate abandonment Attempt CAS lost"));
    }

    let worker_updated = sqlx::query(
        "UPDATE stage_worker_runs
         SET status='superseded',lease_token=NULL,lease_owner=NULL,
             lease_acquired_at=NULL,lease_expires_at=NULL,heartbeat_at=NULL,
             active_tool_call_id=NULL,active_tool_started_at=NULL,
             terminal_at=NOW(),updated_at=NOW()
         WHERE id=$1 AND operation_id=$2 AND organization_id=$3 AND status=$4",
    )
    .bind(authority.worker_run_id)
    .bind(authority.operation_id)
    .bind(authority.organization_id)
    .bind(&authority.worker_status)
    .execute(&mut **tx)
    .await?;
    if worker_updated.rows_affected() != 1 {
        return Err(conflict("Candidate abandonment Worker CAS lost"));
    }

    if authority.approval_status == "approved" {
        let approval_updated = sqlx::query(
            "UPDATE attack_candidate_approvals
             SET status='expired',row_version=row_version+1
             WHERE id=$1 AND candidate_id=$2 AND operation_id=$3
               AND status='approved'",
        )
        .bind(authority.approval_id)
        .bind(authority.candidate_id)
        .bind(authority.operation_id)
        .execute(&mut **tx)
        .await?;
        if approval_updated.rows_affected() != 1 {
            return Err(conflict("Candidate abandonment Approval CAS lost"));
        }
    }

    let candidate_updated = sqlx::query(
        "UPDATE attack_candidates
         SET disposition='proposed',row_version=row_version+1,updated_at=NOW()
         WHERE candidate_id=$1 AND operation_uuid=$2 AND scope_snapshot_id=$3
           AND wave_run_id=$4 AND wave_unit_id=$5 AND organization_id=$6
           AND disposition='approved' AND terminal_attempt_id IS NULL
           AND terminal_finding_id IS NULL",
    )
    .bind(authority.candidate_id)
    .bind(authority.operation_id)
    .bind(authority.scope_snapshot_id)
    .bind(authority.wave_run_id)
    .bind(authority.wave_unit_id)
    .bind(authority.organization_id)
    .execute(&mut **tx)
    .await?;
    if candidate_updated.rows_affected() != 1 {
        return Err(conflict("Candidate abandonment Candidate CAS lost"));
    }

    super::attack_candidate_approvals::reopen_review_after_candidate_abandon(
        tx,
        authority.operation_id,
        authority.scope_snapshot_id,
        authority.wave_run_id,
    )
    .await?;
    Ok(())
}

/// Deterministically return every Candidate whose approval can no longer
/// authorize its first side effect to review. Rows that already started an
/// action or wrote a terminal intent are intentionally excluded and follow
/// the normal outcome-unknown / terminal-intent recovery paths instead.
pub async fn expire_candidate_starts_before_claim(
    pool: &PgPool,
    operation_id: Uuid,
) -> crate::Result<u32> {
    if operation_id.is_nil() {
        return Err(conflict("invalid Candidate start expiry operation"));
    }
    let mut tx = pool.begin().await?;
    candidate_attempts::lock_v2_operation(&mut tx, operation_id).await?;
    let attempt_ids: Vec<Uuid> = sqlx::query_scalar(
        r#"SELECT attempt.id
             FROM candidate_attempts attempt
             JOIN attack_candidate_approvals approval
               ON approval.id=attempt.approval_id
              AND approval.candidate_id=attempt.candidate_id
              AND approval.operation_id=attempt.operation_id
             JOIN attack_candidates candidate
               ON candidate.candidate_id=attempt.candidate_id
              AND candidate.operation_uuid=attempt.operation_id
             JOIN stage_worker_runs worker ON worker.id=attempt.stage_worker_run_id
            WHERE attempt.operation_id=$1 AND attempt.status IN ('queued','running')
              AND approval.status IN ('approved','expired')
              AND approval.start_before<=NOW()
              AND candidate.disposition='approved'
              AND candidate.terminal_attempt_id IS NULL
              AND candidate.terminal_finding_id IS NULL
              AND worker.status IN ('running','queued','recovery_required')
              AND NOT EXISTS(
                    SELECT 1 FROM candidate_attempt_actions action
                     WHERE action.attempt_id=attempt.id
                       AND (
                           action.status<>'planned'
                           OR action.started_at IS NOT NULL
                           OR action.authorization_receipt_id IS NOT NULL
                       )
                  )
              AND NOT EXISTS(
                    SELECT 1 FROM candidate_attempt_terminal_intents intent
                     WHERE intent.attempt_id=attempt.id
                  )
            ORDER BY approval.start_before,attempt.id"#,
    )
    .bind(operation_id)
    .fetch_all(&mut *tx)
    .await?;
    for attempt_id in &attempt_ids {
        abandon_before_side_effect(&mut tx, operation_id, *attempt_id, None).await?;
    }

    // Also handle approvals that expired before an Attempt was claimed, plus
    // rows stranded by the old expires_at-only reaper. Historical abandoned
    // or retryable-failed Attempts do not keep the current approval alive.
    let unclaimed: Vec<(Uuid, Uuid, Uuid, Uuid, Uuid, String)> = sqlx::query_as(
        r#"SELECT approval.id,approval.candidate_id,approval.scope_snapshot_id,
                  approval.wave_run_id,approval.wave_unit_id,approval.status
             FROM attack_candidate_approvals approval
             JOIN attack_candidates candidate
               ON candidate.candidate_id=approval.candidate_id
              AND candidate.operation_uuid=approval.operation_id
              AND candidate.scope_snapshot_id=approval.scope_snapshot_id
              AND candidate.wave_run_id=approval.wave_run_id
              AND candidate.wave_unit_id=approval.wave_unit_id
              AND candidate.organization_id=approval.organization_id
              AND candidate.target_identity_hash=approval.target_identity_hash
              AND candidate.candidate_plan_hash=approval.candidate_plan_hash
            WHERE approval.operation_id=$1
              AND approval.status IN ('approved','expired')
              AND approval.start_before<=NOW()
              AND candidate.disposition='approved'
              AND candidate.terminal_attempt_id IS NULL
              AND candidate.terminal_finding_id IS NULL
              AND NOT EXISTS(
                    SELECT 1 FROM candidate_attempts attempt
                     WHERE attempt.approval_id=approval.id
                       AND attempt.status NOT IN ('abandoned','retryable_failed')
                  )
            ORDER BY approval.start_before,approval.candidate_id
            FOR UPDATE OF approval,candidate"#,
    )
    .bind(operation_id)
    .fetch_all(&mut *tx)
    .await?;
    let mut review_scopes = BTreeSet::new();
    for (approval_id, candidate_id, scope_snapshot_id, wave_run_id, wave_unit_id, status) in
        &unclaimed
    {
        if status == "approved" {
            let approval_updated = sqlx::query(
                "UPDATE attack_candidate_approvals
                 SET status='expired',row_version=row_version+1
                 WHERE id=$1 AND status='approved'",
            )
            .bind(approval_id)
            .execute(&mut *tx)
            .await?;
            if approval_updated.rows_affected() != 1 {
                return Err(conflict("unclaimed Candidate Approval expiry CAS lost"));
            }
        }
        let candidate_updated = sqlx::query(
            "UPDATE attack_candidates
             SET disposition='proposed',row_version=row_version+1,updated_at=NOW()
             WHERE candidate_id=$1 AND operation_uuid=$2 AND scope_snapshot_id=$3
               AND wave_run_id=$4 AND wave_unit_id=$5 AND disposition='approved'
               AND terminal_attempt_id IS NULL AND terminal_finding_id IS NULL",
        )
        .bind(candidate_id)
        .bind(operation_id)
        .bind(scope_snapshot_id)
        .bind(wave_run_id)
        .bind(wave_unit_id)
        .execute(&mut *tx)
        .await?;
        if candidate_updated.rows_affected() != 1 {
            return Err(conflict("unclaimed Candidate review reopen CAS lost"));
        }
        review_scopes.insert((*scope_snapshot_id, *wave_run_id));
    }
    for (scope_snapshot_id, wave_run_id) in review_scopes {
        super::attack_candidate_approvals::reopen_review_after_candidate_abandon(
            &mut tx,
            operation_id,
            scope_snapshot_id,
            wave_run_id,
        )
        .await?;
    }

    let affected = attempt_ids.len().saturating_add(unclaimed.len());
    let expired =
        u32::try_from(affected).map_err(|_| conflict("Candidate start expiry count overflow"))?;
    tx.commit().await?;
    Ok(expired)
}

fn recovery_payload(
    resolution: CandidateRecoveryResolution,
    evidence_ids: &[i64],
) -> crate::Result<serde_json::Value> {
    let mut evidence_ids = evidence_ids.to_vec();
    evidence_ids.sort_unstable();
    let before = evidence_ids.len();
    evidence_ids.dedup();
    if evidence_ids.len() != before || evidence_ids.iter().any(|id| *id <= 0) {
        return Err(conflict("invalid Candidate recovery evidence"));
    }
    match resolution {
        CandidateRecoveryResolution::AcceptExternalResultWithExactEvidence
            if evidence_ids.is_empty() =>
        {
            Err(conflict(
                "external Candidate recovery requires exact evidence",
            ))
        }
        CandidateRecoveryResolution::AcceptExternalResultWithExactEvidence => {
            Ok(serde_json::json!({"evidence_ids": evidence_ids}))
        }
        _ if !evidence_ids.is_empty() => Err(conflict(
            "Candidate recovery evidence is only valid for an external result",
        )),
        CandidateRecoveryResolution::TerminalizeBlockedOutcomeUnknown => {
            Ok(serde_json::json!({"reason_code": "operator_outcome_unknown"}))
        }
        CandidateRecoveryResolution::AbandonBeforeSideEffect => {
            Ok(serde_json::json!({"reason_code": "approval_start_expired"}))
        }
    }
}

/// Record exactly one of the three legal operator recovery decisions using a
/// request-id plus row-version CAS. Frozen target/plan/args/budget ownership is
/// never accepted from the caller.
pub async fn resolve_candidate_recovery(
    pool: &PgPool,
    command: ResolveCandidateRecovery,
) -> crate::Result<ResolvedCandidateRecovery> {
    if !valid_request_id(&command.request_id)
        || command.operation_id.is_nil()
        || command.recovery_case_id.is_nil()
        || command.expected_row_version < 0
        || command.expected_attempt_row_version < 0
        || command.resolved_by.is_nil()
    {
        return Err(conflict("invalid Candidate recovery resolution"));
    }
    let payload = recovery_payload(command.resolution, &command.evidence_ids)?;
    let mut tx = pool.begin().await?;
    candidate_attempts::lock_v2_operation(&mut tx, command.operation_id).await?;
    let select_sql = format!(
        "SELECT {RECOVERY_CASE_COLUMNS} FROM candidate_recovery_cases \
         WHERE id=$1 AND operation_id=$2 FOR UPDATE"
    );
    let current = sqlx::query_as::<_, CandidateRecoveryCaseRow>(&select_sql)
        .bind(command.recovery_case_id)
        .bind(command.operation_id)
        .fetch_optional(&mut *tx)
        .await?
        .ok_or_else(|| crate::DbError::NotFound("candidate_recovery_case".to_string()))?;
    if current.status != "open" {
        if current.resolution_request_id.as_deref() == Some(command.request_id.as_str())
            && current.resolution_kind.as_deref() == Some(command.resolution.as_str())
            && current.resolution_payload.as_ref() == Some(&payload)
            && current.resolved_by == Some(command.resolved_by)
            && current.attempt_row_version == command.expected_attempt_row_version
        {
            tx.commit().await?;
            return Ok(ResolvedCandidateRecovery {
                recovery_case: current,
                replayed: true,
            });
        }
        return Err(conflict("Candidate recovery resolution replay drift"));
    }
    if current.row_version != command.expected_row_version {
        return Err(conflict("Candidate recovery row-version CAS lost"));
    }
    if current.attempt_row_version != command.expected_attempt_row_version {
        return Err(conflict("Candidate recovery Attempt version drift"));
    }
    let live_attempt_row_version: Option<i64> = sqlx::query_scalar(
        "SELECT row_version FROM candidate_attempts
         WHERE id=$1 AND operation_id=$2 FOR UPDATE",
    )
    .bind(current.attempt_id)
    .bind(command.operation_id)
    .fetch_optional(&mut *tx)
    .await?;
    if live_attempt_row_version != Some(command.expected_attempt_row_version) {
        return Err(conflict("Candidate recovery Attempt row-version CAS lost"));
    }
    if command.resolution == CandidateRecoveryResolution::AcceptExternalResultWithExactEvidence {
        for evidence_id in payload["evidence_ids"]
            .as_array()
            .into_iter()
            .flatten()
            .filter_map(serde_json::Value::as_i64)
        {
            sqlx::query(
                "INSERT INTO candidate_recovery_evidence(\
                     recovery_case_id,evidence_id,role) VALUES($1,$2,'external_result')",
            )
            .bind(command.recovery_case_id)
            .bind(evidence_id)
            .execute(&mut *tx)
            .await?;
        }
    }
    let update_sql = format!(
        "UPDATE candidate_recovery_cases \
         SET status='decision_recorded',resolution_kind=$2,resolution_request_id=$3,\
             resolution_payload=$4,resolved_by=$5,row_version=row_version+1 \
         WHERE id=$1 AND status='open' AND row_version=$6 \
         RETURNING {RECOVERY_CASE_COLUMNS}"
    );
    let recovery_case = sqlx::query_as::<_, CandidateRecoveryCaseRow>(&update_sql)
        .bind(command.recovery_case_id)
        .bind(command.resolution.as_str())
        .bind(&command.request_id)
        .bind(&payload)
        .bind(command.resolved_by)
        .bind(command.expected_row_version)
        .fetch_optional(&mut *tx)
        .await?
        .ok_or_else(|| conflict("Candidate recovery decision CAS lost"))?;
    tx.commit().await?;
    Ok(ResolvedCandidateRecovery {
        recovery_case,
        replayed: false,
    })
}

/// Apply one already-recorded recovery decision under server authority. This
/// transaction is the only consumer allowed to turn a recovery decision into
/// Candidate/Attempt/Worker terminal truth. It is safe to retry after response
/// loss because `resolved` is an immutable replay state.
pub async fn converge_candidate_recovery(
    pool: &PgPool,
    operation_id: Uuid,
    recovery_case_id: Uuid,
) -> crate::Result<ConvergedCandidateRecovery> {
    if operation_id.is_nil() || recovery_case_id.is_nil() {
        return Err(conflict("invalid Candidate recovery convergence"));
    }
    let mut tx = pool.begin().await?;
    candidate_attempts::lock_v2_operation(&mut tx, operation_id).await?;
    let select_sql = format!(
        "SELECT {RECOVERY_CASE_COLUMNS} FROM candidate_recovery_cases
         WHERE id=$1 AND operation_id=$2 FOR UPDATE"
    );
    let current = sqlx::query_as::<_, CandidateRecoveryCaseRow>(&select_sql)
        .bind(recovery_case_id)
        .bind(operation_id)
        .fetch_optional(&mut *tx)
        .await?
        .ok_or_else(|| crate::DbError::NotFound("candidate_recovery_case".to_string()))?;
    if current.status == "resolved" {
        let candidate_reopened = current.resolution_kind.as_deref()
            == Some(CandidateRecoveryResolution::AbandonBeforeSideEffect.as_str());
        tx.commit().await?;
        return Ok(ConvergedCandidateRecovery {
            recovery_case: current,
            terminalized: None,
            candidate_reopened,
            replayed: true,
        });
    }
    if current.status != "decision_recorded" {
        return Err(conflict(
            "Candidate recovery has no durable decision to converge",
        ));
    }

    let live_attempt_version: Option<i64> = sqlx::query_scalar(
        "SELECT row_version FROM candidate_attempts
         WHERE id=$1 AND operation_id=$2 FOR UPDATE",
    )
    .bind(current.attempt_id)
    .bind(operation_id)
    .fetch_optional(&mut *tx)
    .await?;
    if live_attempt_version != Some(current.attempt_row_version) {
        return Err(conflict(
            "Candidate recovery Attempt changed after its decision CAS",
        ));
    }

    let (terminalized, candidate_reopened) = match current.resolution_kind.as_deref() {
        Some("abandon_before_side_effect") => {
            abandon_before_side_effect(
                &mut tx,
                operation_id,
                current.attempt_id,
                Some(current.attempt_row_version),
            )
            .await?;
            (None, true)
        }
        Some("terminalize_blocked_outcome_unknown")
        | Some("accept_external_result_with_exact_evidence") => {
            let terminalized = super::finding_lineage::terminalize_blocked_candidate_recovery(
                &mut tx,
                super::finding_lineage::TerminalizeBlockedCandidateRecovery { recovery_case_id },
            )
            .await?;
            (Some(terminalized), false)
        }
        _ => return Err(conflict("unknown Candidate recovery decision")),
    };

    let update_sql = format!(
        "UPDATE candidate_recovery_cases
         SET status='resolved',row_version=row_version+1
         WHERE id=$1 AND operation_id=$2 AND status='decision_recorded'
           AND row_version=$3
         RETURNING {RECOVERY_CASE_COLUMNS}"
    );
    let recovery_case = sqlx::query_as::<_, CandidateRecoveryCaseRow>(&update_sql)
        .bind(recovery_case_id)
        .bind(operation_id)
        .bind(current.row_version)
        .fetch_optional(&mut *tx)
        .await?
        .ok_or_else(|| conflict("Candidate recovery completion CAS lost"))?;
    tx.commit().await?;
    Ok(ConvergedCandidateRecovery {
        recovery_case,
        terminalized,
        candidate_reopened,
        replayed: false,
    })
}

/// Return and converge the oldest durable decision for one operation. The
/// select is intentionally advisory; the convergence transaction re-locks and
/// revalidates the row, so concurrent schedulers either perform the write once
/// or receive the immutable resolved replay.
pub async fn converge_next_candidate_recovery(
    pool: &PgPool,
    operation_id: Uuid,
) -> crate::Result<Option<ConvergedCandidateRecovery>> {
    if operation_id.is_nil() {
        return Err(conflict("invalid Candidate recovery operation"));
    }
    let recovery_case_id: Option<Uuid> = sqlx::query_scalar(
        "SELECT id FROM candidate_recovery_cases
         WHERE operation_id=$1 AND status='decision_recorded'
         ORDER BY decided_at,created_at,id LIMIT 1",
    )
    .bind(operation_id)
    .fetch_optional(pool)
    .await?;
    match recovery_case_id {
        Some(recovery_case_id) => converge_candidate_recovery(pool, operation_id, recovery_case_id)
            .await
            .map(Some),
        None => Ok(None),
    }
}

#[cfg(test)]
mod tests {
    use super::{recovery_payload, CandidateRecoveryResolution};

    #[test]
    fn recovery_payload_is_closed_over_exactly_three_decisions() {
        assert!(recovery_payload(
            CandidateRecoveryResolution::AcceptExternalResultWithExactEvidence,
            &[]
        )
        .is_err());
        assert!(
            recovery_payload(CandidateRecoveryResolution::AbandonBeforeSideEffect, &[7]).is_err()
        );
        assert_eq!(
            recovery_payload(
                CandidateRecoveryResolution::TerminalizeBlockedOutcomeUnknown,
                &[]
            )
            .unwrap(),
            serde_json::json!({"reason_code": "operator_outcome_unknown"})
        );
        assert_eq!(
            recovery_payload(CandidateRecoveryResolution::AbandonBeforeSideEffect, &[]).unwrap(),
            serde_json::json!({"reason_code": "approval_start_expired"})
        );
        assert_eq!(
            recovery_payload(
                CandidateRecoveryResolution::AcceptExternalResultWithExactEvidence,
                &[11, 7]
            )
            .unwrap(),
            serde_json::json!({"evidence_ids": [7, 11]})
        );
    }

    #[test]
    fn verification_queue_projection_is_exact_wave_scoped_and_omits_runtime_secrets() {
        let source = include_str!("candidate_recovery.rs");
        let start = source
            .find("pub async fn list_verification_queue")
            .expect("queue read model function");
        let end = source[start..]
            .find("\nfn recovery_payload")
            .map(|offset| start + offset)
            .expect("queue function boundary");
        let body = &source[start..end];
        assert!(body.contains("wave.id=$1 AND wave.operation_id=$2"));
        assert!(body.contains("attempt.operation_id=$1 AND attempt.wave_run_id=$2"));
        assert!(body.contains("SET TRANSACTION ISOLATION LEVEL REPEATABLE READ READ ONLY"));
        for forbidden_sql in [
            "intent.lease_token",
            "worker.lease_token",
            "worker.checkpoint",
            "barrier.checkpoint_hash",
            "action.canonical_args",
            "action.outcome,",
            "intent.submitted_result",
            "intent.tool_result_text",
            "recovery.resolution_payload",
            "recovery.expected_action_args_hash",
            "recovery.expected_budget_hash",
        ] {
            assert!(
                !body.contains(forbidden_sql),
                "queue projection leaked forbidden SQL field {forbidden_sql}"
            );
        }
    }
}
