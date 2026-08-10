//! SQLx-free persistence boundary for Plan C verification campaigns.
//!
//! All inputs are server-built typed identities.  In particular, callers
//! cannot provide Tool Truth roots, freshness claims, canonical request
//! payloads, credentials, network handles, or database rows.  Production
//! repositories derive those authorities under their own transaction guard.

use async_trait::async_trait;
use golish_core::investigation_projection::{PlanCProjectionMutationRouteV1, ProjectionRouteV1};
use uuid::Uuid;

use crate::harness::verification_campaign::{
    HypothesisRevisionOutcome, ObjectiveCampaignOutcome, PreparedActionDisposition,
};

pub type RepoResult<T> = Result<T, VerificationCampaignRepositoryError>;

#[derive(Debug, Clone, PartialEq, Eq, thiserror::Error)]
pub enum VerificationCampaignRepositoryError {
    #[error("verification_campaign_repository_unavailable: {operation}")]
    Unavailable { operation: &'static str },
    #[error("verification_campaign_repository_invalid_request: {detail}")]
    InvalidRequest { detail: String },
    #[error("verification_campaign_repository_not_found: {detail}")]
    NotFound { detail: String },
    #[error("verification_campaign_repository_conflict: {detail}")]
    Conflict { detail: String },
    #[error("verification_campaign_repository_authority_mismatch: {detail}")]
    AuthorityMismatch { detail: String },
    #[error("verification_campaign_repository_infrastructure: {detail}")]
    Infrastructure { detail: String },
}

impl VerificationCampaignRepositoryError {
    pub const fn code(&self) -> &'static str {
        match self {
            Self::Unavailable { .. } => "verification_campaign_repository_unavailable",
            Self::InvalidRequest { .. } => "verification_campaign_repository_invalid_request",
            Self::NotFound { .. } => "verification_campaign_repository_not_found",
            Self::Conflict { .. } => "verification_campaign_repository_conflict",
            Self::AuthorityMismatch { .. } => "verification_campaign_repository_authority_mismatch",
            Self::Infrastructure { .. } => "verification_campaign_repository_infrastructure",
        }
    }

