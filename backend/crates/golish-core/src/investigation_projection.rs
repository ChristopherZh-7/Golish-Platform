//! Closed projection catalogs shared by canonical writers and read-model projectors.
//!
//! The database stores these values as `TEXT`, but it is not their source of
//! truth.  This module deliberately provides no stringly-typed escape hatch:
//! every accepted entity, change, timeline event, invalidation reason, and
//! Plan C mutation route is represented by a closed Rust enum.

use crate::hypothesis_semantic_key::CanonicalJsonObject;
use serde::{de, Deserialize, Deserializer, Serialize};
use sha2::{Digest, Sha256};

const SHA256_PREFIX: &str = "sha256:";
const SHA256_HEX_LEN: usize = 64;
const MAX_ENTITY_ID_BYTES: usize = 512;
const MAX_REDACTED_BODY_BYTES: usize = 64 * 1024;

#[derive(Debug, thiserror::Error, Clone, PartialEq, Eq)]
pub enum InvestigationProjectionCatalogError {
    #[error("unknown projection entity kind: {0}")]
    UnknownEntityKind(String),
    #[error("unknown projection change kind: {0}")]
    UnknownChangeKind(String),
    #[error("unknown projection invalidation reason: {0}")]
    UnknownInvalidationReason(String),
    #[error("unknown timeline event kind: {0}")]
    UnknownTimelineEventKind(String),
    #[error("unknown projection source-time status: {0}")]
    UnknownSourceTimeStatus(String),
    #[error("unsupported Plan C projection mutation: {0}")]
    UnsupportedPlanCMutation(String),
    #[error("projection terminal manifest is not exact-five: {0}")]
    TerminalManifestNotExactFive(&'static str),
    #[error("projection terminal manifest identity mismatch: {0}")]
    TerminalManifestIdentityMismatch(&'static str),
    #[error("Plan B verification-plan projection route is invalid: {0}")]
    PlanBVerificationPlanRouteInvalid(&'static str),
    #[error("invalid bounded projection record: {0}")]
    InvalidProjectionRecord(&'static str),
}

macro_rules! public_ts_catalog {
    ($name:ident, $error:ident, [$( $variant:ident => $wire:literal ),+ $(,)?]) => {
        #[derive(
            Debug, Clone, Copy, PartialEq, Eq, Hash, Serialize, Deserialize, ts_rs::TS,
        )]
        #[serde(rename_all = "snake_case")]
        #[ts(export_to = "../../../../frontend/lib/generated/")]
        pub enum $name {
            $( $variant ),+
        }

        impl $name {
            pub const ALL: [Self; public_ts_catalog!(@count $( $variant )+)] = [
                $( Self::$variant ),+
            ];

            pub const fn as_str(self) -> &'static str {
                match self {
                    $( Self::$variant => $wire ),+
                }
            }
        }

        impl TryFrom<&str> for $name {
            type Error = InvestigationProjectionCatalogError;

            fn try_from(value: &str) -> Result<Self, Self::Error> {
                match value {
                    $( $wire => Ok(Self::$variant), )+
                    value => Err(InvestigationProjectionCatalogError::$error(value.to_owned())),
                }
            }
        }
    };
    (@count $($item:ident)+) => {
        <[()]>::len(&[$(public_ts_catalog!(@replace $item)),+])
    };
    (@replace $_item:ident) => { () };
}

public_ts_catalog!(
    ProjectionEntityKind,
    UnknownEntityKind,
    [
        Generation => "generation",
        Hypothesis => "hypothesis",
        HypothesisVerificationPlan => "hypothesis_verification_plan",
        HypothesisVerificationObjectiveOutcome => "hypothesis_verification_objective_outcome",
        HypothesisRevisionAdjudication => "hypothesis_revision_adjudication",
        HypothesisRevisionTerminalDecision => "hypothesis_revision_terminal_decision",
        HypothesisStateEvent => "hypothesis_state_event",
        Finding => "finding",
        Refutation => "refutation",
        Relation => "relation",
        Residual => "residual",
        CapabilityAssessment => "capability_assessment",
        CapabilityAssessmentSet => "capability_assessment_set",
        LegacyCandidateProjection => "legacy_candidate_projection",
        LegacyAttemptProjection => "legacy_attempt_projection",
        ShadowComparison => "shadow_comparison",
        Campaign => "campaign",
        CampaignRound => "campaign_round",
        Consult => "consult",
        Strategy => "strategy",
        StrategyObligation => "strategy_obligation",
        PreparedAction => "prepared_action",
        Authorization => "authorization",
        ActionExecution => "action_execution",
        ConflictLease => "conflict_lease",
        BudgetLedgerEntry => "budget_ledger_entry",
        CleanupObligation => "cleanup_obligation",
        CallbackObligation => "callback_obligation",
        Oracle => "oracle",
        OracleCensus => "oracle_census",
        Adjudication => "adjudication",
        CampaignTerminal => "campaign_terminal",
        FactDelta => "fact_delta",
        FactDeltaConsumption => "fact_delta_consumption",
        HypothesisEvolutionProposal => "hypothesis_evolution_proposal",
        HypothesisEvolutionDecision => "hypothesis_evolution_decision",
        Consolidation => "consolidation",
        FixedPoint => "fixed_point",
        EnrichmentObligation => "enrichment_obligation",
        ApplicationFactRefinementObligation => "application_fact_refinement_obligation",
        Coverage => "coverage",
        Report => "report"
    ]
);

#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum ProjectionChangeKind {
    Insert,
    Supersede,
    Close,
    Compare,
    Invalidate,
}

impl ProjectionChangeKind {
    pub const ALL: [Self; 5] = [
        Self::Insert,
        Self::Supersede,
        Self::Close,
        Self::Compare,
        Self::Invalidate,
    ];

    pub const fn as_str(self) -> &'static str {
        match self {
            Self::Insert => "insert",
            Self::Supersede => "supersede",
            Self::Close => "close",
            Self::Compare => "compare",
            Self::Invalidate => "invalidate",
        }
    }
}

impl TryFrom<&str> for ProjectionChangeKind {
    type Error = InvestigationProjectionCatalogError;

    fn try_from(value: &str) -> Result<Self, Self::Error> {
        match value {
            "insert" => Ok(Self::Insert),
            "supersede" => Ok(Self::Supersede),
            "close" => Ok(Self::Close),
            "compare" => Ok(Self::Compare),
            "invalidate" => Ok(Self::Invalidate),
            value => Err(InvestigationProjectionCatalogError::UnknownChangeKind(
                value.to_owned(),
            )),
        }
    }
}

public_ts_catalog!(
    ProjectionInvalidationReason,
    UnknownInvalidationReason,
    [
        SourceSuperseded => "source_superseded",
        SourceQuarantined => "source_quarantined",
        AuthorityStale => "authority_stale",
        SourceDeleted => "source_deleted",
        LegacyProjectionUnsupported => "legacy_projection_unsupported",
        LegacyProjectionDerivationFailed => "legacy_projection_derivation_failed",
        LegacyProjectionDiverged => "legacy_projection_diverged",
        ContractUnsupported => "contract_unsupported"
    ]
);

