use chrono::{DateTime, Utc};
use golish_core::investigation_projection::{
    ProjectionChangeKind, ProjectionEntityKind, ProjectionEntityV1, ProjectionInvalidationReason,
    ProjectionSourceTimeStatusV1, TimelineEventKind,
};
use serde::Serialize;
use sha2::{Digest, Sha256};
use uuid::Uuid;

pub const INVESTIGATION_PROJECTION_STALE: &str = "INVESTIGATION_PROJECTION_STALE";
pub const INVESTIGATION_PROJECTION_PAYLOAD_INVALID: &str =
    "INVESTIGATION_PROJECTION_PAYLOAD_INVALID";

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum ProjectionStaleReason {
    ChangeSeqAdvanced,
    TemporalCutoffExpired,
    AuthorityEpochChanged,
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct ProjectionBatchEnqueueReceipt {
    pub batch_id: Uuid,
    pub operation_id: Uuid,
    pub source_batch_seq: i64,
    pub predecessor_batch_id: Option<Uuid>,
    pub member_count: i64,
    pub member_set_hash: String,
    pub replayed: bool,
}

#[derive(Debug, Clone, sqlx::FromRow, PartialEq, Eq)]
pub struct ProjectionBatchClaim {
    pub batch_id: Uuid,
    pub operation_id: Uuid,
    pub source_batch_seq: i64,
    pub predecessor_batch_id: Option<Uuid>,
}

#[derive(Debug, Clone, sqlx::FromRow, PartialEq, Eq)]
pub struct ProjectionBatchReceipt {
    pub receipt_id: Uuid,
    pub batch_id: Uuid,
    pub operation_id: Uuid,
    pub source_batch_seq: i64,
    pub predecessor_batch_id: Option<Uuid>,
    pub first_change_seq: i64,
    pub last_change_seq: i64,
    pub entity_version_manifest_hash: String,
    pub change_manifest_hash: String,
    pub timeline_manifest_hash: String,
    pub projected_at: DateTime<Utc>,
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub enum ProjectionProjectOutcome {
    Applied(ProjectionBatchReceipt),
    Replay(ProjectionBatchReceipt),
    PredecessorPending(ProjectionBatchClaim),
}

#[derive(Debug, Clone, sqlx::FromRow, PartialEq, Eq)]
pub struct CapturedProjectionHead {
    pub operation_id: Uuid,
    pub projection_schema_version: i32,
    pub change_seq: i64,
    pub last_projected_batch_id: Option<Uuid>,
    pub cursor_salt: Vec<u8>,
}

/// Operation contract and cursor identity captured with the projection head.
/// `cursor_salt` is an internal signing key and must never cross IPC.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct InvestigationOperationReadAuthority {
    pub operation_id: Uuid,
    pub tool_truth_contract: String,
    pub investigation_contract_version: String,
    pub investigation_rollout_mode: String,
    pub cursor_salt: [u8; 32],
}

/// The four values that define one stable read/pagination snapshot.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct InvestigationTemporalReadAuthority {
    pub projection_schema_version: i32,
    pub as_of_change_seq: i64,
    pub as_of_temporal_cutoff: DateTime<Utc>,
    pub authority_epoch_set_hash: String,
    pub earliest_effective_valid_until: DateTime<Utc>,
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct InvestigationReadAuthority {
    pub operation: InvestigationOperationReadAuthority,
    pub temporal: InvestigationTemporalReadAuthority,
}

/// Exact externally visible Investigation stage identity. Read commands must
/// supply all three members; the repository never resolves a newest run.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct InvestigationStageRunSelector {
    pub stage_execution_id: Uuid,
    pub stage_run_request_id: String,
    pub scope_snapshot_id: Uuid,
}

