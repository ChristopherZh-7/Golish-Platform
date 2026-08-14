use serde::{Deserialize, Serialize};
use ts_rs::TS;

use golish_core::investigation_projection::{
    ProjectionEntityKind, ProjectionInvalidationReason, ProjectionSourceTimeStatusV1,
    TimelineEventKind,
};

#[derive(Clone, Debug, Deserialize, Serialize, TS)]
#[serde(rename_all = "camelCase", deny_unknown_fields)]
#[ts(export, export_to = "../../../../frontend/lib/generated/")]
pub struct InvestigationHypothesisListRequest {
    pub session_id: String,
    pub operation_id: String,
    pub stage_execution_id: String,
    pub stage_run_request_id: String,
    pub organization_ids: Vec<String>,
    pub epistemic_states: Vec<String>,
    pub readiness_states: Vec<String>,
    pub capability_states: Vec<String>,
    pub source_kinds: Vec<String>,
    pub cursor: Option<String>,
    #[ts(type = "number | null")]
    pub expected_change_seq: Option<i64>,
    pub expected_temporal_cutoff: Option<String>,
    pub expected_authority_epoch_set_hash: Option<String>,
    pub expected_earliest_effective_valid_until: Option<String>,
    pub page_size: u32,
}

#[derive(Clone, Debug, Deserialize, Serialize, TS)]
#[serde(rename_all = "camelCase", deny_unknown_fields)]
#[ts(export, export_to = "../../../../frontend/lib/generated/")]
pub struct InvestigationScopeRequest {
    pub session_id: String,
    pub operation_id: String,
    pub stage_execution_id: String,
    pub stage_run_request_id: String,
    #[ts(type = "number | null")]
    pub expected_change_seq: Option<i64>,
    pub expected_temporal_cutoff: Option<String>,
    pub expected_authority_epoch_set_hash: Option<String>,
    pub expected_earliest_effective_valid_until: Option<String>,
}

#[derive(Clone, Debug, Deserialize, Serialize, TS)]
#[serde(rename_all = "camelCase", deny_unknown_fields)]
#[ts(export, export_to = "../../../../frontend/lib/generated/")]
pub struct InvestigationRequestStopRequest {
    pub session_id: String,
    pub operation_id: String,
    pub stage_execution_id: String,
    pub stage_run_request_id: String,
    pub expected_investigation_run_state_head: String,
    #[ts(type = "number")]
    pub expected_change_seq: i64,
    pub idempotency_key: String,
}

#[derive(Clone, Debug, PartialEq, Eq, Serialize, TS)]
#[serde(rename_all = "camelCase")]
#[ts(export, export_to = "../../../../frontend/lib/generated/")]
pub struct InvestigationControlProjectionV1 {
    pub operation_id: String,
    pub stage_execution_id: String,
    pub stage_run_request_id: String,
    pub stage_topology_contract: String,
    pub investigation_run_state: String,
    pub investigation_run_state_head: String,
    #[ts(type = "number")]
    pub change_seq: i64,
    #[ts(type = "number")]
    pub stop_epoch: i64,
    pub stop_allowed: bool,
    pub stop_unavailable_reason: Option<String>,
    pub reset_allowed: bool,
    pub reset_unavailable_reason: Option<String>,
    pub successor_fork_allowed: bool,
    pub successor_fork_unavailable_reason: Option<String>,
    pub adoption_contract_version: u32,
    pub control_policy_version: u32,
}

#[derive(Clone, Debug, Serialize, TS)]
#[serde(rename_all = "camelCase")]
#[ts(export, export_to = "../../../../frontend/lib/generated/")]
pub struct InvestigationRequestStopResponse {
    pub stop_intent_id: String,
    pub idempotency_key: String,
    #[ts(type = "number")]
    pub stop_epoch: i64,
    #[ts(type = "number")]
    pub frozen_work_count: i64,
    pub frozen_work_set_sha256: String,
    pub receipt_sha256: String,
    pub control_projection: InvestigationControlProjectionV1,
}