public_ts_catalog!(
    TimelineEventKind,
    UnknownTimelineEventKind,
    [
        GenerationSealed => "generation_sealed",
        HypothesisInserted => "hypothesis_inserted",
        HypothesisSuperseded => "hypothesis_superseded",
        HypothesisClosed => "hypothesis_closed",
        HypothesisInvalidated => "hypothesis_invalidated",
        HypothesisVerificationPlanSealed => "hypothesis_verification_plan_sealed",
        HypothesisVerificationObjectiveOutcomeClosed => "hypothesis_verification_objective_outcome_closed",
        HypothesisVerificationObjectiveOutcomeInvalidated => "hypothesis_verification_objective_outcome_invalidated",
        HypothesisRevisionAdjudicationClosed => "hypothesis_revision_adjudication_closed",
        HypothesisRevisionAdjudicationInvalidated => "hypothesis_revision_adjudication_invalidated",
        HypothesisRevisionTerminalDecisionClosed => "hypothesis_revision_terminal_decision_closed",
        HypothesisRevisionTerminalDecisionInvalidated => "hypothesis_revision_terminal_decision_invalidated",
        HypothesisStateEventInserted => "hypothesis_state_event_inserted",
        HypothesisStateEventInvalidated => "hypothesis_state_event_invalidated",
        FindingInserted => "finding_inserted",
        FindingInvalidated => "finding_invalidated",
        RefutationInserted => "refutation_inserted",
        RefutationInvalidated => "refutation_invalidated",
        RelationInserted => "relation_inserted",
        RelationInvalidated => "relation_invalidated",
        ResidualInserted => "residual_inserted",
        ResidualClosed => "residual_closed",
        ResidualInvalidated => "residual_invalidated",
        CapabilityAssessmentInserted => "capability_assessment_inserted",
        CapabilityAssessmentInvalidated => "capability_assessment_invalidated",
        CapabilityAssessmentSetSealed => "capability_assessment_set_sealed",
        LegacyCandidateProjectionMaterialized => "legacy_candidate_projection_materialized",
        LegacyCandidateProjectionInvalidated => "legacy_candidate_projection_invalidated",
        LegacyAttemptProjectionMaterialized => "legacy_attempt_projection_materialized",
        LegacyAttemptProjectionInvalidated => "legacy_attempt_projection_invalidated",
        ShadowComparisonRecorded => "shadow_comparison_recorded",
        CampaignInserted => "campaign_inserted",
        CampaignSuperseded => "campaign_superseded",
        CampaignClosed => "campaign_closed",
        CampaignRoundInserted => "campaign_round_inserted",
        CampaignRoundClosed => "campaign_round_closed",
        ConsultInserted => "consult_inserted",
        ConsultClosed => "consult_closed",
        StrategyInserted => "strategy_inserted",
        StrategyObligationInserted => "strategy_obligation_inserted",
        PreparedActionInserted => "prepared_action_inserted",
        PreparedActionSuperseded => "prepared_action_superseded",
        AuthorizationInserted => "authorization_inserted",
        ActionExecutionInserted => "action_execution_inserted",
        ActionExecutionClosed => "action_execution_closed",
        ConflictLeaseAcquired => "conflict_lease_acquired",
        ConflictLeaseRecoveryHeld => "conflict_lease_recovery_held",
        ConflictLeaseReleased => "conflict_lease_released",
        BudgetLedgerEntryRecorded => "budget_ledger_entry_recorded",
        CleanupObligationInserted => "cleanup_obligation_inserted",
        CleanupObligationClosed => "cleanup_obligation_closed",
        CallbackObligationInserted => "callback_obligation_inserted",
        CallbackObligationClosed => "callback_obligation_closed",
        OracleInserted => "oracle_inserted",
        OracleInvalidated => "oracle_invalidated",
        OracleCensusSealed => "oracle_census_sealed",
        AdjudicationInserted => "adjudication_inserted",
        CampaignTerminalClosed => "campaign_terminal_closed",
        CampaignTerminalInvalidated => "campaign_terminal_invalidated",
        FactDeltaInserted => "fact_delta_inserted",
        FactDeltaInvalidated => "fact_delta_invalidated",
        FactDeltaConsumed => "fact_delta_consumed",
        FactDeltaConsumptionClosed => "fact_delta_consumption_closed",
        HypothesisEvolutionProposed => "hypothesis_evolution_proposed",
        HypothesisEvolutionDecided => "hypothesis_evolution_decided",
        ConsolidationClosed => "consolidation_closed",
        FixedPointClosed => "fixed_point_closed",
        EnrichmentObligationInserted => "enrichment_obligation_inserted",
        EnrichmentObligationClosed => "enrichment_obligation_closed",
        ApplicationFactRefinementObligationInserted => "application_fact_refinement_obligation_inserted",
        ApplicationFactRefinementObligationClosed => "application_fact_refinement_obligation_closed",
        CoverageDenominatorSealed => "coverage_denominator_sealed",
        CoverageResultRecorded => "coverage_result_recorded",
        CoverageClosed => "coverage_closed",
        CoverageInvalidated => "coverage_invalidated",
        ReportInserted => "report_inserted",
        ReportClosed => "report_closed",
        ReportSuperseded => "report_superseded"
    ]
);

public_ts_catalog!(
    ProjectionSourceTimeStatusV1,
    UnknownSourceTimeStatus,
    [Known => "known", HistoricalUnknown => "historical_unknown"]
);

/// Return the single allowed timeline event for an entity/change pair.
///
/// `None` is a closed-catalog rejection, never a generic-update fallback.
pub const fn projection_timeline_event_kind(
    entity: ProjectionEntityKind,
    change: ProjectionChangeKind,
) -> Option<TimelineEventKind> {
    use ProjectionChangeKind::{Close, Compare, Insert, Invalidate, Supersede};
    use ProjectionEntityKind as Entity;
    use TimelineEventKind as Event;

    match (entity, change) {
        (Entity::Generation, Insert) => Some(Event::GenerationSealed),
        (Entity::Hypothesis, Insert) => Some(Event::HypothesisInserted),
        (Entity::Hypothesis, Supersede) => Some(Event::HypothesisSuperseded),
        (Entity::Hypothesis, Close) => Some(Event::HypothesisClosed),
        (Entity::Hypothesis, Invalidate) => Some(Event::HypothesisInvalidated),
        (Entity::HypothesisVerificationPlan, Close) => {
            Some(Event::HypothesisVerificationPlanSealed)
        }
        (Entity::HypothesisVerificationObjectiveOutcome, Close) => {
            Some(Event::HypothesisVerificationObjectiveOutcomeClosed)
        }
        (Entity::HypothesisVerificationObjectiveOutcome, Invalidate) => {
            Some(Event::HypothesisVerificationObjectiveOutcomeInvalidated)
        }
        (Entity::HypothesisRevisionAdjudication, Close) => {
            Some(Event::HypothesisRevisionAdjudicationClosed)
        }
        (Entity::HypothesisRevisionAdjudication, Invalidate) => {
            Some(Event::HypothesisRevisionAdjudicationInvalidated)
        }
        (Entity::HypothesisRevisionTerminalDecision, Close) => {
            Some(Event::HypothesisRevisionTerminalDecisionClosed)
        }
        (Entity::HypothesisRevisionTerminalDecision, Invalidate) => {
            Some(Event::HypothesisRevisionTerminalDecisionInvalidated)
        }
        (Entity::HypothesisStateEvent, Insert) => Some(Event::HypothesisStateEventInserted),
        (Entity::HypothesisStateEvent, Invalidate) => Some(Event::HypothesisStateEventInvalidated),
        (Entity::Finding, Insert) => Some(Event::FindingInserted),
        (Entity::Finding, Invalidate) => Some(Event::FindingInvalidated),
        (Entity::Refutation, Insert) => Some(Event::RefutationInserted),
        (Entity::Refutation, Invalidate) => Some(Event::RefutationInvalidated),
        (Entity::Relation, Insert) => Some(Event::RelationInserted),
        (Entity::Relation, Invalidate) => Some(Event::RelationInvalidated),
        (Entity::Residual, Insert) => Some(Event::ResidualInserted),
        (Entity::Residual, Close) => Some(Event::ResidualClosed),
        (Entity::Residual, Invalidate) => Some(Event::ResidualInvalidated),
        (Entity::CapabilityAssessment, Insert) => Some(Event::CapabilityAssessmentInserted),
        (Entity::CapabilityAssessment, Invalidate) => Some(Event::CapabilityAssessmentInvalidated),
        (Entity::CapabilityAssessmentSet, Close) => Some(Event::CapabilityAssessmentSetSealed),
        (Entity::LegacyCandidateProjection, Insert) => {
            Some(Event::LegacyCandidateProjectionMaterialized)
        }
        (Entity::LegacyCandidateProjection, Invalidate) => {
            Some(Event::LegacyCandidateProjectionInvalidated)
        }
        (Entity::LegacyAttemptProjection, Insert) => {
            Some(Event::LegacyAttemptProjectionMaterialized)
        }
        (Entity::LegacyAttemptProjection, Invalidate) => {
            Some(Event::LegacyAttemptProjectionInvalidated)
        }
        (Entity::ShadowComparison, Compare) => Some(Event::ShadowComparisonRecorded),
        (Entity::Campaign, Insert) => Some(Event::CampaignInserted),
        (Entity::Campaign, Supersede) => Some(Event::CampaignSuperseded),
        (Entity::Campaign, Close) => Some(Event::CampaignClosed),
        (Entity::CampaignRound, Insert) => Some(Event::CampaignRoundInserted),
        (Entity::CampaignRound, Close) => Some(Event::CampaignRoundClosed),
        (Entity::Consult, Insert) => Some(Event::ConsultInserted),
        (Entity::Consult, Close) => Some(Event::ConsultClosed),
        (Entity::Strategy, Insert) => Some(Event::StrategyInserted),
        (Entity::StrategyObligation, Insert) => Some(Event::StrategyObligationInserted),
        (Entity::PreparedAction, Insert) => Some(Event::PreparedActionInserted),
        (Entity::PreparedAction, Supersede) => Some(Event::PreparedActionSuperseded),
        (Entity::Authorization, Insert) => Some(Event::AuthorizationInserted),
        (Entity::ActionExecution, Insert) => Some(Event::ActionExecutionInserted),
        (Entity::ActionExecution, Close) => Some(Event::ActionExecutionClosed),
        (Entity::ConflictLease, Insert) => Some(Event::ConflictLeaseAcquired),
        (Entity::ConflictLease, Supersede) => Some(Event::ConflictLeaseRecoveryHeld),
        (Entity::ConflictLease, Close) => Some(Event::ConflictLeaseReleased),
        (Entity::BudgetLedgerEntry, Insert) => Some(Event::BudgetLedgerEntryRecorded),
        (Entity::CleanupObligation, Insert) => Some(Event::CleanupObligationInserted),
        (Entity::CleanupObligation, Close) => Some(Event::CleanupObligationClosed),
        (Entity::CallbackObligation, Insert) => Some(Event::CallbackObligationInserted),
        (Entity::CallbackObligation, Close) => Some(Event::CallbackObligationClosed),
        (Entity::Oracle, Insert) => Some(Event::OracleInserted),
        (Entity::Oracle, Invalidate) => Some(Event::OracleInvalidated),
        (Entity::OracleCensus, Close) => Some(Event::OracleCensusSealed),
        (Entity::Adjudication, Insert) => Some(Event::AdjudicationInserted),
        (Entity::CampaignTerminal, Close) => Some(Event::CampaignTerminalClosed),
        (Entity::CampaignTerminal, Invalidate) => Some(Event::CampaignTerminalInvalidated),
        (Entity::FactDelta, Insert) => Some(Event::FactDeltaInserted),
        (Entity::FactDelta, Invalidate) => Some(Event::FactDeltaInvalidated),
        (Entity::FactDeltaConsumption, Insert) => Some(Event::FactDeltaConsumed),
        (Entity::FactDeltaConsumption, Close) => Some(Event::FactDeltaConsumptionClosed),
        (Entity::HypothesisEvolutionProposal, Insert) => Some(Event::HypothesisEvolutionProposed),
        (Entity::HypothesisEvolutionDecision, Insert) => Some(Event::HypothesisEvolutionDecided),
        (Entity::Consolidation, Close) => Some(Event::ConsolidationClosed),
        (Entity::FixedPoint, Close) => Some(Event::FixedPointClosed),
        (Entity::EnrichmentObligation, Insert) => Some(Event::EnrichmentObligationInserted),
        (Entity::EnrichmentObligation, Close) => Some(Event::EnrichmentObligationClosed),
        (Entity::ApplicationFactRefinementObligation, Insert) => {
            Some(Event::ApplicationFactRefinementObligationInserted)
        }
        (Entity::ApplicationFactRefinementObligation, Close) => {
            Some(Event::ApplicationFactRefinementObligationClosed)
        }
        (Entity::Coverage, Insert) => Some(Event::CoverageDenominatorSealed),
        (Entity::Coverage, Supersede) => Some(Event::CoverageResultRecorded),
        (Entity::Coverage, Close) => Some(Event::CoverageClosed),
        (Entity::Coverage, Invalidate) => Some(Event::CoverageInvalidated),
        (Entity::Report, Insert) => Some(Event::ReportInserted),
        (Entity::Report, Close) => Some(Event::ReportClosed),
        (Entity::Report, Supersede) => Some(Event::ReportSuperseded),
        _ => None,
    }
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash)]
pub struct ProjectionRouteV1 {
    pub entity_kind: ProjectionEntityKind,
    pub change_kind: ProjectionChangeKind,
    pub timeline_event_kind: TimelineEventKind,
}

