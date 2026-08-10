//! Authorized, materialized-only Investigation audit IPC.

mod cursor;
mod dto;

use std::{collections::BTreeSet, sync::OnceLock};

use golish_app_core::domain::operator::{OperatorChannel, TrustedOperatorPrincipalProvider};
use golish_core::{
    CampaignWritePolicy, ComparePolicy, InvestigationAuthority, InvestigationErrorCode,
    InvestigationRolloutMode, LegacyProjectionPolicy,
};
use golish_db::repo::investigation_projection::{
    capture_investigation_read_authority_for_stage_run, get_investigation_campaign_for_stage_run,
    get_investigation_hypothesis_for_stage_run, list_investigation_campaigns_for_stage_run,
    list_investigation_hypotheses_for_stage_run, read_investigation_summary_for_stage_run,
    read_investigation_timeline_for_stage_run, InvestigationCampaignDetail,
    InvestigationCampaignFilters, InvestigationCampaignListItem, InvestigationCampaignListQuery,
    InvestigationCampaignSortKey, InvestigationHypothesisDetail, InvestigationHypothesisFilters,
    InvestigationHypothesisListItem, InvestigationHypothesisListQuery,
    InvestigationHypothesisSortKey, InvestigationOperationReadAuthority,
    InvestigationPageValidationInput, InvestigationProjectionError, InvestigationReadAuthority,
    InvestigationStageRunReadAuthority, InvestigationStageRunSelector, InvestigationTimelineQuery,
};
use golish_db::repo::unified_investigation_runtime::{
    InvestigationRunHeadRow, InvestigationStageIdentity, PgUnifiedInvestigationRuntimeRepository,
    RequestInvestigationStopInput, UnifiedInvestigationRuntimeStoreError,
};
use tauri::State;
use uuid::Uuid;

use crate::state::AgentState;

use self::cursor::{
    canonical_filter_digest, canonicalize_investigation_filters, clamp_investigation_page_size,
    continue_current_cursor, issue_current_cursor, InvestigationCursorBinding,
    InvestigationCursorCurrentAuthority, InvestigationCursorFailure,
    InvestigationCursorTemporalBinding, InvestigationCursorV2, InvestigationFilterConflict,
    InvestigationFilterInput, InvestigationFilterPolicy, InvestigationStableSortKeyV1,
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
const STOP_DATABASE_MESSAGE: &str = "investigation stop is unavailable";
const UNIFIED_STAGE_TOPOLOGY: &str = "unified_investigation_v1";

static STOP_REQUEST_SERIALIZER: OnceLock<tokio::sync::Mutex<()>> = OnceLock::new();

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
const CAMPAIGN_STATES: &[&str] = &[
    "admitted",
    "running",
    "stopping",
    "draining",
    "terminal",
    "superseded",
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
    scope_snapshot_id: Uuid,
    stage_topology_contract: String,
    organization_ids: BTreeSet<Uuid>,
}

impl AuthorizedInvestigationScope {
    pub fn operation_id(&self) -> Uuid {
        self.operation_id
    }

    fn scope_snapshot_id(&self) -> Uuid {
        self.scope_snapshot_id
    }

    fn stage_topology_contract(&self) -> &str {
        &self.stage_topology_contract
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
        scope_snapshot_id: snapshot.id,
        stage_topology_contract: operation.stage_topology_contract,
        organization_ids,
    })
}

fn map_unified_head_lookup_error(
    error: UnifiedInvestigationRuntimeStoreError,
) -> InvestigationCommandError {
    match error {
        UnifiedInvestigationRuntimeStoreError::InvalidInput(_) => {
            InvestigationCommandError::invalid_argument()
        }
        UnifiedInvestigationRuntimeStoreError::IdentityConflict(_) => {
            InvestigationCommandError::new(
                InvestigationErrorCode::AuthorityCorrupt,
                AUTHORITY_CORRUPT_MESSAGE,
                None,
                false,
            )
        }
        UnifiedInvestigationRuntimeStoreError::CasConflict(_)
        | UnifiedInvestigationRuntimeStoreError::Sqlx(_) => InvestigationCommandError::new(
            InvestigationErrorCode::Database,
            STOP_DATABASE_MESSAGE,
            None,
            false,
        ),
    }
}

fn map_stop_error(
    error: UnifiedInvestigationRuntimeStoreError,
    current_change_seq: i64,
) -> InvestigationCommandError {
    match error {
        UnifiedInvestigationRuntimeStoreError::InvalidInput(_) => {
            InvestigationCommandError::invalid_argument()
        }
        UnifiedInvestigationRuntimeStoreError::IdentityConflict(_) => {
            InvestigationCommandError::forbidden()
        }
        UnifiedInvestigationRuntimeStoreError::CasConflict(_) => InvestigationCommandError::new(
            InvestigationErrorCode::ProjectionStale,
            PROJECTION_STALE_MESSAGE,
            Some(current_change_seq),
            true,
        ),
        UnifiedInvestigationRuntimeStoreError::Sqlx(sqlx::Error::Database(database_error))
            if database_error
                .message()
                .contains("INVESTIGATION_STOP_HEAD_CAS_INVALID") =>
        {
            InvestigationCommandError::new(
                InvestigationErrorCode::ProjectionStale,
                PROJECTION_STALE_MESSAGE,
                Some(current_change_seq),
                true,
            )
        }
        UnifiedInvestigationRuntimeStoreError::Sqlx(sqlx::Error::Database(database_error))
            if database_error
                .message()
                .contains("INVESTIGATION_STOP_REPLAY_MISMATCH") =>
        {
            InvestigationCommandError::invalid_argument()
        }
        UnifiedInvestigationRuntimeStoreError::Sqlx(_) => InvestigationCommandError::new(
            InvestigationErrorCode::Database,
            STOP_DATABASE_MESSAGE,
            None,
            false,
        ),
    }
}