#[derive(Clone, Debug, Deserialize, Serialize, TS)]
#[serde(rename_all = "camelCase", deny_unknown_fields)]
#[ts(export, export_to = "../../../../frontend/lib/generated/")]
pub struct InvestigationHypothesisGetRequest {
    pub session_id: String,
    pub operation_id: String,
    pub stage_execution_id: String,
    pub stage_run_request_id: String,
    pub revision_id: String,
    #[ts(type = "number")]
    pub expected_change_seq: i64,
    pub expected_temporal_cutoff: String,
    pub expected_authority_epoch_set_hash: String,
    pub expected_earliest_effective_valid_until: String,
}

#[derive(Clone, Debug, Serialize, TS)]
#[serde(rename_all = "camelCase")]
#[ts(export, export_to = "../../../../frontend/lib/generated/")]
pub struct InvestigationTemporalSnapshotView {
    pub contract_version: u32,
    pub as_of_temporal_cutoff: String,
    pub authority_epoch_set_hash: String,
    pub earliest_effective_valid_until: String,
}

#[derive(Clone, Debug, Serialize, TS)]
#[serde(rename_all = "camelCase")]
#[ts(export, export_to = "../../../../frontend/lib/generated/")]
pub struct InvestigationProjectionEnvelope {
    pub projection_schema_version: u32,
    #[ts(type = "number")]
    pub change_seq: i64,
    pub read_at: String,
    pub temporal_snapshot: InvestigationTemporalSnapshotView,
    pub tool_truth_contract: String,
    pub investigation_contract_version: String,
    pub investigation_rollout_mode: String,
    pub mode_policy: InvestigationModePolicyView,
    pub next_cursor: Option<String>,
}

#[derive(Clone, Debug, Serialize, TS)]
#[serde(rename_all = "camelCase")]
#[ts(export, export_to = "../../../../frontend/lib/generated/")]
pub struct InvestigationModePolicyView {
    pub canonical_writer: String,
    pub gate_authority: String,
    pub allow_legacy_mutation: bool,
    pub campaign_write_policy: String,
    pub compare_policy: String,
    pub legacy_projection_policy: String,
}

#[derive(Clone, Debug, Serialize, TS)]
#[serde(rename_all = "camelCase")]
#[ts(export, export_to = "../../../../frontend/lib/generated/")]
pub struct InvestigationCommandError {
    pub code: String,
    pub message: String,
    #[ts(type = "number | null")]
    pub current_change_seq: Option<i64>,
    pub restart_required: bool,
}

#[derive(Clone, Debug, Serialize, TS)]
#[serde(rename_all = "camelCase")]
#[ts(export, export_to = "../../../../frontend/lib/generated/")]
pub struct InvestigationSummaryView {
    pub envelope: InvestigationProjectionEnvelope,
    pub control_projection: InvestigationControlProjectionV1,
    pub active_generation_id: Option<String>,
    pub active_generation_seal_hash: Option<String>,
    #[ts(type = "number")]
    pub current_hypothesis_count: i64,
    #[ts(type = "number")]
    pub closed_hypothesis_count: i64,
    #[ts(type = "number")]
    pub contested_hypothesis_count: i64,
    #[ts(type = "number")]
    pub residual_count: i64,
    #[ts(type = "number")]
    pub generation_count: i64,
    #[ts(type = "number")]
    pub wave_count: i64,
    #[ts(type = "number")]
    pub campaign_count: i64,
    #[ts(type = "number")]
    pub open_obligation_count: i64,
    pub generations: Vec<InvestigationGenerationSummaryView>,
    pub waves: Vec<InvestigationWaveSummaryView>,
    pub open_obligations: Vec<InvestigationOpenObligationSummaryView>,
    pub source_census: Vec<InvestigationSourceCensusMemberView>,
    pub main_actor: InvestigationActorTopologyNodeView,
    pub actor_topology: Vec<InvestigationActorTopologyNodeView>,
    pub coverage_denominator: InvestigationCoverageDenominatorView,
    pub coverage_sufficiency: String,
    pub authority_time_members: Vec<InvestigationAuthorityTimeViewV1>,
    pub control_decision: String,
    pub coverage_grade: String,
}

#[derive(Clone, Debug, Serialize, TS)]
#[serde(rename_all = "camelCase")]
#[ts(export, export_to = "../../../../frontend/lib/generated/")]
pub struct InvestigationSourceCensusMemberView {
    pub organization_id: String,
    pub snapshot_id: String,
    #[ts(type = "number")]
    pub context_item_count: i64,
    pub context_item_set_sha256: String,
    #[ts(type = "number")]
    pub methodology_hit_count: i64,
    pub methodology_result_set_sha256: String,
    #[ts(type = "number")]
    pub omission_count: i64,
    pub omission_set_sha256: String,
}