macro_rules! plan_c_routes {
    ($( $variant:ident => ($wire:literal, $entity:ident, $change:ident, $event:ident) ),+ $(,)?) => {
        #[derive(Debug, Clone, Copy, PartialEq, Eq, Hash)]
        pub enum PlanCProjectionMutationRouteV1 {
            $( $variant ),+
        }

        impl PlanCProjectionMutationRouteV1 {
            pub const ALL: [Self; plan_c_routes!(@count $( $variant )+)] = [
                $( Self::$variant ),+
            ];

            pub const fn as_str(self) -> &'static str {
                match self {
                    $( Self::$variant => $wire ),+
                }
            }

            pub const fn route(self) -> ProjectionRouteV1 {
                match self {
                    $( Self::$variant => ProjectionRouteV1 {
                        entity_kind: ProjectionEntityKind::$entity,
                        change_kind: ProjectionChangeKind::$change,
                        timeline_event_kind: TimelineEventKind::$event,
                    } ),+
                }
            }
        }

        impl TryFrom<&str> for PlanCProjectionMutationRouteV1 {
            type Error = InvestigationProjectionCatalogError;

            fn try_from(value: &str) -> Result<Self, Self::Error> {
                match value {
                    $( $wire => Ok(Self::$variant), )+
                    value => Err(InvestigationProjectionCatalogError::UnsupportedPlanCMutation(
                        value.to_owned(),
                    )),
                }
            }
        }
    };
    (@count $($item:ident)+) => {
        <[()]>::len(&[$(plan_c_routes!(@replace $item)),+])
    };
    (@replace $_item:ident) => { () };
}

