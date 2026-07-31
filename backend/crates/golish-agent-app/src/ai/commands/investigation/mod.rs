//! Authorized, materialized-only Investigation audit IPC.

mod cursor;
mod dto;

use std::collections::BTreeSet;

use golish_app_core::domain::operator::{OperatorChannel, TrustedOperatorPrincipalProvider};
use golish_core::{
    CampaignWritePolicy, ComparePolicy, InvestigationAuthority, InvestigationErrorCode,
    InvestigationRolloutMode, LegacyProjectionPolicy,
};
use golish_db::repo::investigation_projection::{
    capture_investigation_read_authority, get_investigation_hypothesis,
    list_investigation_hypotheses, read_investigation_summary, InvestigationHypothesisDetail,
    InvestigationHypothesisFilters, InvestigationHypothesisListItem,
    InvestigationHypothesisListQuery, InvestigationHypothesisSortKey,
    InvestigationOperationReadAuthority, InvestigationPageValidationInput,
    InvestigationProjectionError, InvestigationReadAuthority,
};
use tauri::State;
use uuid::Uuid;

use crate::state::AgentState;

use self::cursor::{
    canonicalize_investigation_filters, clamp_investigation_page_size, continue_current_cursor,
    issue_current_cursor, InvestigationCursorBinding, InvestigationCursorCurrentAuthority,
    InvestigationCursorFailure, InvestigationCursorTemporalBinding, InvestigationCursorV2,
    InvestigationFilterConflict, InvestigationFilterInput, InvestigationFilterPolicy,
    InvestigationStableSortKeyV1,
};
pub use self::dto::*;

const FORBIDDEN_MESSAGE: &str = "investigation scope is not authorized";
const INVALID_ID_MESSAGE: &str = "investigation identifier is invalid";
const INVALID_ARGUMENT_MESSAGE: &str = "investigation request is invalid";
const CURSOR_INVALID_MESSAGE: &str = "investigation cursor is invalid";
const PROJECTION_STALE_MESSAGE: &str =
    "investigation projection changed; restart from the first page";
const AUTHORITY_CORRUPT_MESSAGE: &str = "investigation authority is inconsistent";
const DATABASE_MESSAGE: &str = "investigation read is unavailable";