async fn exact_run_head(
    pool: &sqlx::PgPool,
    scope: &AuthorizedInvestigationScope,
    stage_execution_id: Uuid,
    stage_run_request_id: &str,
) -> Result<InvestigationRunHeadRow, InvestigationCommandError> {
    if scope.stage_topology_contract() != UNIFIED_STAGE_TOPOLOGY {
        return Err(InvestigationCommandError::forbidden());
    }
    let repository =
        PgUnifiedInvestigationRuntimeRepository::new(std::sync::Arc::new(pool.clone()));
    let head = repository
        .load_run_head_for_stage_selector(
            scope.operation_id(),
            stage_execution_id,
            stage_run_request_id,
        )
        .await
        .map_err(map_unified_head_lookup_error)?
        .ok_or_else(InvestigationCommandError::forbidden)?;
    if head.scope_snapshot_id != scope.scope_snapshot_id() {
        return Err(InvestigationCommandError::forbidden());
    }
    Ok(head)
}

fn control_projection(
    stage_topology_contract: &str,
    head: &InvestigationRunHeadRow,
) -> InvestigationControlProjectionV1 {
    let stop_allowed = head.run_state == "running" && head.admission_open;
    let terminal = matches!(head.run_state.as_str(), "closed" | "abandoned");
    InvestigationControlProjectionV1 {
        operation_id: head.operation_id.to_string(),
        stage_execution_id: head.stage_execution_id.to_string(),
        stage_run_request_id: head.owning_stage_run_request_id.clone(),
        stage_topology_contract: stage_topology_contract.to_owned(),
        investigation_run_state: head.run_state.clone(),
        investigation_run_state_head: head.head_sha256.clone(),
        change_seq: head.change_seq,
        stop_epoch: head.stop_epoch,
        stop_allowed,
        stop_unavailable_reason: (!stop_allowed).then(|| match head.run_state.as_str() {
            "stop_pending" | "draining" => "investigation_stop_already_requested".to_owned(),
            "closed" | "abandoned" => "investigation_run_terminal".to_owned(),
            _ => "investigation_stop_not_authorized".to_owned(),
        }),
        // Unified Investigation never rewrites a stage in place. A terminal
        // run may be adopted by a successor fork through the separate frozen
        // operation contract; it is still not resettable.
        reset_allowed: false,
        reset_unavailable_reason: Some("unified_stage_reset_not_supported".to_owned()),
        successor_fork_allowed: terminal,
        successor_fork_unavailable_reason: (!terminal)
            .then(|| "investigation_run_not_terminal".to_owned()),
        adoption_contract_version: 1,
        control_policy_version: 1,
    }
}

#[allow(dead_code)]
async fn requested_control_projection(
    pool: &sqlx::PgPool,
    scope: &AuthorizedInvestigationScope,
    stage_execution_id: Option<&str>,
    stage_run_request_id: Option<&str>,
) -> Result<Option<InvestigationControlProjectionV1>, InvestigationCommandError> {
    let (Some(stage_execution_id), Some(stage_run_request_id)) =
        (stage_execution_id, stage_run_request_id)
    else {
        if stage_execution_id.is_some() || stage_run_request_id.is_some() {
            return Err(InvestigationCommandError::invalid_argument());
        }
        return Ok(None);
    };
    let stage_execution_id = parse_uuid(stage_execution_id)?;
    let head = exact_run_head(pool, scope, stage_execution_id, stage_run_request_id).await?;
    Ok(Some(control_projection(
        scope.stage_topology_contract(),
        &head,
    )))
}

async fn exact_read_selector(
    pool: &sqlx::PgPool,
    scope: &AuthorizedInvestigationScope,
    stage_execution_id: &str,
    stage_run_request_id: &str,
) -> Result<InvestigationStageRunSelector, InvestigationCommandError> {
    let stage_execution_id = parse_uuid(stage_execution_id)?;
    let head = exact_run_head(pool, scope, stage_execution_id, stage_run_request_id).await?;
    Ok(InvestigationStageRunSelector {
        stage_execution_id: head.stage_execution_id,
        stage_run_request_id: head.owning_stage_run_request_id,
        scope_snapshot_id: head.scope_snapshot_id,
    })
}

