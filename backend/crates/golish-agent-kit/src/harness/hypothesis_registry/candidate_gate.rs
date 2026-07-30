//! Pure, fail-closed Candidate analysis Gate.
//!
//! The types in this module intentionally do not implement `Deserialize`.
//! Model artifacts are parsed separately and cannot carry authority hashes,
//! exact-set seals, coverage receipts, or compiled verification authority.

use std::collections::{BTreeMap, BTreeSet};

use golish_core::hypothesis_semantic_key::CandidateMutationEpistemicState;
use golish_core::hypothesis_verification::{
    HypothesisClaimComponentV1, HypothesisVerificationPlanV1,
};
use golish_core::verification_contract::VerificationContractV1;
use golish_pentest_domain::tool_truth::{TemporalValidityStatus, ToolTruthRootFamilyV1};
use serde::Deserialize;
use sha2::{Digest, Sha256};
use uuid::Uuid;

use super::rollout::{candidate_mutation_state, CandidateAuthoritySnapshotDispositionV1};
use super::verification_contract_compiler::validate_compiled_contract_set;
use super::verification_plan_compiler::validate_compiled_plan_set;

const HASH_PREFIX: &str = "sha256:";

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum CandidateGateBlockKind {
    AuthorityBundle,
    TemporalValidity,
    KnowledgeFeed,
    AttemptChain,
    ChunkCensus,
    ReadReceipt,
    ProposalCensus,
    CoverageSubreview,
    CoverageSynthesis,
    CoverageReview,
    ControllerDecision,
    SemanticReducer,
    ClaimComponent,
    VerificationContract,
    VerificationPlan,
    GenerationTransition,
    InputDisposition,
    FinalSubmitter,
    TerminalStateForbidden,
    InvalidStateServerOnly,
    ApplicationContextIsNotProof,
    KnowledgeSignalIsNotProof,
    GapIsNotRefutation,
}

#[derive(Debug, Clone, PartialEq, Eq, thiserror::Error)]
#[error("{kind:?}: {detail}")]
pub struct CandidateGateBlock {
    kind: CandidateGateBlockKind,
    detail: &'static str,
}

impl CandidateGateBlock {
    const fn new(kind: CandidateGateBlockKind, detail: &'static str) -> Self {
        Self { kind, detail }
    }

    pub const fn kind(&self) -> CandidateGateBlockKind {
        self.kind
    }

    pub const fn code(&self) -> &'static str {
        match self.kind {
            CandidateGateBlockKind::AuthorityBundle => {
                "HYPOTHESIS_CANDIDATE_AUTHORITY_BUNDLE_INVALID"
            }
            CandidateGateBlockKind::TemporalValidity => {
                "HYPOTHESIS_CANDIDATE_TEMPORAL_VALIDITY_INVALID"
            }
            CandidateGateBlockKind::KnowledgeFeed => "HYPOTHESIS_CANDIDATE_KNOWLEDGE_FEED_INVALID",
            CandidateGateBlockKind::AttemptChain => "HYPOTHESIS_CANDIDATE_ATTEMPT_CHAIN_INVALID",
            CandidateGateBlockKind::ChunkCensus => "HYPOTHESIS_CANDIDATE_CHUNK_CENSUS_INVALID",
            CandidateGateBlockKind::ReadReceipt => "HYPOTHESIS_CANDIDATE_READ_RECEIPT_INVALID",
            CandidateGateBlockKind::ProposalCensus => {
                "HYPOTHESIS_CANDIDATE_PROPOSAL_CENSUS_INVALID"
            }
            CandidateGateBlockKind::CoverageSubreview => {
                "HYPOTHESIS_CANDIDATE_COVERAGE_SUBREVIEW_INVALID"
            }
            CandidateGateBlockKind::CoverageSynthesis => {
                "HYPOTHESIS_CANDIDATE_COVERAGE_SYNTHESIS_INVALID"
            }
            CandidateGateBlockKind::CoverageReview => {
                "HYPOTHESIS_CANDIDATE_COVERAGE_REVIEW_INVALID"
            }
            CandidateGateBlockKind::ControllerDecision => {
                "HYPOTHESIS_CANDIDATE_CONTROLLER_DECISION_INVALID"
            }
            CandidateGateBlockKind::SemanticReducer => {
                "HYPOTHESIS_CANDIDATE_SEMANTIC_REDUCER_INVALID"
            }
            CandidateGateBlockKind::ClaimComponent => {
                "HYPOTHESIS_CLAIM_COMPONENT_EXACT_SET_INVALID"
            }
            CandidateGateBlockKind::VerificationContract => {
                "HYPOTHESIS_VERIFICATION_CONTRACT_EXACT_SET_INVALID"
            }
            CandidateGateBlockKind::VerificationPlan => {
                "HYPOTHESIS_VERIFICATION_PLAN_EXACT_SET_INVALID"
            }
            CandidateGateBlockKind::GenerationTransition => {
                "HYPOTHESIS_GENERATION_TRANSITION_EXACT_SET_INVALID"
            }
            CandidateGateBlockKind::InputDisposition => {
                "HYPOTHESIS_INPUT_DISPOSITION_EXACT_SET_INVALID"
            }
            CandidateGateBlockKind::FinalSubmitter => {
                "HYPOTHESIS_CANDIDATE_FINAL_SUBMITTER_INVALID"
            }
            CandidateGateBlockKind::TerminalStateForbidden => {
                "HYPOTHESIS_CANDIDATE_TERMINAL_STATE_FORBIDDEN"
            }
            CandidateGateBlockKind::InvalidStateServerOnly => {
                "HYPOTHESIS_INVALID_STATE_SERVER_ONLY"
            }
            CandidateGateBlockKind::ApplicationContextIsNotProof => {
                "HYPOTHESIS_APPLICATION_CONTEXT_IS_NOT_PROOF"
            }
            CandidateGateBlockKind::KnowledgeSignalIsNotProof => {
                "HYPOTHESIS_KNOWLEDGE_SIGNAL_IS_NOT_PROOF"
            }
            CandidateGateBlockKind::GapIsNotRefutation => "HYPOTHESIS_GAP_IS_NOT_REFUTATION",
        }
    }
}