const EPISTEMIC_STATES: &[&str] = &[
    "proposed",
    "supported",
    "contested",
    "inconclusive",
    "verified",
    "refuted",
    "invalid",
];
const READINESS_STATES: &[&str] = &[
    "ready_for_strategy",
    "needs_enrichment",
    "deferred",
    "out_of_scope",
    "unsafe",
];
const CAPABILITY_STATES: &[&str] = &["not_available_plan_c"];
const SOURCE_KINDS: &[&str] = &[
    "tool_truth_evidence",
    "finding",
    "verification_receipt",
    "application_context",
    "knowledge_signal",
    "gap",
];
// Every request array is an OR-set within its axis, while the five axes are
// conjunctive. Plan B's only capability value (`not_available_plan_c`) can
// coexist with every epistemic/readiness/source value: it describes the
// absence of Plan C execution, not strategy readiness or evidence origin.
// Therefore Plan B has no real cross-axis exclusion to reject. The shared
// cursor canonicalizer still carries and tests an explicit conflict table so
// Plan D can add a genuinely disjoint filter without changing cursor topology.
const FILTER_CONFLICTS: &[InvestigationFilterConflict<'static>] = &[];

impl InvestigationCommandError {
    fn new(
        code: InvestigationErrorCode,
        message: impl Into<String>,
        current_change_seq: Option<i64>,
        restart_required: bool,
    ) -> Self {
        Self {
            code: code.as_str().to_owned(),
            message: message.into(),
            current_change_seq,
            restart_required,
        }
    }

    fn forbidden() -> Self {
        Self::new(
            InvestigationErrorCode::Forbidden,
            FORBIDDEN_MESSAGE,
            None,
            false,
        )
    }

    fn invalid_id() -> Self {
        Self::new(
            InvestigationErrorCode::InvalidId,
            INVALID_ID_MESSAGE,
            None,
            false,
        )
    }

    fn invalid_argument() -> Self {
        Self::new(
            InvestigationErrorCode::InvalidArgument,
            INVALID_ARGUMENT_MESSAGE,
            None,
            false,
        )
    }

    fn cursor_failure(failure: InvestigationCursorFailure) -> Self {
        let message = match failure {
            InvestigationCursorFailure::Invalid => CURSOR_INVALID_MESSAGE,
            InvestigationCursorFailure::Stale => PROJECTION_STALE_MESSAGE,
        };
        Self {
            code: failure.code().to_owned(),
            message: message.to_owned(),
            current_change_seq: None,
            restart_required: failure.restart_required(),
        }
    }
}

fn map_projection_error(error: InvestigationProjectionError) -> InvestigationCommandError {
    if error.restart_required() {
        return InvestigationCommandError::new(
            InvestigationErrorCode::ProjectionStale,
            PROJECTION_STALE_MESSAGE,
            error.current_change_seq(),
            true,
        );
    }
    match error {
        InvestigationProjectionError::Storage(_) => InvestigationCommandError::new(
            InvestigationErrorCode::Database,
            DATABASE_MESSAGE,
            None,
            false,
        ),
        InvestigationProjectionError::Serialization(_)
        | InvestigationProjectionError::Contract(_)
        | InvestigationProjectionError::InvalidPayload { .. } => InvestigationCommandError::new(
            InvestigationErrorCode::AuthorityCorrupt,
            AUTHORITY_CORRUPT_MESSAGE,
            None,
            false,
        ),
        InvestigationProjectionError::Stale { .. } => unreachable!("handled above"),
    }
}

/// Server-derived authorization for one operation. Selector membership is
/// intentionally checked only after this operation-level authorization has
/// succeeded, so foreign selectors cannot become an existence oracle.
#[derive(Debug)]
pub struct AuthorizedInvestigationScope {
    operation_id: Uuid,
    organization_ids: BTreeSet<Uuid>,
}

impl AuthorizedInvestigationScope {
    pub fn operation_id(&self) -> Uuid {
        self.operation_id
    }

    pub fn authorize_organization_selectors(
        &self,
        selectors: &[Uuid],
    ) -> Result<(), InvestigationCommandError> {
        if selectors
            .iter()
            .all(|organization_id| self.organization_ids.contains(organization_id))
        {
            Ok(())
        } else {
            Err(InvestigationCommandError::forbidden())
        }
    }
}

/// Authorize the principal and operation before any projection read or
/// selector existence branch. All trust/scope failures collapse to one error.
pub async fn authorize_investigation_scope(
    pool: &sqlx::PgPool,
    principal_provider: &dyn TrustedOperatorPrincipalProvider,
    ai_state: &crate::state::AiState,
    trusted_session_id: &str,
    operation_id: Uuid,
) -> Result<AuthorizedInvestigationScope, InvestigationCommandError> {
    let principal = principal_provider
        .current(OperatorChannel::LocalDesktop)
        .await
        .map_err(|_| InvestigationCommandError::forbidden())?;
    if principal.channel() != OperatorChannel::LocalDesktop {
        return Err(InvestigationCommandError::forbidden());
    }
    let persisted_principal = golish_db::repo::operator_principals::current_local(pool)
        .await
        .map_err(|_| InvestigationCommandError::forbidden())?;
    if principal.id().as_uuid() != persisted_principal.id {
        return Err(InvestigationCommandError::forbidden());
    }
    let operation = golish_db::repo::operation_state::get(pool, operation_id)
        .await
        .map_err(|_| InvestigationCommandError::forbidden())?
        .ok_or_else(InvestigationCommandError::forbidden)?;
    let task = golish_db::repo::tasks::get(pool, operation_id)
        .await
        .map_err(|_| InvestigationCommandError::forbidden())?
        .ok_or_else(InvestigationCommandError::forbidden)?;
    let session = golish_db::repo::sessions::get(pool, task.session_id)
        .await
        .map_err(|_| InvestigationCommandError::forbidden())?
        .ok_or_else(InvestigationCommandError::forbidden)?;
    if session.chat_session_key.as_deref() != Some(trusted_session_id) {
        return Err(InvestigationCommandError::forbidden());
    }
    let bridge = ai_state
        .get_session_bridge(trusted_session_id)
        .await
        .ok_or_else(InvestigationCommandError::forbidden)?;
    let trusted_workspace_path = bridge.workspace().read().await.clone();
    let (canonical_workspace_path, workspace_path_sha256) =
        golish_agent_kit::runtime_memory::canonical_workspace_identity(&trusted_workspace_path)
            .map_err(|_| InvestigationCommandError::forbidden())?;
    let project_scope_id = operation
        .project_scope_id
        .ok_or_else(InvestigationCommandError::forbidden)?;
    let project = golish_db::repo::project_scopes::get_active_for_share(pool, project_scope_id)
        .await
        .map_err(|_| InvestigationCommandError::forbidden())?
        .ok_or_else(InvestigationCommandError::forbidden)?;
    if project.canonical_project_path != canonical_workspace_path
        || project.path_sha256 != workspace_path_sha256
    {
        return Err(InvestigationCommandError::forbidden());
    }
    let scope = golish_db::repo::operation_org_scope::load_for_operation(pool, operation_id)
        .await
        .map_err(|_| InvestigationCommandError::forbidden())?
        .ok_or_else(InvestigationCommandError::forbidden)?;
    let snapshot = &scope.snapshot;
    let organization_ids = scope
        .units
        .iter()
        .filter(|unit| unit.snapshot_id == snapshot.id)
        .map(|unit| unit.organization_id)
        .collect::<BTreeSet<_>>();
    let has_exact_root = scope.units.iter().any(|unit| {
        unit.snapshot_id == snapshot.id
            && unit.organization_id == snapshot.root_organization_id
            && unit.role == "root"
    });
    if snapshot.operation_id != operation_id
        || snapshot.project_scope_id != project_scope_id
        || snapshot.sealed_at.is_none()
        || !has_exact_root
    {
        return Err(InvestigationCommandError::forbidden());
    }

    Ok(AuthorizedInvestigationScope {
        operation_id,
        organization_ids,
    })
}

#[tauri::command]
pub async fn investigation_get_summary(
    request: InvestigationScopeRequest,
    state: State<'_, AgentState>,
) -> Result<InvestigationSummaryView, InvestigationCommandError> {
    let operation_id = parse_uuid(&request.operation_id)?;
    let scope = authorize_investigation_scope(
        state.db_pool.as_ref(),
        state.operator_principal_provider.as_ref(),
        &state.ai_state,
        &request.session_id,
        operation_id,
    )
    .await?;
    let summary = read_investigation_summary(state.db_pool.as_ref(), scope.operation_id())
        .await
        .map_err(map_projection_error)?;
    Ok(InvestigationSummaryView {
        envelope: envelope(&summary.authority, None)?,
        active_generation_id: summary.active_generation_id.map(|id| id.to_string()),
        active_generation_seal_hash: summary.active_generation_seal_hash,
        current_hypothesis_count: summary.current_hypothesis_count,
        closed_hypothesis_count: summary.closed_hypothesis_count,
        contested_hypothesis_count: summary.contested_hypothesis_count,
        residual_count: summary.residual_count,
    })
}

#[tauri::command]
pub async fn investigation_list_hypotheses(
    request: InvestigationHypothesisListRequest,
    state: State<'_, AgentState>,
) -> Result<InvestigationHypothesisListView, InvestigationCommandError> {
    let operation_id = parse_uuid(&request.operation_id)?;
    let scope = authorize_investigation_scope(
        state.db_pool.as_ref(),
        state.operator_principal_provider.as_ref(),
        &state.ai_state,
        &request.session_id,
        operation_id,
    )
    .await?;
    let organization_ids = request
        .organization_ids
        .iter()
        .map(|value| parse_uuid(value))
        .collect::<Result<Vec<_>, _>>()?;
    scope.authorize_organization_selectors(&organization_ids)?;
    let filters = canonicalize_investigation_filters(
        InvestigationFilterInput {
            organization_ids: &organization_ids,
            epistemic_states: &request.epistemic_states,
            readiness_states: &request.readiness_states,
            capability_states: &request.capability_states,
            source_kinds: &request.source_kinds,
        },
        filter_policy(),
    )
    .map_err(|_| InvestigationCommandError::invalid_argument())?;
    let filter_digest = filters.digest();
    let page_size = clamp_investigation_page_size(request.page_size);

    let (after, expected_page_authority) = if let Some(token) = request.cursor.as_deref() {
        let captured = capture_investigation_read_authority(state.db_pool.as_ref(), operation_id)
            .await
            .map_err(map_projection_error)?;
        let binding = cursor_binding(&captured.operation, &filter_digest, page_size);
        let current = InvestigationCursorCurrentAuthority {
            current_change_seq: captured.temporal.as_of_change_seq,
            db_now: captured.temporal.as_of_temporal_cutoff,
            current_authority_epoch_set_hash: &captured.temporal.authority_epoch_set_hash,
        };
        let cursor =
            continue_current_cursor(token, &captured.operation.cursor_salt, &binding, &current)
                .map_err(InvestigationCommandError::cursor_failure)?;
        if request
            .expected_change_seq
            .is_some_and(|expected| expected != cursor.as_of_change_seq)
        {
            return Err(InvestigationCommandError::new(
                InvestigationErrorCode::ProjectionStale,
                PROJECTION_STALE_MESSAGE,
                Some(captured.temporal.as_of_change_seq),
                true,
            ));
        }
        let after = hypothesis_sort_key(&cursor.stable_sort_key)?;
        let expected = InvestigationPageValidationInput {
            as_of_change_seq: cursor.as_of_change_seq,
            as_of_temporal_cutoff: cursor.as_of_temporal_cutoff,
            authority_epoch_set_hash: cursor.authority_epoch_set_hash,
            earliest_effective_valid_until: cursor.earliest_effective_valid_until,
        };
        (Some(after), Some(expected))
    } else {
        (None, None)
    };

    let page = list_investigation_hypotheses(
        state.db_pool.as_ref(),
        operation_id,
        InvestigationHypothesisListQuery {
            filters: InvestigationHypothesisFilters {
                organization_ids: filters.organization_ids().to_vec(),
                epistemic_states: filters.epistemic_states().to_vec(),
                readiness_states: filters.readiness_states().to_vec(),
                capability_states: filters.capability_states().to_vec(),
                source_kinds: filters.source_kinds().to_vec(),
            },
            after,
            expected_page_authority,
            page_size,
        },
    )
    .await
    .map_err(map_projection_error)?;
    ensure_current_temporal_authority(&page.authority)?;
    if request
        .expected_change_seq
        .is_some_and(|expected| expected != page.authority.temporal.as_of_change_seq)
    {
        return Err(InvestigationCommandError::new(
            InvestigationErrorCode::ProjectionStale,
            PROJECTION_STALE_MESSAGE,
            Some(page.authority.temporal.as_of_change_seq),
            true,
        ));
    }
    let next_cursor = page
        .next_key
        .as_ref()
        .map(|key| issue_hypothesis_cursor(&page.authority, &filter_digest, page_size, key))
        .transpose()?;
    Ok(InvestigationHypothesisListView {
        envelope: envelope(&page.authority, next_cursor)?,
        hypotheses: page.hypotheses.iter().map(list_item_view).collect(),
    })
}

#[tauri::command]
pub async fn investigation_get_hypothesis(
    request: InvestigationHypothesisGetRequest,
    state: State<'_, AgentState>,
) -> Result<InvestigationHypothesisDetailView, InvestigationCommandError> {
    let operation_id = parse_uuid(&request.operation_id)?;
    let scope = authorize_investigation_scope(
        state.db_pool.as_ref(),
        state.operator_principal_provider.as_ref(),
        &state.ai_state,
        &request.session_id,
        operation_id,
    )
    .await?;
    // Parse and resolve the selector only after operation authorization.
    let revision_id = parse_uuid(&request.revision_id)?;
    let detail =
        get_investigation_hypothesis(state.db_pool.as_ref(), scope.operation_id(), revision_id)
            .await
            .map_err(map_projection_error)?
            .ok_or_else(InvestigationCommandError::forbidden)?;
    scope.authorize_organization_selectors(&[detail.hypothesis.organization_id])?;
    detail_view(&detail)
}

fn parse_uuid(value: &str) -> Result<Uuid, InvestigationCommandError> {
    Uuid::parse_str(value).map_err(|_| InvestigationCommandError::invalid_id())
}

fn filter_policy() -> InvestigationFilterPolicy<'static> {
    InvestigationFilterPolicy {
        epistemic_states: EPISTEMIC_STATES,
        readiness_states: READINESS_STATES,
        capability_states: CAPABILITY_STATES,
        source_kinds: SOURCE_KINDS,
        conflicts: FILTER_CONFLICTS,
    }
}