fn control_projection_from_read_authority(
    stage_topology_contract: &str,
    head: &InvestigationStageRunReadAuthority,
) -> InvestigationControlProjectionV1 {
    let stop_allowed = head.run_state == "running" && head.admission_open;
    let terminal = matches!(head.run_state.as_str(), "closed" | "abandoned");
    InvestigationControlProjectionV1 {
        operation_id: head.operation_id.to_string(),
        stage_execution_id: head.stage_execution_id.to_string(),
        stage_run_request_id: head.stage_run_request_id.clone(),
        stage_topology_contract: stage_topology_contract.to_owned(),
        investigation_run_state: head.run_state.clone(),
        investigation_run_state_head: head.head_sha256.clone(),
        change_seq: head.change_seq,
        stop_epoch: head.stop_epoch,
        stop_allowed,
        stop_unavailable_reason: (!stop_allowed).then(|| match head.run_state.as_str() {
            "stop_pending" | "draining" => "investigation_stop_already_requested".to_owned(),
            "closed" | "abandoned" => "investigation_run_terminal".to_owned(),
            _ => "investigation_stop_not_authorized".to_owned(),
        }),
        reset_allowed: false,
        reset_unavailable_reason: Some("unified_stage_reset_not_supported".to_owned()),
        successor_fork_allowed: terminal,
        successor_fork_unavailable_reason: (!terminal)
            .then(|| "investigation_run_not_terminal".to_owned()),
        adoption_contract_version: 1,
        control_policy_version: 1,
    }
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
    let selector = exact_read_selector(
        state.db_pool.as_ref(),
        &scope,
        &request.stage_execution_id,
        &request.stage_run_request_id,
    )
    .await?;
    let expected = optional_expected_snapshot(
        request.expected_change_seq,
        request.expected_temporal_cutoff.as_deref(),
        request.expected_authority_epoch_set_hash.as_deref(),
        request.expected_earliest_effective_valid_until.as_deref(),
    )?;
    let (summary, stage_run) = read_investigation_summary_for_stage_run(
        state.db_pool.as_ref(),
        scope.operation_id(),
        &selector,
        expected.as_ref(),
    )
    .await
    .map_err(map_projection_error)?;
    let control_projection =
        control_projection_from_read_authority(scope.stage_topology_contract(), &stage_run);
    let main_actor = summary
        .main_actor
        .as_ref()
        .map(actor_topology_view)
        .ok_or_else(|| {
            InvestigationCommandError::new(
                InvestigationErrorCode::AuthorityCorrupt,
                AUTHORITY_CORRUPT_MESSAGE,
                None,
                false,
            )
        })?;
    Ok(InvestigationSummaryView {
        envelope: envelope(&summary.authority, None)?,
        control_projection,
        active_generation_id: summary.active_generation_id.map(|id| id.to_string()),
        active_generation_seal_hash: summary.active_generation_seal_hash,
        current_hypothesis_count: summary.current_hypothesis_count,
        closed_hypothesis_count: summary.closed_hypothesis_count,
        contested_hypothesis_count: summary.contested_hypothesis_count,
        residual_count: summary.residual_count,
        generation_count: summary.generation_count,
        wave_count: summary.wave_count,
        campaign_count: summary.campaign_count,
        open_obligation_count: summary.open_obligation_count,
        generations: summary
            .generations
            .iter()
            .map(|generation| InvestigationGenerationSummaryView {
                generation_id: Some(generation.generation_id.to_string()),
                generation_ordinal: generation.generation_ordinal,
                state: generation.state.clone(),
            })
            .collect(),
        waves: summary
            .waves
            .iter()
            .map(|wave| InvestigationWaveSummaryView {
                wave_id: Some(wave.wave_id.to_string()),
                wave_ordinal: wave.wave_ordinal,
                state: wave.state.clone(),
            })
            .collect(),
        open_obligations: summary
            .open_obligations
            .iter()
            .map(|obligation| InvestigationOpenObligationSummaryView {
                obligation_id: obligation.obligation_id.clone(),
                obligation_kind: obligation.obligation_kind.clone(),
            })
            .collect(),
        source_census: summary
            .source_census
            .iter()
            .map(|member| InvestigationSourceCensusMemberView {
                organization_id: member.organization_id.to_string(),
                snapshot_id: member.snapshot_id.to_string(),
                context_item_count: member.context_item_count,
                context_item_set_sha256: member.context_item_set_sha256.clone(),
                methodology_hit_count: member.methodology_hit_count,
                methodology_result_set_sha256: member.methodology_result_set_sha256.clone(),
                omission_count: member.omission_count,
                omission_set_sha256: member.omission_set_sha256.clone(),
            })
            .collect(),
        main_actor,
        actor_topology: summary
            .actor_topology
            .iter()
            .map(actor_topology_view)
            .collect(),
        coverage_denominator: InvestigationCoverageDenominatorView {
            planned: summary.coverage_denominator.planned,
            tested_complete: summary.coverage_denominator.tested_complete,
            tested_degraded: summary.coverage_denominator.tested_degraded,
            untested: summary.coverage_denominator.untested,
            blocked: summary.coverage_denominator.blocked,
        },
        coverage_sufficiency: summary.coverage_sufficiency,
        authority_time_members: vec![authority_time_view(&summary.authority)],
        control_decision: summary.control_decision,
        coverage_grade: summary.coverage_grade,
    })
}

