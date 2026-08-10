//! Pure Plan C scheduler and operation-frozen cutover boundary.
//!
//! This module owns no provider, transport, credential, executor, lease, or
//! database capability.  It selects the already-frozen Campaign route, asks
//! the narrow repository for durable admission, and evaluates deterministic
//! Campaign/Wave/stage closure snapshots.

use golish_core::investigation_comparison::WholeRecordComparisonStateV1;

use crate::db_traits::{
    AdmitCampaignRequest, CampaignLease, OpenShadowEvaluation, ShadowEvaluation,
    VerificationCampaignRepository, VerificationCampaignRepositoryError,
    VerificationCampaignShadowRepository,
};
use crate::harness::hypothesis_registry::{
    select_campaign_route, CampaignRoute, PersistedOperationContractSnapshot,
};
use crate::harness::verification_campaign::{
    validate_campaign_gate, ArtifactAuthorityV1, CampaignGateSnapshotV1, CampaignPhaseV1,
    CampaignStopReasonV1,
};

#[derive(Debug)]
pub struct VerificationScheduleRequestV1 {
    /// Shadow evaluation is sequenced strictly after terminal legacy truth.
    pub legacy_terminal: bool,
    /// Present only on rank 5/6. The repository still derives all Tool Truth
    /// roots and freshness authority under its own guarded transaction.
    pub canonical_admission: Option<AdmitCampaignRequest>,
    /// Immutable redacted snapshot selector for rank 2..4 only.
    pub shadow_evaluation: Option<OpenShadowEvaluation>,
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub enum VerificationScheduleDecisionV1 {
    LegacyAuthority,
    LegacyAuthorityShadowPending,
    ShadowEvaluationOpened(ShadowEvaluation),
    CanonicalCampaignAdmitted(CampaignLease),
}

impl VerificationScheduleDecisionV1 {
    /// The only scheduler result that can be handed to the canonical action
    /// dispatcher is a committed repository admission receipt.
    pub const fn allows_canonical_dispatch(&self) -> bool {
        matches!(self, Self::CanonicalCampaignAdmitted(_))
    }
}

#[derive(Debug, Clone, PartialEq, Eq, thiserror::Error)]
pub enum VerificationCampaignSchedulerError {
    #[error("VERIFICATION_CAMPAIGN_SCHEDULER_OPERATION_CONTRACT_INVALID: {detail}")]
    OperationContractInvalid { detail: String },
    #[error("VERIFICATION_CAMPAIGN_SCHEDULER_CANONICAL_ADMISSION_REQUIRED")]
    CanonicalAdmissionRequired,
    #[error("VERIFICATION_CAMPAIGN_SCHEDULER_SHADOW_SNAPSHOT_REQUIRED")]
    ShadowSnapshotRequired,
    #[error("VERIFICATION_CAMPAIGN_SCHEDULER_REPOSITORY_UNAVAILABLE")]
    RepositoryUnavailable,
    #[error("VERIFICATION_CAMPAIGN_SCHEDULER_REPOSITORY_FAILURE: {repository_code}")]
    RepositoryFailure { repository_code: &'static str },
    #[error("VERIFICATION_CAMPAIGN_SCHEDULER_CAMPAIGN_CENSUS_MISMATCH")]
    CampaignCensusMismatch,
    #[error("VERIFICATION_CAMPAIGN_SCHEDULER_CAMPAIGN_GATE_BLOCKED: {gate_code}")]
    CampaignGateBlocked { gate_code: &'static str },
    #[error("VERIFICATION_CAMPAIGN_SCHEDULER_CAMPAIGN_NONTERMINAL")]
    CampaignNonterminal,
    #[error("VERIFICATION_CAMPAIGN_SCHEDULER_OBJECTIVE_OUTCOME_CENSUS_MISMATCH")]
    ObjectiveOutcomeCensusMismatch,
    #[error("VERIFICATION_CAMPAIGN_SCHEDULER_OPEN_WORK_CENSUS_MISMATCH")]
    OpenWorkCensusMismatch,
    #[error("VERIFICATION_CAMPAIGN_SCHEDULER_FIXED_POINT_MISMATCH")]
    FixedPointMismatch,
    #[error("VERIFICATION_CAMPAIGN_SCHEDULER_FINAL_SEAL_MISMATCH")]
    FinalSealMismatch,
    #[error("VERIFICATION_CAMPAIGN_SCHEDULER_NEW_GENERATION_WITHOUT_RUNNABLE_WORK")]
    NewGenerationWithoutRunnableWork,
    #[error("VERIFICATION_CAMPAIGN_SCHEDULER_STALL_POLICY_INVALID")]
    StallPolicyInvalid,
}

impl VerificationCampaignSchedulerError {
    pub const fn code(&self) -> &'static str {
        match self {
            Self::OperationContractInvalid { .. } => {
                "VERIFICATION_CAMPAIGN_SCHEDULER_OPERATION_CONTRACT_INVALID"
            }
            Self::CanonicalAdmissionRequired => {
                "VERIFICATION_CAMPAIGN_SCHEDULER_CANONICAL_ADMISSION_REQUIRED"
            }
            Self::ShadowSnapshotRequired => {
                "VERIFICATION_CAMPAIGN_SCHEDULER_SHADOW_SNAPSHOT_REQUIRED"
            }
            Self::RepositoryUnavailable => "VERIFICATION_CAMPAIGN_SCHEDULER_REPOSITORY_UNAVAILABLE",
            Self::RepositoryFailure { .. } => "VERIFICATION_CAMPAIGN_SCHEDULER_REPOSITORY_FAILURE",
            Self::CampaignCensusMismatch => {
                "VERIFICATION_CAMPAIGN_SCHEDULER_CAMPAIGN_CENSUS_MISMATCH"
            }
            Self::CampaignGateBlocked { .. } => {
                "VERIFICATION_CAMPAIGN_SCHEDULER_CAMPAIGN_GATE_BLOCKED"
            }
            Self::CampaignNonterminal => "VERIFICATION_CAMPAIGN_SCHEDULER_CAMPAIGN_NONTERMINAL",
            Self::ObjectiveOutcomeCensusMismatch => {
                "VERIFICATION_CAMPAIGN_SCHEDULER_OBJECTIVE_OUTCOME_CENSUS_MISMATCH"
            }
            Self::OpenWorkCensusMismatch => {
                "VERIFICATION_CAMPAIGN_SCHEDULER_OPEN_WORK_CENSUS_MISMATCH"
            }
            Self::FixedPointMismatch => "VERIFICATION_CAMPAIGN_SCHEDULER_FIXED_POINT_MISMATCH",
            Self::FinalSealMismatch => "VERIFICATION_CAMPAIGN_SCHEDULER_FINAL_SEAL_MISMATCH",
            Self::NewGenerationWithoutRunnableWork => {
                "VERIFICATION_CAMPAIGN_SCHEDULER_NEW_GENERATION_WITHOUT_RUNNABLE_WORK"
            }
            Self::StallPolicyInvalid => "VERIFICATION_CAMPAIGN_SCHEDULER_STALL_POLICY_INVALID",
        }
    }
}

fn map_repository_error(
    error: VerificationCampaignRepositoryError,
) -> VerificationCampaignSchedulerError {
    match error {
        VerificationCampaignRepositoryError::Unavailable { .. } => {
            VerificationCampaignSchedulerError::RepositoryUnavailable
        }
        other => VerificationCampaignSchedulerError::RepositoryFailure {
            repository_code: other.code(),
        },
    }
}

/// Production entrypoint. Route selection is delegated entirely to the Plan B
/// operation-frozen snapshot and its single `select_campaign_route` policy.
pub async fn schedule_verification_campaign(
    operation_contract: &PersistedOperationContractSnapshot,
    canonical_repository: Option<&dyn VerificationCampaignRepository>,
    shadow_repository: Option<&dyn VerificationCampaignShadowRepository>,
    request: VerificationScheduleRequestV1,
) -> Result<VerificationScheduleDecisionV1, VerificationCampaignSchedulerError> {
    let route = select_campaign_route(operation_contract).map_err(|error| {
        VerificationCampaignSchedulerError::OperationContractInvalid {
            detail: error.code().to_owned(),
        }
    })?;
    schedule_selected_campaign_route(route, canonical_repository, shadow_repository, request).await
}

async fn schedule_selected_campaign_route(
    route: CampaignRoute,
    canonical_repository: Option<&dyn VerificationCampaignRepository>,
    shadow_repository: Option<&dyn VerificationCampaignShadowRepository>,
    request: VerificationScheduleRequestV1,
) -> Result<VerificationScheduleDecisionV1, VerificationCampaignSchedulerError> {
    match route {
        CampaignRoute::LegacyPath => Ok(VerificationScheduleDecisionV1::LegacyAuthority),
        CampaignRoute::ShadowEvaluationOnly if !request.legacy_terminal => {
            Ok(VerificationScheduleDecisionV1::LegacyAuthorityShadowPending)
        }
        CampaignRoute::ShadowEvaluationOnly => {
            let shadow_repository = shadow_repository
                .ok_or(VerificationCampaignSchedulerError::RepositoryUnavailable)?;
            let shadow_request = request
                .shadow_evaluation
                .ok_or(VerificationCampaignSchedulerError::ShadowSnapshotRequired)?;
            let evaluation = shadow_repository
                .open_evaluation(shadow_request)
                .await
                .map_err(map_repository_error)?;
            Ok(VerificationScheduleDecisionV1::ShadowEvaluationOpened(
                evaluation,
            ))
        }
        CampaignRoute::AuthoritativeCandidate => {
            let canonical_repository = canonical_repository
                .ok_or(VerificationCampaignSchedulerError::RepositoryUnavailable)?;
            let admission = request
                .canonical_admission
                .ok_or(VerificationCampaignSchedulerError::CanonicalAdmissionRequired)?;
            let lease = canonical_repository
                .admit_campaign_with_fresh_tool_truth(admission)
                .await
                .map_err(map_repository_error)?;
            Ok(VerificationScheduleDecisionV1::CanonicalCampaignAdmitted(
                lease,
            ))
        }
    }
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct ShadowEvaluationEffectV1 {
    pub promotion_blocked: bool,
    pub canonical_mutation_allowed: bool,
    pub prepared_action_dispatch_allowed: bool,
}

/// Consumes only the persisted Plan B whole-record comparison state. Shadow
/// mismatch/incomplete may hold promotion, but no shadow result can ever mint
/// canonical mutation or Prepared Action authority.
pub const fn shadow_evaluation_effect(
    state: WholeRecordComparisonStateV1,
) -> ShadowEvaluationEffectV1 {
    ShadowEvaluationEffectV1 {
        promotion_blocked: !matches!(state, WholeRecordComparisonStateV1::Match),
        canonical_mutation_allowed: false,
        prepared_action_dispatch_allowed: false,
    }
}

#[derive(Debug, Clone, Copy, Default, PartialEq, Eq)]
pub struct VerificationOpenWorkCensusV1 {
    pub active_action_count: u32,
    pub pending_authorization_count: u32,
    pub recovery_hold_count: u32,
    pub pending_correction_count: u32,
}

impl VerificationOpenWorkCensusV1 {
    pub const fn is_empty(self) -> bool {
        self.active_action_count == 0
            && self.pending_authorization_count == 0
            && self.recovery_hold_count == 0
            && self.pending_correction_count == 0
    }
}

#[derive(Debug, Clone, Copy, Default, PartialEq, Eq)]
pub struct CoverageLimitationCensusV1 {
    pub blocked: u32,
    pub exhausted: u32,
    pub unassigned: u32,
    pub race_adapter_missing: u32,
}

impl CoverageLimitationCensusV1 {
    pub const fn is_empty(self) -> bool {
        self.blocked == 0
            && self.exhausted == 0
            && self.unassigned == 0
            && self.race_adapter_missing == 0
    }
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum CoverageDisclosureV1 {
    Complete,
    Limited(CoverageLimitationCensusV1),
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum GenerationMaterialityV1 {
    NewMaterial,
    NoNewMaterial,
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct FixedPointWitnessV1 {
    pub receipt_id: uuid::Uuid,
    pub generation_hash: String,
    pub objective_outcome_set_hash: String,
    pub open_work_set_hash: String,
    pub receipt_hash: String,
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct StageFinalSealWitnessV1 {
    pub fixed_point_receipt_id: uuid::Uuid,
    pub fixed_point_receipt_hash: String,
    pub open_work_set_hash: String,
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct VerificationStageClosureSnapshotV1 {
    pub expected_campaign_count: u32,
    pub expected_campaign_set_hash: String,
    pub current_campaign_set_hash: String,
    pub campaigns: Vec<CampaignGateSnapshotV1>,
    pub expected_objective_outcome_count: u32,
    pub expected_objective_outcome_set_hash: String,
    pub current_objective_outcome_count: u32,
    pub open_work: VerificationOpenWorkCensusV1,
    pub wave_consolidation_committed: bool,
    pub revision_adjudication_current: bool,
    pub generation_materiality: GenerationMaterialityV1,
    pub runnable_next_wave_obligation_count: u32,
    pub current_generation_hash: String,
    pub current_objective_outcome_set_hash: String,
    pub empty_open_work_set_hash: String,
    pub current_open_work_set_hash: String,
    pub fixed_point: Option<FixedPointWitnessV1>,
    pub final_seal: Option<StageFinalSealWitnessV1>,
    pub coverage_limitations: CoverageLimitationCensusV1,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum VerificationStageClosureDecisionV1 {
    AwaitCampaignCensus,
    DrainOpenWork,
    AwaitObjectiveOutcomes,
    ConsolidateWave,
    OpenNextWave,
    AdjudicateRevision,
    RecordFixedPoint,
    SealStage { coverage: CoverageDisclosureV1 },
    StageClosed { coverage: CoverageDisclosureV1 },
}

fn coverage_disclosure(census: CoverageLimitationCensusV1) -> CoverageDisclosureV1 {
    if census.is_empty() {
        CoverageDisclosureV1::Complete
    } else {
        CoverageDisclosureV1::Limited(census)
    }
}

/// Deterministic three-level closeout: Campaign local terminality, Wave
/// consolidation/new-work decision, then revision/fixed-point/stage seal.
pub fn evaluate_verification_stage_closure(
    snapshot: &VerificationStageClosureSnapshotV1,
) -> Result<VerificationStageClosureDecisionV1, VerificationCampaignSchedulerError> {
    if snapshot.expected_campaign_count == 0 {
        return Err(VerificationCampaignSchedulerError::CampaignCensusMismatch);
    }
    if snapshot.expected_objective_outcome_count == 0 {
        return Err(VerificationCampaignSchedulerError::ObjectiveOutcomeCensusMismatch);
    }
    let actual_campaign_count = u32::try_from(snapshot.campaigns.len())
        .map_err(|_| VerificationCampaignSchedulerError::CampaignCensusMismatch)?;
    if actual_campaign_count > snapshot.expected_campaign_count {
        return Err(VerificationCampaignSchedulerError::CampaignCensusMismatch);
    }
    if actual_campaign_count < snapshot.expected_campaign_count {
        return Ok(VerificationStageClosureDecisionV1::AwaitCampaignCensus);
    }
    if snapshot.current_campaign_set_hash != snapshot.expected_campaign_set_hash {
        return Err(VerificationCampaignSchedulerError::CampaignCensusMismatch);
    }
    for campaign in &snapshot.campaigns {
        validate_campaign_gate(campaign).map_err(|error| {
            VerificationCampaignSchedulerError::CampaignGateBlocked {
                gate_code: error.code(),
            }
        })?;
        if campaign.authority != ArtifactAuthorityV1::Canonical {
            return Err(VerificationCampaignSchedulerError::CampaignGateBlocked {
                gate_code: "VERIFICATION_CAMPAIGN_SHADOW_AUTHORITY_FORBIDDEN",
            });
        }
        if campaign.phase != CampaignPhaseV1::Terminal {
            return Err(VerificationCampaignSchedulerError::CampaignNonterminal);
        }
    }

    if snapshot.current_objective_outcome_count > snapshot.expected_objective_outcome_count {
        return Err(VerificationCampaignSchedulerError::ObjectiveOutcomeCensusMismatch);
    }
    if snapshot.current_objective_outcome_count < snapshot.expected_objective_outcome_count {
        return Ok(VerificationStageClosureDecisionV1::AwaitObjectiveOutcomes);
    }
    if snapshot.current_objective_outcome_set_hash != snapshot.expected_objective_outcome_set_hash {
        return Err(VerificationCampaignSchedulerError::ObjectiveOutcomeCensusMismatch);
    }
    if !snapshot.open_work.is_empty() {
        return Ok(VerificationStageClosureDecisionV1::DrainOpenWork);
    }
    if snapshot.current_open_work_set_hash != snapshot.empty_open_work_set_hash {
        return Err(VerificationCampaignSchedulerError::OpenWorkCensusMismatch);
    }
    if !snapshot.wave_consolidation_committed {
        return Ok(VerificationStageClosureDecisionV1::ConsolidateWave);
    }
    match snapshot.generation_materiality {
        GenerationMaterialityV1::NewMaterial => {
            if snapshot.runnable_next_wave_obligation_count == 0 {
                return Err(VerificationCampaignSchedulerError::NewGenerationWithoutRunnableWork);
            }
            return Ok(VerificationStageClosureDecisionV1::OpenNextWave);
        }
        GenerationMaterialityV1::NoNewMaterial
            if snapshot.runnable_next_wave_obligation_count > 0 =>
        {
            return Ok(VerificationStageClosureDecisionV1::OpenNextWave);
        }
        GenerationMaterialityV1::NoNewMaterial => {}
    }
    if !snapshot.revision_adjudication_current {
        return Ok(VerificationStageClosureDecisionV1::AdjudicateRevision);
    }

    let Some(fixed_point) = &snapshot.fixed_point else {
        return Ok(VerificationStageClosureDecisionV1::RecordFixedPoint);
    };
    if fixed_point.receipt_id.is_nil()
        || fixed_point.receipt_hash.is_empty()
        || fixed_point.generation_hash != snapshot.current_generation_hash
        || fixed_point.objective_outcome_set_hash != snapshot.current_objective_outcome_set_hash
        || fixed_point.open_work_set_hash != snapshot.current_open_work_set_hash
    {
        return Err(VerificationCampaignSchedulerError::FixedPointMismatch);
    }

    let coverage = coverage_disclosure(snapshot.coverage_limitations);
    let Some(final_seal) = &snapshot.final_seal else {
        return Ok(VerificationStageClosureDecisionV1::SealStage { coverage });
    };
    if final_seal.fixed_point_receipt_id != fixed_point.receipt_id
        || final_seal.fixed_point_receipt_hash.is_empty()
        || final_seal.fixed_point_receipt_hash != fixed_point.receipt_hash
        || final_seal.open_work_set_hash != snapshot.current_open_work_set_hash
    {
        return Err(VerificationCampaignSchedulerError::FinalSealMismatch);
    }
    Ok(VerificationStageClosureDecisionV1::StageClosed { coverage })
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct CampaignProgressSnapshotV1 {
    pub previous_semantic_fingerprint: Option<String>,
    pub current_semantic_fingerprint: String,
    pub consecutive_no_progress_rounds: u32,
    pub max_no_progress_rounds: u32,
    pub budget_exhausted: bool,
    pub deadline_reached: bool,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum CampaignStallDecisionV1 {
    Continue,
    Stop(CampaignStopReasonV1),
}

/// The caller supplies the Task 1 semantic fingerprint, which already excludes
/// timestamps, prose and unrelated evidence. This function never hashes prose
/// or accepts an alternate progress signal.
pub fn decide_campaign_stall(
    progress: &CampaignProgressSnapshotV1,
) -> Result<CampaignStallDecisionV1, VerificationCampaignSchedulerError> {
    if progress.current_semantic_fingerprint.is_empty()
        || progress.max_no_progress_rounds == 0
        || progress.consecutive_no_progress_rounds > progress.max_no_progress_rounds
    {
        return Err(VerificationCampaignSchedulerError::StallPolicyInvalid);
    }
    if progress.budget_exhausted {
        return Ok(CampaignStallDecisionV1::Stop(
            CampaignStopReasonV1::BudgetExhausted,
        ));
    }
    if progress.deadline_reached {
        return Ok(CampaignStallDecisionV1::Stop(
            CampaignStopReasonV1::DeadlineReached,
        ));
    }
    if progress.previous_semantic_fingerprint.as_deref()
        == Some(progress.current_semantic_fingerprint.as_str())
        && progress.consecutive_no_progress_rounds == progress.max_no_progress_rounds
    {
        return Ok(CampaignStallDecisionV1::Stop(
            CampaignStopReasonV1::NoProgress,
        ));
    }
    Ok(CampaignStallDecisionV1::Continue)
}

#[cfg(test)]
mod tests {
    use std::sync::atomic::{AtomicUsize, Ordering};

    use async_trait::async_trait;
    use golish_core::investigation_comparison::WholeRecordComparisonStateV1;
    use uuid::Uuid;

    use super::*;
    use crate::db_traits::RepoResult;
    use crate::harness::verification_campaign::CampaignCoverageDenominatorSealV1;

    struct RecordingRepository {
        canonical_calls: AtomicUsize,
        shadow_calls: AtomicUsize,
        unavailable: bool,
    }

    impl RecordingRepository {
        fn available() -> Self {
            Self {
                canonical_calls: AtomicUsize::new(0),
                shadow_calls: AtomicUsize::new(0),
                unavailable: false,
            }
        }

        fn unavailable() -> Self {
            Self {
                canonical_calls: AtomicUsize::new(0),
                shadow_calls: AtomicUsize::new(0),
                unavailable: true,
            }
        }
    }

    #[async_trait]
    impl VerificationCampaignRepository for RecordingRepository {
        async fn admit_campaign_with_fresh_tool_truth(
            &self,
            request: AdmitCampaignRequest,
        ) -> RepoResult<CampaignLease> {
            self.canonical_calls.fetch_add(1, Ordering::SeqCst);
            if self.unavailable {
                return Err(VerificationCampaignRepositoryError::Unavailable {
                    operation: "admit_campaign_with_fresh_tool_truth",
                });
            }
            Ok(CampaignLease {
                campaign_id: Uuid::new_v4(),
                operation_id: request.operation_id,
                objective_id: request.objective_id,
                campaign_dispatch_generation: 7,
                row_version: 0,
                replayed: false,
            })
        }
    }

    #[async_trait]
    impl VerificationCampaignShadowRepository for RecordingRepository {
        async fn open_evaluation(
            &self,
            request: OpenShadowEvaluation,
        ) -> RepoResult<ShadowEvaluation> {
            self.shadow_calls.fetch_add(1, Ordering::SeqCst);
            Ok(ShadowEvaluation {
                evaluation_id: Uuid::new_v4(),
                operation_id: request.operation_id,
                frozen_snapshot_id: request.frozen_snapshot_id,
                item_count: 1,
                item_set_hash: hash("shadow-items"),
                row_version: 0,
                replayed: false,
            })
        }
    }

    fn hash(label: &str) -> String {
        format!("sha256:{label:0<64}")
    }

    fn admission_request() -> AdmitCampaignRequest {
        AdmitCampaignRequest {
            stable_consumer_request_id: Uuid::new_v4(),
            operation_id: Uuid::new_v4(),
            scope_snapshot_id: Uuid::new_v4(),
            organization_id: Uuid::new_v4(),
            generation_seal_id: Uuid::new_v4(),
            verification_plan_id: Uuid::new_v4(),
            objective_id: Uuid::new_v4(),
            wave_coverage_seal_id: Uuid::new_v4(),
            capability_assessment_set_seal_id: Uuid::new_v4(),
            expected_campaign_id: None,
        }
    }

    fn shadow_request() -> OpenShadowEvaluation {
        OpenShadowEvaluation {
            stable_request_id: Uuid::new_v4(),
            operation_id: Uuid::new_v4(),
            scope_snapshot_id: Uuid::new_v4(),
            organization_id: Uuid::new_v4(),
            hypothesis_revision_id: Uuid::new_v4(),
            verification_plan_id: Uuid::new_v4(),
            frozen_snapshot_id: Uuid::new_v4(),
            frozen_snapshot_hash: hash("shadow-snapshot"),
            obligation_census_hash: hash("shadow-obligations"),
        }
    }

    #[tokio::test]
    async fn verification_campaign_scheduler_legacy_and_shadow_never_call_canonical_repo() {
        let repository = RecordingRepository::available();

        let legacy = schedule_selected_campaign_route(
            CampaignRoute::LegacyPath,
            None,
            None,
            VerificationScheduleRequestV1 {
                legacy_terminal: false,
                canonical_admission: Some(admission_request()),
                shadow_evaluation: Some(shadow_request()),
            },
        )
        .await
        .expect("legacy route remains legacy");
        assert_eq!(legacy, VerificationScheduleDecisionV1::LegacyAuthority);

        let waiting = schedule_selected_campaign_route(
            CampaignRoute::ShadowEvaluationOnly,
            None,
            None,
            VerificationScheduleRequestV1 {
                legacy_terminal: false,
                canonical_admission: Some(admission_request()),
                shadow_evaluation: Some(shadow_request()),
            },
        )
        .await
        .expect("shadow waits for legacy terminal");
        assert_eq!(
            waiting,
            VerificationScheduleDecisionV1::LegacyAuthorityShadowPending
        );
        assert_eq!(repository.canonical_calls.load(Ordering::SeqCst), 0);
        assert_eq!(repository.shadow_calls.load(Ordering::SeqCst), 0);

        let opened = schedule_selected_campaign_route(
            CampaignRoute::ShadowEvaluationOnly,
            None,
            Some(&repository),
            VerificationScheduleRequestV1 {
                legacy_terminal: true,
                canonical_admission: Some(admission_request()),
                shadow_evaluation: Some(shadow_request()),
            },
        )
        .await
        .expect("terminal legacy truth may open isolated shadow evaluation");
        assert!(matches!(
            opened,
            VerificationScheduleDecisionV1::ShadowEvaluationOpened(_)
        ));
        assert_eq!(repository.canonical_calls.load(Ordering::SeqCst), 0);
        assert_eq!(repository.shadow_calls.load(Ordering::SeqCst), 1);
    }

    #[tokio::test]
    async fn verification_campaign_scheduler_authoritative_waits_for_repository_admission() {
        let repository = RecordingRepository::available();
        let decision = schedule_selected_campaign_route(
            CampaignRoute::AuthoritativeCandidate,
            Some(&repository),
            None,
            VerificationScheduleRequestV1 {
                legacy_terminal: false,
                canonical_admission: Some(admission_request()),
                shadow_evaluation: None,
            },
        )
        .await
        .expect("repository admission succeeds");

        assert!(matches!(
            &decision,
            VerificationScheduleDecisionV1::CanonicalCampaignAdmitted(_)
        ));
        assert!(decision.allows_canonical_dispatch());
        assert_eq!(repository.canonical_calls.load(Ordering::SeqCst), 1);

        let unavailable = RecordingRepository::unavailable();
        let error = schedule_selected_campaign_route(
            CampaignRoute::AuthoritativeCandidate,
            Some(&unavailable),
            None,
            VerificationScheduleRequestV1 {
                legacy_terminal: false,
                canonical_admission: Some(admission_request()),
                shadow_evaluation: None,
            },
        )
        .await
        .expect_err("unavailable repository fails closed");
        assert_eq!(
            error.code(),
            "VERIFICATION_CAMPAIGN_SCHEDULER_REPOSITORY_UNAVAILABLE"
        );

        let missing = schedule_selected_campaign_route(
            CampaignRoute::AuthoritativeCandidate,
            Some(&repository),
            Some(&repository),
            VerificationScheduleRequestV1 {
                legacy_terminal: false,
                canonical_admission: None,
                shadow_evaluation: None,
            },
        )
        .await
        .expect_err("authoritative route cannot dispatch before admission input");
        assert_eq!(
            missing.code(),
            "VERIFICATION_CAMPAIGN_SCHEDULER_CANONICAL_ADMISSION_REQUIRED"
        );
    }

    #[test]
    fn verification_shadow_evaluator_divergence_only_blocks_promotion() {
        for state in [
            WholeRecordComparisonStateV1::Mismatch,
            WholeRecordComparisonStateV1::Incomplete,
        ] {
            let effect = shadow_evaluation_effect(state);
            assert!(effect.promotion_blocked);
            assert!(!effect.canonical_mutation_allowed);
            assert!(!effect.prepared_action_dispatch_allowed);
        }
        let matching = shadow_evaluation_effect(WholeRecordComparisonStateV1::Match);
        assert!(!matching.promotion_blocked);
        assert!(!matching.canonical_mutation_allowed);
        assert!(!matching.prepared_action_dispatch_allowed);
    }

    fn terminal_campaign() -> CampaignGateSnapshotV1 {
        CampaignGateSnapshotV1 {
            authority: ArtifactAuthorityV1::Canonical,
            phase: CampaignPhaseV1::Terminal,
            actions: Vec::new(),
            denominator: Some(CampaignCoverageDenominatorSealV1 {
                seal_version: 1,
                objective_id: Uuid::new_v4(),
                plan_objective_member_hash: hash("objective"),
                verification_contract_hash: hash("contract"),
                claim_component_member_hashes: Vec::new(),
                claim_component_set_hash: hash("claims"),
                members: Vec::new(),
                member_set_hash: hash("members"),
                seal_hash: hash("seal"),
            }),
            coverage_results: Vec::new(),
            fact_delta_bundle_count: 1,
            fact_delta_consumed: false,
        }
    }

    fn closure_snapshot() -> VerificationStageClosureSnapshotV1 {
        VerificationStageClosureSnapshotV1 {
            expected_campaign_count: 1,
            expected_campaign_set_hash: hash("campaigns"),
            current_campaign_set_hash: hash("campaigns"),
            campaigns: vec![terminal_campaign()],
            expected_objective_outcome_count: 1,
            expected_objective_outcome_set_hash: hash("outcomes"),
            current_objective_outcome_count: 1,
            open_work: VerificationOpenWorkCensusV1::default(),
            wave_consolidation_committed: false,
            revision_adjudication_current: false,
            generation_materiality: GenerationMaterialityV1::NoNewMaterial,
            runnable_next_wave_obligation_count: 0,
            current_generation_hash: hash("generation"),
            current_objective_outcome_set_hash: hash("outcomes"),
            empty_open_work_set_hash: hash("open-work"),
            current_open_work_set_hash: hash("open-work"),
            fixed_point: None,
            final_seal: None,
            coverage_limitations: CoverageLimitationCensusV1::default(),
        }
    }

    #[test]
    fn verification_campaign_gate_rejects_nonterminal_campaign() {
        let mut snapshot = closure_snapshot();
        snapshot.campaigns[0].phase = CampaignPhaseV1::Planning;
        let error = evaluate_verification_stage_closure(&snapshot)
            .expect_err("nonterminal Campaign cannot close the stage Gate");
        assert_eq!(
            error.code(),
            "VERIFICATION_CAMPAIGN_SCHEDULER_CAMPAIGN_NONTERMINAL"
        );
    }

    #[test]
    fn verification_campaign_fixed_point_requires_exact_closed_work_chain() {
        let mut snapshot = closure_snapshot();
        assert_eq!(
            evaluate_verification_stage_closure(&snapshot).unwrap(),
            VerificationStageClosureDecisionV1::ConsolidateWave
        );

        snapshot.wave_consolidation_committed = true;
        assert_eq!(
            evaluate_verification_stage_closure(&snapshot).unwrap(),
            VerificationStageClosureDecisionV1::AdjudicateRevision
        );

        snapshot.revision_adjudication_current = true;
        assert_eq!(
            evaluate_verification_stage_closure(&snapshot).unwrap(),
            VerificationStageClosureDecisionV1::RecordFixedPoint
        );

        snapshot.fixed_point = Some(FixedPointWitnessV1 {
            receipt_id: Uuid::new_v4(),
            generation_hash: snapshot.current_generation_hash.clone(),
            objective_outcome_set_hash: snapshot.current_objective_outcome_set_hash.clone(),
            open_work_set_hash: snapshot.current_open_work_set_hash.clone(),
            receipt_hash: hash("fixed-point"),
        });
        assert!(matches!(
            evaluate_verification_stage_closure(&snapshot).unwrap(),
            VerificationStageClosureDecisionV1::SealStage {
                coverage: CoverageDisclosureV1::Complete
            }
        ));

        let fixed_point = snapshot.fixed_point.as_ref().unwrap();
        snapshot.final_seal = Some(StageFinalSealWitnessV1 {
            fixed_point_receipt_id: fixed_point.receipt_id,
            fixed_point_receipt_hash: fixed_point.receipt_hash.clone(),
            open_work_set_hash: snapshot.current_open_work_set_hash.clone(),
        });
        assert!(matches!(
            evaluate_verification_stage_closure(&snapshot).unwrap(),
            VerificationStageClosureDecisionV1::StageClosed {
                coverage: CoverageDisclosureV1::Complete
            }
        ));
    }

    #[test]
    fn verification_campaign_open_work_and_generation_materiality_are_exact() {
        let mut snapshot = closure_snapshot();
        snapshot.open_work.pending_authorization_count = 1;
        assert_eq!(
            evaluate_verification_stage_closure(&snapshot).unwrap(),
            VerificationStageClosureDecisionV1::DrainOpenWork
        );

        snapshot.open_work = VerificationOpenWorkCensusV1::default();
        snapshot.wave_consolidation_committed = true;
        snapshot.generation_materiality = GenerationMaterialityV1::NewMaterial;
        let error = evaluate_verification_stage_closure(&snapshot)
            .expect_err("new generation without runnable work is inconsistent");
        assert_eq!(
            error.code(),
            "VERIFICATION_CAMPAIGN_SCHEDULER_NEW_GENERATION_WITHOUT_RUNNABLE_WORK"
        );

        snapshot.runnable_next_wave_obligation_count = 1;
        assert_eq!(
            evaluate_verification_stage_closure(&snapshot).unwrap(),
            VerificationStageClosureDecisionV1::OpenNextWave
        );

        let mut drifted = closure_snapshot();
        drifted.current_campaign_set_hash = hash("foreign-campaign-set");
        assert_eq!(
            evaluate_verification_stage_closure(&drifted)
                .unwrap_err()
                .code(),
            "VERIFICATION_CAMPAIGN_SCHEDULER_CAMPAIGN_CENSUS_MISMATCH"
        );
    }

    #[test]
    fn verification_campaign_gate_never_reports_residual_coverage_as_complete() {
        let mut snapshot = closure_snapshot();
        snapshot.wave_consolidation_committed = true;
        snapshot.revision_adjudication_current = true;
        snapshot.coverage_limitations = CoverageLimitationCensusV1 {
            blocked: 1,
            exhausted: 2,
            unassigned: 1,
            race_adapter_missing: 1,
        };
        snapshot.fixed_point = Some(FixedPointWitnessV1 {
            receipt_id: Uuid::new_v4(),
            generation_hash: snapshot.current_generation_hash.clone(),
            objective_outcome_set_hash: snapshot.current_objective_outcome_set_hash.clone(),
            open_work_set_hash: snapshot.current_open_work_set_hash.clone(),
            receipt_hash: hash("limited-fixed-point"),
        });

        let decision = evaluate_verification_stage_closure(&snapshot).unwrap();
        assert!(matches!(
            decision,
            VerificationStageClosureDecisionV1::SealStage {
                coverage: CoverageDisclosureV1::Limited(_)
            }
        ));
    }

    #[test]
    fn verification_campaign_fixed_point_stall_uses_only_semantic_fingerprint() {
        let stable = hash("semantic-attempt");
        let stalled = decide_campaign_stall(&CampaignProgressSnapshotV1 {
            previous_semantic_fingerprint: Some(stable.clone()),
            current_semantic_fingerprint: stable,
            consecutive_no_progress_rounds: 3,
            max_no_progress_rounds: 3,
            budget_exhausted: false,
            deadline_reached: false,
        })
        .expect("valid bounded policy");
        assert_eq!(
            stalled,
            CampaignStallDecisionV1::Stop(CampaignStopReasonV1::NoProgress)
        );

        let budget = decide_campaign_stall(&CampaignProgressSnapshotV1 {
            previous_semantic_fingerprint: None,
            current_semantic_fingerprint: hash("new-semantic-attempt"),
            consecutive_no_progress_rounds: 0,
            max_no_progress_rounds: 3,
            budget_exhausted: true,
            deadline_reached: false,
        })
        .unwrap();
        assert_eq!(
            budget,
            CampaignStallDecisionV1::Stop(CampaignStopReasonV1::BudgetExhausted)
        );
    }
}