fn cursor_binding<'a>(
    operation: &'a InvestigationOperationReadAuthority,
    filter_digest: &'a str,
    page_size: u32,
) -> InvestigationCursorBinding<'a> {
    InvestigationCursorBinding {
        resource_kind: "hypotheses",
        operation_id: operation.operation_id,
        tool_truth_contract: &operation.tool_truth_contract,
        investigation_contract_version: &operation.investigation_contract_version,
        investigation_rollout_mode: &operation.investigation_rollout_mode,
        filter_digest,
        page_size,
        expected_temporal: None,
    }
}

fn hypothesis_sort_key(
    key: &InvestigationStableSortKeyV1,
) -> Result<InvestigationHypothesisSortKey, InvestigationCommandError> {
    let InvestigationStableSortKeyV1::Hypothesis {
        organization_ordinal,
        group_key,
        readiness_rank,
        epistemic_rank,
        root_id,
        revision_ordinal,
    } = key
    else {
        return Err(InvestigationCommandError::cursor_failure(
            InvestigationCursorFailure::Invalid,
        ));
    };
    Ok(InvestigationHypothesisSortKey {
        organization_ordinal: *organization_ordinal,
        group_key: group_key.clone(),
        readiness_rank: *readiness_rank,
        epistemic_rank: *epistemic_rank,
        root_id: *root_id,
        revision_ordinal: *revision_ordinal,
    })
}