/// Server-owned control authority captured in the same read-only repeatable
/// read transaction as the materialized projection head.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct InvestigationStageRunReadAuthority {
    pub authority_id: Uuid,
    pub operation_id: Uuid,
    pub stage_execution_id: Uuid,
    pub stage_run_request_id: String,
    pub scope_snapshot_id: Uuid,
    pub run_state: String,
    pub admission_open: bool,
    pub stop_epoch: i64,
    pub change_seq: i64,
    pub head_version: i64,
    pub head_sha256: String,
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct InvestigationPageValidationInput {
    pub as_of_change_seq: i64,
    pub as_of_temporal_cutoff: DateTime<Utc>,
    pub authority_epoch_set_hash: String,
    pub earliest_effective_valid_until: DateTime<Utc>,
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub enum InvestigationPageValidation {
    Current(InvestigationReadAuthority),
    Stale {
        current_change_seq: i64,
        restart_required: bool,
    },
}

/// Frozen six-field Hypothesis keyset shared by Plan B and future Plan D.
#[derive(Debug, Clone, PartialEq, Eq, PartialOrd, Ord, Serialize)]
pub struct InvestigationHypothesisSortKey {
    pub organization_ordinal: i32,
    pub group_key: String,
    pub readiness_rank: i16,
    pub epistemic_rank: i16,
    pub root_id: Uuid,
    pub revision_ordinal: i32,
}

#[derive(Debug, Clone, Default, PartialEq, Eq)]
pub struct InvestigationHypothesisFilters {
    pub organization_ids: Vec<Uuid>,
    pub epistemic_states: Vec<String>,
    pub readiness_states: Vec<String>,
    pub capability_states: Vec<String>,
    pub source_kinds: Vec<String>,
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct InvestigationHypothesisListQuery {
    pub filters: InvestigationHypothesisFilters,
    pub after: Option<InvestigationHypothesisSortKey>,
    /// Signed cursor authority decoded by the app.  When present, the DB
    /// revalidates it and runs the page query in this same read transaction.
    pub expected_page_authority: Option<InvestigationPageValidationInput>,
    pub page_size: u32,
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct InvestigationHypothesisListItem {
    pub sort_key: InvestigationHypothesisSortKey,
    pub root_id: Uuid,
    pub revision_id: Uuid,
    pub organization_id: Uuid,
    pub subject_kind: String,
    pub subject_identity_hash: String,
    pub target_type_at_time: String,
    pub target_value_at_time: String,
    pub predicate_schema: String,
    pub predicate_summary: String,
    pub trust_boundary: String,
    pub polarity: String,
    pub epistemic_state: String,
    pub lifecycle_state: String,
    pub planning_readiness: String,
    pub support_count: i64,
    pub contradiction_count: i64,
    pub gap_count: i64,
    pub legacy_projection_status: Option<String>,
    pub residual_codes: Vec<String>,
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct InvestigationHypothesisListPage {
    pub authority: InvestigationReadAuthority,
    pub hypotheses: Vec<InvestigationHypothesisListItem>,
    pub next_key: Option<InvestigationHypothesisSortKey>,
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct InvestigationHypothesisDetail {
    pub authority: InvestigationReadAuthority,
    pub hypothesis: InvestigationHypothesisListItem,
    pub predecessor_revision_id: Option<Uuid>,
    pub lineage_revision_ids: Vec<Uuid>,
    pub support_ref_ids: Vec<String>,
    pub contradiction_ref_ids: Vec<String>,
    pub application_context_ref_ids: Vec<String>,
    pub gap_ref_ids: Vec<String>,
    pub verification_objective_summaries: Vec<String>,
    pub actor_topology: Vec<InvestigationActorTopologyNode>,
    pub legacy_unavailable_fields: Vec<String>,
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct InvestigationSummary {
    pub authority: InvestigationReadAuthority,
    pub active_generation_id: Option<Uuid>,
    pub active_generation_seal_hash: Option<String>,
    pub current_hypothesis_count: i64,
    pub closed_hypothesis_count: i64,
    pub contested_hypothesis_count: i64,
    pub residual_count: i64,
    pub generation_count: i64,
    pub wave_count: i64,
    pub campaign_count: i64,
    pub open_obligation_count: i64,
    pub control_decision: String,
    pub coverage_grade: String,
    pub coverage_denominator: InvestigationCoverageDenominator,
    pub coverage_sufficiency: String,
    pub generations: Vec<InvestigationGenerationSummary>,
    pub waves: Vec<InvestigationWaveSummary>,
    pub open_obligations: Vec<InvestigationOpenObligationSummary>,
    pub source_census: Vec<InvestigationSourceCensusMember>,
    pub main_actor: Option<InvestigationActorTopologyNode>,
    pub actor_topology: Vec<InvestigationActorTopologyNode>,
}

#[derive(Debug, Clone, PartialEq, Eq, sqlx::FromRow)]
pub struct InvestigationSourceCensusMember {
    pub organization_id: Uuid,
    pub snapshot_id: Uuid,
    pub context_item_count: i64,
    pub context_item_set_sha256: String,
    pub methodology_hit_count: i64,
    pub methodology_result_set_sha256: String,
    pub omission_count: i64,
    pub omission_set_sha256: String,
}

#[derive(Debug, Clone, PartialEq, Eq, sqlx::FromRow)]
pub struct InvestigationActorTopologyNode {
    pub actor_kind: String,
    pub organization_id: Uuid,
    pub hypothesis_revision_id: Option<Uuid>,
    pub task_id: Option<Uuid>,
    pub subtask_id: Option<Uuid>,
    pub worker_run_id: Uuid,
    pub owning_stage_run_request_id: String,
    pub transcript_request_id: String,
    pub parent_actor_transcript_request_id: Option<String>,
    pub parent_dispatch_tool_request_id: Option<String>,
    pub status: String,
    pub identity_valid: bool,
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct InvestigationGenerationSummary {
    pub generation_id: Uuid,
    pub generation_ordinal: i64,
    pub state: String,
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct InvestigationWaveSummary {
    pub wave_id: Uuid,
    pub wave_ordinal: i64,
    pub state: String,
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct InvestigationOpenObligationSummary {
    pub obligation_id: String,
    pub obligation_kind: String,
}

#[derive(Debug, Clone, Default, PartialEq, Eq)]
pub struct InvestigationCoverageDenominator {
    pub planned: i64,
    pub tested_complete: i64,
    pub tested_degraded: i64,
    pub untested: i64,
    pub blocked: i64,
}

#[derive(Debug, Clone, PartialEq, Eq, PartialOrd, Ord, Serialize)]
pub struct InvestigationCampaignSortKey {
    pub wave_ordinal: i64,
    pub campaign_ordinal: i64,
    pub campaign_id: Uuid,
}

#[derive(Debug, Clone, Default, PartialEq, Eq)]
pub struct InvestigationCampaignFilters {
    pub wave_ids: Vec<Uuid>,
    pub campaign_states: Vec<String>,
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct InvestigationCampaignListQuery {
    pub filters: InvestigationCampaignFilters,
    pub after: Option<InvestigationCampaignSortKey>,
    pub expected_page_authority: Option<InvestigationPageValidationInput>,
    pub page_size: u32,
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct InvestigationCampaignListItem {
    pub sort_key: InvestigationCampaignSortKey,
    pub campaign_id: Uuid,
    pub wave_id: Uuid,
    pub hypothesis_revision_id: Uuid,
    pub label: String,
    pub state: String,
    pub coverage_status: String,
    pub authority_ref_hash: String,
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct InvestigationCampaignListPage {
    pub authority: InvestigationReadAuthority,
    pub campaigns: Vec<InvestigationCampaignListItem>,
    pub next_key: Option<InvestigationCampaignSortKey>,
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct InvestigationCampaignDetail {
    pub authority: InvestigationReadAuthority,
    pub campaign: InvestigationCampaignListItem,
    pub organization_id: Uuid,
    pub round_ids: Vec<Uuid>,
    pub prepared_action_ids: Vec<Uuid>,
    pub authorized_action_count: u64,
    pub blocked_action_count: u64,
    pub open_residual_ids: Vec<Uuid>,
    pub redacted_round_summaries: Vec<String>,
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct InvestigationLegacyProjection {
    pub status: Option<String>,
    pub unavailable_fields: Vec<String>,
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct MaterializedProjectionEntity {
    pub entity_kind: ProjectionEntityKind,
    pub entity_id: String,
    pub entity_version: i64,
    pub projection_hash: String,
    pub entity: ProjectionEntityV1,
    pub change_seq: i64,
    pub invalidation_reason: Option<ProjectionInvalidationReason>,
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct InvestigationProjectionChange {
    pub change_seq: i64,
    pub event_id: Uuid,
    pub batch_id: Uuid,
    pub source_batch_seq: i64,
    pub outbox_member_id: Uuid,
    pub entity_kind: ProjectionEntityKind,
    pub entity_id: String,
    pub entity_version: i64,
    pub change_kind: ProjectionChangeKind,
    pub timeline_event_kind: TimelineEventKind,
    pub invalidation_reason: Option<ProjectionInvalidationReason>,
    pub change_hash: String,
    pub source_occurred_at: Option<DateTime<Utc>>,
    pub source_time_status: ProjectionSourceTimeStatusV1,
    pub projected_at: DateTime<Utc>,
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct ProjectionReadPage {
    pub head: CapturedProjectionHead,
    pub entities: Vec<MaterializedProjectionEntity>,
    pub changes: Vec<InvestigationProjectionChange>,
}

#[derive(Debug, thiserror::Error)]
pub enum InvestigationProjectionError {
    #[error("INVESTIGATION_PROJECTION_STORAGE: {0}")]
    Storage(#[from] sqlx::Error),
    #[error("INVESTIGATION_PROJECTION_SERIALIZATION: {0}")]
    Serialization(#[from] serde_json::Error),
    #[error("{0}")]
    Contract(&'static str),
    #[error("{code}: {message}")]
    InvalidPayload { code: &'static str, message: String },
    #[error("{code}: projection snapshot changed at sequence {current_change_seq}")]
    Stale {
        code: &'static str,
        current_change_seq: i64,
        reason: ProjectionStaleReason,
    },
}

impl InvestigationProjectionError {
    pub const fn code(&self) -> &'static str {
        match self {
            Self::Storage(_) => "INVESTIGATION_PROJECTION_STORAGE",
            Self::Serialization(_) => "INVESTIGATION_PROJECTION_SERIALIZATION",
            Self::Contract(code) => code,
            Self::InvalidPayload { code, .. } => code,
            Self::Stale { code, .. } => code,
        }
    }

    pub const fn current_change_seq(&self) -> Option<i64> {
        match self {
            Self::Stale {
                current_change_seq, ..
            } => Some(*current_change_seq),
            _ => None,
        }
    }

    pub const fn restart_required(&self) -> bool {
        matches!(self, Self::Stale { .. })
    }

    pub const fn stale_reason(&self) -> Option<ProjectionStaleReason> {
        match self {
            Self::Stale { reason, .. } => Some(*reason),
            _ => None,
        }
    }
}

pub(crate) fn invalid_payload(message: impl Into<String>) -> InvestigationProjectionError {
    InvestigationProjectionError::InvalidPayload {
        code: INVESTIGATION_PROJECTION_PAYLOAD_INVALID,
        message: message.into(),
    }
}

pub type InvestigationProjectionResult<T> = Result<T, InvestigationProjectionError>;

pub(crate) fn sha256_bytes(bytes: &[u8]) -> String {
    let digest = Sha256::digest(bytes);
    let mut hex = String::with_capacity(64);
    for byte in digest {
        use std::fmt::Write;
        write!(hex, "{byte:02x}").expect("writing to String cannot fail");
    }
    format!("sha256:{hex}")
}

pub(crate) fn sha256_json<T: Serialize>(value: &T) -> InvestigationProjectionResult<String> {
    Ok(sha256_bytes(&serde_json::to_vec(value)?))
}
