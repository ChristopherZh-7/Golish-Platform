//! Versioned, DB-free artifacts for a verification Campaign.
//!
//! These DTOs contain only typed authority and canonical hashes. Human prose,
//! timestamps, SQL row ids and credentials are deliberately absent from every
//! semantic hash projection in this module.

use golish_core::hypothesis_verification::{
    HypothesisVerificationPlanObjectiveV1, HypothesisVerificationPlanV1,
};
use golish_core::verification_contract::{VerificationContractError, VerificationContractV1};
use serde::{Deserialize, Serialize};
use sha2::{Digest, Sha256};
use std::collections::BTreeSet;
use uuid::Uuid;

#[derive(Debug, Clone, PartialEq, Eq, thiserror::Error)]
#[error("{code}: {message}")]
pub struct VerificationCampaignError {
    code: &'static str,
    message: &'static str,
    residual_reason: Option<ResidualReasonCodeV1>,
}

impl VerificationCampaignError {
    pub const fn code(&self) -> &'static str {
        self.code
    }

    pub const fn residual_reason(&self) -> Option<ResidualReasonCodeV1> {
        self.residual_reason
    }

    pub(crate) const fn new(code: &'static str, message: &'static str) -> Self {
        Self {
            code,
            message,
            residual_reason: None,
        }
    }

    pub(crate) const fn with_residual(
        code: &'static str,
        message: &'static str,
        residual_reason: ResidualReasonCodeV1,
    ) -> Self {
        Self {
            code,
            message,
            residual_reason: Some(residual_reason),
        }
    }
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum PreparedActionDisposition {
    CompileRejected,
    Denied,
    Expired,
    Superseded,
    Succeeded,
    Failed,
    OutcomeUnknown,
    ManuallyBlocked,
}