fn issue_hypothesis_cursor(
    authority: &InvestigationReadAuthority,
    filter_digest: &str,
    page_size: u32,
    key: &InvestigationHypothesisSortKey,
) -> Result<String, InvestigationCommandError> {
    let cursor = InvestigationCursorV2::new(
        "hypotheses",
        authority.operation.operation_id,
        InvestigationCursorTemporalBinding {
            as_of_change_seq: authority.temporal.as_of_change_seq,
            as_of_temporal_cutoff: authority.temporal.as_of_temporal_cutoff,
            authority_epoch_set_hash: authority.temporal.authority_epoch_set_hash.clone(),
            earliest_effective_valid_until: authority.temporal.earliest_effective_valid_until,
        },
        authority.operation.tool_truth_contract.clone(),
        authority.operation.investigation_contract_version.clone(),
        authority.operation.investigation_rollout_mode.clone(),
        filter_digest.to_owned(),
        page_size,
        InvestigationStableSortKeyV1::Hypothesis {
            organization_ordinal: key.organization_ordinal,
            group_key: key.group_key.clone(),
            readiness_rank: key.readiness_rank,
            epistemic_rank: key.epistemic_rank,
            root_id: key.root_id,
            revision_ordinal: key.revision_ordinal,
        },
    )
    .map_err(InvestigationCommandError::cursor_failure)?;
    issue_current_cursor(&cursor, &authority.operation.cursor_salt)
        .map_err(InvestigationCommandError::cursor_failure)
}