    fn unavailable(operation: &'static str) -> Self {
        Self::Unavailable { operation }
    }
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct SealWaveCoverage {
    pub stable_request_id: Uuid,
    pub operation_id: Uuid,
    pub scope_snapshot_id: Uuid,
    pub organization_id: Uuid,
    pub generation_seal_id: Uuid,
    pub verification_plan_id: Uuid,
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct WaveCoverageSeal {
    pub seal_id: Uuid,
    pub operation_id: Uuid,
    pub generation_seal_id: Uuid,
    pub member_count: u32,
    pub member_set_hash: String,
    pub replayed: bool,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, serde::Serialize, serde::Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum CapabilityAssessmentDispositionV1 {
    Available,
    AdapterMissing,
    PolicyDenied,
    PrerequisiteMissing,
    Unassessed,
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct RecordCapabilityAssessment {
    pub stable_request_id: Uuid,
    pub operation_id: Uuid,
    pub scope_snapshot_id: Uuid,
    pub organization_id: Uuid,
    pub wave_coverage_seal_id: Uuid,
    pub objective_id: Uuid,
    pub capability_id: String,
    pub disposition: CapabilityAssessmentDispositionV1,
    pub adapter_contract_version: Option<String>,
    pub adapter_contract_digest: Option<String>,
    pub residual_reason_code: Option<String>,
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct CapabilityAssessment {
    pub assessment_id: Uuid,
    pub operation_id: Uuid,
    pub objective_id: Uuid,
    pub capability_id: String,
    pub disposition: CapabilityAssessmentDispositionV1,
    pub assessment_hash: String,
    pub replayed: bool,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct SealCapabilityAssessmentSet {
    pub stable_request_id: Uuid,
    pub operation_id: Uuid,
    pub scope_snapshot_id: Uuid,
    pub organization_id: Uuid,
    pub wave_coverage_seal_id: Uuid,
    pub objective_id: Uuid,
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct CapabilityAssessmentSetSeal {
    pub seal_id: Uuid,
    pub operation_id: Uuid,
    pub member_count: u32,
    pub member_set_hash: String,
    pub replayed: bool,
}

/// Admission selector.  Tool Truth roots and freshness data are deliberately
/// absent; the Pg repository derives the relevant-root census from this
/// server-owned identity inside the Plan A authority-host callback.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct AdmitCampaignRequest {
    pub stable_consumer_request_id: Uuid,
    pub operation_id: Uuid,
    pub scope_snapshot_id: Uuid,
    pub organization_id: Uuid,
    pub generation_seal_id: Uuid,
    pub verification_plan_id: Uuid,
    pub objective_id: Uuid,
    pub wave_coverage_seal_id: Uuid,
    pub capability_assessment_set_seal_id: Uuid,
    /// Canonical Investigation reservations pre-allocate the Campaign id.
    /// Legacy admission leaves this absent and uses the stable-request id.
    pub expected_campaign_id: Option<Uuid>,
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct CampaignLease {
    pub campaign_id: Uuid,
    pub operation_id: Uuid,
    pub objective_id: Uuid,
    pub campaign_dispatch_generation: i64,
    pub row_version: i64,
    pub replayed: bool,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct OpenCampaignRound {
    pub stable_request_id: Uuid,
    pub operation_id: Uuid,
    pub campaign_id: Uuid,
    pub expected_campaign_row_version: i64,
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct CampaignRound {
    pub round_id: Uuid,
    pub campaign_id: Uuid,
    pub ordinal: u32,
    pub consult_census_id: Uuid,
    pub consult_member_count: u32,
    pub consult_member_set_hash: String,
    pub row_version: i64,
    pub replayed: bool,
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct PersistStrategyDecision {
    pub stable_request_id: Uuid,
    pub operation_id: Uuid,
    pub campaign_id: Uuid,
    pub round_id: Uuid,
    pub strategy_decision_id: Uuid,
    pub strategy_schema: String,
    pub strategy_version: u32,
    /// Host-validated cognitive strategy. It contains only closed capability
    /// ids and evidence/control references; no executable action material.
    pub typed_strategy: serde_json::Value,
    pub strategy_hash: String,
    pub obligation_set_hash: String,
    pub expected_round_row_version: i64,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct SealCampaignCoverageDenominator {
    pub stable_request_id: Uuid,
    pub operation_id: Uuid,
    pub campaign_id: Uuid,
    pub round_id: Uuid,
    pub objective_id: Uuid,
    pub verification_contract_id: Uuid,
    pub expected_campaign_row_version: i64,
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct CampaignCoverageDenominatorSeal {
    pub seal_id: Uuid,
    pub campaign_id: Uuid,
    pub member_count: u32,
    pub member_set_hash: String,
    pub replayed: bool,
}

/// Agent-selected identity for one host-compiled action.  Target, adapter,
/// credential, policy, budgets, conflicts and oracle bindings are derived by
/// the repository from sealed Campaign authority.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct ProposePreparedAction {
    pub stable_request_id: Uuid,
    pub operation_id: Uuid,
    pub campaign_id: Uuid,
    pub round_id: Uuid,
    pub strategy_artifact_id: Uuid,
    pub strategy_obligation_id: Uuid,
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct PreparedActionProposal {
    pub prepared_action_id: Uuid,
    pub campaign_id: Uuid,
    pub capability_id: String,
    pub coverage_member_hash: String,
    pub private_manifest_hash: String,
    pub row_version: i64,
    pub replayed: bool,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct BeginPreparedAction {
    pub stable_request_id: Uuid,
    pub operation_id: Uuid,
    pub campaign_id: Uuid,
    pub prepared_action_id: Uuid,
    pub authorization_receipt_id: Uuid,
    pub expected_action_row_version: i64,
    pub expected_campaign_dispatch_generation: i64,
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct ActionBeginReceipt {
    pub execution_id: Uuid,
    pub prepared_action_id: Uuid,
    pub execution_ordinal: u32,
    pub budget_reservation_set_hash: String,
    pub conflict_lease_set_hash: String,
    pub row_version: i64,
    pub replayed: bool,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct RecordActionSubexecution {
    pub stable_request_id: Uuid,
    pub operation_id: Uuid,
    pub campaign_id: Uuid,
    pub prepared_action_id: Uuid,
    pub execution_id: Uuid,
    pub subexecution_ordinal: u32,
    pub capability_execution_receipt_id: Uuid,
    pub expected_execution_row_version: i64,
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct ActionSubexecutionReceipt {
    pub subexecution_id: Uuid,
    pub execution_id: Uuid,
    pub subexecution_ordinal: u32,
    pub receipt_hash: String,
    pub replayed: bool,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct CloseoutPreparedAction {
    pub stable_request_id: Uuid,
    pub operation_id: Uuid,
    pub campaign_id: Uuid,
    pub prepared_action_id: Uuid,
    pub execution_id: Uuid,
    pub capability_execution_receipt_id: Uuid,
    pub expected_execution_row_version: i64,
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct ActionCloseout {
    pub execution_id: Uuid,
    pub prepared_action_id: Uuid,
    pub terminal_disposition: PreparedActionDisposition,
    pub capability_execution_receipt_id: Uuid,
    pub closeout_hash: String,
    pub row_version: i64,
    pub replayed: bool,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum UnknownActionRecoveryDispositionV1 {
    OutcomeUnknown,
    ReconciledSucceeded,
    ReconciledFailed,
    ManuallyBlocked,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct RecoverUnknownPreparedAction {
    pub stable_request_id: Uuid,
    pub operation_id: Uuid,
    pub campaign_id: Uuid,
    pub prepared_action_id: Uuid,
    pub execution_id: Uuid,
    pub disposition: UnknownActionRecoveryDispositionV1,
    pub expected_execution_row_version: i64,
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct ActionRecoveryCloseout {
    pub execution_id: Uuid,
    pub disposition: UnknownActionRecoveryDispositionV1,
    pub recovery_receipt_id: Uuid,
    pub closeout_hash: String,
    pub row_version: i64,
    pub replayed: bool,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct SealOracleCensus {
    pub stable_request_id: Uuid,
    pub operation_id: Uuid,
    pub campaign_id: Uuid,
    pub coverage_denominator_seal_id: Uuid,
    pub expected_campaign_row_version: i64,
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct OracleCensusSeal {
    pub seal_id: Uuid,
    pub campaign_id: Uuid,
    pub member_count: u32,
    pub member_set_hash: String,
    pub replayed: bool,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct CloseCampaignObjective {
    pub stable_request_id: Uuid,
    pub operation_id: Uuid,
    pub campaign_id: Uuid,
    pub objective_id: Uuid,
    pub oracle_census_seal_id: Uuid,
    pub coverage_denominator_seal_id: Uuid,
    pub expected_campaign_row_version: i64,
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct ObjectiveOutcomeReceipt {
    pub outcome_receipt_id: Uuid,
    pub campaign_id: Uuid,
    pub objective_id: Uuid,
    pub outcome: ObjectiveCampaignOutcome,
    pub fact_delta_bundle_id: Uuid,
    pub outcome_hash: String,
    pub replayed: bool,
}

/// Revision selector.  The repository derives the exact current objective
/// outcome set and the relevant Tool Truth root census under lock; callers do
/// not provide either collection.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct AdjudicateHypothesisRevision {
    pub stable_consumer_request_id: Uuid,
    pub operation_id: Uuid,
    pub scope_snapshot_id: Uuid,
    pub organization_id: Uuid,
    pub generation_seal_id: Uuid,
    pub hypothesis_revision_id: Uuid,
    pub verification_plan_id: Uuid,
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct HypothesisRevisionAdjudicationReceipt {
    pub adjudication_receipt_id: Uuid,
    pub hypothesis_revision_id: Uuid,
    pub outcome: HypothesisRevisionOutcome,
    pub objective_outcome_set_seal_id: Uuid,
    pub authority_bundle_seal_id: Uuid,
    pub adjudication_hash: String,
    pub replayed: bool,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum CampaignAuthorityQuarantineReasonV1 {
    SemanticAuthorityChanged,
    TemporalAuthorityExpired,
    TargetStateChanged,
    ProjectionDiverged,
    ManualSafetyHold,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct QuarantineCampaignAuthority {
    pub stable_request_id: Uuid,
    pub operation_id: Uuid,
    pub campaign_id: Uuid,
    pub reason: CampaignAuthorityQuarantineReasonV1,
    pub source_receipt_id: Uuid,
    pub expected_campaign_row_version: i64,
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct AuthorityQuarantineReceipt {
    pub quarantine_receipt_id: Uuid,
    pub campaign_id: Uuid,
    pub member_count: u32,
    pub member_set_hash: String,
    pub row_version: i64,
    pub replayed: bool,
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct OpenShadowEvaluation {
    pub stable_request_id: Uuid,
    pub operation_id: Uuid,
    pub scope_snapshot_id: Uuid,
    pub organization_id: Uuid,
    pub hypothesis_revision_id: Uuid,
    pub verification_plan_id: Uuid,
    pub frozen_snapshot_id: Uuid,
    pub frozen_snapshot_hash: String,
    pub obligation_census_hash: String,
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct ShadowEvaluation {
    pub evaluation_id: Uuid,
    pub operation_id: Uuid,
    pub frozen_snapshot_id: Uuid,
    pub item_count: u32,
    pub item_set_hash: String,
    pub row_version: i64,
    pub replayed: bool,
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct RecordShadowReceiptReplay {
    pub stable_request_id: Uuid,
    pub operation_id: Uuid,
    pub evaluation_id: Uuid,
    pub evaluation_item_id: Uuid,
    pub compiled_semantic_signature: String,
    pub frozen_target_hash: String,
    pub adapter_contract_version: String,
    pub oracle_contract_version: String,
    pub legacy_capability_receipt_id: Uuid,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct ComparisonId(pub Uuid);

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct CloseShadowEvaluation {
    pub stable_request_id: Uuid,
    pub operation_id: Uuid,
    pub evaluation_id: Uuid,
    pub expected_row_version: i64,
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct ShadowEvaluationReceipt {
    pub evaluation_id: Uuid,
    pub comparison_count: u32,
    pub comparison_id_set_hash: String,
    pub receipt_hash: String,
    pub row_version: i64,
    pub replayed: bool,
}

pub type VerificationCampaignProjectionContractV1 = ProjectionRouteV1;

/// Exhaustive catalog of canonical Plan C mutations which can change a read
/// model, Gate input, open-work state, or report input.  Every implementation
/// must emit the associated typed projection record in the same transaction.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum VerificationCampaignMutationProducerV1 {
    CampaignAdmission,
    OpenRoundWithConsultCensus,
    ConsultTerminal,
    StrategyDecision,
    ObligationDecision,
    BudgetReserve,
    BudgetConsume,
    BudgetUnknownHold,
    BudgetSettle,
    BudgetExhaust,
    ConflictLeaseAcquire,
    ConflictRecoveryHold,
    ConflictLeaseRelease,
    CleanupOpen,
    CleanupClose,
    CallbackOpen,
    CallbackClose,
    CapabilityAssessment,
    CapabilityAssessmentSetSeal,
    WaveCoverageDenominatorSeal,
    CampaignCoverageDenominatorSeal,
    PreparedAction,
    PreparedActionSuperseded,
    PreparedActionAuthorization,
    ActionExecutionBegin,
    ActionSubexecution,
    ActionExecutionClose,
    OracleAssessment,
    OracleCensusSeal,
    CampaignAdjudication,
    CampaignTerminal,
    ObjectiveOutcome,
    Coverage,
    FactDelta,
    RevisionAdjudication,
    RevisionTerminal,
    AuthorityQuarantine,
    CorrectionConsumption,
    EvolutionProposal,
    EvolutionDecision,
    EnrichmentObligation,
    ApplicationRefinementObligation,
    Consolidation,
    FixedPoint,
}

impl VerificationCampaignMutationProducerV1 {
    pub const ALL: [Self; 44] = [
        Self::CampaignAdmission,
        Self::OpenRoundWithConsultCensus,
        Self::ConsultTerminal,
        Self::StrategyDecision,
        Self::ObligationDecision,
        Self::BudgetReserve,
        Self::BudgetConsume,
        Self::BudgetUnknownHold,
        Self::BudgetSettle,
        Self::BudgetExhaust,
        Self::ConflictLeaseAcquire,
        Self::ConflictRecoveryHold,
        Self::ConflictLeaseRelease,
        Self::CleanupOpen,
        Self::CleanupClose,
        Self::CallbackOpen,
        Self::CallbackClose,
        Self::CapabilityAssessment,
        Self::CapabilityAssessmentSetSeal,
        Self::WaveCoverageDenominatorSeal,
        Self::CampaignCoverageDenominatorSeal,
        Self::PreparedAction,
        Self::PreparedActionSuperseded,
        Self::PreparedActionAuthorization,
        Self::ActionExecutionBegin,
        Self::ActionSubexecution,
        Self::ActionExecutionClose,
        Self::OracleAssessment,
        Self::OracleCensusSeal,
        Self::CampaignAdjudication,
        Self::CampaignTerminal,
        Self::ObjectiveOutcome,
        Self::Coverage,
        Self::FactDelta,
        Self::RevisionAdjudication,
        Self::RevisionTerminal,
        Self::AuthorityQuarantine,
        Self::CorrectionConsumption,
        Self::EvolutionProposal,
        Self::EvolutionDecision,
        Self::EnrichmentObligation,
        Self::ApplicationRefinementObligation,
        Self::Consolidation,
        Self::FixedPoint,
    ];

    pub const fn as_str(self) -> &'static str {
        match self {
            Self::CampaignAdmission => "campaign_admission",
            Self::OpenRoundWithConsultCensus => "open_round_with_consult_census",
            Self::ConsultTerminal => "consult_terminal",
            Self::StrategyDecision => "strategy_decision",
            Self::ObligationDecision => "obligation_decision",
            Self::BudgetReserve => "budget_reserve",
            Self::BudgetConsume => "budget_consume",
            Self::BudgetUnknownHold => "budget_unknown_hold",
            Self::BudgetSettle => "budget_settle",
            Self::BudgetExhaust => "budget_exhaust",
            Self::ConflictLeaseAcquire => "conflict_lease_acquire",
            Self::ConflictRecoveryHold => "conflict_recovery_hold",
            Self::ConflictLeaseRelease => "conflict_lease_release",
            Self::CleanupOpen => "cleanup_open",
            Self::CleanupClose => "cleanup_close",
            Self::CallbackOpen => "callback_open",
            Self::CallbackClose => "callback_close",
            Self::CapabilityAssessment => "capability_assessment",
            Self::CapabilityAssessmentSetSeal => "capability_assessment_set_seal",
            Self::WaveCoverageDenominatorSeal => "wave_coverage_denominator_seal",
            Self::CampaignCoverageDenominatorSeal => "campaign_coverage_denominator_seal",
            Self::PreparedAction => "prepared_action",
            Self::PreparedActionSuperseded => "prepared_action_superseded",
            Self::PreparedActionAuthorization => "prepared_action_authorization",
            Self::ActionExecutionBegin => "action_execution_begin",
            Self::ActionSubexecution => "action_subexecution",
            Self::ActionExecutionClose => "action_execution_close",
            Self::OracleAssessment => "oracle_assessment",
            Self::OracleCensusSeal => "oracle_census_seal",
            Self::CampaignAdjudication => "campaign_adjudication",
            Self::CampaignTerminal => "campaign_terminal",
            Self::ObjectiveOutcome => "objective_outcome",
            Self::Coverage => "coverage",
            Self::FactDelta => "fact_delta",
            Self::RevisionAdjudication => "revision_adjudication",
            Self::RevisionTerminal => "revision_terminal",
            Self::AuthorityQuarantine => "authority_quarantine",
            Self::CorrectionConsumption => "correction_consumption",
            Self::EvolutionProposal => "evolution_proposal",
            Self::EvolutionDecision => "evolution_decision",
            Self::EnrichmentObligation => "enrichment_obligation",
            Self::ApplicationRefinementObligation => "application_refinement_obligation",
            Self::Consolidation => "consolidation",
            Self::FixedPoint => "fixed_point",
        }
    }

    pub const fn projection_contract(self) -> VerificationCampaignProjectionContractV1 {
        use PlanCProjectionMutationRouteV1 as Route;

        let route = match self {
            Self::CampaignAdmission => Route::CampaignInserted,
            Self::OpenRoundWithConsultCensus => Route::CampaignRoundInserted,
            Self::ConsultTerminal => Route::ConsultClosed,
            Self::StrategyDecision => Route::StrategyInserted,
            Self::ObligationDecision => Route::StrategyObligationInserted,
            Self::BudgetReserve
            | Self::BudgetConsume
            | Self::BudgetUnknownHold
            | Self::BudgetSettle
            | Self::BudgetExhaust => Route::BudgetLedgerEntryRecorded,
            Self::ConflictLeaseAcquire => Route::ConflictLeaseAcquired,
            Self::ConflictRecoveryHold => Route::ConflictLeaseRecoveryHeld,
            Self::ConflictLeaseRelease => Route::ConflictLeaseReleased,
            Self::CleanupOpen => Route::CleanupObligationInserted,
            Self::CleanupClose => Route::CleanupObligationClosed,
            Self::CallbackOpen => Route::CallbackObligationInserted,
            Self::CallbackClose => Route::CallbackObligationClosed,
            Self::CapabilityAssessment => Route::CapabilityAssessmentInserted,
            Self::CapabilityAssessmentSetSeal => Route::CapabilityAssessmentSetSealed,
            Self::WaveCoverageDenominatorSeal => Route::CoverageDenominatorSealed,
            Self::CampaignCoverageDenominatorSeal => Route::CoverageCampaignDenominatorSealed,
            Self::PreparedAction => Route::PreparedActionInserted,
            Self::PreparedActionSuperseded => Route::PreparedActionSuperseded,
            Self::PreparedActionAuthorization => Route::AuthorizationInserted,
            Self::ActionExecutionBegin | Self::ActionSubexecution => Route::ActionExecutionInserted,
            Self::ActionExecutionClose => Route::ActionExecutionClosed,
            Self::OracleAssessment => Route::OracleInserted,
            Self::OracleCensusSeal => Route::OracleCensusSealed,
            Self::CampaignAdjudication => Route::AdjudicationInserted,
            Self::CampaignTerminal => Route::CampaignTerminalClosed,
            Self::ObjectiveOutcome => Route::HypothesisVerificationObjectiveOutcomeClosed,
            Self::Coverage => Route::CoverageResultRecorded,
            Self::FactDelta => Route::FactDeltaInserted,
            Self::RevisionAdjudication => Route::HypothesisRevisionAdjudicationClosed,
            Self::RevisionTerminal => Route::HypothesisRevisionTerminalDecisionClosed,
            Self::AuthorityQuarantine => Route::CampaignSuperseded,
            Self::CorrectionConsumption => Route::FactDeltaConsumptionClosed,
            Self::EvolutionProposal => Route::HypothesisEvolutionProposed,
            Self::EvolutionDecision => Route::HypothesisEvolutionDecided,
            Self::EnrichmentObligation => Route::EnrichmentObligationInserted,
            Self::ApplicationRefinementObligation => {
                Route::ApplicationFactRefinementObligationInserted
            }
            Self::Consolidation => Route::ConsolidationClosed,
            Self::FixedPoint => Route::FixedPointClosed,
        };
        route.route()
    }
}

#[async_trait]
pub trait VerificationCampaignRepository: Send + Sync {
    async fn seal_wave_coverage_denominator(
        &self,
        _request: SealWaveCoverage,
    ) -> RepoResult<WaveCoverageSeal> {
        Err(VerificationCampaignRepositoryError::unavailable(
            "seal_wave_coverage_denominator",
        ))
    }

    async fn record_capability_assessment(
        &self,
        _request: RecordCapabilityAssessment,
    ) -> RepoResult<CapabilityAssessment> {
        Err(VerificationCampaignRepositoryError::unavailable(
            "record_capability_assessment",
        ))
    }

    async fn seal_capability_assessment_set(
        &self,
        _request: SealCapabilityAssessmentSet,
    ) -> RepoResult<CapabilityAssessmentSetSeal> {
        Err(VerificationCampaignRepositoryError::unavailable(
            "seal_capability_assessment_set",
        ))
    }

    async fn admit_campaign_with_fresh_tool_truth(
        &self,
        _request: AdmitCampaignRequest,
    ) -> RepoResult<CampaignLease> {
        Err(VerificationCampaignRepositoryError::unavailable(
            "admit_campaign_with_fresh_tool_truth",
        ))
    }

    async fn open_round(&self, _request: OpenCampaignRound) -> RepoResult<CampaignRound> {
        Err(VerificationCampaignRepositoryError::unavailable(
            "open_round",
        ))
    }

    async fn persist_strategy_decision(&self, _request: PersistStrategyDecision) -> RepoResult<()> {
        Err(VerificationCampaignRepositoryError::unavailable(
            "persist_strategy_decision",
        ))
    }

    async fn seal_coverage_denominator(
        &self,
        _request: SealCampaignCoverageDenominator,
    ) -> RepoResult<CampaignCoverageDenominatorSeal> {
        Err(VerificationCampaignRepositoryError::unavailable(
            "seal_coverage_denominator",
        ))
    }

    async fn propose_prepared_action(
        &self,
        _request: ProposePreparedAction,
    ) -> RepoResult<PreparedActionProposal> {
        Err(VerificationCampaignRepositoryError::unavailable(
            "propose_prepared_action",
        ))
    }

    async fn begin_action(&self, _request: BeginPreparedAction) -> RepoResult<ActionBeginReceipt> {
        Err(VerificationCampaignRepositoryError::unavailable(
            "begin_action",
        ))
    }

    async fn record_action_subexecution(
        &self,
        _request: RecordActionSubexecution,
    ) -> RepoResult<ActionSubexecutionReceipt> {
        Err(VerificationCampaignRepositoryError::unavailable(
            "record_action_subexecution",
        ))
    }

    async fn closeout_action(
        &self,
        _request: CloseoutPreparedAction,
    ) -> RepoResult<ActionCloseout> {
        Err(VerificationCampaignRepositoryError::unavailable(
            "closeout_action",
        ))
    }

    async fn recover_unknown_action(
        &self,
        _request: RecoverUnknownPreparedAction,
    ) -> RepoResult<ActionRecoveryCloseout> {
        Err(VerificationCampaignRepositoryError::unavailable(
            "recover_unknown_action",
        ))
    }

    async fn seal_oracle_census(&self, _request: SealOracleCensus) -> RepoResult<OracleCensusSeal> {
        Err(VerificationCampaignRepositoryError::unavailable(
            "seal_oracle_census",
        ))
    }

    async fn close_campaign_objective(
        &self,
        _request: CloseCampaignObjective,
    ) -> RepoResult<ObjectiveOutcomeReceipt> {
        Err(VerificationCampaignRepositoryError::unavailable(
            "close_campaign_objective",
        ))
    }

    async fn adjudicate_hypothesis_revision_with_fresh_tool_truth(
        &self,
        _request: AdjudicateHypothesisRevision,
    ) -> RepoResult<HypothesisRevisionAdjudicationReceipt> {
        Err(VerificationCampaignRepositoryError::unavailable(
            "adjudicate_hypothesis_revision_with_fresh_tool_truth",
        ))
    }

    async fn quarantine_campaign_authority(
        &self,
        _request: QuarantineCampaignAuthority,
    ) -> RepoResult<AuthorityQuarantineReceipt> {
        Err(VerificationCampaignRepositoryError::unavailable(
            "quarantine_campaign_authority",
        ))
    }
}

#[async_trait]
pub trait VerificationCampaignShadowRepository: Send + Sync {
    async fn open_evaluation(
        &self,
        _request: OpenShadowEvaluation,
    ) -> RepoResult<ShadowEvaluation> {
        Err(VerificationCampaignRepositoryError::unavailable(
            "open_shadow_evaluation",
        ))
    }

    async fn record_receipt_replay_and_compare_v1(
        &self,
        _request: RecordShadowReceiptReplay,
    ) -> RepoResult<ComparisonId> {
        Err(VerificationCampaignRepositoryError::unavailable(
            "record_receipt_replay_and_compare_v1",
        ))
    }

    async fn close_evaluation(
        &self,
        _request: CloseShadowEvaluation,
    ) -> RepoResult<ShadowEvaluationReceipt> {
        Err(VerificationCampaignRepositoryError::unavailable(
            "close_shadow_evaluation",
        ))
    }
}

#[cfg(test)]
mod tests {
    use std::sync::Arc;

    use uuid::Uuid;

    use super::*;

    struct LegacyRepository;

    #[async_trait]
    impl VerificationCampaignRepository for LegacyRepository {}

    #[async_trait]
    impl VerificationCampaignShadowRepository for LegacyRepository {}

    fn assert_repository_is_object_safe(_: Arc<dyn VerificationCampaignRepository>) {}

    fn assert_shadow_repository_is_object_safe(_: Arc<dyn VerificationCampaignShadowRepository>) {}

    #[tokio::test]
    async fn verification_campaign_repository_default_is_typed_unavailable() {
        let repository = Arc::new(LegacyRepository);
        assert_repository_is_object_safe(repository.clone());

        let error = repository
            .seal_wave_coverage_denominator(SealWaveCoverage {
                stable_request_id: Uuid::new_v4(),
                operation_id: Uuid::new_v4(),
                scope_snapshot_id: Uuid::new_v4(),
                organization_id: Uuid::new_v4(),
                generation_seal_id: Uuid::new_v4(),
                verification_plan_id: Uuid::new_v4(),
            })
            .await
            .expect_err("legacy repositories must fail closed");

        assert_eq!(error.code(), "verification_campaign_repository_unavailable");
        assert!(matches!(
            error,
            VerificationCampaignRepositoryError::Unavailable { .. }
        ));
    }

    #[tokio::test]
    async fn verification_campaign_shadow_repository_default_is_typed_unavailable() {
        let repository = Arc::new(LegacyRepository);
        assert_shadow_repository_is_object_safe(repository.clone());

        let error = repository
            .open_evaluation(OpenShadowEvaluation {
                stable_request_id: Uuid::new_v4(),
                operation_id: Uuid::new_v4(),
                scope_snapshot_id: Uuid::new_v4(),
                organization_id: Uuid::new_v4(),
                hypothesis_revision_id: Uuid::new_v4(),
                verification_plan_id: Uuid::new_v4(),
                frozen_snapshot_id: Uuid::new_v4(),
                frozen_snapshot_hash: "sha256:frozen".to_owned(),
                obligation_census_hash: "sha256:obligations".to_owned(),
            })
            .await
            .expect_err("shadow evaluation must also fail closed");

        assert_eq!(error.code(), "verification_campaign_repository_unavailable");
    }

    #[test]
    fn verification_campaign_repository_producer_catalog_is_exhaustive_and_unique() {
        assert_eq!(VerificationCampaignMutationProducerV1::ALL.len(), 44);
        let mut names = VerificationCampaignMutationProducerV1::ALL
            .iter()
            .map(|producer| producer.as_str())
            .collect::<Vec<_>>();
        names.sort_unstable();
        names.dedup();
        assert_eq!(
            names.len(),
            VerificationCampaignMutationProducerV1::ALL.len()
        );
        assert!(VerificationCampaignMutationProducerV1::ALL
            .iter()
            .all(|producer| !producer
                .projection_contract()
                .entity_kind
                .as_str()
                .is_empty()));
    }
}