#[derive(Clone, Debug, Serialize, TS)]
#[serde(rename_all = "camelCase")]
#[ts(export, export_to = "../../../../frontend/lib/generated/")]
pub struct InvestigationActorTopologyNodeView {
    pub actor_kind: String,
    pub organization_id: String,
    pub hypothesis_revision_id: Option<String>,
    pub task_id: Option<String>,
    pub subtask_id: Option<String>,
    pub worker_run_id: String,
    pub owning_stage_run_request_id: String,
    pub transcript_request_id: String,
    pub parent_actor_transcript_request_id: Option<String>,
    pub parent_dispatch_tool_request_id: Option<String>,
    pub status: String,
}

#[derive(Clone, Debug, Serialize, TS)]
#[serde(rename_all = "camelCase")]
#[ts(export, export_to = "../../../../frontend/lib/generated/")]
pub struct InvestigationGenerationSummaryView {
    pub generation_id: Option<String>,
    #[ts(type = "number")]
    pub generation_ordinal: i64,
    pub state: String,
}

#[derive(Clone, Debug, Serialize, TS)]
#[serde(rename_all = "camelCase")]
#[ts(export, export_to = "../../../../frontend/lib/generated/")]
pub struct InvestigationWaveSummaryView {
    pub wave_id: Option<String>,
    #[ts(type = "number")]
    pub wave_ordinal: i64,
    pub state: String,
}

#[derive(Clone, Debug, Serialize, TS)]
#[serde(rename_all = "camelCase")]
#[ts(export, export_to = "../../../../frontend/lib/generated/")]
pub struct InvestigationOpenObligationSummaryView {
    pub obligation_id: String,
    pub obligation_kind: String,
}

#[derive(Clone, Debug, Serialize, TS)]
#[serde(rename_all = "camelCase")]
#[ts(export, export_to = "../../../../frontend/lib/generated/")]
pub struct InvestigationCoverageDenominatorView {
    #[ts(type = "number")]
    pub planned: i64,
    #[ts(type = "number")]
    pub tested_complete: i64,
    #[ts(type = "number")]
    pub tested_degraded: i64,
    #[ts(type = "number")]
    pub untested: i64,
    #[ts(type = "number")]
    pub blocked: i64,
}

#[derive(Clone, Debug, Serialize, TS)]
#[serde(rename_all = "camelCase")]
#[ts(export, export_to = "../../../../frontend/lib/generated/")]
pub struct InvestigationAuthorityTimeViewV1 {
    pub observed_as_of: String,
    pub effective_valid_until: Option<String>,
    pub authority_epoch_hash: String,
    pub temporal_status: String,
}

#[derive(Clone, Debug, Deserialize, Serialize, TS)]
#[serde(rename_all = "camelCase", deny_unknown_fields)]
#[ts(export, export_to = "../../../../frontend/lib/generated/")]
pub struct InvestigationCampaignListRequest {
    pub session_id: String,
    pub operation_id: String,
    pub stage_execution_id: String,
    pub stage_run_request_id: String,
    pub wave_ids: Vec<String>,
    pub campaign_states: Vec<String>,
    pub cursor: Option<String>,
    #[ts(type = "number | null")]
    pub expected_change_seq: Option<i64>,
    pub expected_temporal_cutoff: Option<String>,
    pub expected_authority_epoch_set_hash: Option<String>,
    pub expected_earliest_effective_valid_until: Option<String>,
    pub page_size: Option<u32>,
}

#[derive(Clone, Debug, Serialize, TS)]
#[serde(rename_all = "camelCase")]
#[ts(export, export_to = "../../../../frontend/lib/generated/")]
pub struct InvestigationCampaignListItemView {
    pub campaign_id: String,
    #[ts(type = "number")]
    pub wave_ordinal: i64,
    #[ts(type = "number")]
    pub campaign_ordinal: i64,
    pub label: String,
    pub state: String,
    pub coverage_status: String,
    pub authority_time: InvestigationAuthorityTimeViewV1,
}