/// Stage-level stop mutation. It is deliberately separate from every
/// Hypothesis/Campaign selector: the caller submits only the exact server
/// projected run head, while PostgreSQL freezes the complete open-work set.
pub async fn request_investigation_stop_authorized(
    pool: &sqlx::PgPool,
    principal_provider: &dyn TrustedOperatorPrincipalProvider,
    ai_state: &crate::state::AiState,
    request: InvestigationRequestStopRequest,
) -> Result<InvestigationRequestStopResponse, InvestigationCommandError> {
    let operation_id = parse_uuid(&request.operation_id)?;
    let scope = authorize_investigation_scope(
        pool,
        principal_provider,
        ai_state,
        &request.session_id,
        operation_id,
    )
    .await?;
    let stage_execution_id = parse_uuid(&request.stage_execution_id)?;
    let idempotency_key = parse_uuid(&request.idempotency_key)?;
    if request.expected_change_seq < 0
        || !is_tagged_sha256(&request.expected_investigation_run_state_head)
        || request.stage_run_request_id.trim().is_empty()
        || request.stage_run_request_id.len() > 512
    {
        return Err(InvestigationCommandError::invalid_argument());
    }

    // Serializing local-operator stop requests makes the refresh hint at-most
    // once in one process. A crash after commit may lose the hint by design;
    // cold restore reads the DB authority and never depends on event delivery.
    let serializer = STOP_REQUEST_SERIALIZER.get_or_init(|| tokio::sync::Mutex::new(()));
    let _guard = serializer.lock().await;
    let before = exact_run_head(
        pool,
        &scope,
        stage_execution_id,
        &request.stage_run_request_id,
    )
    .await?;
    let should_emit = before.run_state == "running"
        && before.admission_open
        && before.change_seq == request.expected_change_seq
        && before.head_sha256 == request.expected_investigation_run_state_head;
    let stop_intent_id = Uuid::new_v5(&idempotency_key, b"investigation-stop-intent.v1");
    let repository =
        PgUnifiedInvestigationRuntimeRepository::new(std::sync::Arc::new(pool.clone()));
    let receipt = repository
        .request_stop(&RequestInvestigationStopInput {
            identity: InvestigationStageIdentity {
                authority_id: before.authority_id,
                operation_id,
                stage_execution_id,
                owning_stage_run_request_id: request.stage_run_request_id.clone(),
                scope_snapshot_id: scope.scope_snapshot_id(),
            },
            stop_intent_id,
            idempotency_key,
            expected_run_head_sha256: request.expected_investigation_run_state_head,
            expected_change_seq: u64::try_from(request.expected_change_seq)
                .map_err(|_| InvestigationCommandError::invalid_argument())?,
        })
        .await
        .map_err(|error| map_stop_error(error, before.change_seq))?;
    let after = exact_run_head(
        pool,
        &scope,
        stage_execution_id,
        &request.stage_run_request_id,
    )
    .await?;
    if after.authority_id != receipt.authority_id
        || after.stop_epoch != receipt.stop_epoch
        || after.change_seq <= receipt.expected_change_seq
        || after.admission_open
        || !matches!(
            after.run_state.as_str(),
            "stop_pending" | "draining" | "closed"
        )
    {
        return Err(InvestigationCommandError::new(
            InvestigationErrorCode::AuthorityCorrupt,
            AUTHORITY_CORRUPT_MESSAGE,
            None,
            false,
        ));
    }
    let control_projection = control_projection(scope.stage_topology_contract(), &after);
    if should_emit {
        if let Some(bridge) = ai_state.get_session_bridge(&request.session_id).await {
            bridge.emit_event(
                golish_core::events::AiEvent::InvestigationProjectionChanged {
                    operation_id: operation_id.to_string(),
                    stage_execution_id: stage_execution_id.to_string(),
                    stage_run_request_id: request.stage_run_request_id.clone(),
                    change_seq: after.change_seq,
                },
            );
        }
    }
    Ok(InvestigationRequestStopResponse {
        stop_intent_id: receipt.stop_intent_id.to_string(),
        idempotency_key: receipt.idempotency_key.to_string(),
        stop_epoch: receipt.stop_epoch,
        frozen_work_count: receipt.frozen_work_count,
        frozen_work_set_sha256: receipt.frozen_work_set_sha256,
        receipt_sha256: receipt.receipt_sha256,
        control_projection,
    })
}