fn envelope(
    authority: &InvestigationReadAuthority,
    next_cursor: Option<String>,
) -> Result<InvestigationProjectionEnvelope, InvestigationCommandError> {
    ensure_current_temporal_authority(authority)?;
    let projection_schema_version = u32::try_from(authority.temporal.projection_schema_version)
        .map_err(|_| {
            InvestigationCommandError::new(
                InvestigationErrorCode::AuthorityCorrupt,
                AUTHORITY_CORRUPT_MESSAGE,
                None,
                false,
            )
        })?;
    let (_, mode) = golish_db::repo::investigation_rollout::parse_frozen_pair(
        &authority.operation.investigation_contract_version,
        &authority.operation.investigation_rollout_mode,
    )
    .map_err(|_| {
        InvestigationCommandError::new(
            InvestigationErrorCode::AuthorityCorrupt,
            AUTHORITY_CORRUPT_MESSAGE,
            None,
            false,
        )
    })?;
    Ok(InvestigationProjectionEnvelope {
        projection_schema_version,
        change_seq: authority.temporal.as_of_change_seq,
        read_at: authority.temporal.as_of_temporal_cutoff.to_rfc3339(),
        temporal_snapshot: InvestigationTemporalSnapshotView {
            contract_version: 2,
            as_of_temporal_cutoff: authority.temporal.as_of_temporal_cutoff.to_rfc3339(),
            authority_epoch_set_hash: authority.temporal.authority_epoch_set_hash.clone(),
            earliest_effective_valid_until: authority
                .temporal
                .earliest_effective_valid_until
                .to_rfc3339(),
        },
        tool_truth_contract: authority.operation.tool_truth_contract.clone(),
        investigation_contract_version: authority.operation.investigation_contract_version.clone(),
        investigation_rollout_mode: authority.operation.investigation_rollout_mode.clone(),
        mode_policy: mode_policy(mode),
        next_cursor,
    })
}

