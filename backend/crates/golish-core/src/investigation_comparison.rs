//! Canonical whole-record comparison contract for investigation rollout.
//!
//! `comparison_record.v1` compares complete semantic records. It intentionally
//! has no database row ids, timestamps, worker leases, or presentation prose.
//! Exact-set members are sorted and deduplicated before the record hash is
//! calculated, and an absent side is always `incomplete` rather than a partial
//! field-by-field comparison.

use serde::{Deserialize, Serialize};
use serde_json::Value;
use sha2::{Digest, Sha256};

pub const INVESTIGATION_COMPARISON_RECORD_SCHEMA_V1: &str = "comparison_record.v1";
const SHA256_PREFIX: &str = "sha256:";
const SHA256_HEX_LEN: usize = 64;

#[derive(Debug, Clone, PartialEq, Eq, thiserror::Error)]
pub enum InvestigationComparisonError {
    #[error("comparison record field is not a SHA-256 digest: {0}")]
    InvalidHash(&'static str),
    #[error("comparison record field is blank: {0}")]
    BlankField(&'static str),
    #[error("comparison record exact-set member is not a SHA-256 digest: {0}")]
    InvalidExactSetMember(&'static str),
    #[error("comparison record exact-set is not a subset of its denominator: {0}")]
    ExactSetNotSubset(&'static str),
    #[error("comparison record serialization failed: {0}")]
    Serialization(String),
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize)]
pub struct ComparisonExactSetV1 {
    member_count: u32,
    member_hashes: Vec<String>,
    set_hash: String,
}

impl ComparisonExactSetV1 {
    pub fn seal(
        domain: &'static str,
        mut member_hashes: Vec<String>,
    ) -> Result<Self, InvestigationComparisonError> {
        if member_hashes.iter().any(|value| !is_hash(value)) {
            return Err(InvestigationComparisonError::InvalidExactSetMember(domain));
        }
        member_hashes.sort();
        member_hashes.dedup();
        let set_hash = hash_value(domain, &member_hashes)?;
        Ok(Self {
            member_count: member_hashes.len() as u32,
            member_hashes,
            set_hash,
        })
    }

    pub const fn member_count(&self) -> u32 {
        self.member_count
    }

    pub fn member_hashes(&self) -> &[String] {
        &self.member_hashes
    }

    pub fn set_hash(&self) -> &str {
        &self.set_hash
    }
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum ComparisonHypothesisDispositionV1 {
    Proposed,
    Supported,
    Contested,
    Inconclusive,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum ComparisonHypothesisReadinessV1 {
    PlanningReady,
    ReportingOnlyPlanCUnavailable,
    Blocked,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(tag = "availability", rename_all = "snake_case")]
pub enum PlanCMemberAuthorityV1 {
    NotAvailablePlanC,
    PendingCampaignAdmission,
    Available { member_hash: String },
}

impl PlanCMemberAuthorityV1 {
    fn validate(&self, field: &'static str) -> Result<(), InvestigationComparisonError> {
        match self {
            Self::NotAvailablePlanC | Self::PendingCampaignAdmission => Ok(()),
            Self::Available { member_hash } if is_hash(member_hash) => Ok(()),
            Self::Available { .. } => Err(InvestigationComparisonError::InvalidHash(field)),
        }
    }
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(tag = "availability", rename_all = "snake_case")]
pub enum PlanCExactSetAuthorityInputV1 {
    NotAvailablePlanC,
    PendingCampaignAdmission,
    Available { member_hashes: Vec<String> },
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize)]
#[serde(tag = "availability", rename_all = "snake_case")]
pub enum PlanCExactSetAuthorityV1 {
    NotAvailablePlanC,
    PendingCampaignAdmission,
    Available { exact_set: ComparisonExactSetV1 },
}

impl PlanCExactSetAuthorityInputV1 {
    fn compile(
        self,
        domain: &'static str,
    ) -> Result<PlanCExactSetAuthorityV1, InvestigationComparisonError> {
        match self {
            Self::NotAvailablePlanC => Ok(PlanCExactSetAuthorityV1::NotAvailablePlanC),
            Self::PendingCampaignAdmission => {
                Ok(PlanCExactSetAuthorityV1::PendingCampaignAdmission)
            }
            Self::Available { member_hashes } => Ok(PlanCExactSetAuthorityV1::Available {
                exact_set: ComparisonExactSetV1::seal(domain, member_hashes)?,
            }),
        }
    }
}

/// Every Plan C-owned authority slot is present in V1 even before Plan C is
/// deployed. This prevents a later rollout from changing V1 identity by
/// replacing omitted/null fields with real authority.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct PlanCComparisonAuthorityInputV1 {
    pub capability_assessments: PlanCExactSetAuthorityInputV1,
    pub revision_adjudication: PlanCMemberAuthorityV1,
    pub objective_outcomes: PlanCExactSetAuthorityInputV1,
    pub claim_component_outcomes: PlanCExactSetAuthorityInputV1,
    pub transition_decision: PlanCMemberAuthorityV1,
    pub transition_receipt: PlanCMemberAuthorityV1,
    pub campaign_evidence_members: PlanCExactSetAuthorityInputV1,
    pub oracle_evidence_members: PlanCExactSetAuthorityInputV1,
}

impl PlanCComparisonAuthorityInputV1 {
    pub const fn not_available_plan_c() -> Self {
        Self {
            capability_assessments: PlanCExactSetAuthorityInputV1::NotAvailablePlanC,
            revision_adjudication: PlanCMemberAuthorityV1::NotAvailablePlanC,
            objective_outcomes: PlanCExactSetAuthorityInputV1::NotAvailablePlanC,
            claim_component_outcomes: PlanCExactSetAuthorityInputV1::NotAvailablePlanC,
            transition_decision: PlanCMemberAuthorityV1::NotAvailablePlanC,
            transition_receipt: PlanCMemberAuthorityV1::NotAvailablePlanC,
            campaign_evidence_members: PlanCExactSetAuthorityInputV1::NotAvailablePlanC,
            oracle_evidence_members: PlanCExactSetAuthorityInputV1::NotAvailablePlanC,
        }
    }

    /// Candidate authority is sealed and Plan C is installed, but the
    /// canonical Campaign denominator/admission transaction has not committed
    /// yet. This is intentionally distinct from the historical
    /// `not_available_plan_c` compatibility state.
    pub const fn pending_campaign_admission() -> Self {
        Self {
            capability_assessments: PlanCExactSetAuthorityInputV1::PendingCampaignAdmission,
            revision_adjudication: PlanCMemberAuthorityV1::PendingCampaignAdmission,
            objective_outcomes: PlanCExactSetAuthorityInputV1::PendingCampaignAdmission,
            claim_component_outcomes: PlanCExactSetAuthorityInputV1::PendingCampaignAdmission,
            transition_decision: PlanCMemberAuthorityV1::PendingCampaignAdmission,
            transition_receipt: PlanCMemberAuthorityV1::PendingCampaignAdmission,
            campaign_evidence_members: PlanCExactSetAuthorityInputV1::PendingCampaignAdmission,
            oracle_evidence_members: PlanCExactSetAuthorityInputV1::PendingCampaignAdmission,
        }
    }

    fn compile(self) -> Result<PlanCComparisonAuthorityV1, InvestigationComparisonError> {
        self.revision_adjudication
            .validate("plan_c.revision_adjudication")?;
        self.transition_decision
            .validate("plan_c.transition_decision")?;
        self.transition_receipt
            .validate("plan_c.transition_receipt")?;
        Ok(PlanCComparisonAuthorityV1 {
            capability_assessments: self
                .capability_assessments
                .compile("comparison_record.plan_c.capability_assessments.v1")?,
            revision_adjudication: self.revision_adjudication,
            objective_outcomes: self
                .objective_outcomes
                .compile("comparison_record.plan_c.objective_outcomes.v1")?,
            claim_component_outcomes: self
                .claim_component_outcomes
                .compile("comparison_record.plan_c.claim_component_outcomes.v1")?,
            transition_decision: self.transition_decision,
            transition_receipt: self.transition_receipt,
            campaign_evidence_members: self
                .campaign_evidence_members
                .compile("comparison_record.plan_c.campaign_evidence_members.v1")?,
            oracle_evidence_members: self
                .oracle_evidence_members
                .compile("comparison_record.plan_c.oracle_evidence_members.v1")?,
        })
    }
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize)]
pub struct PlanCComparisonAuthorityV1 {
    pub capability_assessments: PlanCExactSetAuthorityV1,
    pub revision_adjudication: PlanCMemberAuthorityV1,
    pub objective_outcomes: PlanCExactSetAuthorityV1,
    pub claim_component_outcomes: PlanCExactSetAuthorityV1,
    pub transition_decision: PlanCMemberAuthorityV1,
    pub transition_receipt: PlanCMemberAuthorityV1,
    pub campaign_evidence_members: PlanCExactSetAuthorityV1,
    pub oracle_evidence_members: PlanCExactSetAuthorityV1,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct CheckedAuthorityComparisonV1 {
    pub bundle_seal_hash: String,
    pub root_set_hash: String,
    pub bundle_member_set_hash: String,
    pub receipt_set_hash: String,
    pub denominator_graph_bundle_hash: String,
    pub semantic_authority_bundle_hash: String,
    pub freshness_attestation_bundle_hash: String,
    pub temporal_validity_bundle_hash: String,
    pub temporal_validity_policy_set_hash: String,
    pub temporal_validity_decision_set_hash: String,
    pub target_state_epoch_set_hash: String,
    pub observation_window_hash: String,
    pub gate_temporal_reevaluation_hash: String,
}

impl CheckedAuthorityComparisonV1 {
    fn validate(&self) -> Result<(), InvestigationComparisonError> {
        require_hashes(&[
            ("checked_authority.bundle_seal_hash", &self.bundle_seal_hash),
            ("checked_authority.root_set_hash", &self.root_set_hash),
            (
                "checked_authority.bundle_member_set_hash",
                &self.bundle_member_set_hash,
            ),
            ("checked_authority.receipt_set_hash", &self.receipt_set_hash),
            (
                "checked_authority.denominator_graph_bundle_hash",
                &self.denominator_graph_bundle_hash,
            ),
            (
                "checked_authority.semantic_authority_bundle_hash",
                &self.semantic_authority_bundle_hash,
            ),
            (
                "checked_authority.freshness_attestation_bundle_hash",
                &self.freshness_attestation_bundle_hash,
            ),
            (
                "checked_authority.temporal_validity_bundle_hash",
                &self.temporal_validity_bundle_hash,
            ),
            (
                "checked_authority.temporal_validity_policy_set_hash",
                &self.temporal_validity_policy_set_hash,
            ),
            (
                "checked_authority.temporal_validity_decision_set_hash",
                &self.temporal_validity_decision_set_hash,
            ),
            (
                "checked_authority.target_state_epoch_set_hash",
                &self.target_state_epoch_set_hash,
            ),
            (
                "checked_authority.observation_window_hash",
                &self.observation_window_hash,
            ),
            (
                "checked_authority.gate_temporal_reevaluation_hash",
                &self.gate_temporal_reevaluation_hash,
            ),
        ])
    }
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct KnowledgeFeedComparisonV1 {
    pub catalog_policy_seal_hash: String,
    pub required_member_set_hash: String,
    pub signature_algorithm_set_hash: String,
    pub trust_store_hash: String,
    pub key_revocation_epoch_hash: String,
    pub snapshot_set_hash: String,
    pub product_version_census_hash: String,
    pub match_census_hash: String,
    pub source_set_hash: String,
    pub gate_reevaluation_hash: String,
    pub obligation_set_hash: String,
}

impl KnowledgeFeedComparisonV1 {
    fn validate(&self) -> Result<(), InvestigationComparisonError> {
        require_hashes(&[
            (
                "knowledge_feed.catalog_policy_seal_hash",
                &self.catalog_policy_seal_hash,
            ),
            (
                "knowledge_feed.required_member_set_hash",
                &self.required_member_set_hash,
            ),
            (
                "knowledge_feed.signature_algorithm_set_hash",
                &self.signature_algorithm_set_hash,
            ),
            ("knowledge_feed.trust_store_hash", &self.trust_store_hash),
            (
                "knowledge_feed.key_revocation_epoch_hash",
                &self.key_revocation_epoch_hash,
            ),
            ("knowledge_feed.snapshot_set_hash", &self.snapshot_set_hash),
            (
                "knowledge_feed.product_version_census_hash",
                &self.product_version_census_hash,
            ),
            ("knowledge_feed.match_census_hash", &self.match_census_hash),
            ("knowledge_feed.source_set_hash", &self.source_set_hash),
            (
                "knowledge_feed.gate_reevaluation_hash",
                &self.gate_reevaluation_hash,
            ),
            (
                "knowledge_feed.obligation_set_hash",
                &self.obligation_set_hash,
            ),
        ])
    }
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct GenerationComparisonV1 {
    pub generation_ordinal: u32,
    pub generation_seal_hash: String,
    pub generation_member_set_hash: String,
    pub generation_event_set_hash: String,
    pub open_obligation_set_hash: String,
}

impl GenerationComparisonV1 {
    fn validate(&self) -> Result<(), InvestigationComparisonError> {
        require_hashes(&[
            (
                "generation.generation_seal_hash",
                &self.generation_seal_hash,
            ),
            (
                "generation.generation_member_set_hash",
                &self.generation_member_set_hash,
            ),
            (
                "generation.generation_event_set_hash",
                &self.generation_event_set_hash,
            ),
            (
                "generation.open_obligation_set_hash",
                &self.open_obligation_set_hash,
            ),
        ])
    }
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct PlanBCheckedComparisonAuthorityInputV1 {
    pub checked_authority: CheckedAuthorityComparisonV1,
    pub knowledge_feed: KnowledgeFeedComparisonV1,
    pub claim_component_member_hashes: Vec<String>,
    pub verification_contract_member_hashes: Vec<String>,
    pub verification_plan_member_hashes: Vec<String>,
    pub verification_plan_objective_member_hashes: Vec<String>,
    pub verification_plan_path_member_hashes: Vec<String>,
    pub coverage_subreview_member_hashes: Vec<String>,
    pub coverage_synthesis_member_hashes: Vec<String>,
    pub coverage_final_review_member_hashes: Vec<String>,
    pub coverage_checklist_member_hashes: Vec<String>,
    pub sampling_degraded_residual_member_hashes: Vec<String>,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(tag = "basis", rename_all = "snake_case")]
pub enum ComparisonAuthorityBasisInputV1 {
    PlanBChecked {
        #[serde(flatten)]
        authority: Box<PlanBCheckedComparisonAuthorityInputV1>,
    },
    GrandfatheredLegacy {
        adapter_contract_hash: String,
        tool_truth_member_hashes: Vec<String>,
        candidate_plan_member_hashes: Vec<String>,
        coverage_member_hashes: Vec<String>,
    },
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize)]
pub struct PlanBCheckedComparisonAuthorityV1 {
    checked_authority: CheckedAuthorityComparisonV1,
    knowledge_feed: KnowledgeFeedComparisonV1,
    claim_components: ComparisonExactSetV1,
    verification_contracts: ComparisonExactSetV1,
    verification_plans: ComparisonExactSetV1,
    verification_plan_objectives: ComparisonExactSetV1,
    verification_plan_paths: ComparisonExactSetV1,
    coverage_subreviews: ComparisonExactSetV1,
    coverage_synthesis: ComparisonExactSetV1,
    coverage_final_reviews: ComparisonExactSetV1,
    coverage_checklist: ComparisonExactSetV1,
    candidate_hypothesis_coverage_sampling_degraded_residuals: ComparisonExactSetV1,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize)]
#[serde(tag = "basis", rename_all = "snake_case")]
pub enum ComparisonAuthorityBasisV1 {
    PlanBChecked {
        #[serde(flatten)]
        authority: Box<PlanBCheckedComparisonAuthorityV1>,
    },
    GrandfatheredLegacy {
        adapter_contract_hash: String,
        tool_truth: ComparisonExactSetV1,
        candidate_plans: ComparisonExactSetV1,
        coverage: ComparisonExactSetV1,
    },
}

impl ComparisonAuthorityBasisV1 {
    fn is_same_basis_as(&self, other: &Self) -> bool {
        matches!(
            (self, other),
            (Self::PlanBChecked { .. }, Self::PlanBChecked { .. })
                | (
                    Self::GrandfatheredLegacy { .. },
                    Self::GrandfatheredLegacy { .. }
                )
        )
    }
}

impl ComparisonAuthorityBasisInputV1 {
    fn compile(self) -> Result<ComparisonAuthorityBasisV1, InvestigationComparisonError> {
        match self {
            Self::PlanBChecked { authority } => {
                let PlanBCheckedComparisonAuthorityInputV1 {
                    checked_authority,
                    knowledge_feed,
                    claim_component_member_hashes,
                    verification_contract_member_hashes,
                    verification_plan_member_hashes,
                    verification_plan_objective_member_hashes,
                    verification_plan_path_member_hashes,
                    coverage_subreview_member_hashes,
                    coverage_synthesis_member_hashes,
                    coverage_final_review_member_hashes,
                    coverage_checklist_member_hashes,
                    sampling_degraded_residual_member_hashes,
                } = *authority;
                checked_authority.validate()?;
                knowledge_feed.validate()?;
                Ok(ComparisonAuthorityBasisV1::PlanBChecked {
                    authority: Box::new(PlanBCheckedComparisonAuthorityV1 {
                        checked_authority,
                        knowledge_feed,
                        claim_components: ComparisonExactSetV1::seal(
                            "comparison_record.claim_components.v1",
                            claim_component_member_hashes,
                        )?,
                        verification_contracts: ComparisonExactSetV1::seal(
                            "comparison_record.verification_contracts.v1",
                            verification_contract_member_hashes,
                        )?,
                        verification_plans: ComparisonExactSetV1::seal(
                            "comparison_record.verification_plans.v1",
                            verification_plan_member_hashes,
                        )?,
                        verification_plan_objectives: ComparisonExactSetV1::seal(
                            "comparison_record.verification_plan_objectives.v1",
                            verification_plan_objective_member_hashes,
                        )?,
                        verification_plan_paths: ComparisonExactSetV1::seal(
                            "comparison_record.verification_plan_paths.v1",
                            verification_plan_path_member_hashes,
                        )?,
                        coverage_subreviews: ComparisonExactSetV1::seal(
                            "comparison_record.coverage_subreviews.v1",
                            coverage_subreview_member_hashes,
                        )?,
                        coverage_synthesis: ComparisonExactSetV1::seal(
                            "comparison_record.coverage_synthesis.v1",
                            coverage_synthesis_member_hashes,
                        )?,
                        coverage_final_reviews: ComparisonExactSetV1::seal(
                            "comparison_record.coverage_final_reviews.v1",
                            coverage_final_review_member_hashes,
                        )?,
                        coverage_checklist: ComparisonExactSetV1::seal(
                            "comparison_record.coverage_checklist.v1",
                            coverage_checklist_member_hashes,
                        )?,
                        candidate_hypothesis_coverage_sampling_degraded_residuals:
                            ComparisonExactSetV1::seal(
                            "comparison_record.candidate_hypothesis_coverage_sampling_degraded.v1",
                            sampling_degraded_residual_member_hashes,
                        )?,
                    }),
                })
            }
            Self::GrandfatheredLegacy {
                adapter_contract_hash,
                tool_truth_member_hashes,
                candidate_plan_member_hashes,
                coverage_member_hashes,
            } => {
                require_hash(
                    "authority_basis.adapter_contract_hash",
                    &adapter_contract_hash,
                )?;
                Ok(ComparisonAuthorityBasisV1::GrandfatheredLegacy {
                    adapter_contract_hash,
                    tool_truth: ComparisonExactSetV1::seal(
                        "comparison_record.grandfathered_legacy.tool_truth.v1",
                        tool_truth_member_hashes,
                    )?,
                    candidate_plans: ComparisonExactSetV1::seal(
                        "comparison_record.grandfathered_legacy.candidate_plans.v1",
                        candidate_plan_member_hashes,
                    )?,
                    coverage: ComparisonExactSetV1::seal(
                        "comparison_record.grandfathered_legacy.coverage.v1",
                        coverage_member_hashes,
                    )?,
                })
            }
        }
    }

    fn sampling_degraded_member_hashes(&self) -> &[String] {
        match self {
            Self::PlanBChecked { authority } => &authority.sampling_degraded_residual_member_hashes,
            Self::GrandfatheredLegacy { .. } => &[],
        }
    }
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct InvestigationComparisonRecordInputV1 {
    pub semantic_key_hash: String,
    pub revision_ingredients_hash: String,
    pub authority_basis: ComparisonAuthorityBasisInputV1,
    pub generation: GenerationComparisonV1,
    pub disposition: ComparisonHypothesisDispositionV1,
    pub readiness: ComparisonHypothesisReadinessV1,
    pub plan_c: PlanCComparisonAuthorityInputV1,
    pub finding_lineage_member_hashes: Vec<String>,
    pub refutation_lineage_member_hashes: Vec<String>,
    pub residual_member_hashes: Vec<String>,
    pub coverage_member_hashes: Vec<String>,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize)]
pub struct InvestigationComparisonRecordV1 {
    schema: &'static str,
    semantic_key_hash: String,
    revision_ingredients_hash: String,
    authority_basis: ComparisonAuthorityBasisV1,
    generation: GenerationComparisonV1,
    disposition: ComparisonHypothesisDispositionV1,
    readiness: ComparisonHypothesisReadinessV1,
    plan_c: PlanCComparisonAuthorityV1,
    finding_lineage: ComparisonExactSetV1,
    refutation_lineage: ComparisonExactSetV1,
    residual_membership: ComparisonExactSetV1,
    coverage_membership: ComparisonExactSetV1,
    #[serde(skip)]
    record_hash: String,
}

impl InvestigationComparisonRecordV1 {
    pub fn compile(
        input: InvestigationComparisonRecordInputV1,
    ) -> Result<Self, InvestigationComparisonError> {
        require_hash("semantic_key_hash", &input.semantic_key_hash)?;
        require_hash(
            "revision_ingredients_hash",
            &input.revision_ingredients_hash,
        )?;
        input.generation.validate()?;

        let residual_membership = ComparisonExactSetV1::seal(
            "comparison_record.residual_membership.v1",
            input.residual_member_hashes,
        )?;
        if input
            .authority_basis
            .sampling_degraded_member_hashes()
            .iter()
            .any(|member| !residual_membership.member_hashes().contains(member))
        {
            return Err(InvestigationComparisonError::ExactSetNotSubset(
                "candidate_hypothesis_coverage_sampling_degraded",
            ));
        }

        let mut record = Self {
            schema: INVESTIGATION_COMPARISON_RECORD_SCHEMA_V1,
            semantic_key_hash: input.semantic_key_hash,
            revision_ingredients_hash: input.revision_ingredients_hash,
            authority_basis: input.authority_basis.compile()?,
            generation: input.generation,
            disposition: input.disposition,
            readiness: input.readiness,
            plan_c: input.plan_c.compile()?,
            finding_lineage: ComparisonExactSetV1::seal(
                "comparison_record.finding_lineage.v1",
                input.finding_lineage_member_hashes,
            )?,
            refutation_lineage: ComparisonExactSetV1::seal(
                "comparison_record.refutation_lineage.v1",
                input.refutation_lineage_member_hashes,
            )?,
            residual_membership,
            coverage_membership: ComparisonExactSetV1::seal(
                "comparison_record.coverage_membership.v1",
                input.coverage_member_hashes,
            )?,
            record_hash: String::new(),
        };
        record.record_hash = hash_bytes(
            INVESTIGATION_COMPARISON_RECORD_SCHEMA_V1,
            &record.canonical_bytes()?,
        );
        Ok(record)
    }

    pub const fn schema(&self) -> &'static str {
        self.schema
    }

    pub fn record_hash(&self) -> &str {
        &self.record_hash
    }

    pub fn canonical_bytes(&self) -> Result<Vec<u8>, InvestigationComparisonError> {
        let value = serde_json::to_value(self)
            .map_err(|error| InvestigationComparisonError::Serialization(error.to_string()))?;
        serde_json::to_vec(&canonicalize_value(value))
            .map_err(|error| InvestigationComparisonError::Serialization(error.to_string()))
    }

    pub fn canonical_json(&self) -> Result<String, InvestigationComparisonError> {
        String::from_utf8(self.canonical_bytes()?)
            .map_err(|error| InvestigationComparisonError::Serialization(error.to_string()))
    }
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize)]
#[serde(rename_all = "snake_case")]
pub enum WholeRecordComparisonStateV1 {
    Match,
    Mismatch,
    Incomplete,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize)]
pub struct WholeRecordComparisonV1 {
    pub state: WholeRecordComparisonStateV1,
    pub legacy_hash: Option<String>,
    pub registry_hash: Option<String>,
}

/// Compare two complete records. There is deliberately no map of individual
/// fields and no fallback path that combines one side with data from the other.
pub fn compare_whole_records_v1(
    legacy: Option<&InvestigationComparisonRecordV1>,
    registry: Option<&InvestigationComparisonRecordV1>,
) -> WholeRecordComparisonV1 {
    let legacy_hash = legacy.map(|record| record.record_hash.clone());
    let registry_hash = registry.map(|record| record.record_hash.clone());
    let state = match (legacy, registry) {
        (Some(legacy), Some(registry))
            if !legacy
                .authority_basis
                .is_same_basis_as(&registry.authority_basis) =>
        {
            WholeRecordComparisonStateV1::Incomplete
        }
        (Some(legacy), Some(registry)) if legacy.record_hash == registry.record_hash => {
            WholeRecordComparisonStateV1::Match
        }
        (Some(_), Some(_)) => WholeRecordComparisonStateV1::Mismatch,
        _ => WholeRecordComparisonStateV1::Incomplete,
    };
    WholeRecordComparisonV1 {
        state,
        legacy_hash,
        registry_hash,
    }
}

fn require_hash(field: &'static str, value: &str) -> Result<(), InvestigationComparisonError> {
    if is_hash(value) {
        Ok(())
    } else {
        Err(InvestigationComparisonError::InvalidHash(field))
    }
}

fn require_hashes(fields: &[(&'static str, &String)]) -> Result<(), InvestigationComparisonError> {
    for (field, value) in fields {
        require_hash(field, value)?;
    }
    Ok(())
}

fn is_hash(value: &str) -> bool {
    value.len() == SHA256_PREFIX.len() + SHA256_HEX_LEN
        && value.starts_with(SHA256_PREFIX)
        && value[SHA256_PREFIX.len()..]
            .bytes()
            .all(|byte| byte.is_ascii_digit() || (b'a'..=b'f').contains(&byte))
}

fn hash_value<T: Serialize>(
    domain: &'static str,
    value: &T,
) -> Result<String, InvestigationComparisonError> {
    let value = serde_json::to_value(value)
        .map_err(|error| InvestigationComparisonError::Serialization(error.to_string()))?;
    let bytes = serde_json::to_vec(&canonicalize_value(value))
        .map_err(|error| InvestigationComparisonError::Serialization(error.to_string()))?;
    Ok(hash_bytes(domain, &bytes))
}

fn hash_bytes(domain: &str, bytes: &[u8]) -> String {
    let mut hasher = Sha256::new();
    hasher.update(domain.as_bytes());
    hasher.update([0]);
    hasher.update(bytes);
    format!("sha256:{}", hex_lower(&hasher.finalize()))
}

fn hex_lower(bytes: &[u8]) -> String {
    const HEX: &[u8; 16] = b"0123456789abcdef";
    let mut output = String::with_capacity(bytes.len() * 2);
    for byte in bytes {
        output.push(HEX[(byte >> 4) as usize] as char);
        output.push(HEX[(byte & 0x0f) as usize] as char);
    }
    output
}

fn canonicalize_value(value: Value) -> Value {
    match value {
        Value::Array(values) => Value::Array(values.into_iter().map(canonicalize_value).collect()),
        Value::Object(values) => {
            let mut entries = values.into_iter().collect::<Vec<_>>();
            entries.sort_by(|left, right| left.0.cmp(&right.0));
            Value::Object(
                entries
                    .into_iter()
                    .map(|(key, value)| (key, canonicalize_value(value)))
                    .collect(),
            )
        }
        scalar => scalar,
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn hash(byte: char) -> String {
        format!("sha256:{}", byte.to_string().repeat(64))
    }

    fn hash_index(index: u64) -> String {
        format!("sha256:{index:064x}")
    }

    fn fixture() -> InvestigationComparisonRecordInputV1 {
        let authority_hashes = (0..13).map(hash_index).collect::<Vec<_>>();
        let feed_hashes = (0..11)
            .map(|index| hash_index(index + 20))
            .collect::<Vec<_>>();
        InvestigationComparisonRecordInputV1 {
            semantic_key_hash: hash('a'),
            revision_ingredients_hash: hash('b'),
            authority_basis: ComparisonAuthorityBasisInputV1::PlanBChecked {
                authority: Box::new(PlanBCheckedComparisonAuthorityInputV1 {
                    checked_authority: CheckedAuthorityComparisonV1 {
                        bundle_seal_hash: authority_hashes[0].clone(),
                        root_set_hash: authority_hashes[1].clone(),
                        bundle_member_set_hash: authority_hashes[2].clone(),
                        receipt_set_hash: authority_hashes[3].clone(),
                        denominator_graph_bundle_hash: authority_hashes[4].clone(),
                        semantic_authority_bundle_hash: authority_hashes[5].clone(),
                        freshness_attestation_bundle_hash: authority_hashes[6].clone(),
                        temporal_validity_bundle_hash: authority_hashes[7].clone(),
                        temporal_validity_policy_set_hash: authority_hashes[8].clone(),
                        temporal_validity_decision_set_hash: authority_hashes[9].clone(),
                        target_state_epoch_set_hash: authority_hashes[10].clone(),
                        observation_window_hash: authority_hashes[11].clone(),
                        gate_temporal_reevaluation_hash: authority_hashes[12].clone(),
                    },
                    knowledge_feed: KnowledgeFeedComparisonV1 {
                        catalog_policy_seal_hash: feed_hashes[0].clone(),
                        required_member_set_hash: feed_hashes[1].clone(),
                        signature_algorithm_set_hash: feed_hashes[2].clone(),
                        trust_store_hash: feed_hashes[3].clone(),
                        key_revocation_epoch_hash: feed_hashes[4].clone(),
                        snapshot_set_hash: feed_hashes[5].clone(),
                        product_version_census_hash: feed_hashes[6].clone(),
                        match_census_hash: feed_hashes[7].clone(),
                        source_set_hash: feed_hashes[8].clone(),
                        gate_reevaluation_hash: feed_hashes[9].clone(),
                        obligation_set_hash: feed_hashes[10].clone(),
                    },
                    claim_component_member_hashes: vec![hash('c'), hash('a'), hash('c')],
                    verification_contract_member_hashes: vec![hash('d')],
                    verification_plan_member_hashes: vec![hash('8')],
                    verification_plan_objective_member_hashes: vec![hash('e')],
                    verification_plan_path_member_hashes: vec![hash('f')],
                    coverage_subreview_member_hashes: vec![hash('1')],
                    coverage_synthesis_member_hashes: vec![hash('2')],
                    coverage_final_review_member_hashes: vec![hash('3')],
                    coverage_checklist_member_hashes: vec![hash('4')],
                    sampling_degraded_residual_member_hashes: vec![hash('5')],
                }),
            },
            generation: GenerationComparisonV1 {
                generation_ordinal: 2,
                generation_seal_hash: hash('c'),
                generation_member_set_hash: hash('d'),
                generation_event_set_hash: hash('e'),
                open_obligation_set_hash: hash('f'),
            },
            disposition: ComparisonHypothesisDispositionV1::Supported,
            readiness: ComparisonHypothesisReadinessV1::ReportingOnlyPlanCUnavailable,
            plan_c: PlanCComparisonAuthorityInputV1::not_available_plan_c(),
            finding_lineage_member_hashes: vec![],
            refutation_lineage_member_hashes: vec![],
            residual_member_hashes: vec![hash('5'), hash('6')],
            coverage_member_hashes: vec![hash('7')],
        }
    }

    #[test]
    fn comparison_record_canonicalizes_exact_sets_deterministically() {
        let left = InvestigationComparisonRecordV1::compile(fixture()).expect("fixture compiles");
        let mut reordered = fixture();
        let ComparisonAuthorityBasisInputV1::PlanBChecked { authority } =
            &mut reordered.authority_basis
        else {
            panic!("fixture uses Plan B authority");
        };
        authority.claim_component_member_hashes = vec![hash('a'), hash('c')];
        let right = InvestigationComparisonRecordV1::compile(reordered).expect("fixture compiles");

        assert_eq!(left.record_hash(), right.record_hash());
        assert_eq!(
            left.canonical_bytes().unwrap(),
            right.canonical_bytes().unwrap()
        );
        let ComparisonAuthorityBasisV1::PlanBChecked { authority } = &left.authority_basis else {
            panic!("fixture compiles as Plan B authority");
        };
        assert_eq!(authority.claim_components.member_count(), 2);
    }

    #[test]
    fn comparison_record_hash_changes_on_semantic_field_drift() {
        let left = InvestigationComparisonRecordV1::compile(fixture()).expect("fixture compiles");
        let mut drifted = fixture();
        let ComparisonAuthorityBasisInputV1::PlanBChecked { authority } =
            &mut drifted.authority_basis
        else {
            panic!("fixture uses Plan B authority");
        };
        authority.knowledge_feed.trust_store_hash = hash('9');
        let right = InvestigationComparisonRecordV1::compile(drifted).expect("fixture compiles");

        assert_ne!(left.record_hash(), right.record_hash());
        assert_eq!(
            compare_whole_records_v1(Some(&left), Some(&right)).state,
            WholeRecordComparisonStateV1::Mismatch
        );
    }

    #[test]
    fn comparison_record_rejects_cross_basis_as_incomplete() {
        let plan_b = InvestigationComparisonRecordV1::compile(fixture()).expect("fixture compiles");
        let mut legacy_input = fixture();
        legacy_input.authority_basis = ComparisonAuthorityBasisInputV1::GrandfatheredLegacy {
            adapter_contract_hash: hash('0'),
            tool_truth_member_hashes: vec![hash('1')],
            candidate_plan_member_hashes: vec![hash('2')],
            coverage_member_hashes: vec![hash('3')],
        };
        let legacy = InvestigationComparisonRecordV1::compile(legacy_input)
            .expect("grandfathered fixture compiles");

        let result = compare_whole_records_v1(Some(&legacy), Some(&plan_b));
        assert_eq!(result.state, WholeRecordComparisonStateV1::Incomplete);
        assert!(result.legacy_hash.is_some());
        assert!(result.registry_hash.is_some());
    }

    #[test]
    fn comparison_record_plan_c_unavailable_is_typed_not_null() {
        let record = InvestigationComparisonRecordV1::compile(fixture()).expect("fixture compiles");
        let json = record.canonical_json().expect("record serializes");

        assert_eq!(json.matches("not_available_plan_c").count(), 8);
        assert!(!json.contains(":null"));
        assert!(json.contains("candidate_hypothesis_coverage_sampling_degraded"));
    }

    #[test]
    fn comparison_record_plan_c_pending_admission_is_not_unavailable() {
        let mut input = fixture();
        input.readiness = ComparisonHypothesisReadinessV1::PlanningReady;
        input.plan_c = PlanCComparisonAuthorityInputV1::pending_campaign_admission();
        let record = InvestigationComparisonRecordV1::compile(input).expect("fixture compiles");
        let json = record.canonical_json().expect("record serializes");

        assert_eq!(json.matches("pending_campaign_admission").count(), 8);
        assert!(!json.contains("not_available_plan_c"));
        assert!(!json.contains(":null"));
    }

    #[test]
    fn comparison_record_whole_record_missing_is_incomplete() {
        let complete =
            InvestigationComparisonRecordV1::compile(fixture()).expect("fixture compiles");

        let result = compare_whole_records_v1(Some(&complete), None);
        assert_eq!(result.state, WholeRecordComparisonStateV1::Incomplete);
        assert_eq!(result.legacy_hash.as_deref(), Some(complete.record_hash()));
        assert_eq!(result.registry_hash, None);
    }

    #[test]
    fn comparison_record_sampling_residual_must_belong_to_residual_denominator() {
        let mut invalid = fixture();
        invalid.residual_member_hashes.clear();

        assert_eq!(
            InvestigationComparisonRecordV1::compile(invalid),
            Err(InvestigationComparisonError::ExactSetNotSubset(
                "candidate_hypothesis_coverage_sampling_degraded"
            ))
        );
    }

    #[test]
    fn comparison_record_golden_hash_is_stable() {
        let record = InvestigationComparisonRecordV1::compile(fixture()).expect("fixture compiles");
        assert_eq!(
            record.record_hash(),
            "sha256:78ef6fe6d596096e54f9b8e2d94ee056f8df8e4748bd6e31d5edb386fb307699"
        );
    }
}