#[tauri::command]
pub async fn investigation_request_stop(
    request: InvestigationRequestStopRequest,
    state: State<'_, AgentState>,
) -> Result<InvestigationRequestStopResponse, InvestigationCommandError> {
    request_investigation_stop_authorized(
        state.db_pool.as_ref(),
        state.operator_principal_provider.as_ref(),
        &state.ai_state,
        request,
    )
    .await
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
    let selector = exact_read_selector(
        state.db_pool.as_ref(),
        &scope,
        &request.stage_execution_id,
        &request.stage_run_request_id,
    )
    .await?;
    let expected = optional_expected_snapshot(
        request.expected_change_seq,
        request.expected_temporal_cutoff.as_deref(),
        request.expected_authority_epoch_set_hash.as_deref(),
        request.expected_earliest_effective_valid_until.as_deref(),
    )?;
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
    let filter_digest = canonical_filter_digest(&(
        request.stage_execution_id.as_str(),
        request.stage_run_request_id.as_str(),
        filters.digest(),
    ));
    let page_size = clamp_investigation_page_size(request.page_size);

    let (after, expected_page_authority) = if let Some(token) = request.cursor.as_deref() {
        let (captured, _) = capture_investigation_read_authority_for_stage_run(
            state.db_pool.as_ref(),
            operation_id,
            &selector,
        )
        .await
        .map_err(map_projection_error)?;
        let binding =
            resource_cursor_binding("hypotheses", &captured.operation, &filter_digest, page_size);
        let current = InvestigationCursorCurrentAuthority {
            current_change_seq: captured.temporal.as_of_change_seq,
            db_now: captured.temporal.as_of_temporal_cutoff,
            current_authority_epoch_set_hash: &captured.temporal.authority_epoch_set_hash,
        };
        let cursor =
            continue_current_cursor(token, &captured.operation.cursor_salt, &binding, &current)
                .map_err(InvestigationCommandError::cursor_failure)?;
        ensure_cursor_matches_expected_snapshot(
            expected.as_ref(),
            &cursor,
            captured.temporal.as_of_change_seq,
        )?;
        let after = hypothesis_sort_key(&cursor.stable_sort_key)?;
        let expected = InvestigationPageValidationInput {
            as_of_change_seq: cursor.as_of_change_seq,
            as_of_temporal_cutoff: cursor.as_of_temporal_cutoff,
            authority_epoch_set_hash: cursor.authority_epoch_set_hash,
            earliest_effective_valid_until: cursor.earliest_effective_valid_until,
        };
        (Some(after), Some(expected))
    } else {
        (None, expected.clone())
    };

    let page = list_investigation_hypotheses_for_stage_run(
        state.db_pool.as_ref(),
        operation_id,
        &selector,
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
    let selector = exact_read_selector(
        state.db_pool.as_ref(),
        &scope,
        &request.stage_execution_id,
        &request.stage_run_request_id,
    )
    .await?;
    let expected = exact_expected_snapshot(
        request.expected_change_seq,
        &request.expected_temporal_cutoff,
        &request.expected_authority_epoch_set_hash,
        &request.expected_earliest_effective_valid_until,
    )?;
    // Parse and resolve the selector only after operation authorization.
    let revision_id = parse_uuid(&request.revision_id)?;
    let detail = get_investigation_hypothesis_for_stage_run(
        state.db_pool.as_ref(),
        scope.operation_id(),
        &selector,
        revision_id,
        &expected,
    )
    .await
    .map_err(map_projection_error)?
    .ok_or_else(InvestigationCommandError::forbidden)?;
    scope.authorize_organization_selectors(&[detail.hypothesis.organization_id])?;
    detail_view(&detail)
}

#[tauri::command]
pub async fn investigation_list_campaigns(
    request: InvestigationCampaignListRequest,
    state: State<'_, AgentState>,
) -> Result<InvestigationCampaignPageResponse, InvestigationCommandError> {
    let operation_id = parse_uuid(&request.operation_id)?;
    let scope = authorize_investigation_scope(
        state.db_pool.as_ref(),
        state.operator_principal_provider.as_ref(),
        &state.ai_state,
        &request.session_id,
        operation_id,
    )
    .await?;
    let selector = exact_read_selector(
        state.db_pool.as_ref(),
        &scope,
        &request.stage_execution_id,
        &request.stage_run_request_id,
    )
    .await?;
    let expected = optional_expected_snapshot(
        request.expected_change_seq,
        request.expected_temporal_cutoff.as_deref(),
        request.expected_authority_epoch_set_hash.as_deref(),
        request.expected_earliest_effective_valid_until.as_deref(),
    )?;
    let wave_ids = request
        .wave_ids
        .iter()
        .map(|value| parse_uuid(value))
        .collect::<Result<BTreeSet<_>, _>>()?
        .into_iter()
        .collect::<Vec<_>>();
    let campaign_states = canonical_closed_values(&request.campaign_states, CAMPAIGN_STATES)?;
    let filter_digest = canonical_filter_digest(&(
        request.stage_execution_id.as_str(),
        request.stage_run_request_id.as_str(),
        wave_ids.as_slice(),
        campaign_states.as_slice(),
    ));
    let page_size = clamp_investigation_page_size(request.page_size.unwrap_or(50));
    let (after, expected_page_authority) = if let Some(token) = request.cursor.as_deref() {
        let (captured, _) = capture_investigation_read_authority_for_stage_run(
            state.db_pool.as_ref(),
            operation_id,
            &selector,
        )
        .await
        .map_err(map_projection_error)?;
        let binding =
            resource_cursor_binding("campaigns", &captured.operation, &filter_digest, page_size);
        let current = InvestigationCursorCurrentAuthority {
            current_change_seq: captured.temporal.as_of_change_seq,
            db_now: captured.temporal.as_of_temporal_cutoff,
            current_authority_epoch_set_hash: &captured.temporal.authority_epoch_set_hash,
        };
        let cursor =
            continue_current_cursor(token, &captured.operation.cursor_salt, &binding, &current)
                .map_err(InvestigationCommandError::cursor_failure)?;
        ensure_cursor_matches_expected_snapshot(
            expected.as_ref(),
            &cursor,
            captured.temporal.as_of_change_seq,
        )?;
        let after = campaign_sort_key(&cursor.stable_sort_key)?;
        let expected = cursor_page_authority(&cursor);
        (Some(after), Some(expected))
    } else {
        (None, expected.clone())
    };
    let page = list_investigation_campaigns_for_stage_run(
        state.db_pool.as_ref(),
        scope.operation_id(),
        &selector,
        InvestigationCampaignListQuery {
            filters: InvestigationCampaignFilters {
                wave_ids,
                campaign_states,
            },
            after,
            expected_page_authority,
            page_size,
        },
    )
    .await
    .map_err(map_projection_error)?;
    reject_page_expected_change_seq(request.expected_change_seq, &page.authority)?;
    let next_cursor = page
        .next_key
        .as_ref()
        .map(|key| issue_campaign_cursor(&page.authority, &filter_digest, page_size, key))
        .transpose()?;
    Ok(InvestigationCampaignPageResponse {
        envelope: envelope(&page.authority, next_cursor)?,
        campaigns: page
            .campaigns
            .iter()
            .map(|campaign| campaign_list_item_view(campaign, &page.authority))
            .collect(),
    })
}

#[tauri::command]
pub async fn investigation_get_campaign(
    request: InvestigationCampaignDetailRequest,
    state: State<'_, AgentState>,
) -> Result<InvestigationCampaignDetailResponse, InvestigationCommandError> {
    let operation_id = parse_uuid(&request.operation_id)?;
    let scope = authorize_investigation_scope(
        state.db_pool.as_ref(),
        state.operator_principal_provider.as_ref(),
        &state.ai_state,
        &request.session_id,
        operation_id,
    )
    .await?;
    let selector = exact_read_selector(
        state.db_pool.as_ref(),
        &scope,
        &request.stage_execution_id,
        &request.stage_run_request_id,
    )
    .await?;
    let expected = exact_expected_snapshot(
        request.expected_change_seq,
        &request.expected_temporal_cutoff,
        &request.expected_authority_epoch_set_hash,
        &request.expected_earliest_effective_valid_until,
    )?;
    let campaign_id = parse_uuid(&request.campaign_id)?;
    let detail = get_investigation_campaign_for_stage_run(
        state.db_pool.as_ref(),
        scope.operation_id(),
        &selector,
        campaign_id,
        &expected,
    )
    .await
    .map_err(map_projection_error)?
    .ok_or_else(InvestigationCommandError::forbidden)?;
    scope.authorize_organization_selectors(&[detail.organization_id])?;
    campaign_detail_response(&detail)
}

#[tauri::command]
pub async fn investigation_list_timeline(
    request: InvestigationTimelineListRequest,
    state: State<'_, AgentState>,
) -> Result<InvestigationTimelinePageResponse, InvestigationCommandError> {
    let operation_id = parse_uuid(&request.operation_id)?;
    let scope = authorize_investigation_scope(
        state.db_pool.as_ref(),
        state.operator_principal_provider.as_ref(),
        &state.ai_state,
        &request.session_id,
        operation_id,
    )
    .await?;
    let selector = exact_read_selector(
        state.db_pool.as_ref(),
        &scope,
        &request.stage_execution_id,
        &request.stage_run_request_id,
    )
    .await?;
    let expected = optional_expected_snapshot(
        request.expected_change_seq,
        request.expected_temporal_cutoff.as_deref(),
        request.expected_authority_epoch_set_hash.as_deref(),
        request.expected_earliest_effective_valid_until.as_deref(),
    )?;
    let mut event_kinds = request.event_kinds.clone();
    event_kinds.sort_by_key(|kind| kind.as_str());
    event_kinds.dedup();
    let filter_digest = canonical_filter_digest(&(
        request.stage_execution_id.as_str(),
        request.stage_run_request_id.as_str(),
        event_kinds
            .iter()
            .map(|kind| kind.as_str())
            .collect::<Vec<_>>(),
    ));
    let page_size = clamp_investigation_page_size(request.page_size.unwrap_or(50));
    let (after, expected_page_authority) = if let Some(token) = request.cursor.as_deref() {
        let (captured, _) = capture_investigation_read_authority_for_stage_run(
            state.db_pool.as_ref(),
            operation_id,
            &selector,
        )
        .await
        .map_err(map_projection_error)?;
        let binding =
            resource_cursor_binding("timeline", &captured.operation, &filter_digest, page_size);
        let current = InvestigationCursorCurrentAuthority {
            current_change_seq: captured.temporal.as_of_change_seq,
            db_now: captured.temporal.as_of_temporal_cutoff,
            current_authority_epoch_set_hash: &captured.temporal.authority_epoch_set_hash,
        };
        let cursor =
            continue_current_cursor(token, &captured.operation.cursor_salt, &binding, &current)
                .map_err(InvestigationCommandError::cursor_failure)?;
        ensure_cursor_matches_expected_snapshot(
            expected.as_ref(),
            &cursor,
            captured.temporal.as_of_change_seq,
        )?;
        let after = timeline_sort_key(&cursor.stable_sort_key)?;
        let expected = cursor_page_authority(&cursor);
        (Some(after), Some(expected))
    } else {
        (None, expected.clone())
    };
    let page = read_investigation_timeline_for_stage_run(
        state.db_pool.as_ref(),
        scope.operation_id(),
        &selector,
        InvestigationTimelineQuery {
            event_kinds,
            after,
            expected_page_authority,
            page_size,
        },
    )
    .await
    .map_err(map_projection_error)?;
    reject_page_expected_change_seq(request.expected_change_seq, &page.authority)?;
    let next_cursor = page
        .next_key
        .as_ref()
        .map(|key| issue_timeline_cursor(&page.authority, &filter_digest, page_size, *key))
        .transpose()?;
    let authority_time = authority_time_view(&page.authority);
    Ok(InvestigationTimelinePageResponse {
        envelope: envelope(&page.authority, next_cursor)?,
        events: page
            .events
            .into_iter()
            .map(|event| InvestigationTimelineItemView {
                event_id: event.event_id.to_string(),
                change_seq: event.change_seq,
                event_kind: event.event_kind,
                entity_kind: event.entity.kind,
                entity_id: event.entity.entity_id,
                entity_version: event.entity.entity_version,
                source_occurred_at: event.source_occurred_at.map(|value| value.to_rfc3339()),
                source_time_status: event.source_time_status,
                projected_at: event.projected_at.to_rfc3339(),
                invalidation_reason: event.invalidation_reason,
                authority_time: authority_time.clone(),
            })
            .collect(),
    })
}

fn parse_uuid(value: &str) -> Result<Uuid, InvestigationCommandError> {
    Uuid::parse_str(value).map_err(|_| InvestigationCommandError::invalid_id())
}

fn optional_expected_snapshot(
    expected_change_seq: Option<i64>,
    expected_temporal_cutoff: Option<&str>,
    expected_authority_epoch_set_hash: Option<&str>,
    expected_earliest_effective_valid_until: Option<&str>,
) -> Result<Option<InvestigationPageValidationInput>, InvestigationCommandError> {
    let values_present = [
        expected_change_seq.is_some(),
        expected_temporal_cutoff.is_some(),
        expected_authority_epoch_set_hash.is_some(),
        expected_earliest_effective_valid_until.is_some(),
    ];
    if values_present.iter().all(|present| !present) {
        return Ok(None);
    }
    if !values_present.iter().all(|present| *present) {
        return Err(InvestigationCommandError::invalid_argument());
    }
    let expected_change_seq = expected_change_seq
        .filter(|value| *value >= 0)
        .ok_or_else(InvestigationCommandError::invalid_argument)?;
    let parse_time = |value: &str| {
        chrono::DateTime::parse_from_rfc3339(value)
            .map(|value| value.with_timezone(&chrono::Utc))
            .map_err(|_| InvestigationCommandError::invalid_argument())
    };
    let expected_temporal_cutoff =
        parse_time(expected_temporal_cutoff.expect("all optional snapshot fields are present"))?;
    let authority_epoch_set_hash = expected_authority_epoch_set_hash
        .expect("all optional snapshot fields are present")
        .to_owned();
    let earliest_effective_valid_until = parse_time(
        expected_earliest_effective_valid_until.expect("all optional snapshot fields are present"),
    )?;
    if !is_tagged_sha256(&authority_epoch_set_hash)
        || expected_temporal_cutoff > earliest_effective_valid_until
    {
        return Err(InvestigationCommandError::invalid_argument());
    }
    Ok(Some(InvestigationPageValidationInput {
        as_of_change_seq: expected_change_seq,
        as_of_temporal_cutoff: expected_temporal_cutoff,
        authority_epoch_set_hash,
        earliest_effective_valid_until,
    }))
}

fn exact_expected_snapshot(
    expected_change_seq: i64,
    expected_temporal_cutoff: &str,
    expected_authority_epoch_set_hash: &str,
    expected_earliest_effective_valid_until: &str,
) -> Result<InvestigationPageValidationInput, InvestigationCommandError> {
    optional_expected_snapshot(
        Some(expected_change_seq),
        Some(expected_temporal_cutoff),
        Some(expected_authority_epoch_set_hash),
        Some(expected_earliest_effective_valid_until),
    )?
    .ok_or_else(InvestigationCommandError::invalid_argument)
}

fn ensure_cursor_matches_expected_snapshot(
    expected: Option<&InvestigationPageValidationInput>,
    cursor: &InvestigationCursorV2,
    current_change_seq: i64,
) -> Result<(), InvestigationCommandError> {
    let Some(expected) = expected else {
        return Err(InvestigationCommandError::invalid_argument());
    };
    if expected.as_of_change_seq != cursor.as_of_change_seq
        || expected.as_of_temporal_cutoff != cursor.as_of_temporal_cutoff
        || expected.authority_epoch_set_hash != cursor.authority_epoch_set_hash
        || expected.earliest_effective_valid_until != cursor.earliest_effective_valid_until
    {
        return Err(InvestigationCommandError::new(
            InvestigationErrorCode::ProjectionStale,
            PROJECTION_STALE_MESSAGE,
            Some(current_change_seq),
            true,
        ));
    }
    Ok(())
}

fn is_tagged_sha256(value: &str) -> bool {
    value.strip_prefix("sha256:").is_some_and(|hex| {
        hex.len() == 64
            && hex
                .bytes()
                .all(|byte| byte.is_ascii_hexdigit() && !byte.is_ascii_uppercase())
    })
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

fn resource_cursor_binding<'a>(
    resource_kind: &'a str,
    operation: &'a InvestigationOperationReadAuthority,
    filter_digest: &'a str,
    page_size: u32,
) -> InvestigationCursorBinding<'a> {
    InvestigationCursorBinding {
        resource_kind,
        operation_id: operation.operation_id,
        tool_truth_contract: &operation.tool_truth_contract,
        investigation_contract_version: &operation.investigation_contract_version,
        investigation_rollout_mode: &operation.investigation_rollout_mode,
        filter_digest,
        page_size,
        expected_temporal: None,
    }
}