fn ensure_current_temporal_authority(
    authority: &InvestigationReadAuthority,
) -> Result<(), InvestigationCommandError> {
    if authority.temporal.as_of_temporal_cutoff > authority.temporal.earliest_effective_valid_until
    {
        return Err(InvestigationCommandError::new(
            InvestigationErrorCode::ProjectionStale,
            PROJECTION_STALE_MESSAGE,
            Some(authority.temporal.as_of_change_seq),
            true,
        ));
    }
    Ok(())
}

fn mode_policy(mode: InvestigationRolloutMode) -> InvestigationModePolicyView {
    let policy = mode.policy();
    InvestigationModePolicyView {
        canonical_writer: authority_wire(policy.canonical_writer).to_owned(),
        gate_authority: authority_wire(policy.gate_authority).to_owned(),
        allow_legacy_mutation: policy.allow_legacy_mutation,
        campaign_write_policy: campaign_wire(policy.campaign_write_policy).to_owned(),
        allow_prepared_action_jit: policy.allow_prepared_action_jit,
        compare_policy: compare_wire(policy.compare_policy).to_owned(),
        legacy_projection_policy: legacy_projection_wire(policy.legacy_projection).to_owned(),
    }
}

const fn authority_wire(value: InvestigationAuthority) -> &'static str {
    match value {
        InvestigationAuthority::Legacy => "legacy",
        InvestigationAuthority::Registry => "registry",
    }
}

const fn campaign_wire(value: CampaignWritePolicy) -> &'static str {
    match value {
        CampaignWritePolicy::Off => "off",
        CampaignWritePolicy::ShadowAudit => "shadow_audit",
        CampaignWritePolicy::CompareOnly => "compare_only",
        CampaignWritePolicy::Canonical => "canonical",
    }
}

const fn compare_wire(value: ComparePolicy) -> &'static str {
    match value {
        ComparePolicy::Off => "off",
        ComparePolicy::PromotionBlocking => "promotion_blocking",
        ComparePolicy::WholeRecordExact => "whole_record_exact",
        ComparePolicy::AuditOnly => "audit_only",
    }
}

const fn legacy_projection_wire(value: LegacyProjectionPolicy) -> &'static str {
    match value {
        LegacyProjectionPolicy::Native => "native",
        LegacyProjectionPolicy::CanonicalDerivedFailClosed => "canonical_derived_fail_closed",
        LegacyProjectionPolicy::HistoricalReadOnly => "historical_read_only",
    }
}

fn list_item_view(item: &InvestigationHypothesisListItem) -> InvestigationHypothesisListItemView {
    InvestigationHypothesisListItemView {
        root_id: item.root_id.to_string(),
        revision_id: item.revision_id.to_string(),
        organization_id: item.organization_id.to_string(),
        subject_kind: item.subject_kind.clone(),
        subject_identity_hash: item.subject_identity_hash.clone(),
        target_type_at_time: item.target_type_at_time.clone(),
        target_value_at_time: item.target_value_at_time.clone(),
        predicate_schema: item.predicate_schema.clone(),
        predicate_summary: item.predicate_summary.clone(),
        trust_boundary: item.trust_boundary.clone(),
        polarity: item.polarity.clone(),
        epistemic_state: item.epistemic_state.clone(),
        lifecycle_state: item.lifecycle_state.clone(),
        planning_readiness: item.planning_readiness.clone(),
        support_count: item.support_count,
        contradiction_count: item.contradiction_count,
        gap_count: item.gap_count,
        legacy_projection_status: item.legacy_projection_status.clone(),
        residual_codes: item.residual_codes.clone(),
    }
}