/// A domain-separated exact-set seal. Expected and observed members remain
/// separate so a caller cannot turn omission into an apparently valid set.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct CandidateExactSetSealV1 {
    pub domain: &'static str,
    pub expected_member_hashes: Vec<String>,
    pub observed_member_hashes: Vec<String>,
    pub observed_set_hash: String,
}

impl CandidateExactSetSealV1 {
    pub fn seal(domain: &'static str, members: Vec<String>) -> Self {
        let members = canonical_members(members);
        Self {
            domain,
            expected_member_hashes: members.clone(),
            observed_set_hash: exact_set_hash(domain, &members),
            observed_member_hashes: members,
        }
    }

    fn validate(
        &self,
        expected_domain: &'static str,
        kind: CandidateGateBlockKind,
    ) -> Result<(), CandidateGateBlock> {
        if self.domain != expected_domain
            || has_duplicates(&self.expected_member_hashes)
            || has_duplicates(&self.observed_member_hashes)
            || canonical_members(self.expected_member_hashes.clone())
                != canonical_members(self.observed_member_hashes.clone())
            || exact_set_hash(
                self.domain,
                &canonical_members(self.observed_member_hashes.clone()),
            ) != self.observed_set_hash
        {
            return Err(CandidateGateBlock::new(
                kind,
                "exact set is open or drifted",
            ));
        }
        Ok(())
    }
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct CandidateAuthorityRootGateV1 {
    pub root_family: ToolTruthRootFamilyV1,
    pub graph_hash: String,
    pub semantic_hash: String,
    pub freshness_hash: String,
    pub temporal_hash: String,
    pub target_state_epoch_hash: String,
    pub temporal_status: TemporalValidityStatus,
    pub member_hash: String,
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct CandidateAuthorityGateV1 {
    pub disposition: CandidateAuthoritySnapshotDispositionV1,
    pub bundle_seal_id: Uuid,
    pub operation_id: Uuid,
    pub organization_id: Uuid,
    pub checked_request_id: Uuid,
    pub gate_request_id: Uuid,
    pub caller_filtered_or_reused_guard: bool,
    pub old_consistent_row_used: bool,
    pub root_set: CandidateExactSetSealV1,
    pub bundle_member_set: CandidateExactSetSealV1,
    pub receipt_set: CandidateExactSetSealV1,
    pub temporal_decision_set: CandidateExactSetSealV1,
    pub roots: Vec<CandidateAuthorityRootGateV1>,
    pub current_target_state_epoch_set_hash: String,
    pub snapshot_target_state_epoch_set_hash: String,
    pub gate_temporal_reevaluation_hash: String,
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct CandidateKnowledgeFeedMemberV1 {
    pub member_hash: String,
    pub product_version_known: bool,
    pub signature_valid: bool,
    pub provenance_valid: bool,
    pub age_valid_at_gate: bool,
    pub key_current_and_not_revoked: bool,
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct CandidateKnowledgeFeedGateV1 {
    pub required_member_set: CandidateExactSetSealV1,
    pub signed_snapshot_set: CandidateExactSetSealV1,
    pub product_version_census: CandidateExactSetSealV1,
    pub match_census: CandidateExactSetSealV1,
    pub signature_algorithm_set: CandidateExactSetSealV1,
    pub members: Vec<CandidateKnowledgeFeedMemberV1>,
    pub catalog_policy_seal_hash: String,
    pub trust_store_hash: String,
    pub snapshot_trust_store_hash: String,
    pub key_revocation_epoch_hash: String,
    pub snapshot_key_revocation_epoch_hash: String,
    pub gate_reevaluation_hash: String,
    pub obligation_set_hash: String,
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct PriorCandidateAttemptV1 {
    pub attempt_id: Uuid,
    pub ordinal: u32,
    pub terminal: bool,
    pub member_hash: String,
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct CandidateAttemptGateV1 {
    pub active_attempt_id: Uuid,
    pub active_attempt_ordinal: u32,
    pub active_attempt_unique: bool,
    pub prior_attempts: Vec<PriorCandidateAttemptV1>,
    pub prior_terminal_attempt_chain_hash: String,
    pub material_attempt_ids: Vec<Uuid>,
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct CandidateReadGateV1 {
    pub input_set: CandidateExactSetSealV1,
    pub chunk_set: CandidateExactSetSealV1,
    pub page_receipt_set: CandidateExactSetSealV1,
    pub server_read_receipt_set: CandidateExactSetSealV1,
    pub source_bytes_complete: bool,
    pub context_truncated: bool,
    pub caller_claimed_read_complete: bool,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum CandidateCoverageOutcomeV1 {
    Adequate,
    MissedHypothesis,
    Blocked,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum CandidateCoverageSynthesisNodeKindV1 {
    CrossChunk,
    CrossInputPartition,
    CrossInputReduce,
    CrossDimensionReduce,
    GlobalSemanticRoot,
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct CandidateCoverageSynthesisNodeV1 {
    pub node_hash: String,
    pub node_kind: CandidateCoverageSynthesisNodeKindV1,
    pub expected_child_hashes: Vec<String>,
    pub observed_child_hashes: Vec<String>,
    pub worker_run_id: Uuid,
    pub primary_analyst_worker_run_ids: Vec<Uuid>,
    pub transitive_descendant_worker_run_ids: Vec<Uuid>,
    pub outcome: CandidateCoverageOutcomeV1,
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct CandidateCoverageGateV1 {
    pub h1_proposal_set: CandidateExactSetSealV1,
    pub per_input_h1_disposition_set: CandidateExactSetSealV1,
    pub checklist_member_set: CandidateExactSetSealV1,
    pub chunk_partition_set: CandidateExactSetSealV1,
    pub expected_subreview_set: CandidateExactSetSealV1,
    pub observed_subreview_set: CandidateExactSetSealV1,
    pub synthesis_node_set: CandidateExactSetSealV1,
    pub synthesis_nodes: Vec<CandidateCoverageSynthesisNodeV1>,
    pub per_input_review_set: CandidateExactSetSealV1,
    pub h2_proposal_set: CandidateExactSetSealV1,
    pub global_review_hash: String,
    pub global_review_outcome: CandidateCoverageOutcomeV1,
    pub unresolved_feed_dependent_checklist_members: u32,
    pub missed_hypothesis: bool,
    pub sampling_used: bool,
    pub retry_limit_reached: bool,
}

#[derive(Debug, Clone, PartialEq, Eq, Deserialize)]
#[serde(tag = "kind", content = "id", rename_all = "snake_case")]
pub enum RevisionSourceRef {
    ToolTruthEvidence(String),
    Finding(String),
    VerificationReceipt(String),
    ApplicationContext(String),
    KnowledgeSignal(String),
    Gap(String),
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct CandidateHypothesisMutation {
    pub proposal_id: Uuid,
    pub organization_id: Uuid,
    pub semantic_key_hash: String,
    pub operator_rank: u8,
    pub state: CandidateMutationEpistemicState,
    pub proof_refs: Vec<RevisionSourceRef>,
    pub refutation_refs: Vec<RevisionSourceRef>,
    pub generation_transition_hash: String,
    pub mutation_hash: String,
}

#[derive(Debug, Deserialize)]
#[serde(deny_unknown_fields)]
struct ControllerMutationArtifact {
    proposal_id: Uuid,
    organization_id: Uuid,
    semantic_key_hash: String,
    operator_rank: u8,
    state: String,
    #[serde(default)]
    proof_refs: Vec<RevisionSourceRef>,
    #[serde(default)]
    refutation_refs: Vec<RevisionSourceRef>,
    generation_transition_hash: String,
}

impl CandidateHypothesisMutation {
    pub fn parse_controller_artifact(value: serde_json::Value) -> Result<Self, CandidateGateBlock> {
        let artifact: ControllerMutationArtifact = serde_json::from_value(value).map_err(|_| {
            CandidateGateBlock::new(
                CandidateGateBlockKind::ControllerDecision,
                "controller mutation artifact is malformed",
            )
        })?;
        let state = candidate_mutation_state(&artifact.state).map_err(|error| match error {
            super::rollout::CandidateMutationError::TerminalStateForbidden => {
                CandidateGateBlock::new(
                    CandidateGateBlockKind::TerminalStateForbidden,
                    "Candidate cannot write terminal truth",
                )
            }
            super::rollout::CandidateMutationError::InvalidStateServerOnly => {
                CandidateGateBlock::new(
                    CandidateGateBlockKind::InvalidStateServerOnly,
                    "invalid is server-only",
                )
            }
            super::rollout::CandidateMutationError::Unknown(_) => CandidateGateBlock::new(
                CandidateGateBlockKind::ControllerDecision,
                "unknown Candidate state",
            ),
        })?;
        let mut mutation = Self {
            proposal_id: artifact.proposal_id,
            organization_id: artifact.organization_id,
            semantic_key_hash: artifact.semantic_key_hash,
            operator_rank: artifact.operator_rank,
            state,
            proof_refs: artifact.proof_refs,
            refutation_refs: artifact.refutation_refs,
            generation_transition_hash: artifact.generation_transition_hash,
            mutation_hash: String::new(),
        };
        mutation.mutation_hash = mutation.canonical_hash();
        Ok(mutation)
    }

    pub fn reseal(&mut self) {
        self.mutation_hash = self.canonical_hash();
    }

    fn canonical_hash(&self) -> String {
        let proof = self
            .proof_refs
            .iter()
            .map(source_ref_key)
            .collect::<Vec<_>>();
        let refutations = self
            .refutation_refs
            .iter()
            .map(source_ref_key)
            .collect::<Vec<_>>();
        hash_parts(
            "candidate_hypothesis_mutation.v1",
            &[
                self.proposal_id.to_string(),
                self.organization_id.to_string(),
                self.semantic_key_hash.clone(),
                self.operator_rank.to_string(),
                self.state.as_str().to_owned(),
                proof.join("\u{1f}"),
                refutations.join("\u{1f}"),
                self.generation_transition_hash.clone(),
            ],
        )
    }
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum InputProcessingDispositionV1 {
    Analyzed,
    Informational,
    DuplicateInput,
    NotSecurityRelevant,
    Gap,
    Blocked,
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct InputProcessingDispositionDecision {
    pub input_id: Uuid,
    pub disposition: InputProcessingDispositionV1,
    pub decision_hash: String,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum InputHypothesisRelationKindV1 {
    CreatesHypothesis,
    SupportsExisting,
    ContradictsExisting,
    QualifiesExisting,
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct InputHypothesisRelationDecision {
    pub input_id: Uuid,
    pub hypothesis_root_id: Uuid,
    pub relation: InputHypothesisRelationKindV1,
    pub decision_hash: String,
}

#[derive(Debug, Clone)]
pub struct CandidateCompiledAuthorityV1 {
    pub claim_components: Vec<HypothesisClaimComponentV1>,
    pub claim_component_set: CandidateExactSetSealV1,
    pub verification_contracts: Vec<VerificationContractV1>,
    pub verification_contract_set: CandidateExactSetSealV1,
    pub verification_plans: Vec<HypothesisVerificationPlanV1>,
    pub verification_plan_set: CandidateExactSetSealV1,
}

#[derive(Debug, Clone)]
pub struct FrozenCandidateGateMaterialV1 {
    pub snapshot_id: Uuid,
    pub snapshot_hash: String,
    pub candidate_snapshot_authority_hash: String,
    pub operation_id: Uuid,
    pub organization_id: Uuid,
    pub authority: CandidateAuthorityGateV1,
    pub knowledge_feed: CandidateKnowledgeFeedGateV1,
    pub attempt: CandidateAttemptGateV1,
    pub read: CandidateReadGateV1,
    pub coverage: CandidateCoverageGateV1,
    pub proposal_census: CandidateExactSetSealV1,
    pub critic_census: CandidateExactSetSealV1,
    pub controller_decision_set: CandidateExactSetSealV1,
    pub mutations: Vec<CandidateHypothesisMutation>,
    pub mutation_set: CandidateExactSetSealV1,
    pub compiled: CandidateCompiledAuthorityV1,
    pub input_dispositions: Vec<InputProcessingDispositionDecision>,
    pub input_disposition_set: CandidateExactSetSealV1,
    pub input_relations: Vec<InputHypothesisRelationDecision>,
    pub input_relation_set: CandidateExactSetSealV1,
    pub generation_transition_set: CandidateExactSetSealV1,
    pub planning_ready: bool,
    pub capability_assessment_present: bool,
    pub final_submitter_worker_run_id: Uuid,
    pub controller_worker_run_id: Uuid,
}

/// Opaque wrapper consumed by the Gate. The repository/bridge constructs this
/// from locked frozen rows; it has no serde or controller-artifact path.
#[derive(Debug, Clone)]
pub struct CandidateGateSnapshot(FrozenCandidateGateMaterialV1);

impl CandidateGateSnapshot {
    #[doc(hidden)]
    pub fn from_repository_material(material: FrozenCandidateGateMaterialV1) -> Self {
        Self(material)
    }

    #[doc(hidden)]
    pub fn material_for_host_tests_mut(&mut self) -> &mut FrozenCandidateGateMaterialV1 {
        &mut self.0
    }
}

#[derive(Debug, Clone)]
pub struct CandidateGatePass {
    pub snapshot_id: Uuid,
    pub snapshot_hash: String,
    pub candidate_snapshot_authority_hash: String,
    pub tool_truth_authority_bundle_seal_id: Uuid,
    pub tool_truth_authority_root_set_hash: String,
    pub tool_truth_authority_bundle_member_set_hash: String,
    pub tool_truth_authority_receipt_set_hash: String,
    pub denominator_graph_bundle_hash: String,
    pub semantic_authority_bundle_hash: String,
    pub freshness_attestation_bundle_hash: String,
    pub temporal_validity_bundle_hash: String,
    pub temporal_validity_policy_digest: String,
    pub temporal_validity_decision_set_hash: String,
    pub target_state_epoch_set_hash: String,
    pub gate_temporal_reevaluation_hash: String,
    pub knowledge_feed_catalog_policy_seal_hash: String,
    pub knowledge_feed_required_member_set_hash: String,
    pub knowledge_feed_signature_algorithm_set_hash: String,
    pub knowledge_feed_trust_store_hash: String,
    pub knowledge_feed_key_revocation_epoch_hash: String,
    pub knowledge_feed_snapshot_set_hash: String,
    pub product_version_census_hash: String,
    pub knowledge_feed_match_census_hash: String,
    pub gate_knowledge_feed_reevaluation_hash: String,
    pub stale_revalidation_obligation_set_hash: String,
    pub knowledge_feed_obligation_set_hash: String,
    pub active_analysis_attempt_id: Uuid,
    pub active_analysis_attempt_ordinal: u32,
    pub prior_terminal_attempt_chain_hash: String,
    pub proposal_census_hash: String,
    pub critic_census_hash: String,
    pub controller_decision_set_hash: String,
    pub mutation_set: Vec<CandidateHypothesisMutation>,
    pub mutation_set_hash: String,
    pub hypothesis_claim_components: Vec<HypothesisClaimComponentV1>,
    pub hypothesis_claim_component_set_hash: String,
    pub verification_contracts: Vec<VerificationContractV1>,
    pub verification_contract_set_hash: String,
    pub hypothesis_verification_plans: Vec<HypothesisVerificationPlanV1>,
    pub hypothesis_verification_plan_set_hash: String,
    pub input_dispositions: Vec<InputProcessingDispositionDecision>,
    pub input_relations: Vec<InputHypothesisRelationDecision>,
    pub input_chunk_census_set_hash: String,
    pub hypothesis_coverage_subreview_census_set_hash: String,
    pub hypothesis_coverage_synthesis_census_set_hash: String,
    pub hypothesis_coverage_global_semantic_root_hash: String,
    pub hypothesis_coverage_global_review_hash: String,
    pub hypothesis_coverage_review_set_hash: String,
    pub hypothesis_coverage_checklist_set_hash: String,
    pub generation_transition_set_hash: String,
    pub final_submitter_worker_run_id: Uuid,
}

pub fn validate_candidate_gate(
    snapshot: &CandidateGateSnapshot,
) -> Result<CandidateGatePass, CandidateGateBlock> {
    let material = &snapshot.0;
    validate_authority(material)?;
    validate_knowledge_feed(material)?;
    validate_attempt(&material.attempt)?;
    validate_read(&material.read)?;
    material.proposal_census.validate(
        "candidate_proposals.v1",
        CandidateGateBlockKind::ProposalCensus,
    )?;
    material.critic_census.validate(
        "candidate_critics.v1",
        CandidateGateBlockKind::ProposalCensus,
    )?;
    validate_coverage(&material.coverage)?;
    material.controller_decision_set.validate(
        "candidate_controller_decisions.v1",
        CandidateGateBlockKind::ControllerDecision,
    )?;
    validate_mutations(material)?;
    validate_compiled(material)?;
    validate_dispositions(material)?;
    material.generation_transition_set.validate(
        "candidate_generation_transitions.v1",
        CandidateGateBlockKind::GenerationTransition,
    )?;
    let transition_hashes = material
        .mutations
        .iter()
        .map(|mutation| mutation.generation_transition_hash.clone())
        .collect::<Vec<_>>();
    if canonical_members(transition_hashes)
        != canonical_members(
            material
                .generation_transition_set
                .observed_member_hashes
                .clone(),
        )
    {
        return Err(CandidateGateBlock::new(
            CandidateGateBlockKind::GenerationTransition,
            "mutation transitions differ from the acyclic transition set",
        ));
    }
    if !material.planning_ready || material.capability_assessment_present {
        return Err(CandidateGateBlock::new(
            CandidateGateBlockKind::VerificationPlan,
            "Plan B readiness is invalid or includes future capability authority",
        ));
    }
    if material.final_submitter_worker_run_id.is_nil()
        || material.final_submitter_worker_run_id != material.controller_worker_run_id
    {
        return Err(CandidateGateBlock::new(
            CandidateGateBlockKind::FinalSubmitter,
            "final submitter must be the Candidate Controller",
        ));
    }

    let authority = &material.authority;
    let roots = authority_roots_by_family(&authority.roots);
    Ok(CandidateGatePass {
        snapshot_id: material.snapshot_id,
        snapshot_hash: material.snapshot_hash.clone(),
        candidate_snapshot_authority_hash: material.candidate_snapshot_authority_hash.clone(),
        tool_truth_authority_bundle_seal_id: authority.bundle_seal_id,
        tool_truth_authority_root_set_hash: authority.root_set.observed_set_hash.clone(),
        tool_truth_authority_bundle_member_set_hash: authority
            .bundle_member_set
            .observed_set_hash
            .clone(),
        tool_truth_authority_receipt_set_hash: authority.receipt_set.observed_set_hash.clone(),
        denominator_graph_bundle_hash: exact_set_hash(
            "candidate_denominator_graph_bundle.v1",
            &roots
                .values()
                .map(|root| root.graph_hash.clone())
                .collect::<Vec<_>>(),
        ),
        semantic_authority_bundle_hash: exact_set_hash(
            "candidate_semantic_authority_bundle.v1",
            &roots
                .values()
                .map(|root| root.semantic_hash.clone())
                .collect::<Vec<_>>(),
        ),
        freshness_attestation_bundle_hash: exact_set_hash(
            "candidate_freshness_attestation_bundle.v1",
            &roots
                .values()
                .map(|root| root.freshness_hash.clone())
                .collect::<Vec<_>>(),
        ),
        temporal_validity_bundle_hash: exact_set_hash(
            "candidate_temporal_validity_bundle.v1",
            &roots
                .values()
                .map(|root| root.temporal_hash.clone())
                .collect::<Vec<_>>(),
        ),
        temporal_validity_policy_digest: authority.temporal_decision_set.observed_set_hash.clone(),
        temporal_validity_decision_set_hash: authority
            .temporal_decision_set
            .observed_set_hash
            .clone(),
        target_state_epoch_set_hash: authority.snapshot_target_state_epoch_set_hash.clone(),
        gate_temporal_reevaluation_hash: authority.gate_temporal_reevaluation_hash.clone(),
        knowledge_feed_catalog_policy_seal_hash: material
            .knowledge_feed
            .catalog_policy_seal_hash
            .clone(),
        knowledge_feed_required_member_set_hash: material
            .knowledge_feed
            .required_member_set
            .observed_set_hash
            .clone(),
        knowledge_feed_signature_algorithm_set_hash: material
            .knowledge_feed
            .signature_algorithm_set
            .observed_set_hash
            .clone(),
        knowledge_feed_trust_store_hash: material.knowledge_feed.trust_store_hash.clone(),
        knowledge_feed_key_revocation_epoch_hash: material
            .knowledge_feed
            .key_revocation_epoch_hash
            .clone(),
        knowledge_feed_snapshot_set_hash: material
            .knowledge_feed
            .signed_snapshot_set
            .observed_set_hash
            .clone(),
        product_version_census_hash: material
            .knowledge_feed
            .product_version_census
            .observed_set_hash
            .clone(),
        knowledge_feed_match_census_hash: material
            .knowledge_feed
            .match_census
            .observed_set_hash
            .clone(),
        gate_knowledge_feed_reevaluation_hash: material
            .knowledge_feed
            .gate_reevaluation_hash
            .clone(),
        stale_revalidation_obligation_set_hash: hash_parts(
            "candidate_stale_revalidation_obligation_set.v1",
            &[],
        ),
        knowledge_feed_obligation_set_hash: material.knowledge_feed.obligation_set_hash.clone(),
        active_analysis_attempt_id: material.attempt.active_attempt_id,
        active_analysis_attempt_ordinal: material.attempt.active_attempt_ordinal,
        prior_terminal_attempt_chain_hash: material
            .attempt
            .prior_terminal_attempt_chain_hash
            .clone(),
        proposal_census_hash: material.proposal_census.observed_set_hash.clone(),
        critic_census_hash: material.critic_census.observed_set_hash.clone(),
        controller_decision_set_hash: material.controller_decision_set.observed_set_hash.clone(),
        mutation_set: material.mutations.clone(),
        mutation_set_hash: material.mutation_set.observed_set_hash.clone(),
        hypothesis_claim_components: material.compiled.claim_components.clone(),
        hypothesis_claim_component_set_hash: material
            .compiled
            .claim_component_set
            .observed_set_hash
            .clone(),
        verification_contracts: material.compiled.verification_contracts.clone(),
        verification_contract_set_hash: material
            .compiled
            .verification_contract_set
            .observed_set_hash
            .clone(),
        hypothesis_verification_plans: material.compiled.verification_plans.clone(),
        hypothesis_verification_plan_set_hash: material
            .compiled
            .verification_plan_set
            .observed_set_hash
            .clone(),
        input_dispositions: material.input_dispositions.clone(),
        input_relations: material.input_relations.clone(),
        input_chunk_census_set_hash: material.read.chunk_set.observed_set_hash.clone(),
        hypothesis_coverage_subreview_census_set_hash: material
            .coverage
            .observed_subreview_set
            .observed_set_hash
            .clone(),
        hypothesis_coverage_synthesis_census_set_hash: material
            .coverage
            .synthesis_node_set
            .observed_set_hash
            .clone(),
        hypothesis_coverage_global_semantic_root_hash: material
            .coverage
            .synthesis_nodes
            .iter()
            .find(|node| node.node_kind == CandidateCoverageSynthesisNodeKindV1::GlobalSemanticRoot)
            .map(|node| node.node_hash.clone())
            .expect("validated exactly one global root"),
        hypothesis_coverage_global_review_hash: material.coverage.global_review_hash.clone(),
        hypothesis_coverage_review_set_hash: material
            .coverage
            .per_input_review_set
            .observed_set_hash
            .clone(),
        hypothesis_coverage_checklist_set_hash: material
            .coverage
            .checklist_member_set
            .observed_set_hash
            .clone(),
        generation_transition_set_hash: material
            .generation_transition_set
            .observed_set_hash
            .clone(),
        final_submitter_worker_run_id: material.final_submitter_worker_run_id,
    })
}

fn validate_authority(material: &FrozenCandidateGateMaterialV1) -> Result<(), CandidateGateBlock> {
    let authority = &material.authority;
    if authority.disposition != CandidateAuthoritySnapshotDispositionV1::SealedReady
        || authority.operation_id != material.operation_id
        || authority.organization_id != material.organization_id
        || authority.checked_request_id.is_nil()
        || authority.gate_request_id.is_nil()
        || authority.checked_request_id == authority.gate_request_id
        || authority.caller_filtered_or_reused_guard
        || authority.old_consistent_row_used
    {
        return Err(CandidateGateBlock::new(
            CandidateGateBlockKind::AuthorityBundle,
            "Checked bundle is not same-scope fresh repository material",
        ));
    }
    authority.root_set.validate(
        "candidate_roots.v1",
        CandidateGateBlockKind::AuthorityBundle,
    )?;
    authority.bundle_member_set.validate(
        "candidate_bundle_members.v1",
        CandidateGateBlockKind::AuthorityBundle,
    )?;
    authority.receipt_set.validate(
        "candidate_receipts.v1",
        CandidateGateBlockKind::AuthorityBundle,
    )?;
    let roots = authority_roots_by_family(&authority.roots);
    if roots.len() != ToolTruthRootFamilyV1::ALL.len()
        || !ToolTruthRootFamilyV1::ALL
            .iter()
            .all(|family| roots.contains_key(family))
    {
        return Err(CandidateGateBlock::new(
            CandidateGateBlockKind::AuthorityBundle,
            "TI/EAS/Enum/Vuln exact root census is incomplete",
        ));
    }
    if canonical_members(
        authority
            .roots
            .iter()
            .map(|root| root.member_hash.clone())
            .collect(),
    ) != canonical_members(authority.root_set.observed_member_hashes.clone())
        || authority
            .roots
            .iter()
            .any(|root| !valid_hash(&root.member_hash))
    {
        return Err(CandidateGateBlock::new(
            CandidateGateBlockKind::AuthorityBundle,
            "root members do not bind the exact root census",
        ));
    }
    authority.temporal_decision_set.validate(
        "candidate_temporal_decisions.v1",
        CandidateGateBlockKind::TemporalValidity,
    )?;
    if authority.current_target_state_epoch_set_hash
        != authority.snapshot_target_state_epoch_set_hash
        || !valid_hash(&authority.gate_temporal_reevaluation_hash)
        || roots.values().any(|root| {
            root.temporal_status != TemporalValidityStatus::Fresh
                || ![
                    &root.graph_hash,
                    &root.semantic_hash,
                    &root.freshness_hash,
                    &root.temporal_hash,
                    &root.target_state_epoch_hash,
                ]
                .into_iter()
                .all(|hash| valid_hash(hash))
        })
    {
        return Err(CandidateGateBlock::new(
            CandidateGateBlockKind::TemporalValidity,
            "Gate-time TTL, epoch, skew, or temporal decision is stale",
        ));
    }
    Ok(())
}

fn validate_knowledge_feed(
    material: &FrozenCandidateGateMaterialV1,
) -> Result<(), CandidateGateBlock> {
    let feed = &material.knowledge_feed;
    for (exact_set, domain) in [
        (&feed.required_member_set, "candidate_feed_required.v1"),
        (&feed.signed_snapshot_set, "candidate_feed_snapshots.v1"),
        (&feed.product_version_census, "candidate_products.v1"),
        (&feed.match_census, "candidate_matches.v1"),
        (
            &feed.signature_algorithm_set,
            "candidate_signature_algorithms.v1",
        ),
    ] {
        exact_set.validate(domain, CandidateGateBlockKind::KnowledgeFeed)?;
    }
    let feed_members = feed
        .members
        .iter()
        .map(|member| member.member_hash.clone())
        .collect::<Vec<_>>();
    if canonical_members(feed_members.clone())
        != canonical_members(feed.required_member_set.observed_member_hashes.clone())
        || canonical_members(feed_members)
            != canonical_members(feed.signed_snapshot_set.observed_member_hashes.clone())
    {
        return Err(CandidateGateBlock::new(
            CandidateGateBlockKind::KnowledgeFeed,
            "feed member denominator differs from signed snapshots",
        ));
    }
    if ![
        &feed.catalog_policy_seal_hash,
        &feed.trust_store_hash,
        &feed.key_revocation_epoch_hash,
        &feed.gate_reevaluation_hash,
        &feed.obligation_set_hash,
    ]
    .into_iter()
    .all(|hash| valid_hash(hash))
        || feed.trust_store_hash != feed.snapshot_trust_store_hash
        || feed.key_revocation_epoch_hash != feed.snapshot_key_revocation_epoch_hash
        || feed.members.iter().any(|member| {
            !valid_hash(&member.member_hash)
                || !member.product_version_known
                || !member.signature_valid
                || !member.provenance_valid
                || !member.age_valid_at_gate
                || !member.key_current_and_not_revoked
        })
    {
        return Err(CandidateGateBlock::new(
            CandidateGateBlockKind::KnowledgeFeed,
            "managed feed signature, version, provenance, age, or key authority is invalid",
        ));
    }
    Ok(())
}

fn validate_attempt(attempt: &CandidateAttemptGateV1) -> Result<(), CandidateGateBlock> {
    if attempt.active_attempt_id.is_nil()
        || !attempt.active_attempt_unique
        || attempt
            .material_attempt_ids
            .iter()
            .any(|id| *id != attempt.active_attempt_id)
        || attempt.prior_attempts.len() != attempt.active_attempt_ordinal as usize
    {
        return Err(CandidateGateBlock::new(
            CandidateGateBlockKind::AttemptChain,
            "active attempt is ambiguous or old material was reused",
        ));
    }
    let mut member_hashes = Vec::with_capacity(attempt.prior_attempts.len());
    for (ordinal, prior) in attempt.prior_attempts.iter().enumerate() {
        if prior.ordinal != ordinal as u32
            || !prior.terminal
            || prior.attempt_id.is_nil()
            || !valid_hash(&prior.member_hash)
        {
            return Err(CandidateGateBlock::new(
                CandidateGateBlockKind::AttemptChain,
                "prior terminal attempt chain is forked or incomplete",
            ));
        }
        member_hashes.push(prior.member_hash.clone());
    }
    if exact_set_hash("candidate_prior_terminal_attempt_chain.v1", &member_hashes)
        != attempt.prior_terminal_attempt_chain_hash
    {
        return Err(CandidateGateBlock::new(
            CandidateGateBlockKind::AttemptChain,
            "prior terminal attempt chain hash drifted",
        ));
    }
    Ok(())
}

fn validate_read(read: &CandidateReadGateV1) -> Result<(), CandidateGateBlock> {
    read.input_set
        .validate("candidate_inputs.v1", CandidateGateBlockKind::ChunkCensus)?;
    read.chunk_set
        .validate("candidate_chunks.v1", CandidateGateBlockKind::ChunkCensus)?;
    read.page_receipt_set.validate(
        "candidate_page_receipts.v1",
        CandidateGateBlockKind::ReadReceipt,
    )?;
    read.server_read_receipt_set.validate(
        "candidate_server_reads.v1",
        CandidateGateBlockKind::ReadReceipt,
    )?;
    if !read.source_bytes_complete || read.context_truncated || read.caller_claimed_read_complete {
        return Err(CandidateGateBlock::new(
            CandidateGateBlockKind::ReadReceipt,
            "page receipts prove delivery only; server read closure is incomplete",
        ));
    }
    Ok(())
}

fn validate_coverage(coverage: &CandidateCoverageGateV1) -> Result<(), CandidateGateBlock> {
    coverage
        .h1_proposal_set
        .validate("candidate_h1.v1", CandidateGateBlockKind::ProposalCensus)?;
    coverage.per_input_h1_disposition_set.validate(
        "candidate_h1_dispositions.v1",
        CandidateGateBlockKind::ProposalCensus,
    )?;
    coverage.checklist_member_set.validate(
        "candidate_checklist.v1",
        CandidateGateBlockKind::CoverageSubreview,
    )?;
    coverage.chunk_partition_set.validate(
        "candidate_partitions.v1",
        CandidateGateBlockKind::CoverageSubreview,
    )?;
    coverage.expected_subreview_set.validate(
        "candidate_subreviews_expected.v1",
        CandidateGateBlockKind::CoverageSubreview,
    )?;
    coverage.observed_subreview_set.validate(
        "candidate_subreviews_observed.v1",
        CandidateGateBlockKind::CoverageSubreview,
    )?;
    if coverage.expected_subreview_set.expected_member_hashes
        != coverage.observed_subreview_set.observed_member_hashes
    {
        return Err(CandidateGateBlock::new(
            CandidateGateBlockKind::CoverageSubreview,
            "checklist by partition subreview denominator is incomplete",
        ));
    }
    coverage.synthesis_node_set.validate(
        "candidate_synthesis.v1",
        CandidateGateBlockKind::CoverageSynthesis,
    )?;
    if canonical_members(
        coverage
            .synthesis_nodes
            .iter()
            .map(|node| node.node_hash.clone())
            .collect(),
    ) != canonical_members(coverage.synthesis_node_set.observed_member_hashes.clone())
    {
        return Err(CandidateGateBlock::new(
            CandidateGateBlockKind::CoverageSynthesis,
            "recursive synthesis node census is incomplete",
        ));
    }
    let global_roots = coverage
        .synthesis_nodes
        .iter()
        .filter(|node| node.node_kind == CandidateCoverageSynthesisNodeKindV1::GlobalSemanticRoot)
        .count();
    if global_roots != 1 {
        return Err(CandidateGateBlock::new(
            CandidateGateBlockKind::CoverageSynthesis,
            "exactly one global semantic root is required",
        ));
    }
    for node in &coverage.synthesis_nodes {
        if canonical_members(node.expected_child_hashes.clone())
            != canonical_members(node.observed_child_hashes.clone())
            || node
                .primary_analyst_worker_run_ids
                .contains(&node.worker_run_id)
            || node
                .transitive_descendant_worker_run_ids
                .contains(&node.worker_run_id)
            || node.outcome != CandidateCoverageOutcomeV1::Adequate
        {
            return Err(CandidateGateBlock::new(
                CandidateGateBlockKind::CoverageSynthesis,
                "recursive child closure or transitive worker separation failed",
            ));
        }
    }
    coverage.per_input_review_set.validate(
        "candidate_reviews.v1",
        CandidateGateBlockKind::CoverageReview,
    )?;
    coverage
        .h2_proposal_set
        .validate("candidate_h2.v1", CandidateGateBlockKind::CoverageReview)?;
    if !valid_hash(&coverage.global_review_hash)
        || coverage.global_review_outcome != CandidateCoverageOutcomeV1::Adequate
        || coverage.unresolved_feed_dependent_checklist_members != 0
        || coverage.missed_hypothesis
        || coverage.sampling_used
        || coverage.retry_limit_reached
    {
        return Err(CandidateGateBlock::new(
            CandidateGateBlockKind::CoverageReview,
            "coverage is missed, blocked, sampled, or has unresolved checklist members",
        ));
    }
    Ok(())
}

fn validate_mutations(material: &FrozenCandidateGateMaterialV1) -> Result<(), CandidateGateBlock> {
    material.mutation_set.validate(
        "candidate_mutations.v1",
        CandidateGateBlockKind::SemanticReducer,
    )?;
    let mut previous_key: Option<(Uuid, String, u8, Uuid)> = None;
    let mut observed_hashes = Vec::with_capacity(material.mutations.len());
    for mutation in &material.mutations {
        if mutation.organization_id != material.organization_id
            || !valid_hash(&mutation.semantic_key_hash)
            || !valid_hash(&mutation.generation_transition_hash)
            || mutation.canonical_hash() != mutation.mutation_hash
        {
            return Err(CandidateGateBlock::new(
                CandidateGateBlockKind::SemanticReducer,
                "mutation scope, identity, transition, or canonical hash drifted",
            ));
        }
        for source in &mutation.proof_refs {
            match source {
                RevisionSourceRef::ApplicationContext(_) => {
                    return Err(CandidateGateBlock::new(
                        CandidateGateBlockKind::ApplicationContextIsNotProof,
                        "application context cannot satisfy proof",
                    ));
                }
                RevisionSourceRef::KnowledgeSignal(_) => {
                    return Err(CandidateGateBlock::new(
                        CandidateGateBlockKind::KnowledgeSignalIsNotProof,
                        "knowledge signals are analysis context only",
                    ));
                }
                RevisionSourceRef::Gap(_) => {
                    return Err(CandidateGateBlock::new(
                        CandidateGateBlockKind::SemanticReducer,
                        "a gap cannot satisfy proof",
                    ));
                }
                _ => {}
            }
        }
        if mutation
            .refutation_refs
            .iter()
            .any(|source| matches!(source, RevisionSourceRef::Gap(_)))
        {
            return Err(CandidateGateBlock::new(
                CandidateGateBlockKind::GapIsNotRefutation,
                "absence of checking cannot refute a hypothesis",
            ));
        }
        let key = (
            mutation.organization_id,
            mutation.semantic_key_hash.clone(),
            mutation.operator_rank,
            mutation.proposal_id,
        );
        if previous_key
            .as_ref()
            .is_some_and(|previous| previous >= &key)
        {
            return Err(CandidateGateBlock::new(
                CandidateGateBlockKind::SemanticReducer,
                "mutation set ordering or identity uniqueness drifted",
            ));
        }
        previous_key = Some(key);
        observed_hashes.push(mutation.mutation_hash.clone());
    }
    if canonical_members(observed_hashes)
        != canonical_members(material.mutation_set.observed_member_hashes.clone())
    {
        return Err(CandidateGateBlock::new(
            CandidateGateBlockKind::SemanticReducer,
            "mutation exact set differs from sealed mutations",
        ));
    }
    Ok(())
}

fn validate_compiled(material: &FrozenCandidateGateMaterialV1) -> Result<(), CandidateGateBlock> {
    let compiled = &material.compiled;
    compiled.claim_component_set.validate(
        "candidate_claim_components.v1",
        CandidateGateBlockKind::ClaimComponent,
    )?;
    let component_hashes = compiled
        .claim_components
        .iter()
        .map(|component| component.member_hash().to_owned())
        .collect::<Vec<_>>();
    if canonical_members(component_hashes)
        != canonical_members(compiled.claim_component_set.observed_member_hashes.clone())
        || compiled.claim_components.is_empty()
        || compiled
            .claim_components
            .iter()
            .any(|component| !component.required())
    {
        return Err(CandidateGateBlock::new(
            CandidateGateBlockKind::ClaimComponent,
            "required claim-component denominator is incomplete",
        ));
    }
    compiled.verification_contract_set.validate(
        "candidate_contracts.v1",
        CandidateGateBlockKind::VerificationContract,
    )?;
    validate_compiled_contract_set(
        &compiled.verification_contracts,
        &compiled.verification_contract_set.observed_member_hashes,
    )
    .map_err(|_| {
        CandidateGateBlock::new(
            CandidateGateBlockKind::VerificationContract,
            "contract predicate/control/pair/order authority drifted",
        )
    })?;
    compiled.verification_plan_set.validate(
        "candidate_plans.v1",
        CandidateGateBlockKind::VerificationPlan,
    )?;
    validate_compiled_plan_set(
        &compiled.verification_plans,
        &compiled.verification_plan_set.observed_member_hashes,
        &compiled.claim_component_set.observed_member_hashes,
        &compiled.verification_contract_set.observed_member_hashes,
    )
    .map_err(|_| {
        CandidateGateBlock::new(
            CandidateGateBlockKind::VerificationPlan,
            "plan objective/component/path/member union is incomplete",
        )
    })?;
    Ok(())
}

fn validate_dispositions(
    material: &FrozenCandidateGateMaterialV1,
) -> Result<(), CandidateGateBlock> {
    material.input_disposition_set.validate(
        "candidate_input_dispositions.v1",
        CandidateGateBlockKind::InputDisposition,
    )?;
    material.input_relation_set.validate(
        "candidate_input_relations.v1",
        CandidateGateBlockKind::InputDisposition,
    )?;
    let dispositions = material
        .input_dispositions
        .iter()
        .map(|decision| decision.decision_hash.clone())
        .collect::<Vec<_>>();
    let relations = material
        .input_relations
        .iter()
        .map(|decision| decision.decision_hash.clone())
        .collect::<Vec<_>>();
    if canonical_members(dispositions)
        != canonical_members(
            material
                .input_disposition_set
                .observed_member_hashes
                .clone(),
        )
        || canonical_members(relations)
            != canonical_members(material.input_relation_set.observed_member_hashes.clone())
    {
        return Err(CandidateGateBlock::new(
            CandidateGateBlockKind::InputDisposition,
            "input disposition or relation set drifted",
        ));
    }
    Ok(())
}

fn authority_roots_by_family(
    roots: &[CandidateAuthorityRootGateV1],
) -> BTreeMap<ToolTruthRootFamilyV1, &CandidateAuthorityRootGateV1> {
    roots.iter().map(|root| (root.root_family, root)).collect()
}

fn source_ref_key(source: &RevisionSourceRef) -> String {
    match source {
        RevisionSourceRef::ToolTruthEvidence(id) => format!("tool_truth:{id}"),
        RevisionSourceRef::Finding(id) => format!("finding:{id}"),
        RevisionSourceRef::VerificationReceipt(id) => format!("verification:{id}"),
        RevisionSourceRef::ApplicationContext(id) => format!("application:{id}"),
        RevisionSourceRef::KnowledgeSignal(id) => format!("knowledge:{id}"),
        RevisionSourceRef::Gap(id) => format!("gap:{id}"),
    }
}

fn valid_hash(value: &str) -> bool {
    value.len() == 71
        && value.starts_with(HASH_PREFIX)
        && value[HASH_PREFIX.len()..]
            .bytes()
            .all(|byte| byte.is_ascii_hexdigit() && !byte.is_ascii_uppercase())
}

fn canonical_members(mut members: Vec<String>) -> Vec<String> {
    members.sort();
    members
}

fn has_duplicates(members: &[String]) -> bool {
    let mut unique = BTreeSet::new();
    members.iter().any(|member| !unique.insert(member))
}

pub fn exact_set_hash(domain: &str, members: &[String]) -> String {
    let members = canonical_members(members.to_vec());
    hash_parts(domain, &members)
}

fn hash_parts(domain: &str, parts: &[String]) -> String {
    let mut hasher = Sha256::new();
    hash_field(&mut hasher, domain.as_bytes());
    for part in parts {
        hash_field(&mut hasher, part.as_bytes());
    }
    let digest = hasher.finalize();
    let mut encoded = String::with_capacity(71);
    encoded.push_str(HASH_PREFIX);
    for byte in digest {
        use std::fmt::Write as _;
        write!(&mut encoded, "{byte:02x}").expect("writing to String cannot fail");
    }
    encoded
}

fn hash_field(hasher: &mut Sha256, bytes: &[u8]) {
    hasher.update((bytes.len() as u64).to_be_bytes());
    hasher.update(bytes);
}