impl PreparedActionDisposition {
    pub const fn forbids_execution(self) -> bool {
        matches!(
            self,
            Self::CompileRejected | Self::Denied | Self::Expired | Self::Superseded
        )
    }
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum ObjectiveCampaignOutcome {
    Continue,
    Proof,
    Refutation,
    Inconclusive,
    Blocked,
    ExhaustedWithResiduals,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum ArtifactAuthorityV1 {
    Canonical,
    Shadow,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, PartialOrd, Ord, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum ResidualReasonCodeV1 {
    AdapterMissing,
    BudgetExhausted,
    CleanupIncomplete,
    CompileRejected,
    NoActionCompilable,
    OutcomeUnknown,
    PolicyDenied,
    PrerequisiteMissing,
    RaceAdapterMissing,
    Superseded,
    AuthorizationExpired,
    ManuallyBlocked,
}

impl ResidualReasonCodeV1 {
    pub const fn as_str(self) -> &'static str {
        match self {
            Self::AdapterMissing => "adapter_missing",
            Self::BudgetExhausted => "budget_exhausted",
            Self::CleanupIncomplete => "cleanup_incomplete",
            Self::CompileRejected => "compile_rejected",
            Self::NoActionCompilable => "no_action_compilable",
            Self::OutcomeUnknown => "outcome_unknown",
            Self::PolicyDenied => "policy_denied",
            Self::PrerequisiteMissing => "prerequisite_missing",
            Self::RaceAdapterMissing => "race_adapter_missing",
            Self::Superseded => "superseded",
            Self::AuthorizationExpired => "authorization_expired",
            Self::ManuallyBlocked => "manually_blocked",
        }
    }
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum PreconditionStatusV1 {
    Satisfied,
    Failed,
    Unknown,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum ControlValidityV1 {
    Valid,
    Invalid,
    NotRequired,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum ObservationCompletenessV1 {
    Complete,
    Partial,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, PartialOrd, Ord, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum ComponentOracleOutcomeV1 {
    Proof,
    Refutation,
    Inconclusive,
    Blocked,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, PartialOrd, Ord, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum OracleLimitationCodeV1 {
    CleanupIncomplete,
    ControlInvalid,
    CoveragePartial,
    PairedRelationIncomplete,
    PreconditionUnsatisfied,
    RaceGroupIncomplete,
    SequenceIncomplete,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum ExecutionLandingStateV1 {
    Started,
    LandedReconciled,
    OutcomeUnknown,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct ClaimComponentBindingV1 {
    pub predicate_component_member_hash: String,
    pub claim_component_member_hash: String,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct PredicateObservationV1 {
    pub predicate_component_member_hash: String,
    pub predicate_ordinal: u32,
    pub outcome: ComponentOracleOutcomeV1,
    pub deterministic_negative: bool,
    pub observation_window_complete: bool,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum PairedRelationOutcomeV1 {
    Satisfied,
    Refuted,
    Inconclusive,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct PairedRelationObservationV1 {
    pub pair_key: String,
    pub baseline_pair_identity_hash: String,
    pub variant_pair_identity_hash: String,
    pub baseline_predicate_component_member_hash: String,
    pub variant_predicate_component_member_hash: String,
    pub required_control_member_hash: String,
    pub comparator_rule_digest: String,
    pub outcome: PairedRelationOutcomeV1,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct OrderedSequenceObservationV1 {
    pub predicate_component_member_hash: String,
    pub step_ordinal: u32,
    pub event_ordinal: u32,
    pub execution_session_hash: String,
    pub causal_chain_hash: String,
    pub outcome: ComponentOracleOutcomeV1,
    pub deterministic_negative: bool,
    pub observation_window_complete: bool,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, PartialOrd, Ord, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum ConflictKeyKindV1 {
    ControlFixture,
    CredentialSession,
    MutableResource,
    TargetRateBucket,
}

#[derive(Debug, Clone, PartialEq, Eq, PartialOrd, Ord, Serialize, Deserialize)]
pub struct ConflictKeyV1 {
    pub kind: ConflictKeyKindV1,
    pub key_hash: String,
}

impl ConflictKeyV1 {
    pub fn new(
        kind: ConflictKeyKindV1,
        key_hash: String,
    ) -> Result<Self, VerificationCampaignError> {
        require_hash(&key_hash).map_err(|_| {
            VerificationCampaignError::new(
                "VERIFICATION_CAMPAIGN_CONFLICT_KEY_INVALID",
                "conflict key must use a canonical sha256 digest",
            )
        })?;
        Ok(Self { kind, key_hash })
    }
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct ConcurrentSubActionV1 {
    pub subaction_id: Uuid,
    pub ordinal: u32,
    pub conflict_keys: Vec<ConflictKeyV1>,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct ConcurrentActionGroupV1 {
    pub group_id: Uuid,
    pub subactions: Vec<ConcurrentSubActionV1>,
    pub barrier_cohort_hash: String,
    pub max_concurrency: u32,
    pub start_window_millis: u64,
    pub union_conflict_keys: Vec<ConflictKeyV1>,
    pub concurrency_oracle_rule_digest: String,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct ConcurrentSubExecutionReceiptV1 {
    pub subaction_id: Uuid,
    pub ordinal: u32,
    pub start_offset_millis: u64,
    pub outcome_known: bool,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct ConcurrentActionGroupReceiptV1 {
    pub group_id: Uuid,
    pub barrier_cohort_hash: String,
    pub subexecutions: Vec<ConcurrentSubExecutionReceiptV1>,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(tag = "kind", content = "group", rename_all = "snake_case")]
pub enum PreparedActionKindV1 {
    SingleActionV1,
    ConcurrentActionGroupV1(ConcurrentActionGroupV1),
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum ObjectiveExecutionClassV1 {
    Standard,
    RaceClass,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct ExecutionKeyV1 {
    pub prepared_action_id: Uuid,
    pub authorization_receipt_id: Uuid,
    pub execution_ordinal: u32,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct SemanticAttemptFingerprintInputV1 {
    pub objective_id: Uuid,
    pub verification_contract_hash: String,
    pub required_control_member_hashes: Vec<String>,
    pub action_contract_digest: String,
    pub adapter_contract_digest: String,
    pub oracle_rule_digest: String,
    pub relevant_evidence_member_hashes: Vec<String>,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct ActionOracleContractV1 {
    pub contract_version: u32,
    pub prepared_action_id: Uuid,
    pub objective_id: Uuid,
    pub plan_objective_member_hash: String,
    pub verification_contract_hash: String,
    pub predicate_component_member_hashes: Vec<String>,
    pub required_control_member_hashes: Vec<String>,
    pub claim_component_bindings: Vec<ClaimComponentBindingV1>,
    pub action_kind: PreparedActionKindV1,
    pub action_oracle_contract_hash: String,
}

impl ActionOracleContractV1 {
    pub fn seal(
        contract: &VerificationContractV1,
        plan_objective_member_hash: String,
        prepared_action_id: Uuid,
        mut claim_component_bindings: Vec<ClaimComponentBindingV1>,
        mut action_kind: PreparedActionKindV1,
    ) -> Result<Self, VerificationContractError> {
        if prepared_action_id.is_nil() {
            return Err(VerificationContractError::InvalidField(
                "prepared_action_id",
            ));
        }
        require_hash(&plan_objective_member_hash)?;
        claim_component_bindings.sort_by(|left, right| {
            (
                &left.predicate_component_member_hash,
                &left.claim_component_member_hash,
            )
                .cmp(&(
                    &right.predicate_component_member_hash,
                    &right.claim_component_member_hash,
                ))
        });
        ensure_unique(
            claim_component_bindings
                .iter()
                .map(|binding| {
                    (
                        binding.predicate_component_member_hash.as_str(),
                        binding.claim_component_member_hash.as_str(),
                    )
                })
                .collect::<Vec<_>>(),
            "action claim component bindings",
        )?;
        let expected_predicates = contract
            .predicate_components()
            .iter()
            .map(|component| component.member_hash())
            .collect::<BTreeSet<_>>();
        if claim_component_bindings.is_empty()
            || claim_component_bindings.iter().any(|binding| {
                require_hash(&binding.predicate_component_member_hash).is_err()
                    || require_hash(&binding.claim_component_member_hash).is_err()
                    || !expected_predicates
                        .contains(binding.predicate_component_member_hash.as_str())
            })
        {
            return Err(VerificationContractError::InvalidReference(
                "action claim component bindings",
            ));
        }
        canonicalize_action_kind(&mut action_kind)?;
        let bound_predicates = claim_component_bindings
            .iter()
            .map(|binding| binding.predicate_component_member_hash.as_str())
            .collect::<BTreeSet<_>>();
        let predicate_component_member_hashes = contract
            .predicate_components()
            .iter()
            .filter(|component| bound_predicates.contains(component.member_hash()))
            .map(|component| component.member_hash().to_owned())
            .collect::<Vec<_>>();
        let required_control_member_hashes = contract
            .required_controls()
            .iter()
            .map(|control| control.member_hash().to_owned())
            .collect::<Vec<_>>();
        let action_oracle_contract_hash = hash_domain(
            "action_oracle_contract.v1",
            &(
                1_u32,
                prepared_action_id,
                contract.objective_id(),
                &plan_objective_member_hash,
                contract.contract_hash(),
                &predicate_component_member_hashes,
                &required_control_member_hashes,
                &claim_component_bindings,
                &action_kind,
            ),
        )?;
        Ok(Self {
            contract_version: 1,
            prepared_action_id,
            objective_id: contract.objective_id(),
            plan_objective_member_hash,
            verification_contract_hash: contract.contract_hash().to_owned(),
            predicate_component_member_hashes,
            required_control_member_hashes,
            claim_component_bindings,
            action_kind,
            action_oracle_contract_hash,
        })
    }
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct ReconciledExecutionReceiptV1 {
    pub receipt_version: u32,
    pub prepared_action_id: Uuid,
    pub verification_contract_hash: String,
    pub execution_key: ExecutionKeyV1,
    pub landing_state: ExecutionLandingStateV1,
    pub precondition: PreconditionStatusV1,
    pub control: ControlValidityV1,
    pub completeness: ObservationCompletenessV1,
    pub cleanup_complete: bool,
    pub predicate_observations: Vec<PredicateObservationV1>,
    pub observed_control_member_hashes: Vec<String>,
    pub paired_relation: Option<PairedRelationObservationV1>,
    pub ordered_sequence: Vec<OrderedSequenceObservationV1>,
    pub concurrent_group_receipt: Option<ConcurrentActionGroupReceiptV1>,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct PredicateOracleAssessmentV1 {
    pub predicate_component_member_hash: String,
    pub predicate_ordinal: u32,
    pub claim_component_member_hashes: Vec<String>,
    pub outcome: ComponentOracleOutcomeV1,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct ActionOracleAssessmentV1 {
    pub assessment_version: u32,
    pub authority: ArtifactAuthorityV1,
    pub prepared_action_id: Uuid,
    pub plan_objective_member_hash: String,
    pub verification_contract_hash: String,
    pub action_oracle_contract_hash: String,
    pub execution_key_hash: String,
    pub predicate_outcomes: Vec<PredicateOracleAssessmentV1>,
    pub required_control_member_hashes: Vec<String>,
    pub control: ControlValidityV1,
    pub paired_relation: Option<PairedRelationObservationV1>,
    pub ordered_sequence: Vec<OrderedSequenceObservationV1>,
    pub concurrent_group_valid: bool,
    pub limitation_codes: Vec<OracleLimitationCodeV1>,
    pub assessment_hash: String,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct OracleCensusV1 {
    pub verification_contract_hash: String,
    pub authority: ArtifactAuthorityV1,
    pub assessments: Vec<ActionOracleAssessmentV1>,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum CoverageObligationKindV1 {
    Predicate,
    RequiredControl,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct CoverageDenominatorMemberV1 {
    pub ordinal: u32,
    pub kind: CoverageObligationKindV1,
    pub contract_member_hash: String,
    pub claim_component_member_hashes: Vec<String>,
    pub member_hash: String,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct CampaignCoverageDenominatorSealV1 {
    pub seal_version: u32,
    pub objective_id: Uuid,
    pub plan_objective_member_hash: String,
    pub verification_contract_hash: String,
    pub claim_component_member_hashes: Vec<String>,
    pub claim_component_set_hash: String,
    pub members: Vec<CoverageDenominatorMemberV1>,
    pub member_set_hash: String,
    pub seal_hash: String,
}

impl CampaignCoverageDenominatorSealV1 {
    pub fn seal(
        contract: &VerificationContractV1,
        objective: &HypothesisVerificationPlanObjectiveV1,
        mut bindings: Vec<ClaimComponentBindingV1>,
        first_action_authorized: bool,
    ) -> Result<Self, VerificationCampaignError> {
        if first_action_authorized {
            return Err(VerificationCampaignError::new(
                "VERIFICATION_CAMPAIGN_DENOMINATOR_SEAL_TOO_LATE",
                "campaign denominator must be sealed before authorization",
            ));
        }
        if objective.objective_id() != contract.objective_id()
            || objective.verification_contract_hash() != contract.contract_hash()
        {
            return Err(VerificationCampaignError::new(
                "VERIFICATION_CAMPAIGN_OBJECTIVE_BINDING_MISMATCH",
                "campaign denominator does not bind the Plan B objective",
            ));
        }
        bindings.sort_by(|left, right| {
            (
                &left.predicate_component_member_hash,
                &left.claim_component_member_hash,
            )
                .cmp(&(
                    &right.predicate_component_member_hash,
                    &right.claim_component_member_hash,
                ))
        });
        let unique_bindings = bindings
            .iter()
            .map(|binding| {
                (
                    binding.predicate_component_member_hash.as_str(),
                    binding.claim_component_member_hash.as_str(),
                )
            })
            .collect::<BTreeSet<_>>();
        let expected_predicates = contract
            .predicate_components()
            .iter()
            .map(|predicate| predicate.member_hash())
            .collect::<BTreeSet<_>>();
        let expected_claims = objective
            .claim_component_member_hashes()
            .iter()
            .map(String::as_str)
            .collect::<BTreeSet<_>>();
        let actual_predicates = bindings
            .iter()
            .map(|binding| binding.predicate_component_member_hash.as_str())
            .collect::<BTreeSet<_>>();
        let actual_claims = bindings
            .iter()
            .map(|binding| binding.claim_component_member_hash.as_str())
            .collect::<BTreeSet<_>>();
        if unique_bindings.len() != bindings.len()
            || actual_predicates != expected_predicates
            || actual_claims != expected_claims
            || bindings.iter().any(|binding| {
                require_hash(&binding.predicate_component_member_hash).is_err()
                    || require_hash(&binding.claim_component_member_hash).is_err()
            })
        {
            return Err(VerificationCampaignError::new(
                "VERIFICATION_CAMPAIGN_CLAIM_COMPONENT_BINDING_MISMATCH",
                "predicate bindings must cover the Plan B claim component exact set",
            ));
        }

        let mut members = Vec::new();
        for predicate in contract.predicate_components() {
            let claim_component_member_hashes = bindings
                .iter()
                .filter(|binding| {
                    binding.predicate_component_member_hash == predicate.member_hash()
                })
                .map(|binding| binding.claim_component_member_hash.clone())
                .collect::<Vec<_>>();
            let ordinal = members.len() as u32;
            let member_hash = hash_domain_campaign(
                "campaign_coverage_denominator_member.v1",
                &(
                    ordinal,
                    CoverageObligationKindV1::Predicate,
                    predicate.member_hash(),
                    &claim_component_member_hashes,
                ),
            )?;
            members.push(CoverageDenominatorMemberV1 {
                ordinal,
                kind: CoverageObligationKindV1::Predicate,
                contract_member_hash: predicate.member_hash().to_owned(),
                claim_component_member_hashes,
                member_hash,
            });
        }
        for control in contract.required_controls() {
            let ordinal = members.len() as u32;
            let member_hash = hash_domain_campaign(
                "campaign_coverage_denominator_member.v1",
                &(
                    ordinal,
                    CoverageObligationKindV1::RequiredControl,
                    control.member_hash(),
                    Vec::<String>::new(),
                ),
            )?;
            members.push(CoverageDenominatorMemberV1 {
                ordinal,
                kind: CoverageObligationKindV1::RequiredControl,
                contract_member_hash: control.member_hash().to_owned(),
                claim_component_member_hashes: Vec::new(),
                member_hash,
            });
        }
        let member_hashes = members
            .iter()
            .map(|member| member.member_hash.clone())
            .collect::<Vec<_>>();
        let member_set_hash =
            exact_set_hash_campaign("campaign_coverage_denominator_members.v1", &member_hashes)?;
        let claim_component_member_hashes = objective.claim_component_member_hashes().to_vec();
        let claim_component_set_hash = objective.claim_component_set_hash().to_owned();
        let seal_hash = hash_domain_campaign(
            "campaign_coverage_denominator.v1",
            &(
                1_u32,
                contract.objective_id(),
                objective.member_hash(),
                contract.contract_hash(),
                &claim_component_set_hash,
                &member_set_hash,
            ),
        )?;
        Ok(Self {
            seal_version: 1,
            objective_id: contract.objective_id(),
            plan_objective_member_hash: objective.member_hash().to_owned(),
            verification_contract_hash: contract.contract_hash().to_owned(),
            claim_component_member_hashes,
            claim_component_set_hash,
            members,
            member_set_hash,
            seal_hash,
        })
    }
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(tag = "status", rename_all = "snake_case")]
pub enum CoverageResultStatusV1 {
    Tested {
        prepared_action_id: Uuid,
        capability_receipt_hash: String,
        oracle_assessment_hash: String,
    },
    Untested {
        residual_risk_hash: String,
    },
    Degraded {
        residual_risk_hash: String,
    },
    Blocked {
        residual_risk_hash: String,
    },
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct CoverageResultV1 {
    pub denominator_member_hash: String,
    pub status: CoverageResultStatusV1,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct ObligationDispositionSetV1 {
    pub denominator: CampaignCoverageDenominatorSealV1,
    pub results: Vec<CoverageResultV1>,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct ClaimComponentCampaignOutcomeV1 {
    pub claim_component_member_hash: String,
    pub outcome: ComponentOracleOutcomeV1,
    pub predicate_component_member_hashes: Vec<String>,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct CampaignAdjudicationV1 {
    pub adjudication_version: u32,
    pub plan_objective_member_hash: String,
    pub verification_contract_hash: String,
    pub outcome: ObjectiveCampaignOutcome,
    pub predicate_outcomes: Vec<PredicateOracleAssessmentV1>,
    pub claim_component_outcomes: Vec<ClaimComponentCampaignOutcomeV1>,
    pub residual_risk_hashes: Vec<String>,
    pub adjudication_hash: String,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct WaveDenominatorMemberV1 {
    pub ordinal: u32,
    pub objective_id: Uuid,
    pub plan_objective_member_hash: String,
    pub verification_contract_hash: String,
    pub claim_component_set_hash: String,
    pub member_hash: String,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct VerificationWaveDenominatorSealV1 {
    pub seal_version: u32,
    pub generation_hash: String,
    pub plan_hash: String,
    pub members: Vec<WaveDenominatorMemberV1>,
    pub member_set_hash: String,
    pub seal_hash: String,
}

impl VerificationWaveDenominatorSealV1 {
    pub fn seal(
        plan: &HypothesisVerificationPlanV1,
        generation_hash: String,
        first_campaign_admitted: bool,
    ) -> Result<Self, VerificationCampaignError> {
        if first_campaign_admitted {
            return Err(VerificationCampaignError::new(
                "VERIFICATION_CAMPAIGN_WAVE_DENOMINATOR_SEAL_TOO_LATE",
                "wave denominator must be sealed before Campaign admission",
            ));
        }
        require_hash(&generation_hash).map_err(|_| {
            VerificationCampaignError::new(
                "VERIFICATION_CAMPAIGN_WAVE_DENOMINATOR_INVALID",
                "generation hash is invalid",
            )
        })?;
        let mut members = Vec::with_capacity(plan.objectives().len());
        for (ordinal, objective) in plan.objectives().iter().enumerate() {
            let member_hash = hash_domain_campaign(
                "verification_wave_denominator_member.v1",
                &(
                    ordinal as u32,
                    objective.objective_id(),
                    objective.member_hash(),
                    objective.verification_contract_hash(),
                    objective.claim_component_set_hash(),
                ),
            )?;
            members.push(WaveDenominatorMemberV1 {
                ordinal: ordinal as u32,
                objective_id: objective.objective_id(),
                plan_objective_member_hash: objective.member_hash().to_owned(),
                verification_contract_hash: objective.verification_contract_hash().to_owned(),
                claim_component_set_hash: objective.claim_component_set_hash().to_owned(),
                member_hash,
            });
        }
        let member_hashes = members
            .iter()
            .map(|member| member.member_hash.clone())
            .collect::<Vec<_>>();
        let member_set_hash =
            exact_set_hash_campaign("verification_wave_denominator_members.v1", &member_hashes)?;
        let seal_hash = hash_domain_campaign(
            "verification_wave_denominator.v1",
            &(1_u32, &generation_hash, plan.plan_hash(), &member_set_hash),
        )?;
        Ok(Self {
            seal_version: 1,
            generation_hash,
            plan_hash: plan.plan_hash().to_owned(),
            members,
            member_set_hash,
            seal_hash,
        })
    }
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(tag = "disposition", rename_all = "snake_case")]
pub enum WaveMemberDispositionV1 {
    Campaign {
        wave_member_hash: String,
        campaign_denominator_hash: String,
    },
    Unassigned {
        wave_member_hash: String,
        residual_risk_hash: String,
    },
}

impl WaveMemberDispositionV1 {
    pub fn wave_member_hash(&self) -> &str {
        match self {
            Self::Campaign {
                wave_member_hash, ..
            }
            | Self::Unassigned {
                wave_member_hash, ..
            } => wave_member_hash,
        }
    }
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(tag = "disposition", rename_all = "snake_case")]
pub enum RoundDispositionV1 {
    Continue,
    NoActionCompilable {
        reason_code: ResidualReasonCodeV1,
        residual_risk_hash: String,
    },
    Stopping {
        reason_code: ResidualReasonCodeV1,
        residual_risk_hash: String,
    },
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct GateActionTruthV1 {
    pub prepared_action_id: Uuid,
    pub authority: ArtifactAuthorityV1,
    pub disposition: PreparedActionDisposition,
    pub authorized: bool,
    pub durable_started: bool,
    pub execution_receipt_hash: Option<String>,
    pub landed_reconciled: bool,
    pub oracle_assessment_hash: Option<String>,
    pub reason_code: Option<ResidualReasonCodeV1>,
    pub residual_risk_hash: Option<String>,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct CampaignGateSnapshotV1 {
    pub authority: ArtifactAuthorityV1,
    pub phase: super::state::CampaignPhaseV1,
    pub actions: Vec<GateActionTruthV1>,
    pub denominator: Option<CampaignCoverageDenominatorSealV1>,
    pub coverage_results: Vec<CoverageResultV1>,
    pub fact_delta_bundle_count: u32,
    /// Audit-only. It is intentionally not part of Campaign-local terminality.
    pub fact_delta_consumed: bool,
}

pub fn canonical_conflict_keys(
    mut keys: Vec<ConflictKeyV1>,
) -> Result<Vec<ConflictKeyV1>, VerificationCampaignError> {
    if keys.iter().any(|key| require_hash(&key.key_hash).is_err()) {
        return Err(VerificationCampaignError::new(
            "VERIFICATION_CAMPAIGN_CONFLICT_KEY_INVALID",
            "conflict keys require canonical hashes",
        ));
    }
    keys.sort();
    keys.dedup();
    Ok(keys)
}

pub fn conflict_key_sets_overlap(left: &[ConflictKeyV1], right: &[ConflictKeyV1]) -> bool {
    let left = left.iter().collect::<BTreeSet<_>>();
    right
        .iter()
        .any(|key| left.iter().any(|candidate| *candidate == key))
}

pub fn execution_key_hash(key: &ExecutionKeyV1) -> Result<String, VerificationContractError> {
    if key.prepared_action_id.is_nil() || key.authorization_receipt_id.is_nil() {
        return Err(VerificationContractError::InvalidField("execution key"));
    }
    hash_domain("verification_execution_key.v1", key)
}

pub fn semantic_attempt_fingerprint(
    input: &SemanticAttemptFingerprintInputV1,
) -> Result<String, VerificationContractError> {
    if input.objective_id.is_nil() {
        return Err(VerificationContractError::InvalidField("objective_id"));
    }
    for hash in [
        &input.verification_contract_hash,
        &input.action_contract_digest,
        &input.adapter_contract_digest,
        &input.oracle_rule_digest,
    ] {
        require_hash(hash)?;
    }
    let mut controls = input.required_control_member_hashes.clone();
    canonicalize_hashes(&mut controls)?;
    let mut evidence = input.relevant_evidence_member_hashes.clone();
    canonicalize_hashes(&mut evidence)?;
    hash_domain(
        "semantic_attempt_fingerprint.v1",
        &(
            input.objective_id,
            &input.verification_contract_hash,
            &controls,
            &input.action_contract_digest,
            &input.adapter_contract_digest,
            &input.oracle_rule_digest,
            &evidence,
        ),
    )
}

pub fn validate_objective_action_kind(
    class: ObjectiveExecutionClassV1,
    kind: &PreparedActionKindV1,
) -> Result<(), VerificationCampaignError> {
    match (class, kind) {
        (ObjectiveExecutionClassV1::Standard, PreparedActionKindV1::SingleActionV1)
        | (
            ObjectiveExecutionClassV1::RaceClass,
            PreparedActionKindV1::ConcurrentActionGroupV1(_),
        ) => Ok(()),
        (ObjectiveExecutionClassV1::RaceClass, PreparedActionKindV1::SingleActionV1) => {
            Err(VerificationCampaignError::with_residual(
                "VERIFICATION_CAMPAIGN_CONCURRENT_GROUP_REQUIRED",
                "race objective requires one atomic concurrent action group",
                ResidualReasonCodeV1::RaceAdapterMissing,
            ))
        }
        (ObjectiveExecutionClassV1::Standard, PreparedActionKindV1::ConcurrentActionGroupV1(_)) => {
            Err(VerificationCampaignError::new(
                "VERIFICATION_CAMPAIGN_CONCURRENT_GROUP_UNEXPECTED",
                "standard objective cannot substitute a concurrent group",
            ))
        }
    }
}

pub(crate) fn canonicalize_action_kind(
    kind: &mut PreparedActionKindV1,
) -> Result<(), VerificationContractError> {
    let PreparedActionKindV1::ConcurrentActionGroupV1(group) = kind else {
        return Ok(());
    };
    if group.group_id.is_nil()
        || group.subactions.len() < 2
        || group.max_concurrency < 2
        || group.max_concurrency as usize > group.subactions.len()
        || group.start_window_millis == 0
    {
        return Err(VerificationContractError::InvalidField(
            "concurrent action group",
        ));
    }
    require_hash(&group.barrier_cohort_hash)?;
    require_hash(&group.concurrency_oracle_rule_digest)?;
    group.subactions.sort_by_key(|subaction| subaction.ordinal);
    let unique_subaction_ids = group
        .subactions
        .iter()
        .map(|subaction| subaction.subaction_id)
        .collect::<BTreeSet<_>>();
    if group
        .subactions
        .iter()
        .enumerate()
        .any(|(ordinal, subaction)| {
            subaction.subaction_id.is_nil()
                || subaction.ordinal != ordinal as u32
                || subaction.conflict_keys.is_empty()
        })
        || unique_subaction_ids.len() != group.subactions.len()
    {
        return Err(VerificationContractError::InvalidField(
            "concurrent action group subactions",
        ));
    }
    let mut union = Vec::new();
    for subaction in &mut group.subactions {
        subaction.conflict_keys = canonical_conflict_keys(subaction.conflict_keys.clone())
            .map_err(|_| VerificationContractError::InvalidField("conflict keys"))?;
        union.extend(subaction.conflict_keys.clone());
    }
    union = canonical_conflict_keys(union)
        .map_err(|_| VerificationContractError::InvalidField("conflict keys"))?;
    group.union_conflict_keys = canonical_conflict_keys(group.union_conflict_keys.clone())
        .map_err(|_| VerificationContractError::InvalidField("conflict keys"))?;
    if union.is_empty() || union != group.union_conflict_keys {
        return Err(VerificationContractError::InvalidReference(
            "concurrent action group conflict-key union",
        ));
    }
    Ok(())
}

pub(crate) fn validate_concurrent_group_receipt(
    group: &ConcurrentActionGroupV1,
    receipt: Option<&ConcurrentActionGroupReceiptV1>,
) -> bool {
    let Some(receipt) = receipt else {
        return false;
    };
    if receipt.group_id != group.group_id
        || receipt.barrier_cohort_hash != group.barrier_cohort_hash
        || receipt.subexecutions.len() != group.subactions.len()
    {
        return false;
    }
    let mut actual = receipt.subexecutions.clone();
    actual.sort_by_key(|member| member.ordinal);
    if actual.iter().any(|member| !member.outcome_known)
        || actual
            .iter()
            .zip(&group.subactions)
            .any(|(receipt, expected)| {
                receipt.ordinal != expected.ordinal || receipt.subaction_id != expected.subaction_id
            })
    {
        return false;
    }
    let Some(min) = actual.iter().map(|member| member.start_offset_millis).min() else {
        return false;
    };
    let Some(max) = actual.iter().map(|member| member.start_offset_millis).max() else {
        return false;
    };
    max.saturating_sub(min) <= group.start_window_millis
}

pub(crate) fn require_hash(value: &str) -> Result<(), VerificationContractError> {
    if value.strip_prefix("sha256:").is_some_and(|digest| {
        digest.len() == 64
            && digest
                .bytes()
                .all(|byte| byte.is_ascii_digit() || (b'a'..=b'f').contains(&byte))
    }) {
        Ok(())
    } else {
        Err(VerificationContractError::InvalidHash(
            "verification campaign hash",
        ))
    }
}

pub(crate) fn canonicalize_hashes(hashes: &mut [String]) -> Result<(), VerificationContractError> {
    for hash in hashes.iter() {
        require_hash(hash)?;
    }
    hashes.sort();
    if hashes.windows(2).any(|pair| pair[0] == pair[1]) {
        return Err(VerificationContractError::DuplicateIdentity(
            "verification campaign hash set".to_owned(),
        ));
    }
    Ok(())
}

fn ensure_unique<T: Ord>(
    values: Vec<T>,
    identity: &'static str,
) -> Result<(), VerificationContractError> {
    let count = values.len();
    if values.into_iter().collect::<BTreeSet<_>>().len() != count {
        Err(VerificationContractError::DuplicateIdentity(
            identity.to_owned(),
        ))
    } else {
        Ok(())
    }
}

pub(crate) fn hash_domain<T: Serialize>(
    domain: &'static str,
    value: &T,
) -> Result<String, VerificationContractError> {
    let canonical = serde_json::to_vec(value)
        .map_err(|error| VerificationContractError::InvalidCanonicalJson(error.to_string()))?;
    let mut hasher = Sha256::new();
    hasher.update(domain.as_bytes());
    hasher.update([0]);
    hasher.update((canonical.len() as u64).to_be_bytes());
    hasher.update(canonical);
    Ok(format!(
        "sha256:{}",
        hasher
            .finalize()
            .iter()
            .map(|byte| format!("{byte:02x}"))
            .collect::<String>()
    ))
}

fn hash_domain_campaign<T: Serialize>(
    domain: &'static str,
    value: &T,
) -> Result<String, VerificationCampaignError> {
    hash_domain(domain, value).map_err(|_| {
        VerificationCampaignError::new(
            "VERIFICATION_CAMPAIGN_CANONICAL_HASH_FAILED",
            "canonical campaign hash projection failed",
        )
    })
}

fn exact_set_hash_campaign(
    domain: &'static str,
    hashes: &[String],
) -> Result<String, VerificationCampaignError> {
    let mut hashes = hashes.to_vec();
    canonicalize_hashes(&mut hashes).map_err(|_| {
        VerificationCampaignError::new(
            "VERIFICATION_CAMPAIGN_EXACT_SET_INVALID",
            "campaign exact set is invalid",
        )
    })?;
    hash_domain_campaign(domain, &hashes)
}
