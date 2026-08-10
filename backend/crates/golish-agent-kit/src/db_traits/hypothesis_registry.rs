//! SQLx-free persistence boundary for the Plan B Hypothesis Registry.
//!
//! Requests in this module are server-built. They deliberately expose neither
//! database rows nor a free-form JSON payload. In particular, snapshot freeze
//! accepts only stable operation/scope identity; Plan A roots, receipts,
//! clocks, epochs, policies, and hashes are resolved inside the repository.

use async_trait::async_trait;
use chrono::{DateTime, Utc};
use golish_core::hypothesis_semantic_key::{
    CandidateMutationEpistemicState, ClaimPolarity, PredicateIdentity,
};
use golish_core::hypothesis_verification::{
    HypothesisClaimComponentV1, HypothesisVerificationPlanV1,
};
use golish_core::verification_contract::VerificationContractV1;
use golish_pentest_domain::tool_truth::{
    EvidenceTemporalValidityPolicyV1, TemporalValidityStatus, ToolTruthRootFamilyV1,
};
use uuid::Uuid;

use crate::task_orchestrator::hypothesis_analysis::CandidateCoverageSemanticSummary;

/// Stable optimistic-concurrency and worker-ownership fence shared by every
/// repository operation that writes analysis state (including page receipts).
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct CandidateRepositoryWriteFenceV1 {
    pub operation_id: Uuid,
    pub scope_snapshot_id: Uuid,
    pub organization_id: Uuid,
    pub snapshot_id: Uuid,
    pub team_plan_id: Uuid,
    pub work_item_id: Uuid,
    pub worker_run_id: Uuid,
    pub lease_token: Uuid,
    pub lease_epoch: u64,
    pub analysis_attempt_id: Uuid,
    pub analysis_attempt_ordinal: u32,
    pub attempt_epoch: u64,
    pub expected_snapshot_row_version: i64,
    pub expected_team_plan_row_version: i64,
    pub expected_work_item_row_version: i64,
    pub expected_worker_row_version: i64,
    pub expected_attempt_row_version: i64,
}

