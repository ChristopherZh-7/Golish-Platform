//! Typed Candidate hypothesis, proof, projection, and finalization contracts.

use serde::{Deserialize, Serialize};
use uuid::Uuid;

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum CandidateInputKind {
    ToolTruthFact,
    ToolTruthObservation,
    ToolTruthEvidence,
    TechniqueOutcome,
    ApplicationContext,
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
    /// Always false for frozen source material. This explicit field prevents
    /// prompt text from being promoted to host instruction authority.
    pub instruction_authority: bool,
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

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum CandidateCoverageSemanticObservationKind {
    PotentialHypothesis,
    SupportingPattern,
    ContradictingPattern,
    CoverageGap,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct CandidateCoverageSemanticObservation {
    pub kind: CandidateCoverageSemanticObservationKind,
    pub subject_kind: String,
    pub subject_identity_hash: String,
    pub predicate_schema: String,
    pub predicate_version: u32,
    pub polarity: String,
    pub trust_boundary: String,
    pub input_ids: Vec<Uuid>,
    pub checklist_member_ids: Vec<Uuid>,
    pub proposal_ids: Vec<Uuid>,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct CandidateCoverageSemanticSummary {
    pub covered_input_ids: Vec<Uuid>,
    pub covered_checklist_member_ids: Vec<Uuid>,
    pub observed_proposal_ids: Vec<Uuid>,
    pub missed_checklist_member_ids: Vec<Uuid>,
    pub blocker_codes: Vec<String>,
    pub semantic_observations: Vec<CandidateCoverageSemanticObservation>,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct CandidateControllerProposalSummary {
    pub proposal_id: Uuid,
    pub semantic_key_hash: String,
    pub structured_claim: String,
    pub trust_boundary: String,
    pub polarity: String,
    pub route_kind: String,
    pub proof_ref_count: u32,
    pub refutation_ref_count: u32,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct CandidateControllerProposalPage {
    pub page_ordinal: u32,
    pub proposal_count: u32,
    pub proposals: Vec<CandidateControllerProposalSummary>,
    pub page_hash: String,
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
    pub proposal_pages: Vec<CandidateControllerProposalPage>,
    pub proposal_page_set_hash: String,
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