#[derive(Clone, Debug, Serialize, TS)]
#[serde(rename_all = "camelCase")]
#[ts(export, export_to = "../../../../frontend/lib/generated/")]
pub struct InvestigationCampaignPageResponse {
    pub envelope: InvestigationProjectionEnvelope,
    pub campaigns: Vec<InvestigationCampaignListItemView>,
}

#[derive(Clone, Debug, Deserialize, Serialize, TS)]
#[serde(rename_all = "camelCase", deny_unknown_fields)]
#[ts(export, export_to = "../../../../frontend/lib/generated/")]
pub struct InvestigationCampaignDetailRequest {
    pub session_id: String,
    pub operation_id: String,
    pub stage_execution_id: String,
    pub stage_run_request_id: String,
    pub campaign_id: String,
    #[ts(type = "number")]
    pub expected_change_seq: i64,
    pub expected_temporal_cutoff: String,
    pub expected_authority_epoch_set_hash: String,
    pub expected_earliest_effective_valid_until: String,
}

#[derive(Clone, Debug, Serialize, TS)]
#[serde(rename_all = "camelCase")]
#[ts(export, export_to = "../../../../frontend/lib/generated/")]
pub struct InvestigationCampaignDetailView {
    pub campaign_id: String,
    pub hypothesis_revision_id: String,
    #[ts(type = "number")]
    pub wave_ordinal: i64,
    #[ts(type = "number")]
    pub campaign_ordinal: i64,
    pub state: String,
    pub coverage_status: String,
    pub round_ids: Vec<String>,
    pub prepared_action_ids: Vec<String>,
    #[ts(type = "number")]
    pub authorized_action_count: u64,
    #[ts(type = "number")]
    pub blocked_action_count: u64,
    pub open_residual_ids: Vec<String>,
    pub redacted_round_summaries: Vec<String>,
    pub authority_time: InvestigationAuthorityTimeViewV1,
}

#[derive(Clone, Debug, Serialize, TS)]
#[serde(rename_all = "camelCase")]
#[ts(export, export_to = "../../../../frontend/lib/generated/")]
pub struct InvestigationCampaignDetailResponse {
    pub envelope: InvestigationProjectionEnvelope,
    pub campaign: InvestigationCampaignDetailView,
}

#[derive(Clone, Debug, Deserialize, Serialize, TS)]
#[serde(rename_all = "camelCase", deny_unknown_fields)]
#[ts(export, export_to = "../../../../frontend/lib/generated/")]
pub struct InvestigationTimelineListRequest {
    pub session_id: String,
    pub operation_id: String,
    pub stage_execution_id: String,
    pub stage_run_request_id: String,
    pub event_kinds: Vec<TimelineEventKind>,
    pub cursor: Option<String>,
    #[ts(type = "number | null")]
    pub expected_change_seq: Option<i64>,
    pub expected_temporal_cutoff: Option<String>,
    pub expected_authority_epoch_set_hash: Option<String>,
    pub expected_earliest_effective_valid_until: Option<String>,
    pub page_size: Option<u32>,
}

#[derive(Clone, Debug, Serialize, TS)]
#[serde(rename_all = "camelCase")]
#[ts(export, export_to = "../../../../frontend/lib/generated/")]
pub struct InvestigationTimelineItemView {
    pub event_id: String,
    #[ts(type = "number")]
    pub change_seq: i64,
    pub event_kind: TimelineEventKind,
    pub entity_kind: ProjectionEntityKind,
    pub entity_id: String,
    #[ts(type = "number")]
    pub entity_version: u64,
    pub source_occurred_at: Option<String>,
    pub source_time_status: ProjectionSourceTimeStatusV1,
    pub projected_at: String,
    pub invalidation_reason: Option<ProjectionInvalidationReason>,
    pub authority_time: InvestigationAuthorityTimeViewV1,
}

#[derive(Clone, Debug, Serialize, TS)]
#[serde(rename_all = "camelCase")]
#[ts(export, export_to = "../../../../frontend/lib/generated/")]
pub struct InvestigationTimelinePageResponse {
    pub envelope: InvestigationProjectionEnvelope,
    pub events: Vec<InvestigationTimelineItemView>,
}

