//! Typed, tool-free Candidate hypothesis-analysis runtime contracts.
//!
//! Every body in this module is server-issued or parser-closed.  The runner
//! never receives a live-source handle, a provider/feed refresh handle, or a
//! caller-selected Tool Truth authority bundle.

use async_trait::async_trait;
use serde::{Deserialize, Serialize};
use uuid::Uuid;

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum CandidateAnalysisAgentRole {
    Controller,
    Analyst,
    Critic,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct CandidateAnalysisAgentBinding {
    pub analysis_attempt_id: Uuid,
    pub analysis_attempt_ordinal: u32,
    pub work_item_id: Uuid,
    pub worker_run_id: Uuid,
    pub role: CandidateAnalysisAgentRole,
    pub lane_ordinal: u32,
    pub read_only: bool,
    pub allowed_tools: Vec<String>,
}

impl CandidateAnalysisAgentBinding {
    pub fn validate_tool_free(&self) -> anyhow::Result<()> {
        anyhow::ensure!(self.read_only, "candidate child must be read-only");
        anyhow::ensure!(
            self.allowed_tools == ["submit_result"],
            "candidate child tool set must be exactly submit_result"
        );
        Ok(())
    }
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct CandidateAnalysisAgentAttempt<T> {
    pub provider_attempt_id: Uuid,
    pub output: T,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum CandidateInputKind {
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

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(tag = "kind", rename_all = "snake_case", deny_unknown_fields)]
pub enum CandidateBoundedPayload {
    ToolTruthRecord {
        record_schema: String,
        redacted_fields: Vec<(String, String)>,
    },
    TechniqueOutcome {
        technique_id: String,
        outcome: String,
        evidence_hashes: Vec<String>,
    },
    KnowledgeFeedMatch {
        feed_snapshot_id: Uuid,
        feed_match_member_id: Uuid,
        feed_kind: String,
        feed_version: String,
        published_at_unix_seconds: i64,
        content_hash: String,
        manifest_hash: String,
        provenance_hash: String,
        signature_receipt_hash: String,
        product_version_match_hash: String,
        matcher_hash: String,
        member_hash: String,
        source_authority: CandidateKnowledgeSignalAuthority,
    },
    PreviousGeneration {
        revision_hash: String,
        lifecycle_state: String,
    },
    ResidualOrObligation {
        reason_code: String,
        authority_hash: String,
    },
    ContentAddressedBlob {
        blob_id: Uuid,
        content_hash: String,
        byte_count: u64,
    },
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct CandidateChunkRef {
    pub input_id: Uuid,
    pub input_key: String,
    pub input_kind: CandidateInputKind,
    pub chunk_id: Uuid,
    pub chunk_ordinal: u32,
    pub chunk_census_hash: String,
    pub source_hash: String,
    pub bounded_payload: CandidateBoundedPayload,
    pub bounded_payload_hash: String,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct CandidateControllerDispatchInput {
    pub snapshot_id: Uuid,
    pub snapshot_authority_hash: String,
    pub input_count: u32,
    pub input_chunk_census_set_hash: String,
    pub relationship_cross_index_hash: String,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct CandidateControllerDispatchPlan {
    pub requested_live_lanes: u32,
    pub requested_inputs_per_microbatch: u32,
    pub objective_clusters: Vec<String>,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct CandidateAnalystInput {
    pub microbatch_id: Uuid,
    pub microbatch_ordinal: u32,
    pub chunks: Vec<CandidateChunkRef>,
    pub relationship_cross_index_hash: String,
    pub trust_boundary_cross_index_hash: String,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum CandidateProposalReadiness {
    ReadyForStrategy,
    NeedsEnrichment,
    Deferred,
    OutOfScope,
    Unsafe,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct CandidateProofReference {
    pub input_id: Uuid,
    pub chunk_id: Uuid,
    pub source_hash: String,
    pub role: CandidateProofReferenceRole,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum CandidateProofReferenceRole {
    Support,
    Contradiction,
    AuthorizationUse,
    Gap,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct CandidateKnowledgeSignalReference {
    pub feed_snapshot_id: Uuid,
    pub feed_match_member_id: Uuid,
    pub feed_match_member_hash: String,
    pub product_version_match_hash: String,
    pub source_authority: CandidateKnowledgeSignalAuthority,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum CandidateKnowledgeSignalAuthority {
    KnowledgeSignalOnly,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct CandidateHypothesisProposal {
    pub proposal_id: Uuid,
    pub subject_kind: String,
    pub subject_identity_hash: String,
    pub predicate_schema: String,
    pub predicate_version: u32,
    pub predicate_arguments: Vec<(String, String)>,
    pub trust_boundary: String,
    pub polarity: String,
    pub structured_claim: String,
    pub preconditions: Vec<String>,
    pub impact: String,
    pub proof_refs: Vec<CandidateProofReference>,
    pub knowledge_signals: Vec<CandidateKnowledgeSignalReference>,
    pub readiness: CandidateProposalReadiness,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct HypothesisProposalArtifact {
    pub proposals: Vec<CandidateHypothesisProposal>,
    pub blocked_input_ids: Vec<Uuid>,
    pub blocker_codes: Vec<String>,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct CandidateProposalRef {
    pub proposal_id: Uuid,
    pub proposal_hash: String,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct CandidateCoverageNodeInput {
    pub synthesis_census_id: Uuid,
    pub synthesis_node_id: Uuid,
    pub level: u32,
    pub partition_ordinal: u32,
    pub node_hash: String,
    pub child_receipt_count: u32,
    pub child_receipt_set_hash: String,
    pub descendant_worker_set_hash: String,
    pub relationship_cross_index_hash: String,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(tag = "mode", rename_all = "snake_case", deny_unknown_fields)]
pub enum CandidateCriticInput {
    ProposalConflict {
        conflict_component_id: Uuid,
        conflict_component_hash: String,
        proposals: Vec<CandidateProposalRef>,
    },
    CoverageSubreview {
        subreview_census_id: Uuid,
        subreview_census_member_id: Uuid,
        snapshot_input_id: Uuid,
        checklist_member_id: Uuid,
        chunk_partition_id: Uuid,
        designated_chunks: Vec<CandidateChunkRef>,
        h1_proposal_refs: Vec<CandidateProposalRef>,
        read_receipt_set_hash: String,
    },
    CoverageCrossChunkSynthesis {
        node: CandidateCoverageNodeInput,
    },
    CoverageCrossInputPartition {
        node: CandidateCoverageNodeInput,
    },
    CoverageCrossInputReduce {
        node: CandidateCoverageNodeInput,
    },
    CoverageCrossDimensionReduce {
        node: CandidateCoverageNodeInput,
    },
    CoverageGlobalSemanticRoot {
        node: CandidateCoverageNodeInput,
    },
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum CandidateCriticOutcome {
    NoMiss,
    MissedHypothesis,
    Blocked,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct CandidateLocalCoverageFinding {
    pub outcome: CandidateCriticOutcome,
    pub missed_hypothesis_refs: Vec<Uuid>,
    pub blocker_codes: Vec<String>,
    pub context_truncated: bool,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(tag = "mode", rename_all = "snake_case", deny_unknown_fields)]
pub enum HypothesisCriticArtifact {
    ProposalConflict {
        conflict_component_id: Uuid,
        outcome: CandidateCriticOutcome,
        related_proposal_ids: Vec<Uuid>,
    },
    CoverageSubreview {
        subreview_census_member_id: Uuid,
        finding: CandidateLocalCoverageFinding,
    },
    CoverageCrossChunkSynthesis {
        synthesis_node_id: Uuid,
        finding: CandidateLocalCoverageFinding,
        descendant_worker_set_hash: String,
    },
    CoverageCrossInputPartition {
        synthesis_node_id: Uuid,
        finding: CandidateLocalCoverageFinding,
        descendant_worker_set_hash: String,
    },
    CoverageCrossInputReduce {
        synthesis_node_id: Uuid,
        finding: CandidateLocalCoverageFinding,
        descendant_worker_set_hash: String,
    },
    CoverageCrossDimensionReduce {
        synthesis_node_id: Uuid,
        finding: CandidateLocalCoverageFinding,
        descendant_worker_set_hash: String,
    },
    CoverageGlobalSemanticRoot {
        synthesis_node_id: Uuid,
        finding: CandidateLocalCoverageFinding,
        descendant_worker_set_hash: String,
    },
}

impl HypothesisCriticArtifact {
    pub fn is_blocked_or_truncated(&self) -> bool {
        match self {
            Self::ProposalConflict { outcome, .. } => *outcome == CandidateCriticOutcome::Blocked,
            Self::CoverageSubreview { finding, .. }
            | Self::CoverageCrossChunkSynthesis { finding, .. }
            | Self::CoverageCrossInputPartition { finding, .. }
            | Self::CoverageCrossInputReduce { finding, .. }
            | Self::CoverageCrossDimensionReduce { finding, .. }
            | Self::CoverageGlobalSemanticRoot { finding, .. } => {
                finding.outcome == CandidateCriticOutcome::Blocked || finding.context_truncated
            }
        }
    }

    pub fn found_miss(&self) -> bool {
        match self {
            Self::ProposalConflict { outcome, .. } => {
                *outcome == CandidateCriticOutcome::MissedHypothesis
            }
            Self::CoverageSubreview { finding, .. }
            | Self::CoverageCrossChunkSynthesis { finding, .. }
            | Self::CoverageCrossInputPartition { finding, .. }
            | Self::CoverageCrossInputReduce { finding, .. }
            | Self::CoverageCrossDimensionReduce { finding, .. }
            | Self::CoverageGlobalSemanticRoot { finding, .. } => {
                finding.outcome == CandidateCriticOutcome::MissedHypothesis
            }
        }
    }
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct CandidateControllerFinalInput {
    pub snapshot_id: Uuid,
    pub analysis_attempt_id: Uuid,
    pub proposal_census_hash: String,
    pub critic_census_hash: String,
    pub coverage_review_set_hash: String,
    pub claim_component_set_hash: String,
    pub verification_contract_set_hash: String,
    pub verification_plan_set_hash: String,
    pub cluster_page_hashes: Vec<String>,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum CandidateControllerDecisionKind {
    Accept,
    AttachExisting,
    Merge,
    Split,
    NarrowSuccessor,
    Blocked,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct CandidateControllerDecision {
    pub proposal_id: Uuid,
    pub decision: CandidateControllerDecisionKind,
    pub related_proposal_ids: Vec<Uuid>,
    pub rationale: String,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct CandidateControllerDecisionArtifact {
    pub decisions: Vec<CandidateControllerDecision>,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum CandidateRuntimeSnapshotDisposition {
    SealedReady,
    BlockedAuthorityBundle,
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct CandidateRuntimeSnapshot {
    pub snapshot_id: Uuid,
    pub disposition: CandidateRuntimeSnapshotDisposition,
    pub input_count: u32,
    pub snapshot_authority_hash: String,
    pub input_chunk_census_set_hash: String,
    pub blocked_residual_hash: Option<String>,
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct CandidateRuntimeAttempt {
    pub analysis_attempt_id: Uuid,
    pub analysis_attempt_ordinal: u32,
    pub controller_binding: CandidateAnalysisAgentBinding,
    pub controller_dispatch_input: CandidateControllerDispatchInput,
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct CandidateAnalystWorkItem {
    pub binding: CandidateAnalysisAgentBinding,
    pub input: CandidateAnalystInput,
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct CandidateCriticWorkItem {
    pub binding: CandidateAnalysisAgentBinding,
    pub input: CandidateCriticInput,
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct CandidateCriticWavePlan {
    pub work_items: Vec<CandidateCriticWorkItem>,
    pub h1_census_hash: String,
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct CandidateFinalizationPlan {
    pub binding: CandidateAnalysisAgentBinding,
    pub input: CandidateControllerFinalInput,
    pub claim_component_compilation: CandidateClaimComponentCompilation,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum CandidateVerificationPlanScope {
    ExactOriginalClaim,
    NarrowSuccessor,
}

/// Host-derived coverage seal. Agent output never supplies these counts or
/// hashes; the runtime uses them to prevent a partial plan from sealing a
/// wider claim.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct CandidateClaimComponentCompilation {
    pub claim_component_count: u32,
    pub planned_component_count: u32,
    pub claim_component_set_hash: String,
    pub planned_component_set_hash: String,
    pub plan_scope: CandidateVerificationPlanScope,
    pub incomplete_residual_hash: String,
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct CandidateArtifactReceipt {
    pub artifact_id: Uuid,
    pub artifact_hash: String,
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub enum CandidateArtifactPersistence {
    Committed(CandidateArtifactReceipt),
    ResponseLost,
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub enum CandidateCriticReduction {
    Ready(Box<CandidateFinalizationPlan>),
    RetryAttempt { next_attempt_ordinal: u32 },
    Blocked { residual_hash: String },
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub enum CandidateAuthorityRevalidation {
    Fresh,
    Invalidated {
        replacement_snapshot_id: Uuid,
        residual_hash: String,
    },
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct HypothesisAnalysisStageRequest {
    pub stable_request_id: Uuid,
    pub operation_id: Uuid,
    pub scope_snapshot_id: Uuid,
    pub organization_id: Uuid,
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub enum HypothesisAnalysisStageOutcome {
    BlockedAuthorityBundle {
        snapshot_id: Uuid,
        residual_hash: String,
    },
    BlockedAnalysis {
        snapshot_id: Uuid,
        residual_hash: String,
    },
    AuthorityInvalidated {
        snapshot_id: Uuid,
        replacement_snapshot_id: Uuid,
        residual_hash: String,
    },
    AnalysisArtifactsReady {
        snapshot_id: Uuid,
        analysis_attempt_id: Uuid,
        analysis_attempt_ordinal: u32,
        analyst_work_item_count: u32,
        critic_work_item_count: u32,
        peak_live_lanes: u32,
        final_receipt: CandidateArtifactReceipt,
    },
}

#[async_trait]
pub trait HypothesisAnalysisAgentRunner: Send + Sync {
    async fn run_controller_dispatch(
        &self,
        binding: CandidateAnalysisAgentBinding,
        input: CandidateControllerDispatchInput,
    ) -> anyhow::Result<CandidateAnalysisAgentAttempt<CandidateControllerDispatchPlan>>;

    async fn run_analyst(
        &self,
        binding: CandidateAnalysisAgentBinding,
        input: CandidateAnalystInput,
    ) -> anyhow::Result<CandidateAnalysisAgentAttempt<HypothesisProposalArtifact>>;

    async fn run_critic(
        &self,
        binding: CandidateAnalysisAgentBinding,
        input: CandidateCriticInput,
    ) -> anyhow::Result<CandidateAnalysisAgentAttempt<HypothesisCriticArtifact>>;

    async fn run_controller_final(
        &self,
        binding: CandidateAnalysisAgentBinding,
        input: CandidateControllerFinalInput,
    ) -> anyhow::Result<CandidateAnalysisAgentAttempt<CandidateControllerDecisionArtifact>>;
}

#[async_trait]
pub trait HypothesisAnalysisRuntimeRepository: Send + Sync {
    async fn freeze_snapshot(
        &self,
        request: HypothesisAnalysisStageRequest,
    ) -> anyhow::Result<CandidateRuntimeSnapshot>;
    async fn open_attempt(
        &self,
        snapshot_id: Uuid,
        attempt_ordinal: u32,
    ) -> anyhow::Result<CandidateRuntimeAttempt>;
    async fn prepare_analyst_wave(
        &self,
        attempt: &CandidateRuntimeAttempt,
        dispatch: &CandidateControllerDispatchPlan,
        host_lane_limit: usize,
    ) -> anyhow::Result<Vec<CandidateAnalystWorkItem>>;
    async fn persist_analyst_artifact(
        &self,
        binding: &CandidateAnalysisAgentBinding,
        attempt: &CandidateAnalysisAgentAttempt<HypothesisProposalArtifact>,
    ) -> anyhow::Result<CandidateArtifactPersistence>;
    async fn prepare_critic_wave(
        &self,
        attempt: &CandidateRuntimeAttempt,
    ) -> anyhow::Result<CandidateCriticWavePlan>;
    async fn persist_critic_artifact(
        &self,
        binding: &CandidateAnalysisAgentBinding,
        attempt: &CandidateAnalysisAgentAttempt<HypothesisCriticArtifact>,
    ) -> anyhow::Result<CandidateArtifactPersistence>;
    async fn load_artifact_receipt(
        &self,
        provider_attempt_id: Uuid,
    ) -> anyhow::Result<Option<CandidateArtifactReceipt>>;
    async fn reduce_and_seal_critic_wave(
        &self,
        attempt: &CandidateRuntimeAttempt,
    ) -> anyhow::Result<CandidateCriticReduction>;
    async fn revalidate_authority(
        &self,
        snapshot_id: Uuid,
        analysis_attempt_id: Uuid,
    ) -> anyhow::Result<CandidateAuthorityRevalidation>;
    async fn persist_controller_final(
        &self,
        binding: &CandidateAnalysisAgentBinding,
        attempt: &CandidateAnalysisAgentAttempt<CandidateControllerDecisionArtifact>,
    ) -> anyhow::Result<CandidateArtifactPersistence>;
}

#[async_trait]
pub trait HypothesisAnalysisStageRuntime: Send + Sync {
    async fn run(
        &self,
        request: HypothesisAnalysisStageRequest,
        runner: &dyn HypothesisAnalysisAgentRunner,
    ) -> anyhow::Result<HypothesisAnalysisStageOutcome>;
}