plan_c_routes!(
    CapabilityAssessmentInserted => ("capability_assessment.insert", CapabilityAssessment, Insert, CapabilityAssessmentInserted),
    CapabilityAssessmentInvalidated => ("capability_assessment.invalidate", CapabilityAssessment, Invalidate, CapabilityAssessmentInvalidated),
    CapabilityAssessmentSetSealed => ("capability_assessment_set.close", CapabilityAssessmentSet, Close, CapabilityAssessmentSetSealed),
    CampaignInserted => ("campaign.insert", Campaign, Insert, CampaignInserted),
    CampaignSuperseded => ("campaign.supersede", Campaign, Supersede, CampaignSuperseded),
    CampaignClosed => ("campaign.close", Campaign, Close, CampaignClosed),
    CampaignRoundInserted => ("campaign_round.insert", CampaignRound, Insert, CampaignRoundInserted),
    CampaignRoundClosed => ("campaign_round.close", CampaignRound, Close, CampaignRoundClosed),
    ConsultInserted => ("consult.insert", Consult, Insert, ConsultInserted),
    ConsultClosed => ("consult.close", Consult, Close, ConsultClosed),
    StrategyInserted => ("strategy.insert", Strategy, Insert, StrategyInserted),
    StrategyObligationInserted => ("strategy_obligation.insert", StrategyObligation, Insert, StrategyObligationInserted),
    PreparedActionInserted => ("prepared_action.insert", PreparedAction, Insert, PreparedActionInserted),
    PreparedActionSuperseded => ("prepared_action.supersede", PreparedAction, Supersede, PreparedActionSuperseded),
    AuthorizationInserted => ("authorization.insert", Authorization, Insert, AuthorizationInserted),
    ActionExecutionInserted => ("action_execution.insert", ActionExecution, Insert, ActionExecutionInserted),
    ActionExecutionClosed => ("action_execution.close", ActionExecution, Close, ActionExecutionClosed),
    ConflictLeaseAcquired => ("conflict_lease.insert", ConflictLease, Insert, ConflictLeaseAcquired),
    ConflictLeaseRecoveryHeld => ("conflict_lease.supersede", ConflictLease, Supersede, ConflictLeaseRecoveryHeld),
    ConflictLeaseReleased => ("conflict_lease.close", ConflictLease, Close, ConflictLeaseReleased),
    BudgetLedgerEntryRecorded => ("budget_ledger_entry.insert", BudgetLedgerEntry, Insert, BudgetLedgerEntryRecorded),
    CleanupObligationInserted => ("cleanup_obligation.insert", CleanupObligation, Insert, CleanupObligationInserted),
    CleanupObligationClosed => ("cleanup_obligation.close", CleanupObligation, Close, CleanupObligationClosed),
    CallbackObligationInserted => ("callback_obligation.insert", CallbackObligation, Insert, CallbackObligationInserted),
    CallbackObligationClosed => ("callback_obligation.close", CallbackObligation, Close, CallbackObligationClosed),
    OracleInserted => ("oracle.insert", Oracle, Insert, OracleInserted),
    OracleInvalidated => ("oracle.invalidate", Oracle, Invalidate, OracleInvalidated),
    OracleCensusSealed => ("oracle_census.close", OracleCensus, Close, OracleCensusSealed),
    AdjudicationInserted => ("adjudication.insert", Adjudication, Insert, AdjudicationInserted),
    CampaignTerminalClosed => ("campaign_terminal.close", CampaignTerminal, Close, CampaignTerminalClosed),
    CampaignTerminalInvalidated => ("campaign_terminal.invalidate", CampaignTerminal, Invalidate, CampaignTerminalInvalidated),
    HypothesisVerificationObjectiveOutcomeClosed => ("hypothesis_verification_objective_outcome.close", HypothesisVerificationObjectiveOutcome, Close, HypothesisVerificationObjectiveOutcomeClosed),
    HypothesisVerificationObjectiveOutcomeInvalidated => ("hypothesis_verification_objective_outcome.invalidate", HypothesisVerificationObjectiveOutcome, Invalidate, HypothesisVerificationObjectiveOutcomeInvalidated),
    HypothesisRevisionAdjudicationClosed => ("hypothesis_revision_adjudication.close", HypothesisRevisionAdjudication, Close, HypothesisRevisionAdjudicationClosed),
    HypothesisRevisionAdjudicationInvalidated => ("hypothesis_revision_adjudication.invalidate", HypothesisRevisionAdjudication, Invalidate, HypothesisRevisionAdjudicationInvalidated),
    HypothesisRevisionTerminalDecisionClosed => ("hypothesis_revision_terminal_decision.close", HypothesisRevisionTerminalDecision, Close, HypothesisRevisionTerminalDecisionClosed),
    HypothesisRevisionTerminalDecisionInvalidated => ("hypothesis_revision_terminal_decision.invalidate", HypothesisRevisionTerminalDecision, Invalidate, HypothesisRevisionTerminalDecisionInvalidated),
    HypothesisInserted => ("hypothesis.insert", Hypothesis, Insert, HypothesisInserted),
    HypothesisSuperseded => ("hypothesis.supersede", Hypothesis, Supersede, HypothesisSuperseded),
    HypothesisClosed => ("hypothesis.close", Hypothesis, Close, HypothesisClosed),
    HypothesisInvalidated => ("hypothesis.invalidate", Hypothesis, Invalidate, HypothesisInvalidated),
    HypothesisStateEventInserted => ("hypothesis_state_event.insert", HypothesisStateEvent, Insert, HypothesisStateEventInserted),
    HypothesisStateEventInvalidated => ("hypothesis_state_event.invalidate", HypothesisStateEvent, Invalidate, HypothesisStateEventInvalidated),
    FindingInserted => ("finding.insert", Finding, Insert, FindingInserted),
    FindingInvalidated => ("finding.invalidate", Finding, Invalidate, FindingInvalidated),
    RefutationInserted => ("refutation.insert", Refutation, Insert, RefutationInserted),
    RefutationInvalidated => ("refutation.invalidate", Refutation, Invalidate, RefutationInvalidated),
    RelationInserted => ("relation.insert", Relation, Insert, RelationInserted),
    RelationInvalidated => ("relation.invalidate", Relation, Invalidate, RelationInvalidated),
    ResidualInserted => ("residual.insert", Residual, Insert, ResidualInserted),
    ResidualClosed => ("residual.close", Residual, Close, ResidualClosed),
    ResidualInvalidated => ("residual.invalidate", Residual, Invalidate, ResidualInvalidated),
    FactDeltaInserted => ("fact_delta.insert", FactDelta, Insert, FactDeltaInserted),
    FactDeltaCorrectionInserted => ("fact_delta.correction_insert", FactDelta, Insert, FactDeltaInserted),
    FactDeltaInvalidated => ("fact_delta.invalidate", FactDelta, Invalidate, FactDeltaInvalidated),
    FactDeltaConsumptionInserted => ("fact_delta_consumption.insert", FactDeltaConsumption, Insert, FactDeltaConsumed),
    FactDeltaConsumptionClosed => ("fact_delta_consumption.close", FactDeltaConsumption, Close, FactDeltaConsumptionClosed),
    HypothesisEvolutionProposed => ("hypothesis_evolution_proposal.insert", HypothesisEvolutionProposal, Insert, HypothesisEvolutionProposed),
    HypothesisEvolutionDecided => ("hypothesis_evolution_decision.insert", HypothesisEvolutionDecision, Insert, HypothesisEvolutionDecided),
    ConsolidationClosed => ("consolidation.close", Consolidation, Close, ConsolidationClosed),
    FixedPointClosed => ("fixed_point.close", FixedPoint, Close, FixedPointClosed),
    EnrichmentObligationInserted => ("enrichment_obligation.insert", EnrichmentObligation, Insert, EnrichmentObligationInserted),
    EnrichmentObligationClosed => ("enrichment_obligation.close", EnrichmentObligation, Close, EnrichmentObligationClosed),
    ApplicationFactRefinementObligationInserted => ("application_fact_refinement_obligation.insert", ApplicationFactRefinementObligation, Insert, ApplicationFactRefinementObligationInserted),
    ApplicationFactRefinementObligationClosed => ("application_fact_refinement_obligation.close", ApplicationFactRefinementObligation, Close, ApplicationFactRefinementObligationClosed),
    CoverageDenominatorSealed => ("coverage.insert", Coverage, Insert, CoverageDenominatorSealed),
    CoverageCampaignDenominatorSealed => ("coverage.campaign_denominator_insert", Coverage, Insert, CoverageDenominatorSealed),
    CoverageMemberRecorded => ("coverage.member_insert", Coverage, Insert, CoverageDenominatorSealed),
    CoverageResultRecorded => ("coverage.supersede", Coverage, Supersede, CoverageResultRecorded),
    CoverageUnassignedResultRecorded => ("coverage.unassigned_result_supersede", Coverage, Supersede, CoverageResultRecorded),
    CoverageClosed => ("coverage.close", Coverage, Close, CoverageClosed),
    CoverageReceiptClosed => ("coverage.receipt_close", Coverage, Close, CoverageClosed),
    CoverageInvalidated => ("coverage.invalidate", Coverage, Invalidate, CoverageInvalidated),
    ReportInserted => ("report.insert", Report, Insert, ReportInserted),
    ReportClosed => ("report.close", Report, Close, ReportClosed),
    ReportSuperseded => ("report.supersede", Report, Supersede, ReportSuperseded)
);

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum TerminalEvidenceProjectionV1 {
    Finding,
    Refutation,
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct PlanBVerificationPlanProjectionMemberV1 {
    revision_id: uuid::Uuid,
    plan_id: uuid::Uuid,
    plan_hash: String,
    route: ProjectionRouteV1,
}

impl PlanBVerificationPlanProjectionMemberV1 {
    pub fn from_server_source(
        revision_id: uuid::Uuid,
        plan_id: uuid::Uuid,
        plan_hash: String,
        route: ProjectionRouteV1,
    ) -> Result<Self, InvestigationProjectionCatalogError> {
        if revision_id.is_nil() || plan_id.is_nil() || !is_sha256_v1(&plan_hash) {
            return Err(
                InvestigationProjectionCatalogError::PlanBVerificationPlanRouteInvalid("identity"),
            );
        }
        Ok(Self {
            revision_id,
            plan_id,
            plan_hash,
            route,
        })
    }
}

pub fn validate_plan_b_verification_plan_exact_one(
    revision_id: uuid::Uuid,
    plan_id: uuid::Uuid,
    plan_hash: &str,
    members: &[PlanBVerificationPlanProjectionMemberV1],
) -> Result<ProjectionRouteV1, InvestigationProjectionCatalogError> {
    let expected_route = ProjectionRouteV1 {
        entity_kind: ProjectionEntityKind::HypothesisVerificationPlan,
        change_kind: ProjectionChangeKind::Close,
        timeline_event_kind: TimelineEventKind::HypothesisVerificationPlanSealed,
    };
    if members.len() != 1 {
        return Err(
            InvestigationProjectionCatalogError::PlanBVerificationPlanRouteInvalid("exact_one"),
        );
    }
    let member = &members[0];
    if member.revision_id != revision_id
        || member.plan_id != plan_id
        || member.plan_hash != plan_hash
        || member.route != expected_route
    {
        return Err(
            InvestigationProjectionCatalogError::PlanBVerificationPlanRouteInvalid("binding"),
        );
    }
    Ok(expected_route)
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct PlanCTerminalProjectionMemberV1 {
    revision_id: uuid::Uuid,
    route: PlanCProjectionMutationRouteV1,
    source_hash: String,
}

impl PlanCTerminalProjectionMemberV1 {
    pub fn from_server_source(
        revision_id: uuid::Uuid,
        route: PlanCProjectionMutationRouteV1,
        source_hash: String,
    ) -> Result<Self, InvestigationProjectionCatalogError> {
        if revision_id.is_nil() || !is_sha256_v1(&source_hash) {
            return Err(
                InvestigationProjectionCatalogError::TerminalManifestIdentityMismatch("identity"),
            );
        }
        Ok(Self {
            revision_id,
            route,
            source_hash,
        })
    }
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct PlanCTerminalExactFiveManifestV1 {
    pub evidence: TerminalEvidenceProjectionV1,
    routes: [PlanCProjectionMutationRouteV1; 5],
}

impl PlanCTerminalExactFiveManifestV1 {
    pub fn routes(&self) -> &[PlanCProjectionMutationRouteV1; 5] {
        &self.routes
    }
}

/// Validate the revision-terminal canonical source manifest before any outbox
/// member is written.  The successor Hypothesis route is intentionally a
/// `Close`, not a Campaign leaf or generic update.
pub fn validate_plan_c_terminal_exact_five(
    members: &[PlanCTerminalProjectionMemberV1],
) -> Result<PlanCTerminalExactFiveManifestV1, InvestigationProjectionCatalogError> {
    use PlanCProjectionMutationRouteV1 as Mutation;

    if members.len() != 5 {
        return Err(
            InvestigationProjectionCatalogError::TerminalManifestNotExactFive("member_count"),
        );
    }
    let revision_id = members[0].revision_id;
    if members
        .iter()
        .any(|member| member.revision_id != revision_id)
    {
        return Err(
            InvestigationProjectionCatalogError::TerminalManifestIdentityMismatch("revision_id"),
        );
    }
    let source_hashes = members
        .iter()
        .map(|member| member.source_hash.as_str())
        .collect::<std::collections::BTreeSet<_>>();
    if source_hashes.len() != members.len() {
        return Err(
            InvestigationProjectionCatalogError::TerminalManifestIdentityMismatch("source_hash"),
        );
    }
    let routes = members
        .iter()
        .map(|member| member.route)
        .collect::<Vec<_>>();
    let required = [
        Mutation::HypothesisRevisionAdjudicationClosed,
        Mutation::HypothesisRevisionTerminalDecisionClosed,
        Mutation::HypothesisStateEventInserted,
        Mutation::HypothesisClosed,
    ];
    if required
        .iter()
        .any(|required| routes.iter().filter(|route| *route == required).count() != 1)
    {
        return Err(
            InvestigationProjectionCatalogError::TerminalManifestNotExactFive("required_member"),
        );
    }
    let finding_count = routes
        .iter()
        .filter(|route| **route == Mutation::FindingInserted)
        .count();
    let refutation_count = routes
        .iter()
        .filter(|route| **route == Mutation::RefutationInserted)
        .count();
    let evidence = match (finding_count, refutation_count) {
        (1, 0) => TerminalEvidenceProjectionV1::Finding,
        (0, 1) => TerminalEvidenceProjectionV1::Refutation,
        _ => {
            return Err(
                InvestigationProjectionCatalogError::TerminalManifestNotExactFive(
                    "finding_refutation_exactly_one",
                ),
            )
        }
    };
    if routes.iter().any(|route| {
        !required.contains(route)
            && *route != Mutation::FindingInserted
            && *route != Mutation::RefutationInserted
    }) {
        return Err(
            InvestigationProjectionCatalogError::TerminalManifestNotExactFive("extra_member"),
        );
    }

    Ok(PlanCTerminalExactFiveManifestV1 {
        evidence,
        routes: [
            Mutation::HypothesisRevisionAdjudicationClosed,
            Mutation::HypothesisRevisionTerminalDecisionClosed,
            match evidence {
                TerminalEvidenceProjectionV1::Finding => Mutation::FindingInserted,
                TerminalEvidenceProjectionV1::Refutation => Mutation::RefutationInserted,
            },
            Mutation::HypothesisStateEventInserted,
            Mutation::HypothesisClosed,
        ],
    })
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct BoundedRedactedProjectionRecordV1 {
    schema_version: u32,
    entity_id: String,
    entity_version: u64,
    content_sha256: String,
    redaction_contract_version: u32,
    canonical_redacted_body: CanonicalJsonObject,
}

impl BoundedRedactedProjectionRecordV1 {
    pub fn try_new(
        entity_id: impl Into<String>,
        entity_version: u64,
        redaction_contract_version: u32,
        canonical_redacted_body: CanonicalJsonObject,
    ) -> Result<Self, InvestigationProjectionCatalogError> {
        let entity_id = entity_id.into();
        if entity_version == 0 || redaction_contract_version == 0 {
            return Err(
                InvestigationProjectionCatalogError::InvalidProjectionRecord("version_zero"),
            );
        }
        if entity_id.trim().is_empty() || entity_id.len() > MAX_ENTITY_ID_BYTES {
            return Err(
                InvestigationProjectionCatalogError::InvalidProjectionRecord("entity_id_bounds"),
            );
        }
        let canonical_bytes = canonical_redacted_body.canonical_bytes();
        if canonical_bytes.is_empty() || canonical_bytes.len() > MAX_REDACTED_BODY_BYTES {
            return Err(
                InvestigationProjectionCatalogError::InvalidProjectionRecord(
                    "redacted_body_bounds",
                ),
            );
        }
        let content_sha256 = projection_content_sha256(&canonical_bytes);
        Ok(Self {
            schema_version: 1,
            entity_id,
            entity_version,
            content_sha256,
            redaction_contract_version,
            canonical_redacted_body,
        })
    }

    pub fn entity_id(&self) -> &str {
        &self.entity_id
    }

    pub const fn entity_version(&self) -> u64 {
        self.entity_version
    }

    pub fn content_sha256(&self) -> &str {
        &self.content_sha256
    }

    pub fn canonical_redacted_body(&self) -> &CanonicalJsonObject {
        &self.canonical_redacted_body
    }
}

#[derive(Deserialize)]
#[serde(rename_all = "camelCase")]
struct PersistedBoundedRedactedProjectionRecordV1 {
    schema_version: u32,
    entity_id: String,
    entity_version: u64,
    content_sha256: String,
    redaction_contract_version: u32,
    canonical_redacted_body: CanonicalJsonObject,
}

impl<'de> Deserialize<'de> for BoundedRedactedProjectionRecordV1 {
    fn deserialize<D>(deserializer: D) -> Result<Self, D::Error>
    where
        D: Deserializer<'de>,
    {
        let persisted = PersistedBoundedRedactedProjectionRecordV1::deserialize(deserializer)?;
        if persisted.schema_version != 1 || !is_sha256_v1(&persisted.content_sha256) {
            return Err(de::Error::custom("invalid projection record identity"));
        }
        let compiled = Self::try_new(
            persisted.entity_id,
            persisted.entity_version,
            persisted.redaction_contract_version,
            persisted.canonical_redacted_body,
        )
        .map_err(de::Error::custom)?;
        if compiled.content_sha256 != persisted.content_sha256 {
            return Err(de::Error::custom("projection body hash mismatch"));
        }
        Ok(compiled)
    }
}

fn is_sha256_v1(value: &str) -> bool {
    value.strip_prefix(SHA256_PREFIX).is_some_and(|hex| {
        hex.len() == SHA256_HEX_LEN
            && hex
                .bytes()
                .all(|byte| byte.is_ascii_hexdigit() && !byte.is_ascii_uppercase())
    })
}

fn projection_content_sha256(bytes: &[u8]) -> String {
    let digest = Sha256::digest(bytes);
    let mut hex = String::with_capacity(SHA256_HEX_LEN);
    for byte in digest {
        use std::fmt::Write;
        write!(hex, "{byte:02x}").expect("writing to String cannot fail");
    }
    format!("{SHA256_PREFIX}{hex}")
}

macro_rules! typed_projection_records {
    ($( $name:ident ),+ $(,)?) => {
        $(
            #[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
            #[serde(transparent)]
            pub struct $name(BoundedRedactedProjectionRecordV1);

            impl $name {
                pub fn try_new(
                    entity_id: impl Into<String>,
                    entity_version: u64,
                    redaction_contract_version: u32,
                    canonical_redacted_body: CanonicalJsonObject,
                ) -> Result<Self, InvestigationProjectionCatalogError> {
                    BoundedRedactedProjectionRecordV1::try_new(
                        entity_id,
                        entity_version,
                        redaction_contract_version,
                        canonical_redacted_body,
                    )
                    .map(Self)
                }

                pub fn record(&self) -> &BoundedRedactedProjectionRecordV1 {
                    &self.0
                }
            }
        )+
    };
}

typed_projection_records!(
    GenerationProjectionRecordV1,
    HypothesisProjectionRecordV1,
    HypothesisVerificationPlanProjectionRecordV1,
    HypothesisVerificationObjectiveOutcomeProjectionRecordV1,
    HypothesisRevisionAdjudicationProjectionRecordV1,
    HypothesisRevisionTerminalDecisionProjectionRecordV1,
    HypothesisStateEventProjectionRecordV1,
    FindingProjectionRecordV1,
    RefutationProjectionRecordV1,
    RelationProjectionRecordV1,
    ResidualProjectionRecordV1,
    CapabilityAssessmentProjectionRecordV1,
    CapabilityAssessmentSetProjectionRecordV1,
    LegacyCandidateProjectionRecordV1,
    LegacyAttemptProjectionRecordV1,
    ShadowComparisonProjectionRecordV1,
    CampaignProjectionRecordV1,
    CampaignRoundProjectionRecordV1,
    ConsultProjectionRecordV1,
    StrategyProjectionRecordV1,
    StrategyObligationProjectionRecordV1,
    PreparedActionProjectionRecordV1,
    AuthorizationProjectionRecordV1,
    ActionExecutionProjectionRecordV1,
    ConflictLeaseProjectionRecordV1,
    BudgetLedgerEntryProjectionRecordV1,
    CleanupObligationProjectionRecordV1,
    CallbackObligationProjectionRecordV1,
    OracleProjectionRecordV1,
    OracleCensusProjectionRecordV1,
    AdjudicationProjectionRecordV1,
    CampaignTerminalProjectionRecordV1,
    FactDeltaProjectionRecordV1,
    FactDeltaConsumptionProjectionRecordV1,
    HypothesisEvolutionProposalProjectionRecordV1,
    HypothesisEvolutionDecisionProjectionRecordV1,
    ConsolidationProjectionRecordV1,
    FixedPointProjectionRecordV1,
    EnrichmentObligationProjectionRecordV1,
    ApplicationFactRefinementObligationProjectionRecordV1,
    CoverageProjectionRecordV1,
    ReportProjectionRecordV1,
);

macro_rules! projection_union {
    ($name:ident) => {
        #[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
        #[serde(tag = "entityKind", content = "record", rename_all = "snake_case")]
        pub enum $name {
            Generation(GenerationProjectionRecordV1),
            Hypothesis(HypothesisProjectionRecordV1),
            HypothesisVerificationPlan(HypothesisVerificationPlanProjectionRecordV1),
            HypothesisVerificationObjectiveOutcome(
                HypothesisVerificationObjectiveOutcomeProjectionRecordV1,
            ),
            HypothesisRevisionAdjudication(HypothesisRevisionAdjudicationProjectionRecordV1),
            HypothesisRevisionTerminalDecision(
                HypothesisRevisionTerminalDecisionProjectionRecordV1,
            ),
            HypothesisStateEvent(HypothesisStateEventProjectionRecordV1),
            Finding(FindingProjectionRecordV1),
            Refutation(RefutationProjectionRecordV1),
            Relation(RelationProjectionRecordV1),
            Residual(ResidualProjectionRecordV1),
            CapabilityAssessment(CapabilityAssessmentProjectionRecordV1),
            CapabilityAssessmentSet(CapabilityAssessmentSetProjectionRecordV1),
            LegacyCandidateProjection(LegacyCandidateProjectionRecordV1),
            LegacyAttemptProjection(LegacyAttemptProjectionRecordV1),
            ShadowComparison(ShadowComparisonProjectionRecordV1),
            Campaign(CampaignProjectionRecordV1),
            CampaignRound(CampaignRoundProjectionRecordV1),
            Consult(ConsultProjectionRecordV1),
            Strategy(StrategyProjectionRecordV1),
            StrategyObligation(StrategyObligationProjectionRecordV1),
            PreparedAction(PreparedActionProjectionRecordV1),
            Authorization(AuthorizationProjectionRecordV1),
            ActionExecution(ActionExecutionProjectionRecordV1),
            ConflictLease(ConflictLeaseProjectionRecordV1),
            BudgetLedgerEntry(BudgetLedgerEntryProjectionRecordV1),
            CleanupObligation(CleanupObligationProjectionRecordV1),
            CallbackObligation(CallbackObligationProjectionRecordV1),
            Oracle(OracleProjectionRecordV1),
            OracleCensus(OracleCensusProjectionRecordV1),
            Adjudication(AdjudicationProjectionRecordV1),
            CampaignTerminal(CampaignTerminalProjectionRecordV1),
            FactDelta(FactDeltaProjectionRecordV1),
            FactDeltaConsumption(FactDeltaConsumptionProjectionRecordV1),
            HypothesisEvolutionProposal(HypothesisEvolutionProposalProjectionRecordV1),
            HypothesisEvolutionDecision(HypothesisEvolutionDecisionProjectionRecordV1),
            Consolidation(ConsolidationProjectionRecordV1),
            FixedPoint(FixedPointProjectionRecordV1),
            EnrichmentObligation(EnrichmentObligationProjectionRecordV1),
            ApplicationFactRefinementObligation(
                ApplicationFactRefinementObligationProjectionRecordV1,
            ),
            Coverage(CoverageProjectionRecordV1),
            Report(ReportProjectionRecordV1),
        }

        impl $name {
            pub const fn entity_kind(&self) -> ProjectionEntityKind {
                match self {
                    Self::Generation(_) => ProjectionEntityKind::Generation,
                    Self::Hypothesis(_) => ProjectionEntityKind::Hypothesis,
                    Self::HypothesisVerificationPlan(_) => {
                        ProjectionEntityKind::HypothesisVerificationPlan
                    }
                    Self::HypothesisVerificationObjectiveOutcome(_) => {
                        ProjectionEntityKind::HypothesisVerificationObjectiveOutcome
                    }
                    Self::HypothesisRevisionAdjudication(_) => {
                        ProjectionEntityKind::HypothesisRevisionAdjudication
                    }
                    Self::HypothesisRevisionTerminalDecision(_) => {
                        ProjectionEntityKind::HypothesisRevisionTerminalDecision
                    }
                    Self::HypothesisStateEvent(_) => ProjectionEntityKind::HypothesisStateEvent,
                    Self::Finding(_) => ProjectionEntityKind::Finding,
                    Self::Refutation(_) => ProjectionEntityKind::Refutation,
                    Self::Relation(_) => ProjectionEntityKind::Relation,
                    Self::Residual(_) => ProjectionEntityKind::Residual,
                    Self::CapabilityAssessment(_) => ProjectionEntityKind::CapabilityAssessment,
                    Self::CapabilityAssessmentSet(_) => {
                        ProjectionEntityKind::CapabilityAssessmentSet
                    }
                    Self::LegacyCandidateProjection(_) => {
                        ProjectionEntityKind::LegacyCandidateProjection
                    }
                    Self::LegacyAttemptProjection(_) => {
                        ProjectionEntityKind::LegacyAttemptProjection
                    }
                    Self::ShadowComparison(_) => ProjectionEntityKind::ShadowComparison,
                    Self::Campaign(_) => ProjectionEntityKind::Campaign,
                    Self::CampaignRound(_) => ProjectionEntityKind::CampaignRound,
                    Self::Consult(_) => ProjectionEntityKind::Consult,
                    Self::Strategy(_) => ProjectionEntityKind::Strategy,
                    Self::StrategyObligation(_) => ProjectionEntityKind::StrategyObligation,
                    Self::PreparedAction(_) => ProjectionEntityKind::PreparedAction,
                    Self::Authorization(_) => ProjectionEntityKind::Authorization,
                    Self::ActionExecution(_) => ProjectionEntityKind::ActionExecution,
                    Self::ConflictLease(_) => ProjectionEntityKind::ConflictLease,
                    Self::BudgetLedgerEntry(_) => ProjectionEntityKind::BudgetLedgerEntry,
                    Self::CleanupObligation(_) => ProjectionEntityKind::CleanupObligation,
                    Self::CallbackObligation(_) => ProjectionEntityKind::CallbackObligation,
                    Self::Oracle(_) => ProjectionEntityKind::Oracle,
                    Self::OracleCensus(_) => ProjectionEntityKind::OracleCensus,
                    Self::Adjudication(_) => ProjectionEntityKind::Adjudication,
                    Self::CampaignTerminal(_) => ProjectionEntityKind::CampaignTerminal,
                    Self::FactDelta(_) => ProjectionEntityKind::FactDelta,
                    Self::FactDeltaConsumption(_) => ProjectionEntityKind::FactDeltaConsumption,
                    Self::HypothesisEvolutionProposal(_) => {
                        ProjectionEntityKind::HypothesisEvolutionProposal
                    }
                    Self::HypothesisEvolutionDecision(_) => {
                        ProjectionEntityKind::HypothesisEvolutionDecision
                    }
                    Self::Consolidation(_) => ProjectionEntityKind::Consolidation,
                    Self::FixedPoint(_) => ProjectionEntityKind::FixedPoint,
                    Self::EnrichmentObligation(_) => ProjectionEntityKind::EnrichmentObligation,
                    Self::ApplicationFactRefinementObligation(_) => {
                        ProjectionEntityKind::ApplicationFactRefinementObligation
                    }
                    Self::Coverage(_) => ProjectionEntityKind::Coverage,
                    Self::Report(_) => ProjectionEntityKind::Report,
                }
            }
        }
    };
}

projection_union!(ProjectionSourceSnapshotV1);
projection_union!(ProjectionEntityV1);

impl From<ProjectionSourceSnapshotV1> for ProjectionEntityV1 {
    fn from(source: ProjectionSourceSnapshotV1) -> Self {
        match source {
            ProjectionSourceSnapshotV1::Generation(value) => Self::Generation(value),
            ProjectionSourceSnapshotV1::Hypothesis(value) => Self::Hypothesis(value),
            ProjectionSourceSnapshotV1::HypothesisVerificationPlan(value) => {
                Self::HypothesisVerificationPlan(value)
            }
            ProjectionSourceSnapshotV1::HypothesisVerificationObjectiveOutcome(value) => {
                Self::HypothesisVerificationObjectiveOutcome(value)
            }
            ProjectionSourceSnapshotV1::HypothesisRevisionAdjudication(value) => {
                Self::HypothesisRevisionAdjudication(value)
            }
            ProjectionSourceSnapshotV1::HypothesisRevisionTerminalDecision(value) => {
                Self::HypothesisRevisionTerminalDecision(value)
            }
            ProjectionSourceSnapshotV1::HypothesisStateEvent(value) => {
                Self::HypothesisStateEvent(value)
            }
            ProjectionSourceSnapshotV1::Finding(value) => Self::Finding(value),
            ProjectionSourceSnapshotV1::Refutation(value) => Self::Refutation(value),
            ProjectionSourceSnapshotV1::Relation(value) => Self::Relation(value),
            ProjectionSourceSnapshotV1::Residual(value) => Self::Residual(value),
            ProjectionSourceSnapshotV1::CapabilityAssessment(value) => {
                Self::CapabilityAssessment(value)
            }
            ProjectionSourceSnapshotV1::CapabilityAssessmentSet(value) => {
                Self::CapabilityAssessmentSet(value)
            }
            ProjectionSourceSnapshotV1::LegacyCandidateProjection(value) => {
                Self::LegacyCandidateProjection(value)
            }
            ProjectionSourceSnapshotV1::LegacyAttemptProjection(value) => {
                Self::LegacyAttemptProjection(value)
            }
            ProjectionSourceSnapshotV1::ShadowComparison(value) => Self::ShadowComparison(value),
            ProjectionSourceSnapshotV1::Campaign(value) => Self::Campaign(value),
            ProjectionSourceSnapshotV1::CampaignRound(value) => Self::CampaignRound(value),
            ProjectionSourceSnapshotV1::Consult(value) => Self::Consult(value),
            ProjectionSourceSnapshotV1::Strategy(value) => Self::Strategy(value),
            ProjectionSourceSnapshotV1::StrategyObligation(value) => {
                Self::StrategyObligation(value)
            }
            ProjectionSourceSnapshotV1::PreparedAction(value) => Self::PreparedAction(value),
            ProjectionSourceSnapshotV1::Authorization(value) => Self::Authorization(value),
            ProjectionSourceSnapshotV1::ActionExecution(value) => Self::ActionExecution(value),
            ProjectionSourceSnapshotV1::ConflictLease(value) => Self::ConflictLease(value),
            ProjectionSourceSnapshotV1::BudgetLedgerEntry(value) => Self::BudgetLedgerEntry(value),
            ProjectionSourceSnapshotV1::CleanupObligation(value) => Self::CleanupObligation(value),
            ProjectionSourceSnapshotV1::CallbackObligation(value) => {
                Self::CallbackObligation(value)
            }
            ProjectionSourceSnapshotV1::Oracle(value) => Self::Oracle(value),
            ProjectionSourceSnapshotV1::OracleCensus(value) => Self::OracleCensus(value),
            ProjectionSourceSnapshotV1::Adjudication(value) => Self::Adjudication(value),
            ProjectionSourceSnapshotV1::CampaignTerminal(value) => Self::CampaignTerminal(value),
            ProjectionSourceSnapshotV1::FactDelta(value) => Self::FactDelta(value),
            ProjectionSourceSnapshotV1::FactDeltaConsumption(value) => {
                Self::FactDeltaConsumption(value)
            }
            ProjectionSourceSnapshotV1::HypothesisEvolutionProposal(value) => {
                Self::HypothesisEvolutionProposal(value)
            }
            ProjectionSourceSnapshotV1::HypothesisEvolutionDecision(value) => {
                Self::HypothesisEvolutionDecision(value)
            }
            ProjectionSourceSnapshotV1::Consolidation(value) => Self::Consolidation(value),
            ProjectionSourceSnapshotV1::FixedPoint(value) => Self::FixedPoint(value),
            ProjectionSourceSnapshotV1::EnrichmentObligation(value) => {
                Self::EnrichmentObligation(value)
            }
            ProjectionSourceSnapshotV1::ApplicationFactRefinementObligation(value) => {
                Self::ApplicationFactRefinementObligation(value)
            }
            ProjectionSourceSnapshotV1::Coverage(value) => Self::Coverage(value),
            ProjectionSourceSnapshotV1::Report(value) => Self::Report(value),
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use ts_rs::TS;

    fn assert_round_trip<T>(all: &[T], as_str: impl Fn(T) -> &'static str)
    where
        T: Copy + std::fmt::Debug + PartialEq,
        for<'a> T: TryFrom<&'a str, Error = InvestigationProjectionCatalogError>,
    {
        for value in all.iter().copied() {
            assert_eq!(T::try_from(as_str(value)), Ok(value));
        }
    }

    fn terminal_members(
        revision_id: uuid::Uuid,
        routes: &[PlanCProjectionMutationRouteV1],
    ) -> Vec<PlanCTerminalProjectionMemberV1> {
        routes
            .iter()
            .enumerate()
            .map(|(index, route)| {
                PlanCTerminalProjectionMemberV1::from_server_source(
                    revision_id,
                    *route,
                    format!("sha256:{:064x}", index + 1),
                )
                .unwrap()
            })
            .collect()
    }

    #[test]
    fn investigation_projection_catalog_round_trips_closed_values() {
        assert_round_trip(&ProjectionEntityKind::ALL, ProjectionEntityKind::as_str);
        assert_round_trip(&ProjectionChangeKind::ALL, ProjectionChangeKind::as_str);
        assert_round_trip(
            &ProjectionInvalidationReason::ALL,
            ProjectionInvalidationReason::as_str,
        );
        assert_round_trip(&TimelineEventKind::ALL, TimelineEventKind::as_str);
        assert_round_trip(
            &ProjectionSourceTimeStatusV1::ALL,
            ProjectionSourceTimeStatusV1::as_str,
        );
        assert!(ProjectionEntityKind::try_from("generic_update").is_err());
        assert!(ProjectionChangeKind::try_from("update").is_err());
        assert!(TimelineEventKind::try_from("campaign_updated").is_err());

        for route in PlanCProjectionMutationRouteV1::ALL {
            assert_eq!(
                projection_timeline_event_kind(
                    route.route().entity_kind,
                    route.route().change_kind
                ),
                Some(route.route().timeline_event_kind),
                "route {} diverged from the shared mapping",
                route.as_str(),
            );
        }
        for event in TimelineEventKind::ALL {
            let occurrences = ProjectionEntityKind::ALL
                .iter()
                .copied()
                .flat_map(|entity| {
                    ProjectionChangeKind::ALL
                        .iter()
                        .copied()
                        .filter_map(move |change| projection_timeline_event_kind(entity, change))
                })
                .filter(|candidate| *candidate == event)
                .count();
            assert_eq!(
                occurrences,
                1,
                "timeline event {} must have exactly one catalog mapping",
                event.as_str(),
            );
        }
        assert_eq!(
            projection_timeline_event_kind(
                ProjectionEntityKind::CampaignTerminal,
                ProjectionChangeKind::Insert,
            ),
            None,
        );
    }

    #[test]
    fn projection_plan_c_route_catalog_is_exact_and_terminal_manifest_is_five() {
        use PlanCProjectionMutationRouteV1 as Mutation;

        assert_round_trip(
            &PlanCProjectionMutationRouteV1::ALL,
            PlanCProjectionMutationRouteV1::as_str,
        );
        let route_names =
            PlanCProjectionMutationRouteV1::ALL.map(PlanCProjectionMutationRouteV1::as_str);
        let unique_route_names = route_names
            .iter()
            .copied()
            .collect::<std::collections::BTreeSet<_>>();
        assert_eq!(unique_route_names.len(), route_names.len());
        assert!(PlanCProjectionMutationRouteV1::try_from("unknown.update").is_err());

        let finding = [
            Mutation::HypothesisClosed,
            Mutation::HypothesisStateEventInserted,
            Mutation::FindingInserted,
            Mutation::HypothesisRevisionTerminalDecisionClosed,
            Mutation::HypothesisRevisionAdjudicationClosed,
        ];
        let revision_id = uuid::Uuid::from_u128(1);
        let finding_manifest =
            validate_plan_c_terminal_exact_five(&terminal_members(revision_id, &finding))
                .expect("accept exact finding manifest");
        assert_eq!(
            finding_manifest.evidence,
            TerminalEvidenceProjectionV1::Finding
        );
        assert_eq!(finding_manifest.routes().len(), 5);

        let mut refutation = finding;
        refutation[2] = Mutation::RefutationInserted;
        assert_eq!(
            validate_plan_c_terminal_exact_five(&terminal_members(revision_id, &refutation))
                .expect("accept exact refutation manifest")
                .evidence,
            TerminalEvidenceProjectionV1::Refutation,
        );

        assert!(
            validate_plan_c_terminal_exact_five(&terminal_members(revision_id, &finding[..4]))
                .is_err()
        );
        let duplicate = [
            Mutation::HypothesisRevisionAdjudicationClosed,
            Mutation::HypothesisRevisionTerminalDecisionClosed,
            Mutation::FindingInserted,
            Mutation::HypothesisStateEventInserted,
            Mutation::HypothesisStateEventInserted,
        ];
        assert!(
            validate_plan_c_terminal_exact_five(&terminal_members(revision_id, &duplicate))
                .is_err()
        );
        let campaign_leaf_substitution = [
            Mutation::HypothesisRevisionAdjudicationClosed,
            Mutation::HypothesisRevisionTerminalDecisionClosed,
            Mutation::CampaignTerminalClosed,
            Mutation::HypothesisStateEventInserted,
            Mutation::HypothesisClosed,
        ];
        assert!(validate_plan_c_terminal_exact_five(&terminal_members(
            revision_id,
            &campaign_leaf_substitution
        ))
        .is_err());
        let extra = [
            Mutation::HypothesisRevisionAdjudicationClosed,
            Mutation::HypothesisRevisionTerminalDecisionClosed,
            Mutation::FindingInserted,
            Mutation::HypothesisStateEventInserted,
            Mutation::HypothesisClosed,
            Mutation::ReportClosed,
        ];
        assert!(
            validate_plan_c_terminal_exact_five(&terminal_members(revision_id, &extra)).is_err()
        );

        let mut mixed_revision = terminal_members(revision_id, &finding);
        mixed_revision[4].revision_id = uuid::Uuid::from_u128(2);
        assert!(matches!(
            validate_plan_c_terminal_exact_five(&mixed_revision),
            Err(
                InvestigationProjectionCatalogError::TerminalManifestIdentityMismatch(
                    "revision_id"
                )
            )
        ));
    }

    #[test]
    fn projection_plan_b_verification_plan_route_is_exact_one_and_not_campaign_leaf() {
        let revision_id = uuid::Uuid::from_u128(10);
        let plan_id = uuid::Uuid::from_u128(11);
        let plan_hash = format!("sha256:{}", "a".repeat(64));
        let expected = ProjectionRouteV1 {
            entity_kind: ProjectionEntityKind::HypothesisVerificationPlan,
            change_kind: ProjectionChangeKind::Close,
            timeline_event_kind: TimelineEventKind::HypothesisVerificationPlanSealed,
        };
        let member = PlanBVerificationPlanProjectionMemberV1::from_server_source(
            revision_id,
            plan_id,
            plan_hash.clone(),
            expected,
        )
        .unwrap();
        assert_eq!(
            validate_plan_b_verification_plan_exact_one(
                revision_id,
                plan_id,
                &plan_hash,
                std::slice::from_ref(&member),
            ),
            Ok(expected)
        );
        assert!(validate_plan_b_verification_plan_exact_one(
            revision_id,
            plan_id,
            &plan_hash,
            &[member.clone(), member],
        )
        .is_err());

        let campaign_leaf = PlanBVerificationPlanProjectionMemberV1::from_server_source(
            revision_id,
            plan_id,
            plan_hash.clone(),
            ProjectionRouteV1 {
                entity_kind: ProjectionEntityKind::CampaignTerminal,
                change_kind: ProjectionChangeKind::Close,
                timeline_event_kind: TimelineEventKind::CampaignTerminalClosed,
            },
        )
        .unwrap();
        assert!(validate_plan_b_verification_plan_exact_one(
            revision_id,
            plan_id,
            &plan_hash,
            &[campaign_leaf],
        )
        .is_err());
    }

    #[test]
    fn projection_ts_decl_golden_enums_are_stable() {
        fn declaration(name: &str, wires: &[&str]) -> String {
            format!(
                "type {name} = {};",
                wires
                    .iter()
                    .map(|wire| format!("\"{wire}\""))
                    .collect::<Vec<_>>()
                    .join(" | ")
            )
        }

        let config = ts_rs::Config::default();
        assert_eq!(
            ProjectionEntityKind::decl(&config),
            declaration(
                "ProjectionEntityKind",
                &[
                    "generation",
                    "hypothesis",
                    "hypothesis_verification_plan",
                    "hypothesis_verification_objective_outcome",
                    "hypothesis_revision_adjudication",
                    "hypothesis_revision_terminal_decision",
                    "hypothesis_state_event",
                    "finding",
                    "refutation",
                    "relation",
                    "residual",
                    "capability_assessment",
                    "capability_assessment_set",
                    "legacy_candidate_projection",
                    "legacy_attempt_projection",
                    "shadow_comparison",
                    "campaign",
                    "campaign_round",
                    "consult",
                    "strategy",
                    "strategy_obligation",
                    "prepared_action",
                    "authorization",
                    "action_execution",
                    "conflict_lease",
                    "budget_ledger_entry",
                    "cleanup_obligation",
                    "callback_obligation",
                    "oracle",
                    "oracle_census",
                    "adjudication",
                    "campaign_terminal",
                    "fact_delta",
                    "fact_delta_consumption",
                    "hypothesis_evolution_proposal",
                    "hypothesis_evolution_decision",
                    "consolidation",
                    "fixed_point",
                    "enrichment_obligation",
                    "application_fact_refinement_obligation",
                    "coverage",
                    "report",
                ],
            )
        );
        assert_eq!(
            ProjectionInvalidationReason::decl(&config),
            declaration(
                "ProjectionInvalidationReason",
                &[
                    "source_superseded",
                    "source_quarantined",
                    "authority_stale",
                    "source_deleted",
                    "legacy_projection_unsupported",
                    "legacy_projection_derivation_failed",
                    "legacy_projection_diverged",
                    "contract_unsupported",
                ],
            )
        );
        assert_eq!(
            TimelineEventKind::decl(&config),
            declaration(
                "TimelineEventKind",
                &[
                    "generation_sealed",
                    "hypothesis_inserted",
                    "hypothesis_superseded",
                    "hypothesis_closed",
                    "hypothesis_invalidated",
                    "hypothesis_verification_plan_sealed",
                    "hypothesis_verification_objective_outcome_closed",
                    "hypothesis_verification_objective_outcome_invalidated",
                    "hypothesis_revision_adjudication_closed",
                    "hypothesis_revision_adjudication_invalidated",
                    "hypothesis_revision_terminal_decision_closed",
                    "hypothesis_revision_terminal_decision_invalidated",
                    "hypothesis_state_event_inserted",
                    "hypothesis_state_event_invalidated",
                    "finding_inserted",
                    "finding_invalidated",
                    "refutation_inserted",
                    "refutation_invalidated",
                    "relation_inserted",
                    "relation_invalidated",
                    "residual_inserted",
                    "residual_closed",
                    "residual_invalidated",
                    "capability_assessment_inserted",
                    "capability_assessment_invalidated",
                    "capability_assessment_set_sealed",
                    "legacy_candidate_projection_materialized",
                    "legacy_candidate_projection_invalidated",
                    "legacy_attempt_projection_materialized",
                    "legacy_attempt_projection_invalidated",
                    "shadow_comparison_recorded",
                    "campaign_inserted",
                    "campaign_superseded",
                    "campaign_closed",
                    "campaign_round_inserted",
                    "campaign_round_closed",
                    "consult_inserted",
                    "consult_closed",
                    "strategy_inserted",
                    "strategy_obligation_inserted",
                    "prepared_action_inserted",
                    "prepared_action_superseded",
                    "authorization_inserted",
                    "action_execution_inserted",
                    "action_execution_closed",
                    "conflict_lease_acquired",
                    "conflict_lease_recovery_held",
                    "conflict_lease_released",
                    "budget_ledger_entry_recorded",
                    "cleanup_obligation_inserted",
                    "cleanup_obligation_closed",
                    "callback_obligation_inserted",
                    "callback_obligation_closed",
                    "oracle_inserted",
                    "oracle_invalidated",
                    "oracle_census_sealed",
                    "adjudication_inserted",
                    "campaign_terminal_closed",
                    "campaign_terminal_invalidated",
                    "fact_delta_inserted",
                    "fact_delta_invalidated",
                    "fact_delta_consumed",
                    "fact_delta_consumption_closed",
                    "hypothesis_evolution_proposed",
                    "hypothesis_evolution_decided",
                    "consolidation_closed",
                    "fixed_point_closed",
                    "enrichment_obligation_inserted",
                    "enrichment_obligation_closed",
                    "application_fact_refinement_obligation_inserted",
                    "application_fact_refinement_obligation_closed",
                    "coverage_denominator_sealed",
                    "coverage_result_recorded",
                    "coverage_closed",
                    "coverage_invalidated",
                    "report_inserted",
                    "report_closed",
                    "report_superseded",
                ],
            )
        );
        assert_eq!(
            ProjectionSourceTimeStatusV1::decl(&config),
            "type ProjectionSourceTimeStatusV1 = \"known\" | \"historical_unknown\";"
        );
    }

    #[test]
    fn projection_source_snapshot_schema_is_closed_bounded_and_kind_preserving() {
        let record = HypothesisProjectionRecordV1::try_new(
            "hypothesis:one",
            1,
            1,
            CanonicalJsonObject::parse_raw("{\"redacted\":true}").unwrap(),
        )
        .expect("construct bounded server-redacted record");
        let expected_content_hash = projection_content_sha256(b"{\"redacted\":true}");
        assert_eq!(record.record().content_sha256(), expected_content_hash);
        let source = ProjectionSourceSnapshotV1::Hypothesis(record);
        assert_eq!(source.entity_kind(), ProjectionEntityKind::Hypothesis);
        let entity = ProjectionEntityV1::from(source);
        assert_eq!(entity.entity_kind(), ProjectionEntityKind::Hypothesis);
        assert!(HypothesisProjectionRecordV1::try_new(
            "hypothesis:bad",
            0,
            1,
            CanonicalJsonObject::parse_raw("{}").unwrap(),
        )
        .is_err());

        let mut persisted = serde_json::to_value(entity).unwrap();
        persisted["record"]["contentSha256"] =
            serde_json::Value::String(format!("sha256:{}", "0".repeat(64)));
        assert!(serde_json::from_value::<ProjectionEntityV1>(persisted).is_err());
    }
}