#[derive(Clone, Debug, Serialize, TS)]
#[serde(rename_all = "camelCase")]
#[ts(export, export_to = "../../../../frontend/lib/generated/")]
pub struct InvestigationHypothesisListItemView {
    pub root_id: String,
    pub revision_id: String,
    pub organization_id: String,
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
    #[ts(type = "number")]
    pub support_count: i64,
    #[ts(type = "number")]
    pub contradiction_count: i64,
    #[ts(type = "number")]
    pub gap_count: i64,
    pub legacy_projection_status: Option<String>,
    pub residual_codes: Vec<String>,
}

#[derive(Clone, Debug, Serialize, TS)]
#[serde(rename_all = "camelCase")]
#[ts(export, export_to = "../../../../frontend/lib/generated/")]
pub struct InvestigationHypothesisListView {
    pub envelope: InvestigationProjectionEnvelope,
    pub hypotheses: Vec<InvestigationHypothesisListItemView>,
}

#[derive(Clone, Debug, Serialize, TS)]
#[serde(rename_all = "camelCase")]
#[ts(export, export_to = "../../../../frontend/lib/generated/")]
pub struct InvestigationHypothesisDetailView {
    pub envelope: InvestigationProjectionEnvelope,
    pub hypothesis: InvestigationHypothesisListItemView,
    pub predecessor_revision_id: Option<String>,
    pub lineage_revision_ids: Vec<String>,
    pub support_ref_ids: Vec<String>,
    pub contradiction_ref_ids: Vec<String>,
    pub application_context_ref_ids: Vec<String>,
    pub gap_ref_ids: Vec<String>,
    pub verification_objective_summaries: Vec<String>,
    pub actor_topology: Vec<InvestigationActorTopologyNodeView>,
    pub legacy_unavailable_fields: Vec<String>,
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn investigation_request_dtos_reject_unknown_fields() {
        let error = serde_json::from_value::<InvestigationScopeRequest>(serde_json::json!({
            "sessionId": "session-1",
            "operationId": "00000000-0000-0000-0000-000000000001",
            "cursorSalt": "must-never-cross-ipc"
        }))
        .expect_err("unknown request fields must fail closed");

        assert!(error.to_string().contains("unknown field"));
    }

    #[test]
    fn investigation_dto_declarations_exclude_sensitive_payload_fields() {
        let config = ts_rs::Config::default();
        let declarations = [
            InvestigationHypothesisListRequest::decl(&config),
            InvestigationScopeRequest::decl(&config),
            InvestigationHypothesisGetRequest::decl(&config),
            InvestigationRequestStopRequest::decl(&config),
            InvestigationControlProjectionV1::decl(&config),
            InvestigationRequestStopResponse::decl(&config),
            InvestigationTemporalSnapshotView::decl(&config),
            InvestigationProjectionEnvelope::decl(&config),
            InvestigationModePolicyView::decl(&config),
            InvestigationCommandError::decl(&config),
            InvestigationSummaryView::decl(&config),
            InvestigationSourceCensusMemberView::decl(&config),
            InvestigationActorTopologyNodeView::decl(&config),
            InvestigationGenerationSummaryView::decl(&config),
            InvestigationWaveSummaryView::decl(&config),
            InvestigationOpenObligationSummaryView::decl(&config),
            InvestigationCoverageDenominatorView::decl(&config),
            InvestigationAuthorityTimeViewV1::decl(&config),
            InvestigationCampaignListRequest::decl(&config),
            InvestigationCampaignListItemView::decl(&config),
            InvestigationCampaignPageResponse::decl(&config),
            InvestigationCampaignDetailRequest::decl(&config),
            InvestigationCampaignDetailView::decl(&config),
            InvestigationCampaignDetailResponse::decl(&config),
            InvestigationTimelineListRequest::decl(&config),
            InvestigationTimelineItemView::decl(&config),
            InvestigationTimelinePageResponse::decl(&config),
            InvestigationHypothesisListItemView::decl(&config),
            InvestigationHypothesisListView::decl(&config),
            InvestigationHypothesisDetailView::decl(&config),
        ]
        .join("\n");

        for forbidden in [
            "rawPayload",
            "credential",
            "prompt",
            "proseArtifact",
            "leaseToken",
            "checkpoint",
            "cursorSalt",
        ] {
            assert!(
                !declarations.contains(forbidden),
                "forbidden field leaked into DTO declarations: {forbidden}"
            );
        }
    }
}