fn canonical_closed_values(
    values: &[String],
    allowed: &[&str],
) -> Result<Vec<String>, InvestigationCommandError> {
    let canonical = values.iter().cloned().collect::<BTreeSet<_>>();
    if canonical
        .iter()
        .any(|value| !allowed.contains(&value.as_str()))
    {
        return Err(InvestigationCommandError::invalid_argument());
    }
    Ok(canonical.into_iter().collect())
}

fn cursor_page_authority(cursor: &InvestigationCursorV2) -> InvestigationPageValidationInput {
    InvestigationPageValidationInput {
        as_of_change_seq: cursor.as_of_change_seq,
        as_of_temporal_cutoff: cursor.as_of_temporal_cutoff,
        authority_epoch_set_hash: cursor.authority_epoch_set_hash.clone(),
        earliest_effective_valid_until: cursor.earliest_effective_valid_until,
    }
}

#[allow(dead_code)]
fn reject_expected_change_seq(
    expected: Option<i64>,
    current: &InvestigationReadAuthority,
    cursor_change_seq: i64,
) -> Result<(), InvestigationCommandError> {
    if expected.is_some_and(|value| value != cursor_change_seq) {
        return Err(InvestigationCommandError::new(
            InvestigationErrorCode::ProjectionStale,
            PROJECTION_STALE_MESSAGE,
            Some(current.temporal.as_of_change_seq),
            true,
        ));
    }
    Ok(())
}

