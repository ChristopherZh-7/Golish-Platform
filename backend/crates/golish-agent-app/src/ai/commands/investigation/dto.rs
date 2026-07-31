use serde::{Deserialize, Serialize};
use ts_rs::TS;

#[derive(Clone, Debug, Deserialize, Serialize, TS)]
#[serde(rename_all = "camelCase", deny_unknown_fields)]
#[ts(export, export_to = "../../../../frontend/lib/generated/")]
pub struct InvestigationHypothesisListRequest {
    /// Live agent session whose server-owned bridge workspace must own the
    /// requested operation. This is a selector, never workspace authority.
    pub session_id: String,
    pub operation_id: String,
    pub organization_ids: Vec<String>,
    pub epistemic_states: Vec<String>,
    pub readiness_states: Vec<String>,
    pub capability_states: Vec<String>,
    pub source_kinds: Vec<String>,
    pub cursor: Option<String>,
    #[ts(type = "number | null")]
    pub expected_change_seq: Option<i64>,
    pub page_size: u32,
}

#[derive(Clone, Debug, Deserialize, Serialize, TS)]
#[serde(rename_all = "camelCase", deny_unknown_fields)]
#[ts(export, export_to = "../../../../frontend/lib/generated/")]
pub struct InvestigationScopeRequest {
    /// Live agent session whose server-owned bridge workspace must own the
    /// requested operation. This is a selector, never workspace authority.
    pub session_id: String,
    pub operation_id: String,
}

#[derive(Clone, Debug, Deserialize, Serialize, TS)]
#[serde(rename_all = "camelCase", deny_unknown_fields)]
#[ts(export, export_to = "../../../../frontend/lib/generated/")]
pub struct InvestigationHypothesisGetRequest {
    /// Live agent session whose server-owned bridge workspace must own the
    /// requested operation. This is a selector, never workspace authority.
    pub session_id: String,
    pub operation_id: String,
    pub revision_id: String,
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
    pub allow_prepared_action_jit: bool,
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
            InvestigationTemporalSnapshotView::decl(&config),
            InvestigationProjectionEnvelope::decl(&config),
            InvestigationModePolicyView::decl(&config),
            InvestigationCommandError::decl(&config),
            InvestigationSummaryView::decl(&config),
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