fn detail_view(
    detail: &InvestigationHypothesisDetail,
) -> Result<InvestigationHypothesisDetailView, InvestigationCommandError> {
    Ok(InvestigationHypothesisDetailView {
        envelope: envelope(&detail.authority, None)?,
        hypothesis: list_item_view(&detail.hypothesis),
        predecessor_revision_id: detail.predecessor_revision_id.map(|id| id.to_string()),
        lineage_revision_ids: detail
            .lineage_revision_ids
            .iter()
            .map(Uuid::to_string)
            .collect(),
        support_ref_ids: detail.support_ref_ids.clone(),
        contradiction_ref_ids: detail.contradiction_ref_ids.clone(),
        application_context_ref_ids: detail.application_context_ref_ids.clone(),
        gap_ref_ids: detail.gap_ref_ids.clone(),
        verification_objective_summaries: detail.verification_objective_summaries.clone(),
        legacy_unavailable_fields: detail.legacy_unavailable_fields.clone(),
    })
}

#[cfg(test)]
mod tests {
    use chrono::{TimeZone, Utc};

    use super::*;

    #[test]
    fn investigation_auth_malformed_id_has_stable_non_oracle_error() {
        let error = parse_uuid("not-a-uuid").expect_err("malformed UUID fails closed");
        assert_eq!(error.code, InvestigationErrorCode::InvalidId.as_str());
        assert_eq!(error.message, INVALID_ID_MESSAGE);
        assert_eq!(error.current_change_seq, None);
        assert!(!error.restart_required);
    }

    #[test]
    fn investigation_filter_catalog_rejects_unknown_values() {
        let unknown = ["model_invented".to_owned()];
        let result = canonicalize_investigation_filters(
            InvestigationFilterInput {
                organization_ids: &[],
                epistemic_states: &unknown,
                readiness_states: &[],
                capability_states: &[],
                source_kinds: &[],
            },
            filter_policy(),
        );
        assert!(result.is_err());
        let error = InvestigationCommandError::invalid_argument();
        assert_eq!(error.code, InvestigationErrorCode::InvalidArgument.as_str());
        assert_eq!(error.message, INVALID_ARGUMENT_MESSAGE);
    }

    #[test]
    fn investigation_filter_policy_has_no_false_plan_b_cross_axis_conflicts() {
        for epistemic in EPISTEMIC_STATES {
            for readiness in READINESS_STATES {
                for source in SOURCE_KINDS {
                    canonicalize_investigation_filters(
                        InvestigationFilterInput {
                            organization_ids: &[],
                            epistemic_states: &[(*epistemic).to_owned()],
                            readiness_states: &[(*readiness).to_owned()],
                            capability_states: &[CAPABILITY_STATES[0].to_owned()],
                            source_kinds: &[(*source).to_owned()],
                        },
                        filter_policy(),
                    )
                    .expect("Plan B axes describe coexisting OR-filter dimensions");
                }
            }
        }
    }

    #[test]
    fn investigation_temporal_snapshot_already_expired_is_stale_before_response() {
        let read_at = Utc
            .with_ymd_and_hms(2026, 7, 31, 8, 0, 1)
            .single()
            .expect("fixture read time");
        let expired_at = Utc
            .with_ymd_and_hms(2026, 7, 31, 8, 0, 0)
            .single()
            .expect("fixture expiry");
        let authority = InvestigationReadAuthority {
            operation: InvestigationOperationReadAuthority {
                operation_id: Uuid::new_v4(),
                tool_truth_contract: "tool_truth_receipt_v1".to_owned(),
                investigation_contract_version: "hypothesis_registry_v1".to_owned(),
                investigation_rollout_mode: "new_only".to_owned(),
                cursor_salt: [7; 32],
            },
            temporal:
                golish_db::repo::investigation_projection::InvestigationTemporalReadAuthority {
                    projection_schema_version: 1,
                    as_of_change_seq: 41,
                    as_of_temporal_cutoff: read_at,
                    authority_epoch_set_hash: format!("sha256:{}", "a".repeat(64)),
                    earliest_effective_valid_until: expired_at,
                },
        };
        let error = ensure_current_temporal_authority(&authority)
            .expect_err("already-expired first page must not escape");
        assert_eq!(error.code, InvestigationErrorCode::ProjectionStale.as_str());
        assert_eq!(error.current_change_seq, Some(41));
        assert!(error.restart_required);
        assert!(envelope(&authority, None).is_err());
    }
}