fn reject_page_expected_change_seq(
    expected: Option<i64>,
    authority: &InvestigationReadAuthority,
) -> Result<(), InvestigationCommandError> {
    if expected.is_some_and(|value| value != authority.temporal.as_of_change_seq) {
        return Err(InvestigationCommandError::new(
            InvestigationErrorCode::ProjectionStale,
            PROJECTION_STALE_MESSAGE,
            Some(authority.temporal.as_of_change_seq),
            true,
        ));
    }
    Ok(())
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

fn campaign_sort_key(
    key: &InvestigationStableSortKeyV1,
) -> Result<InvestigationCampaignSortKey, InvestigationCommandError> {
    let InvestigationStableSortKeyV1::Campaign {
        wave_ordinal,
        campaign_ordinal,
        campaign_id,
    } = key
    else {
        return Err(InvestigationCommandError::cursor_failure(
            InvestigationCursorFailure::Invalid,
        ));
    };
    Ok(InvestigationCampaignSortKey {
        wave_ordinal: *wave_ordinal,
        campaign_ordinal: *campaign_ordinal,
        campaign_id: *campaign_id,
    })
}

fn timeline_sort_key(
    key: &InvestigationStableSortKeyV1,
) -> Result<(i64, Uuid), InvestigationCommandError> {
    let InvestigationStableSortKeyV1::Timeline {
        change_seq,
        event_id,
    } = key
    else {
        return Err(InvestigationCommandError::cursor_failure(
            InvestigationCursorFailure::Invalid,
        ));
    };
    Ok((*change_seq, *event_id))
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

fn issue_campaign_cursor(
    authority: &InvestigationReadAuthority,
    filter_digest: &str,
    page_size: u32,
    key: &InvestigationCampaignSortKey,
) -> Result<String, InvestigationCommandError> {
    issue_resource_cursor(
        authority,
        "campaigns",
        filter_digest,
        page_size,
        InvestigationStableSortKeyV1::Campaign {
            wave_ordinal: key.wave_ordinal,
            campaign_ordinal: key.campaign_ordinal,
            campaign_id: key.campaign_id,
        },
    )
}

fn issue_timeline_cursor(
    authority: &InvestigationReadAuthority,
    filter_digest: &str,
    page_size: u32,
    key: (i64, Uuid),
) -> Result<String, InvestigationCommandError> {
    issue_resource_cursor(
        authority,
        "timeline",
        filter_digest,
        page_size,
        InvestigationStableSortKeyV1::Timeline {
            change_seq: key.0,
            event_id: key.1,
        },
    )
}

fn issue_resource_cursor(
    authority: &InvestigationReadAuthority,
    resource_kind: &str,
    filter_digest: &str,
    page_size: u32,
    stable_sort_key: InvestigationStableSortKeyV1,
) -> Result<String, InvestigationCommandError> {
    let cursor = InvestigationCursorV2::new(
        resource_kind,
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
        stable_sort_key,
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

fn authority_time_view(authority: &InvestigationReadAuthority) -> InvestigationAuthorityTimeViewV1 {
    InvestigationAuthorityTimeViewV1 {
        observed_as_of: authority.temporal.as_of_temporal_cutoff.to_rfc3339(),
        effective_valid_until: Some(
            authority
                .temporal
                .earliest_effective_valid_until
                .to_rfc3339(),
        ),
        authority_epoch_hash: authority.temporal.authority_epoch_set_hash.clone(),
        temporal_status: "current".to_owned(),
    }
}

fn campaign_list_item_view(
    item: &InvestigationCampaignListItem,
    authority: &InvestigationReadAuthority,
) -> InvestigationCampaignListItemView {
    InvestigationCampaignListItemView {
        campaign_id: item.campaign_id.to_string(),
        wave_ordinal: item.sort_key.wave_ordinal,
        campaign_ordinal: item.sort_key.campaign_ordinal,
        label: item.label.clone(),
        state: item.state.clone(),
        coverage_status: item.coverage_status.clone(),
        authority_time: authority_time_view(authority),
    }
}

fn campaign_detail_response(
    detail: &InvestigationCampaignDetail,
) -> Result<InvestigationCampaignDetailResponse, InvestigationCommandError> {
    Ok(InvestigationCampaignDetailResponse {
        envelope: envelope(&detail.authority, None)?,
        campaign: InvestigationCampaignDetailView {
            campaign_id: detail.campaign.campaign_id.to_string(),
            hypothesis_revision_id: detail.campaign.hypothesis_revision_id.to_string(),
            wave_ordinal: detail.campaign.sort_key.wave_ordinal,
            campaign_ordinal: detail.campaign.sort_key.campaign_ordinal,
            state: detail.campaign.state.clone(),
            coverage_status: detail.campaign.coverage_status.clone(),
            round_ids: detail.round_ids.iter().map(Uuid::to_string).collect(),
            prepared_action_ids: detail
                .prepared_action_ids
                .iter()
                .map(Uuid::to_string)
                .collect(),
            authorized_action_count: detail.authorized_action_count,
            blocked_action_count: detail.blocked_action_count,
            open_residual_ids: detail
                .open_residual_ids
                .iter()
                .map(Uuid::to_string)
                .collect(),
            redacted_round_summaries: detail.redacted_round_summaries.clone(),
            authority_time: authority_time_view(&detail.authority),
        },
    })
}

fn actor_topology_view(
    actor: &golish_db::repo::investigation_projection::InvestigationActorTopologyNode,
) -> InvestigationActorTopologyNodeView {
    InvestigationActorTopologyNodeView {
        actor_kind: actor.actor_kind.clone(),
        organization_id: actor.organization_id.to_string(),
        hypothesis_revision_id: actor.hypothesis_revision_id.map(|id| id.to_string()),
        task_id: actor.task_id.map(|id| id.to_string()),
        subtask_id: actor.subtask_id.map(|id| id.to_string()),
        worker_run_id: actor.worker_run_id.to_string(),
        owning_stage_run_request_id: actor.owning_stage_run_request_id.clone(),
        transcript_request_id: actor.transcript_request_id.clone(),
        parent_actor_transcript_request_id: actor.parent_actor_transcript_request_id.clone(),
        parent_dispatch_tool_request_id: actor.parent_dispatch_tool_request_id.clone(),
        status: actor.status.clone(),
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
        actor_topology: detail
            .actor_topology
            .iter()
            .map(actor_topology_view)
            .collect(),
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
