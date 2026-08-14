//! SQLx-free authority boundary for one asset-bound Investigation verification
//! round.
//!
//! Tool Manager discovery and execution stay application-owned.  This port
//! persists the exact asset/target/worker/JIT/budget envelope, a dynamic
//! inventory snapshot, zero or more audited invocations, and the independent
//! hypothesis resolution.  Invocation cardinality is deliberately absent from
//! the resolution contract.

use async_trait::async_trait;
use chrono::{DateTime, Utc};
use serde::{Deserialize, Serialize};
use serde_json::Value;
use uuid::Uuid;

use super::runtime_memory::{ClaimedStageWorkItemView, CompletedStageWorkerView};

pub type InvestigationAssetVerificationResult<T> =
    Result<T, InvestigationAssetVerificationRepositoryError>;

#[derive(Debug, Clone, PartialEq, Eq, thiserror::Error)]
pub enum InvestigationAssetVerificationRepositoryError {
    #[error("investigation_asset_verification_unavailable: {operation}")]
    Unavailable { operation: &'static str },
    #[error("investigation_asset_verification_invalid_request: {detail}")]
    InvalidRequest { detail: String },
    #[error("investigation_asset_verification_not_found: {detail}")]
    NotFound { detail: String },
    #[error("investigation_asset_verification_conflict: {detail}")]
    Conflict { detail: String },
    #[error("investigation_asset_verification_authority_mismatch: {detail}")]
    AuthorityMismatch { detail: String },
    #[error("investigation_asset_verification_infrastructure: {detail}")]
    Infrastructure { detail: String },
}

impl InvestigationAssetVerificationRepositoryError {
    pub const fn code(&self) -> &'static str {
        match self {
            Self::Unavailable { .. } => "investigation_asset_verification_unavailable",
            Self::InvalidRequest { .. } => "investigation_asset_verification_invalid_request",
            Self::NotFound { .. } => "investigation_asset_verification_not_found",
            Self::Conflict { .. } => "investigation_asset_verification_conflict",
            Self::AuthorityMismatch { .. } => "investigation_asset_verification_authority_mismatch",
            Self::Infrastructure { .. } => "investigation_asset_verification_infrastructure",
        }
    }
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum InvestigationAssetVerificationSessionState {
    Open,
    Resolved,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum InvestigationAssetVerificationInvocationState {
    Running,
    Succeeded,
    Failed,
    OutcomeUnknown,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum InvestigationHypothesisResolutionDisposition {
    Verified,
    Refuted,
    Invalid,
}

impl InvestigationHypothesisResolutionDisposition {
    pub const fn as_str(self) -> &'static str {
        match self {
            Self::Verified => "verified",
            Self::Refuted => "refuted",
            Self::Invalid => "invalid",
        }
    }
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct InvestigationAssetVerificationWorkerFence {
    pub worker_run_id: Uuid,
    pub lease_token: Uuid,
    pub attempt_epoch: i64,
    pub checkpoint_version: i64,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct InvestigationAssetVerificationActorView {
    pub role: String,
    pub work_item_id: Uuid,
    pub worker_run_id: Uuid,
    pub message_chain_id: Uuid,
}

/// One Primary-requested specialist call inside a hypothesis verification
/// round. `specialist_role` is the platform role string, not a fixed roster
/// slot; multiple calls may intentionally carry the same role.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct InvestigationDynamicVerificationActorCallView {
    pub actor_call_id: Uuid,
    pub stable_request_id: Uuid,
    pub session_id: Uuid,
    pub actor_ordinal: i64,
    pub subtask_id: Uuid,
    pub specialist_role: String,
    pub objective_redacted: Value,
    pub objective_sha256: String,
    pub work_item_id: Uuid,
    pub worker_run_id: Uuid,
    pub message_chain_id: Uuid,
    pub primary_turn_id: Uuid,
    pub turn_actor_ordinal: i32,
    pub actor_call_sha256: String,
    pub state: String,
    pub created_at: DateTime<Utc>,
    pub completed_at: Option<DateTime<Utc>>,
    pub replayed: bool,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct InvestigationDynamicVerificationInvocationAuthorityView {
    pub invocation_id: Uuid,
    pub actor_call_id: Uuid,
    pub actor_ordinal: i64,
    pub specialist_role: String,
    pub state: InvestigationAssetVerificationInvocationState,
    pub capability_execution_receipt_id: Option<Uuid>,
    pub oracle_receipt_id: Option<Uuid>,
    pub audit_evidence_ids: Vec<i64>,
    pub evidence_set_sha256: Option<String>,
    pub result_sha256: Option<String>,
}

impl InvestigationDynamicVerificationActorCallView {
    pub fn as_actor(&self) -> InvestigationAssetVerificationActorView {
        InvestigationAssetVerificationActorView {
            role: self.specialist_role.clone(),
            work_item_id: self.work_item_id,
            worker_run_id: self.worker_run_id,
            message_chain_id: self.message_chain_id,
        }
    }
}

/// Dynamic-v2 round view. The asset Primary identity is its durable message
/// chain; every hypothesis receives a fresh WorkItem/WorkerRun on that chain.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct InvestigationDynamicVerificationRoundView {
    pub session_id: Uuid,
    pub stable_request_id: Uuid,
    pub operation_id: Uuid,
    pub project_scope_id: Uuid,
    pub stage_execution_id: Uuid,
    pub stage_run_unit_id: Uuid,
    pub scope_snapshot_id: Uuid,
    pub organization_id: Uuid,
    pub asset_lane_id: Uuid,
    pub target_live_id: Uuid,
    pub hypothesis_revision_id: Uuid,
    pub verification_task_id: Uuid,
    pub evolution_epoch: i32,
    pub stage_team_plan_id: Uuid,
    pub dispatch_epoch: i64,
    pub session_authorization_id: Uuid,
    pub authorization_expires_at: DateTime<Utc>,
    pub session_budget_envelope_id: Uuid,
    pub source_primary_work_item_id: Uuid,
    pub source_primary_worker_run_id: Uuid,
    pub primary: InvestigationAssetVerificationActorView,
    pub actor_calls: Vec<InvestigationDynamicVerificationActorCallView>,
    pub maximum_primary_turns: i64,
    pub consumed_primary_turns: i64,
    pub maximum_actor_calls: i64,
    pub consumed_actor_calls: i64,
    pub state: InvestigationAssetVerificationSessionState,
    pub head_version: i64,
    pub resolution_authority_id: Option<Uuid>,
    pub opened_at: DateTime<Utc>,
    pub resolved_at: Option<DateTime<Utc>>,
    pub replayed: bool,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct OpenInvestigationDynamicVerificationRound {
    pub stable_request_id: Uuid,
    pub operation_id: Uuid,
    pub stage_execution_id: Uuid,
    pub stage_run_unit_id: Uuid,
    pub scope_snapshot_id: Uuid,
    pub organization_id: Uuid,
    pub asset_lane_id: Uuid,
    pub target_live_id: Uuid,
    pub hypothesis_revision_id: Uuid,
    pub verification_task_id: Uuid,
    pub session_authorization_id: Uuid,
    pub session_budget_envelope_id: Uuid,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct RenewInvestigationDynamicVerificationAuthorization {
    pub stable_request_id: Uuid,
    pub renewal_id: Uuid,
    pub session_id: Uuid,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct InvestigationDynamicVerificationAuthorizationRenewalView {
    pub renewal_id: Uuid,
    pub stable_request_id: Uuid,
    pub session_id: Uuid,
    pub previous_expires_at: DateTime<Utc>,
    pub renewed_expires_at: DateTime<Utc>,
    pub renewal_sha256: String,
    pub replayed: bool,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct InvestigationDynamicVerificationActorRequest {
    pub actor_call_id: Uuid,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct DispatchInvestigationDynamicVerificationActorBatch {
    pub stable_request_id: Uuid,
    pub primary_turn_id: Uuid,
    pub session_id: Uuid,
    pub expected_session_head_version: i64,
    pub primary_worker_fence: InvestigationAssetVerificationWorkerFence,
    pub source_tool_call_record_id: Uuid,
    pub source_provider_call_id: String,
    pub actors: Vec<InvestigationDynamicVerificationActorRequest>,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct InvestigationDynamicVerificationPrimaryTurnView {
    pub primary_turn_id: Uuid,
    pub stable_request_id: Uuid,
    pub session_id: Uuid,
    pub turn_ordinal: i64,
    pub decision_kind: String,
    pub expected_session_head_version: i64,
    pub source_primary_checkpoint_version: i64,
    pub source_primary_checkpoint_sha256: String,
    pub source_tool_call_record_id: Uuid,
    pub source_provider_call_id: String,
    pub canonical_turn_sha256: String,
    pub actor_call_set_sha256: String,
    pub actors: Vec<InvestigationDynamicVerificationActorCallView>,
    pub replayed: bool,
}

/// Server projection of a completed, not-yet-consumed Primary `submit_result`
/// call. The raw result and PostgreSQL-derived hashes are returned so crash
/// recovery never guesses JSONB canonicalization or trusts compacted history.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct InvestigationDynamicVerificationPendingPrimarySubmissionView {
    pub session_id: Uuid,
    pub source_tool_call_record_id: Uuid,
    pub source_provider_call_id: String,
    pub canonical_turn: Value,
    pub canonical_turn_sha256: String,
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct ClaimInvestigationDynamicVerificationPrimary {
    pub session_id: Uuid,
    pub lease_owner: String,
    pub lease_seconds: i32,
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct ParkInvestigationDynamicVerificationPrimary {
    pub session_id: Uuid,
    pub worker_fence: InvestigationAssetVerificationWorkerFence,
    pub checkpoint: Value,
    pub evidence_watermark: Option<i64>,
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct ClaimInvestigationDynamicVerificationActor {
    pub session_id: Uuid,
    pub actor_call_id: Uuid,
    pub lease_owner: String,
    pub lease_seconds: i32,
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct LoadInvestigationDynamicVerificationActorCompletion {
    pub session_id: Uuid,
    pub actor_call_id: Uuid,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct InvestigationDynamicVerificationPendingActorSubmissionView {
    pub session_id: Uuid,
    pub actor_call_id: Uuid,
    pub source_tool_call_record_id: Uuid,
    pub source_provider_call_id: String,
    pub canonical_observation: Value,
    pub canonical_observation_sha256: String,
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct ListInvestigationDynamicVerificationInvocationAuthorities {
    pub session_id: Uuid,
    pub actor_call_id: Option<Uuid>,
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct ParkInvestigationDynamicVerificationActor {
    pub session_id: Uuid,
    pub actor_call_id: Uuid,
    pub worker_fence: InvestigationAssetVerificationWorkerFence,
    pub checkpoint: Value,
    pub evidence_watermark: Option<i64>,
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct CompleteInvestigationDynamicVerificationActor {
    pub session_id: Uuid,
    pub actor_call_id: Uuid,
    pub worker_fence: InvestigationAssetVerificationWorkerFence,
    pub expected_work_item_row_version: i64,
    pub source_tool_call_record_id: Uuid,
    pub source_provider_call_id: String,
    pub terminal_checkpoint: Value,
    pub evidence_watermark: Option<i64>,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct AuthorizeInvestigationAssetVerificationSession {
    pub stable_request_id: Uuid,
    pub session_authorization_id: Uuid,
    pub session_budget_envelope_id: Uuid,
    pub operation_id: Uuid,
    pub stage_execution_id: Uuid,
    pub stage_run_unit_id: Uuid,
    pub scope_snapshot_id: Uuid,
    pub organization_id: Uuid,
    pub asset_lane_id: Uuid,
    pub target_live_id: Uuid,
    pub hypothesis_revision_id: Uuid,
    pub verification_task_id: Uuid,
    pub allowed_effect_classes: Vec<String>,
    pub maximum_risk_tier: String,
    pub allowed_credential_binding_sha256s: Vec<String>,
    pub credential_binding_set_sha256: String,
    pub maximum_invocations: i64,
    pub maximum_network_requests: i64,
    pub maximum_wall_time_ms: i64,
    pub maximum_output_bytes: i64,
    pub maximum_parallel_invocations: i32,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct InvestigationAssetVerificationSessionAuthorizationView {
    pub session_authorization_id: Uuid,
    pub session_budget_envelope_id: Uuid,
    pub operation_id: Uuid,
    pub project_scope_id: Uuid,
    pub stage_execution_id: Uuid,
    pub stage_run_unit_id: Uuid,
    pub scope_snapshot_id: Uuid,
    pub organization_id: Uuid,
    pub asset_lane_id: Uuid,
    pub target_live_id: Uuid,
    pub hypothesis_revision_id: Uuid,
    pub verification_task_id: Uuid,
    pub allowed_effect_classes: Vec<String>,
    pub maximum_risk_tier: String,
    pub allowed_credential_binding_sha256s: Vec<String>,
    pub credential_binding_set_sha256: String,
    pub authorization_sha256: String,
    pub expires_at: DateTime<Utc>,
    pub maximum_invocations: i64,
    pub remaining_invocations: i64,
    pub maximum_network_requests: i64,
    pub remaining_network_requests: i64,
    pub maximum_wall_time_ms: i64,
    pub remaining_wall_time_ms: i64,
    pub maximum_output_bytes: i64,
    pub remaining_output_bytes: i64,
    pub maximum_parallel_invocations: i32,
    pub replayed: bool,
}

/// Server-selected next canonical hypothesis for one current asset lane.  The
/// caller supplies only the operation/lane boundary; revision and task ids are
/// derived from the canonical head so a model cannot select a foreign or stale
/// hypothesis.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct LoadNextInvestigationAssetVerificationCandidate {
    pub operation_id: Uuid,
    pub asset_lane_id: Uuid,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct InvestigationAssetVerificationCandidateView {
    pub operation_id: Uuid,
    pub stage_execution_id: Uuid,
    pub stage_run_unit_id: Uuid,
    pub scope_snapshot_id: Uuid,
    pub organization_id: Uuid,
    pub asset_lane_id: Uuid,
    pub target_live_id: Uuid,
    pub hypothesis_root_id: Uuid,
    pub hypothesis_revision_id: Uuid,
    pub hypothesis_revision_sha256: String,
    pub hypothesis_claim: Value,
    pub hypothesis_claim_sha256: String,
    pub falsification_conditions: Value,
    pub falsification_conditions_sha256: String,
    pub verification_objectives: Value,
    pub verification_objectives_sha256: String,
    pub hypothesis_head_version: i64,
    pub verification_task_id: Uuid,
    pub verification_plan_id: Uuid,
    pub verification_plan_sha256: String,
    pub priority: i32,
    pub existing_open_round_id: Option<Uuid>,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct DynamicToolInventoryMemberInput {
    pub tool_id: String,
    pub tool_name: String,
    pub config_sha256: String,
    pub executable_identity_sha256: String,
    pub runtime: String,
    pub runtime_version: String,
    pub launch_mode: String,
    pub parameter_schema: Value,
    pub output_schema: Value,
    pub tags: Vec<String>,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct FreezeInvestigationDynamicToolInventory {
    pub stable_request_id: Uuid,
    pub session_id: Uuid,
    pub inventory_source_sha256: String,
    pub members: Vec<DynamicToolInventoryMemberInput>,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct DynamicToolInventoryMemberView {
    pub inventory_member_id: Uuid,
    pub member_ordinal: i32,
    pub tool_id: String,
    pub tool_name: String,
    pub config_sha256: String,
    pub executable_identity_sha256: String,
    pub runtime: String,
    pub runtime_version: String,
    pub launch_mode: String,
    pub parameter_schema: Value,
    pub output_schema: Value,
    pub tags: Vec<String>,
    pub member_sha256: String,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct InvestigationDynamicToolInventoryView {
    pub inventory_snapshot_id: Uuid,
    pub stable_request_id: Uuid,
    pub session_id: Uuid,
    pub inventory_source_sha256: String,
    pub member_count: i64,
    pub member_set_sha256: String,
    pub members: Vec<DynamicToolInventoryMemberView>,
    pub sealed_at: DateTime<Utc>,
    pub replayed: bool,
}

/// Trusted host projection of the real Tool Manager. The model sees only
/// `model_projection`; DB persistence receives the exact hashed members.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct InvestigationReadyToolInventory {
    pub inventory_source_sha256: String,
    pub members: Vec<DynamicToolInventoryMemberInput>,
    pub model_projection: Value,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct BeginInvestigationAssetVerificationInvocation {
    pub stable_request_id: Uuid,
    pub invocation_id: Uuid,
    pub session_id: Uuid,
    pub actor_call_id: Uuid,
    pub worker_fence: InvestigationAssetVerificationWorkerFence,
    /// Generic host wrapper (`pentest_run`, `browser_collect_js_api`, or a
    /// read-only Tool Manager wrapper), not a fixed inner-tool catalog.
    pub wrapper_name: String,
    pub selected_tool_name: Option<String>,
    pub credential_binding_sha256: Option<String>,
    pub model_args_redacted: Value,
    pub model_args_sha256: String,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct CompleteInvestigationAssetVerificationInvocation {
    pub stable_request_id: Uuid,
    pub invocation_id: Uuid,
    pub expected_row_version: i64,
    pub worker_fence: InvestigationAssetVerificationWorkerFence,
    pub disposition: InvestigationAssetVerificationInvocationState,
    pub capability_execution_receipt_id: Option<Uuid>,
    pub oracle_receipt_id: Option<Uuid>,
    pub audit_evidence_ids: Vec<i64>,
    pub evidence_set_sha256: String,
    pub redacted_result: Value,
    pub result_sha256: String,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct InvestigationAssetVerificationInvocationView {
    pub invocation_id: Uuid,
    pub stable_request_id: Uuid,
    pub session_id: Uuid,
    pub invocation_ordinal: i64,
    pub actor_call_id: Option<Uuid>,
    pub actor_ordinal: Option<i64>,
    pub actor_subtask_id: Option<Uuid>,
    pub actor_role: String,
    pub actor_work_item_id: Uuid,
    pub actor_worker_run_id: Uuid,
    pub actor_message_chain_id: Uuid,
    pub inventory_snapshot_id: Uuid,
    pub inventory_member_id: Option<Uuid>,
    pub wrapper_name: String,
    pub selected_tool_name: Option<String>,
    pub selected_tool_config_sha256: Option<String>,
    pub invocation_authorization_id: Uuid,
    pub invocation_authorization_sha256: String,
    pub invocation_authorization_expires_at: DateTime<Utc>,
    pub effect_class: String,
    pub risk_tier: String,
    pub credential_binding_sha256: Option<String>,
    pub network_request_limit: i64,
    pub wall_time_limit_ms: i64,
    pub output_byte_limit: i64,
    pub model_args_redacted: Value,
    pub model_args_sha256: String,
    pub request_manifest_sha256: String,
    pub state: InvestigationAssetVerificationInvocationState,
    pub row_version: i64,
    pub capability_execution_receipt_id: Option<Uuid>,
    pub oracle_receipt_id: Option<Uuid>,
    pub audit_evidence_ids: Vec<i64>,
    pub evidence_set_sha256: Option<String>,
    pub redacted_result: Option<Value>,
    pub result_sha256: Option<String>,
    pub started_at: DateTime<Utc>,
    pub completed_at: Option<DateTime<Utc>>,
    pub replayed: bool,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct LoadInvestigationAssetVerificationInvocationGuard {
    pub invocation_id: Uuid,
    pub worker_fence: InvestigationAssetVerificationWorkerFence,
    pub wrapper_name: String,
    pub selected_tool_name: Option<String>,
    pub selected_tool_config_sha256: Option<String>,
    pub model_args_sha256: String,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct InvestigationAssetVerificationInvocationGuardView {
    pub invocation_id: Uuid,
    pub session_id: Uuid,
    pub operation_id: Uuid,
    pub project_scope_id: Uuid,
    pub stage_execution_id: Uuid,
    pub stage_run_unit_id: Uuid,
    pub scope_snapshot_id: Uuid,
    pub organization_id: Uuid,
    pub asset_lane_id: Uuid,
    pub target_live_id: Uuid,
    pub target_type_at_freeze: String,
    pub target_value_at_freeze: String,
    pub target_name: String,
    pub target_project_path: String,
    pub target_ports: Value,
    pub session_authorization_id: Uuid,
    pub session_authorization_sha256: String,
    pub authorization_expires_at: DateTime<Utc>,
    pub session_budget_envelope_id: Uuid,
    pub invocation_authorization_id: Uuid,
    pub invocation_authorization_sha256: String,
    pub invocation_authorization_expires_at: DateTime<Utc>,
    pub actor_call_id: Option<Uuid>,
    pub actor_ordinal: Option<i64>,
    pub actor_subtask_id: Option<Uuid>,
    pub actor_role: String,
    pub actor_work_item_id: Uuid,
    pub actor_worker_run_id: Uuid,
    pub actor_message_chain_id: Uuid,
    pub inventory_snapshot_id: Uuid,
    pub inventory_member_id: Option<Uuid>,
    pub selected_tool_name: Option<String>,
    pub selected_tool_config_sha256: Option<String>,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct InvestigationPendingHypothesisDiscoveryView {
    pub discovery_authority_id: Uuid,
    pub resolution_authority_id: Uuid,
    pub session_id: Uuid,
    pub asset_lane_id: Uuid,
    pub target_live_id: Uuid,
    pub source_hypothesis_revision_id: Uuid,
    pub discovery_ordinal: i32,
    pub subject_kind: String,
    pub subject_identity_sha256: String,
    pub semantic_key_sha256: String,
    /// Complete compiler-shaped proposal with server-derived subject identity,
    /// stable proposal id, and no caller-authored proof authority.
    pub canonical_proposal: Value,
    pub structured_claim: String,
    pub structured_claim_sha256: String,
    pub rationale_redacted: Value,
    pub discovery_sha256: String,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum InvestigationHypothesisDiscoveryConsumptionDisposition {
    Admitted,
    DismissedDuplicate,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct ListPendingInvestigationHypothesisDiscoveries {
    pub operation_id: Uuid,
    pub asset_lane_id: Uuid,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct AdmitOrDismissInvestigationPendingHypothesisDiscovery {
    pub stable_request_id: Uuid,
    pub discovery_authority_id: Uuid,
    pub expected_asset_lane_id: Uuid,
    pub expected_session_id: Uuid,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct InvestigationPendingHypothesisDiscoveryConsumptionView {
    pub consumption_id: Uuid,
    pub discovery_authority_id: Uuid,
    pub asset_lane_id: Uuid,
    pub target_live_id: Uuid,
    pub disposition: InvestigationHypothesisDiscoveryConsumptionDisposition,
    pub admitted_root_id: Option<Uuid>,
    pub admitted_revision_id: Option<Uuid>,
    pub compiler_receipt_id: Option<Uuid>,
    pub duplicate_of_revision_id: Option<Uuid>,
    pub consumption_sha256: String,
    pub consumed_at: DateTime<Utc>,
    pub replayed: bool,
}

/// Dynamic-v2 terminal claim. Primary owns the conclusion; specialist calls
/// are evidence-producing collaborators, never a fixed approval quorum.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct ResolveInvestigationDynamicHypothesis {
    pub stable_request_id: Uuid,
    pub resolution_authority_id: Uuid,
    pub session_id: Uuid,
    pub expected_session_head_version: i64,
    pub primary_worker_fence: InvestigationAssetVerificationWorkerFence,
    pub primary_turn_id: Uuid,
    pub source_tool_call_record_id: Uuid,
    pub source_provider_call_id: String,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct InvestigationDynamicHypothesisResolutionView {
    pub resolution_authority_id: Uuid,
    pub stable_request_id: Uuid,
    pub session_id: Uuid,
    pub asset_lane_id: Uuid,
    pub target_live_id: Uuid,
    pub hypothesis_revision_id: Uuid,
    pub primary_work_item_id: Uuid,
    pub primary_worker_run_id: Uuid,
    pub primary_message_chain_id: Uuid,
    pub disposition: InvestigationHypothesisResolutionDisposition,
    pub primary_conclusion_sha256: String,
    pub conclusion_redacted: Value,
    pub citation_count: i64,
    pub citation_set_sha256: String,
    pub resolution_sha256: String,
    pub new_hypothesis_proposals: Vec<InvestigationPendingHypothesisDiscoveryView>,
    pub resolved_at: DateTime<Utc>,
    pub replayed: bool,
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct CompleteInvestigationDynamicVerificationPrimary {
    pub session_id: Uuid,
    pub resolution_authority_id: Uuid,
    pub primary_worker_fence: InvestigationAssetVerificationWorkerFence,
    pub expected_work_item_row_version: i64,
    pub expected_plan_row_version: i64,
    pub terminal_checkpoint: Value,
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct ResolvedAndTerminalizedInvestigationDynamicHypothesisView {
    pub resolution: InvestigationDynamicHypothesisResolutionView,
    pub primary_completion: CompletedStageWorkerView,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct LoadPendingInvestigationDynamicVerificationPrimaryTerminalization {
    pub operation_id: Uuid,
    pub asset_lane_id: Uuid,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct PendingInvestigationDynamicVerificationPrimaryTerminalizationView {
    pub round: InvestigationDynamicVerificationRoundView,
    pub resolution: InvestigationDynamicHypothesisResolutionView,
    pub primary_worker_fence: Option<InvestigationAssetVerificationWorkerFence>,
    pub expected_work_item_row_version: i64,
    pub expected_plan_row_version: i64,
}

#[async_trait]
pub trait InvestigationAssetVerificationRepository: Send + Sync {
    /// Dynamic-v2 is the only runnable contract for new verification rounds.
    /// Rows created by the historical fixed-five contract remain queryable for
    /// audit but are never returned from these methods.
    async fn open_dynamic_round(
        &self,
        request: OpenInvestigationDynamicVerificationRound,
    ) -> InvestigationAssetVerificationResult<InvestigationDynamicVerificationRoundView>;

    async fn load_dynamic_round(
        &self,
        session_id: Uuid,
    ) -> InvestigationAssetVerificationResult<Option<InvestigationDynamicVerificationRoundView>>;

    async fn renew_dynamic_authorization(
        &self,
        request: RenewInvestigationDynamicVerificationAuthorization,
    ) -> InvestigationAssetVerificationResult<
        InvestigationDynamicVerificationAuthorizationRenewalView,
    >;

    async fn dispatch_dynamic_actor_batch(
        &self,
        request: DispatchInvestigationDynamicVerificationActorBatch,
    ) -> InvestigationAssetVerificationResult<InvestigationDynamicVerificationPrimaryTurnView>;

    async fn load_pending_dynamic_primary_submission(
        &self,
        session_id: Uuid,
    ) -> InvestigationAssetVerificationResult<
        Option<InvestigationDynamicVerificationPendingPrimarySubmissionView>,
    >;

    async fn claim_dynamic_primary(
        &self,
        request: ClaimInvestigationDynamicVerificationPrimary,
    ) -> InvestigationAssetVerificationResult<ClaimedStageWorkItemView>;

    async fn park_dynamic_primary(
        &self,
        request: ParkInvestigationDynamicVerificationPrimary,
    ) -> InvestigationAssetVerificationResult<ClaimedStageWorkItemView>;

    async fn claim_dynamic_actor(
        &self,
        request: ClaimInvestigationDynamicVerificationActor,
    ) -> InvestigationAssetVerificationResult<ClaimedStageWorkItemView>;

    async fn load_dynamic_actor_completion(
        &self,
        request: LoadInvestigationDynamicVerificationActorCompletion,
    ) -> InvestigationAssetVerificationResult<Option<CompletedStageWorkerView>>;

    async fn load_pending_dynamic_actor_submission(
        &self,
        session_id: Uuid,
        actor_call_id: Uuid,
    ) -> InvestigationAssetVerificationResult<
        Option<InvestigationDynamicVerificationPendingActorSubmissionView>,
    >;

    async fn list_dynamic_invocation_authorities(
        &self,
        request: ListInvestigationDynamicVerificationInvocationAuthorities,
    ) -> InvestigationAssetVerificationResult<
        Vec<InvestigationDynamicVerificationInvocationAuthorityView>,
    >;

    async fn park_dynamic_actor(
        &self,
        request: ParkInvestigationDynamicVerificationActor,
    ) -> InvestigationAssetVerificationResult<ClaimedStageWorkItemView>;

    async fn complete_dynamic_actor(
        &self,
        request: CompleteInvestigationDynamicVerificationActor,
    ) -> InvestigationAssetVerificationResult<CompletedStageWorkerView>;

    async fn resolve_dynamic_hypothesis(
        &self,
        request: ResolveInvestigationDynamicHypothesis,
    ) -> InvestigationAssetVerificationResult<InvestigationDynamicHypothesisResolutionView>;

    async fn load_pending_dynamic_primary_terminalization(
        &self,
        request: LoadPendingInvestigationDynamicVerificationPrimaryTerminalization,
    ) -> InvestigationAssetVerificationResult<
        Option<PendingInvestigationDynamicVerificationPrimaryTerminalizationView>,
    >;

    async fn complete_dynamic_primary(
        &self,
        request: CompleteInvestigationDynamicVerificationPrimary,
    ) -> InvestigationAssetVerificationResult<
        ResolvedAndTerminalizedInvestigationDynamicHypothesisView,
    >;

    async fn authorize_session(
        &self,
        request: AuthorizeInvestigationAssetVerificationSession,
    ) -> InvestigationAssetVerificationResult<InvestigationAssetVerificationSessionAuthorizationView>;

    async fn load_next_unresolved_current_hypothesis(
        &self,
        request: LoadNextInvestigationAssetVerificationCandidate,
    ) -> InvestigationAssetVerificationResult<Option<InvestigationAssetVerificationCandidateView>>;

    async fn freeze_dynamic_inventory(
        &self,
        request: FreezeInvestigationDynamicToolInventory,
    ) -> InvestigationAssetVerificationResult<InvestigationDynamicToolInventoryView>;

    async fn begin_invocation(
        &self,
        request: BeginInvestigationAssetVerificationInvocation,
    ) -> InvestigationAssetVerificationResult<InvestigationAssetVerificationInvocationView>;

    async fn complete_invocation(
        &self,
        request: CompleteInvestigationAssetVerificationInvocation,
    ) -> InvestigationAssetVerificationResult<InvestigationAssetVerificationInvocationView>;

    async fn load_invocation_guard(
        &self,
        request: LoadInvestigationAssetVerificationInvocationGuard,
    ) -> InvestigationAssetVerificationResult<InvestigationAssetVerificationInvocationGuardView>;

    async fn list_pending_hypothesis_discoveries(
        &self,
        request: ListPendingInvestigationHypothesisDiscoveries,
    ) -> InvestigationAssetVerificationResult<Vec<InvestigationPendingHypothesisDiscoveryView>>;

    async fn admit_or_dismiss_pending_hypothesis_discovery(
        &self,
        request: AdmitOrDismissInvestigationPendingHypothesisDiscovery,
    ) -> InvestigationAssetVerificationResult<InvestigationPendingHypothesisDiscoveryConsumptionView>;
}