/// The only caller-visible input to snapshot authority selection. No root,
/// receipt, hash, time, policy, freshness, or target-state epoch can be passed.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct FreezeCandidateAnalysisSnapshot {
    pub stable_consumer_request_id: Uuid,
    pub operation_id: Uuid,
    pub scope_snapshot_id: Uuid,
    pub organization_id: Uuid,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum CandidateAnalysisSnapshotDispositionV1 {
    SealedReady,
    SealedAnalysisReadyWithResiduals,
    BlockedAuthorityBundle,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum CandidateSemanticAuthorityStatusV1 {
    Consistent,
    Pending,
    Orphaned,
    Superseded,
}

/// Exact persisted copy of one Plan A authority root. The two independent
/// semantic and temporal axes remain separate.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct CandidateToolTruthAuthorityRootViewV1 {
    pub ordinal: u32,
    pub root_family: ToolTruthRootFamilyV1,
    pub root_denominator_id: Uuid,
    pub root_denominator_hash: String,
    pub authority_set_seal_id: Uuid,
    pub authority_set_graph_hash: String,
    pub authority_set_semantic_hash: String,
    pub authority_set_freshness_hash: String,
    pub temporal_validity_policy_set_hash: String,
    pub temporal_validity_decision_set_hash: String,
    pub target_state_epoch_set_hash: String,
    pub receipt_count: u32,
    pub receipt_set_hash: String,
    pub semantic_status: CandidateSemanticAuthorityStatusV1,
    pub temporal_status: TemporalValidityStatus,
    pub temporal_policies: Vec<EvidenceTemporalValidityPolicyV1>,
    pub member_hash: String,
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct CandidateAnalysisSnapshotView {
    pub snapshot_id: Uuid,
    pub stable_consumer_request_id: Uuid,
    pub operation_id: Uuid,
    pub scope_snapshot_id: Uuid,
    pub organization_id: Uuid,
    pub disposition: CandidateAnalysisSnapshotDispositionV1,
    pub snapshot_hash: String,
    pub candidate_snapshot_authority_hash: String,
    pub tool_truth_authority_bundle_seal_id: Uuid,
    pub tool_truth_authority_root_count: u32,
    pub tool_truth_authority_root_set_hash: String,
    pub tool_truth_authority_bundle_member_count: u32,
    pub tool_truth_authority_bundle_member_set_hash: String,
    pub tool_truth_authority_receipt_count: u32,
    pub tool_truth_authority_receipt_set_hash: String,
    pub denominator_graph_bundle_hash: String,
    pub semantic_authority_bundle_hash: String,
    pub freshness_attestation_bundle_hash: String,
    pub temporal_validity_bundle_hash: String,
    pub temporal_validity_policy_set_hash: String,
    pub temporal_validity_decision_set_hash: String,
    pub observation_window_hash: String,
    pub target_state_epoch_set_hash: String,
    pub authority_roots: Vec<CandidateToolTruthAuthorityRootViewV1>,
    pub knowledge_feed_catalog_policy_seal_hash: String,
    pub knowledge_feed_required_member_set_hash: String,
    pub knowledge_feed_signature_algorithm_set_hash: String,
    pub knowledge_feed_trust_store_hash: String,
    pub knowledge_feed_key_revocation_epoch_hash: String,
    pub knowledge_feed_snapshot_set_hash: String,
    pub product_version_census_hash: String,
    pub knowledge_feed_match_census_hash: String,
    pub stale_revalidation_obligation_set_hash: String,
    pub knowledge_feed_obligation_set_hash: String,
    pub row_version: i64,
    pub sealed_at: DateTime<Utc>,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum CandidateSnapshotInputKindV1 {
    ToolTruthFact,
    ToolTruthObservation,
    ToolTruthEvidence,
    TechniqueOutcome,
    KnowledgeSignal,
    PreviousGeneration,
    FactDelta,
    Relation,
    ResidualRisk,
    OpenObligation,
}

/// Immutable repository-owned read body. It is output-only and cannot be
/// supplied to a write API.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum CandidateSnapshotReadBodyV1 {
    CanonicalRedactedText {
        schema: String,
        schema_version: u32,
        body: String,
        body_hash: String,
    },
    ContentAddressedBlob {
        schema: String,
        schema_version: u32,
        blob_id: Uuid,
        content_hash: String,
        byte_count: u64,
    },
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct CandidateAnalysisPageItemV1 {
    pub input_id: Uuid,
    pub ordinal: u32,
    pub input_kind: CandidateSnapshotInputKindV1,
    pub stable_key: String,
    pub source_hash: String,
    pub source_size_bytes: u64,
    pub body: CandidateSnapshotReadBodyV1,
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct LoadCandidateAnalysisPage {
    pub fence: CandidateRepositoryWriteFenceV1,
    pub stable_page_request_id: Uuid,
    pub after_input_ordinal: Option<u32>,
    pub page_size: u32,
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct CandidateAnalysisPageView {
    pub snapshot_id: Uuid,
    pub page_receipt_id: Uuid,
    pub first_input_ordinal: Option<u32>,
    pub last_input_ordinal: Option<u32>,
    pub returned_count: u32,
    pub page_hash: String,
    pub items: Vec<CandidateAnalysisPageItemV1>,
    pub next_input_ordinal: Option<u32>,
    pub replayed: bool,
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct LoadCandidateInputChunkPage {
    pub fence: CandidateRepositoryWriteFenceV1,
    pub stable_page_request_id: Uuid,
    pub input_id: Uuid,
    pub chunk_census_id: Uuid,
    pub chunk_census_hash: String,
    pub source_size_bytes: u64,
    pub chunking_contract_version: u32,
    pub redaction_contract_version: u32,
    pub first_chunk_ordinal: u32,
    pub max_chunks: u32,
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct CandidateInputChunkViewV1 {
    pub chunk_id: Uuid,
    pub chunk_ordinal: u32,
    pub source_range_start: u64,
    pub source_range_end: u64,
    pub chunk_hash: String,
    pub body: CandidateSnapshotReadBodyV1,
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct CandidateInputChunkPageView {
    pub snapshot_id: Uuid,
    pub input_id: Uuid,
    pub chunk_census_id: Uuid,
    pub chunk_census_hash: String,
    pub source_size_bytes: u64,
    pub chunking_contract_version: u32,
    pub redaction_contract_version: u32,
    pub page_receipt_id: Uuid,
    pub first_chunk_ordinal: Option<u32>,
    pub last_chunk_ordinal: Option<u32>,
    pub returned_count: u32,
    pub page_hash: String,
    pub chunks: Vec<CandidateInputChunkViewV1>,
    pub next_chunk_ordinal: Option<u32>,
    pub replayed: bool,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum CandidateAnalysisArtifactKindV1 {
    HypothesisProposal,
    ProposalConflictReview,
    HypothesisCoverageSubreview,
    HypothesisCoverageSynthesis,
    HypothesisCoverageReview,
    ControllerDecision,
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct CandidateHypothesisProposalArtifactV1 {
    pub proposal_id: Uuid,
    pub subject_kind: String,
    pub subject_identity_hash: String,
    pub predicate: PredicateIdentity,
    pub trust_boundary: String,
    pub polarity: ClaimPolarity,
    pub prose: String,
    pub confidence: i32,
    pub priority: i32,
    pub tags: Vec<String>,
    pub evidence_refs: Vec<String>,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum ProposalConflictReviewOutcomeV1 {
    NoConflict,
    Duplicate,
    MergeRequired,
    SplitRequired,
    Blocked,
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct ProposalConflictReviewArtifactV1 {
    pub conflict_component_id: Uuid,
    pub proposal_ids: Vec<Uuid>,
    pub outcome: ProposalConflictReviewOutcomeV1,
    pub rationale: String,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum CandidateControllerDecisionKindV1 {
    Accept,
    Reject,
    AttachExisting,
    Merge,
    Split,
    NarrowSuccessor,
    Blocked,
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct CandidateControllerDecisionArtifactV1 {
    pub decision_id: Uuid,
    pub proposal_id: Uuid,
    pub decision: CandidateControllerDecisionKindV1,
    pub related_proposal_ids: Vec<Uuid>,
    pub rationale: String,
}

/// Generic artifact writes are type-limited to these three variants. Coverage
/// subreview, synthesis, and final review have dedicated repository methods.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum CandidateAnalysisArtifactSubmissionV1 {
    HypothesisProposal(CandidateHypothesisProposalArtifactV1),
    ProposalConflictReview(ProposalConflictReviewArtifactV1),
    ControllerDecision(CandidateControllerDecisionArtifactV1),
}

impl CandidateAnalysisArtifactSubmissionV1 {
    pub const fn kind(&self) -> CandidateAnalysisArtifactKindV1 {
        match self {
            Self::HypothesisProposal(_) => CandidateAnalysisArtifactKindV1::HypothesisProposal,
            Self::ProposalConflictReview(_) => {
                CandidateAnalysisArtifactKindV1::ProposalConflictReview
            }
            Self::ControllerDecision(_) => CandidateAnalysisArtifactKindV1::ControllerDecision,
        }
    }
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct RecordCandidateAnalysisArtifact {
    pub fence: CandidateRepositoryWriteFenceV1,
    pub stable_artifact_request_id: Uuid,
    pub artifact: CandidateAnalysisArtifactSubmissionV1,
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct CandidateAnalysisArtifactReceipt {
    pub artifact_id: Uuid,
    pub artifact_kind: CandidateAnalysisArtifactKindV1,
    pub artifact_hash: String,
    pub artifact_row_version: i64,
    pub replayed: bool,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum CandidateAnalysisCensusKindV1 {
    Proposal,
    Critic,
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct SealCandidateAnalysisCensus {
    pub fence: CandidateRepositoryWriteFenceV1,
    pub stable_census_request_id: Uuid,
    pub census_kind: CandidateAnalysisCensusKindV1,
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct CandidateAnalysisCensusView {
    pub census_id: Uuid,
    pub analysis_attempt_id: Uuid,
    pub census_kind: CandidateAnalysisCensusKindV1,
    pub member_count: u32,
    pub member_set_hash: String,
    pub census_hash: String,
    pub row_version: i64,
    pub replayed: bool,
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct SealHypothesisCoverageSubreviewCensus {
    pub fence: CandidateRepositoryWriteFenceV1,
    pub stable_census_request_id: Uuid,
    pub input_id: Uuid,
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct HypothesisCoverageSubreviewCensusView {
    pub census_id: Uuid,
    pub analysis_attempt_id: Uuid,
    pub input_id: Uuid,
    pub member_count: u32,
    pub member_set_hash: String,
    pub census_hash: String,
    pub row_version: i64,
    pub replayed: bool,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum HypothesisCoverageSubreviewOutcomeV1 {
    NoLocalMiss,
    MissedHypothesis,
    Blocked,
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct RecordHypothesisCoverageSubreview {
    pub fence: CandidateRepositoryWriteFenceV1,
    pub stable_review_request_id: Uuid,
    pub subreview_census_id: Uuid,
    pub subreview_census_member_id: Uuid,
    pub outcome: HypothesisCoverageSubreviewOutcomeV1,
    pub missed_proposal_ids: Vec<Uuid>,
    pub blocker_codes: Vec<String>,
    pub semantic_summary: CandidateCoverageSemanticSummary,
    pub review_notes: String,
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct HypothesisCoverageSubreviewReceipt {
    pub subreview_id: Uuid,
    pub subreview_census_id: Uuid,
    pub subreview_census_member_id: Uuid,
    pub subreview_hash: String,
    pub row_version: i64,
    pub replayed: bool,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum HypothesisCoverageSynthesisNodeKindV1 {
    CrossChunk,
    CrossInputPartition,
    CrossInputReduce,
    CrossDimensionReduce,
    GlobalSemanticRoot,
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct SealHypothesisCoverageSynthesisCensus {
    pub fence: CandidateRepositoryWriteFenceV1,
    pub stable_census_request_id: Uuid,
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct HypothesisCoverageSynthesisCensusView {
    pub census_id: Uuid,
    pub analysis_attempt_id: Uuid,
    pub member_count: u32,
    pub member_set_hash: String,
    pub census_hash: String,
    pub global_semantic_root_member_id: Uuid,
    pub row_version: i64,
    pub replayed: bool,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum HypothesisCoverageSynthesisOutcomeV1 {
    NoSemanticMiss,
    MissedHypothesis,
    Blocked,
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct RecordHypothesisCoverageSynthesisReview {
    pub fence: CandidateRepositoryWriteFenceV1,
    pub stable_review_request_id: Uuid,
    pub synthesis_census_id: Uuid,
    pub synthesis_census_member_id: Uuid,
    pub node_kind: HypothesisCoverageSynthesisNodeKindV1,
    pub outcome: HypothesisCoverageSynthesisOutcomeV1,
    pub missed_proposal_ids: Vec<Uuid>,
    pub blocker_codes: Vec<String>,
    pub semantic_summary: CandidateCoverageSemanticSummary,
    pub review_notes: String,
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct HypothesisCoverageSynthesisReceipt {
    pub synthesis_review_id: Uuid,
    pub synthesis_census_id: Uuid,
    pub synthesis_census_member_id: Uuid,
    pub synthesis_hash: String,
    pub row_version: i64,
    pub replayed: bool,
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct ReduceHypothesisCoverageReview {
    pub fence: CandidateRepositoryWriteFenceV1,
    pub stable_reduction_request_id: Uuid,
    pub input_id: Uuid,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum HypothesisCoverageReviewOutcomeV1 {
    Adequate,
    MissedHypothesis,
    Blocked,
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct HypothesisCoverageReviewReceipt {
    pub coverage_review_id: Uuid,
    pub input_id: Uuid,
    pub outcome: HypothesisCoverageReviewOutcomeV1,
    pub coverage_review_hash: String,
    pub row_version: i64,
    pub replayed: bool,
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct LoadCandidateGateMaterial {
    pub operation_id: Uuid,
    pub scope_snapshot_id: Uuid,
    pub organization_id: Uuid,
    pub snapshot_id: Uuid,
    pub analysis_attempt_id: Uuid,
    pub analysis_attempt_ordinal: u32,
    pub expected_snapshot_row_version: i64,
    pub expected_attempt_row_version: i64,
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct SealCandidateCompilation {
    pub fence: CandidateRepositoryWriteFenceV1,
    pub stable_compilation_request_id: Uuid,
    pub mutation_set_hash: String,
    pub claim_component_set_hash: String,
    pub verification_contract_set_hash: String,
    pub verification_plan_set_hash: String,
    pub generation_transition_set_hash: String,
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct CandidateCompilationSealView {
    pub compilation_seal_id: Uuid,
    pub mutation_set_hash: String,
    pub claim_component_set_hash: String,
    pub verification_contract_set_hash: String,
    pub verification_plan_set_hash: String,
    pub generation_transition_set_hash: String,
    pub compiler_seal_hash: String,
    pub row_version: i64,
    pub replayed: bool,
}

/// Server-loaded, hash-bound material for the pure Task 6 Gate. It contains no
/// mutable source bodies and cannot be constructed from a model artifact.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct CandidateGateMaterial {
    pub snapshot: CandidateAnalysisSnapshotView,
    pub active_analysis_attempt_id: Uuid,
    pub active_analysis_attempt_ordinal: u32,
    pub attempt_epoch: u64,
    pub prior_terminal_attempt_chain_hash: String,
    pub gate_temporal_reevaluation_hash: String,
    pub gate_knowledge_feed_reevaluation_hash: String,
    pub input_chunk_census_set_hash: String,
    pub proposal_census_hash: String,
    pub critic_census_hash: String,
    pub coverage_subreview_census_set_hash: String,
    pub coverage_synthesis_census_set_hash: String,
    pub coverage_global_semantic_root_hash: String,
    pub coverage_global_review_hash: String,
    pub coverage_review_set_hash: String,
    pub coverage_checklist_set_hash: String,
    pub controller_decision_set_hash: String,
    pub mutation_set_hash: String,
    pub claim_component_set_hash: String,
    pub verification_contract_set_hash: String,
    pub verification_plan_set_hash: String,
    pub generation_transition_set_hash: String,
    pub compiler_seal_hash: String,
    pub final_submitter_worker_run_id: Uuid,
    pub controller_dispatch_worker_run_id: Uuid,
    pub snapshot_row_version: i64,
    pub attempt_row_version: i64,
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct CandidateGateExpectedAuthorityV1 {
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
    pub prior_terminal_attempt_chain_hash: String,
    pub proposal_census_hash: String,
    pub critic_census_hash: String,
    pub controller_decision_set_hash: String,
    pub input_chunk_census_set_hash: String,
    pub coverage_subreview_census_set_hash: String,
    pub coverage_synthesis_census_set_hash: String,
    pub coverage_global_semantic_root_hash: String,
    pub coverage_global_review_hash: String,
    pub coverage_review_set_hash: String,
    pub coverage_checklist_set_hash: String,
    pub generation_transition_set_hash: String,
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub enum CandidateRegistryMutationDecisionV1 {
    AttachCurrent {
        root_id: Uuid,
        revision_id: Uuid,
    },
    CreateInitial {
        root_id: Uuid,
    },
    ReopenHistorical {
        root_id: Uuid,
        predecessor_revision_id: Uuid,
    },
    NoSemanticChange {
        root_id: Uuid,
        revision_id: Uuid,
    },
    ExplicitTransitionRequired {
        historical_root_id: Uuid,
    },
    Split {
        parent_root_id: Uuid,
        child_root_ids: Vec<Uuid>,
    },
    Merge {
        parent_root_ids: Vec<Uuid>,
        successor_root_id: Uuid,
    },
    Derive {
        source_root_id: Uuid,
        source_revision_id: Uuid,
        derivation_rule_hash: String,
        successor_root_id: Uuid,
    },
    NarrowSuccessor {
        source_root_id: Uuid,
        source_revision_id: Uuid,
        successor_root_id: Uuid,
        covered_claim_component_set_hash: String,
    },
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub enum CandidateRegistryRevisionSourceRefV1 {
    ToolTruthEvidence(String),
    Finding(String),
    VerificationReceipt(String),
    ApplicationContext(String),
    KnowledgeSignal(String),
    Gap(String),
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct CandidateRegistryMutationV1 {
    pub proposal_id: Uuid,
    pub organization_id: Uuid,
    pub semantic_key_hash: String,
    pub operator_rank: u8,
    pub state: CandidateMutationEpistemicState,
    pub proof_refs: Vec<CandidateRegistryRevisionSourceRefV1>,
    pub refutation_refs: Vec<CandidateRegistryRevisionSourceRefV1>,
    pub generation_transition_hash: String,
    pub mutation_hash: String,
    pub decision: CandidateRegistryMutationDecisionV1,
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
pub struct InputProcessingDispositionDecisionV1 {
    pub input_id: Uuid,
    pub disposition: InputProcessingDispositionV1,
    pub reason_code: String,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum InputHypothesisRelationKindV1 {
    CreatesHypothesis,
    SupportsExisting,
    ContradictsExisting,
    QualifiesExisting,
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct InputHypothesisRelationDecisionV1 {
    pub input_id: Uuid,
    pub hypothesis_root_id: Uuid,
    pub relation: InputHypothesisRelationKindV1,
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct CandidateGatePassV1 {
    pub expected_authority: CandidateGateExpectedAuthorityV1,
    pub active_analysis_attempt_id: Uuid,
    pub active_analysis_attempt_ordinal: u32,
    pub mutation_set: Vec<CandidateRegistryMutationV1>,
    pub mutation_set_hash: String,
    pub hypothesis_claim_components: Vec<HypothesisClaimComponentV1>,
    pub hypothesis_claim_component_set_hash: String,
    pub verification_contracts: Vec<VerificationContractV1>,
    pub verification_contract_set_hash: String,
    pub hypothesis_verification_plans: Vec<HypothesisVerificationPlanV1>,
    pub hypothesis_verification_plan_set_hash: String,
    pub input_dispositions: Vec<InputProcessingDispositionDecisionV1>,
    pub input_relations: Vec<InputHypothesisRelationDecisionV1>,
    pub final_submitter_worker_run_id: Uuid,
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct ApplyCandidateGatePass {
    pub fence: CandidateRepositoryWriteFenceV1,
    pub stable_compilation_request_id: Uuid,
    pub stable_apply_request_id: Uuid,
    pub gate_pass: CandidateGatePassV1,
    pub expected_source_head_version: i64,
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct CandidateGenerationSealView {
    pub operation_id: Uuid,
    pub scope_snapshot_id: Uuid,
    pub organization_id: Uuid,
    pub snapshot_id: Uuid,
    pub analysis_attempt_id: Uuid,
    pub generation_id: Uuid,
    pub generation_ordinal: u32,
    pub generation_seal_id: Uuid,
    pub generation_member_count: u32,
    pub generation_member_set_hash: String,
    pub generation_event_set_hash: String,
    pub open_obligation_set_hash: String,
    pub projection_outbox_batch_id: Uuid,
    pub projection_source_batch_seq: i64,
    pub projection_outbox_member_set_hash: String,
    pub post_seal_route: String,
    pub replayed: bool,
}

#[derive(Debug, Clone, PartialEq, Eq, thiserror::Error)]
pub enum HypothesisRegistryError {
    #[error("HYPOTHESIS_REGISTRY_UNAVAILABLE: {0}")]
    Unavailable(String),
    #[error("HYPOTHESIS_REGISTRY_INVALID_REQUEST: {0}")]
    InvalidRequest(String),
    #[error("HYPOTHESIS_REGISTRY_NOT_FOUND: {0}")]
    NotFound(String),
    #[error("HYPOTHESIS_REGISTRY_CONFLICT: {0}")]
    Conflict(String),
    #[error("HYPOTHESIS_REGISTRY_AUTHORITY_MISMATCH: {0}")]
    AuthorityMismatch(String),
    #[error("HYPOTHESIS_REGISTRY_ARTIFACT_KIND_FORBIDDEN: {0}")]
    ArtifactKindForbidden(String),
    #[error("HYPOTHESIS_REGISTRY_STORAGE: {0}")]
    Storage(String),
}

impl HypothesisRegistryError {
    pub const fn code(&self) -> &'static str {
        match self {
            Self::Unavailable(_) => "HYPOTHESIS_REGISTRY_UNAVAILABLE",
            Self::InvalidRequest(_) => "HYPOTHESIS_REGISTRY_INVALID_REQUEST",
            Self::NotFound(_) => "HYPOTHESIS_REGISTRY_NOT_FOUND",
            Self::Conflict(_) => "HYPOTHESIS_REGISTRY_CONFLICT",
            Self::AuthorityMismatch(_) => "HYPOTHESIS_REGISTRY_AUTHORITY_MISMATCH",
            Self::ArtifactKindForbidden(_) => "HYPOTHESIS_REGISTRY_ARTIFACT_KIND_FORBIDDEN",
            Self::Storage(_) => "HYPOTHESIS_REGISTRY_STORAGE",
        }
    }
}

#[async_trait]
pub trait HypothesisRegistryRepository: Send + Sync {
    async fn freeze_candidate_snapshot(
        &self,
        request: FreezeCandidateAnalysisSnapshot,
    ) -> Result<CandidateAnalysisSnapshotView, HypothesisRegistryError>;

    async fn load_snapshot_page(
        &self,
        request: LoadCandidateAnalysisPage,
    ) -> Result<CandidateAnalysisPageView, HypothesisRegistryError>;

    async fn load_snapshot_chunk_page(
        &self,
        request: LoadCandidateInputChunkPage,
    ) -> Result<CandidateInputChunkPageView, HypothesisRegistryError>;

    async fn record_analysis_artifact(
        &self,
        request: RecordCandidateAnalysisArtifact,
    ) -> Result<CandidateAnalysisArtifactReceipt, HypothesisRegistryError>;

    async fn seal_analysis_census(
        &self,
        request: SealCandidateAnalysisCensus,
    ) -> Result<CandidateAnalysisCensusView, HypothesisRegistryError>;

    async fn seal_hypothesis_coverage_subreview_census(
        &self,
        request: SealHypothesisCoverageSubreviewCensus,
    ) -> Result<HypothesisCoverageSubreviewCensusView, HypothesisRegistryError>;

    async fn record_hypothesis_coverage_subreview(
        &self,
        request: RecordHypothesisCoverageSubreview,
    ) -> Result<HypothesisCoverageSubreviewReceipt, HypothesisRegistryError>;

    async fn seal_hypothesis_coverage_synthesis_census(
        &self,
        request: SealHypothesisCoverageSynthesisCensus,
    ) -> Result<HypothesisCoverageSynthesisCensusView, HypothesisRegistryError>;

    async fn record_hypothesis_coverage_synthesis_review(
        &self,
        request: RecordHypothesisCoverageSynthesisReview,
    ) -> Result<HypothesisCoverageSynthesisReceipt, HypothesisRegistryError>;

    async fn reduce_hypothesis_coverage_review(
        &self,
        request: ReduceHypothesisCoverageReview,
    ) -> Result<HypothesisCoverageReviewReceipt, HypothesisRegistryError>;

    async fn seal_candidate_compilation(
        &self,
        request: SealCandidateCompilation,
    ) -> Result<CandidateCompilationSealView, HypothesisRegistryError>;

    async fn load_candidate_gate_material(
        &self,
        request: LoadCandidateGateMaterial,
    ) -> Result<CandidateGateMaterial, HypothesisRegistryError>;

    async fn apply_candidate_gate_pass(
        &self,
        request: ApplyCandidateGatePass,
    ) -> Result<CandidateGenerationSealView, HypothesisRegistryError>;
}
