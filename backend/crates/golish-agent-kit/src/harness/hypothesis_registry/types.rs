use golish_core::hypothesis_semantic_key::{
    CandidateMutationEpistemicState, ClaimPolarity, PredicateIdentity, SemanticClaimV1,
};
use serde::{Deserialize, Serialize};
use uuid::Uuid;

/// Model/Controller proposal input.  `deny_unknown_fields` is intentional:
/// contract ids/hashes and exact-set metadata are host-owned and cannot be
/// smuggled into an artifact.
#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct CandidateProposal {
    pub operation_id: Uuid,
    pub organization_id: Uuid,
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
    pub proposer: String,
    pub requested_state: CandidateMutationEpistemicState,
}

impl SemanticClaimV1 for CandidateProposal {
    fn semantic_organization_id(&self) -> Uuid {
        self.organization_id
    }

    fn semantic_subject_kind(&self) -> &str {
        &self.subject_kind
    }

    fn semantic_subject_identity_hash(&self) -> &str {
        &self.subject_identity_hash
    }

    fn semantic_predicate(&self) -> &PredicateIdentity {
        &self.predicate
    }

    fn semantic_trust_boundary(&self) -> &str {
        &self.trust_boundary
    }

    fn semantic_polarity(&self) -> ClaimPolarity {
        self.polarity
    }
}

#[derive(Debug, Clone)]
pub struct StructuredClaimComponentSourceV1 {
    pub component_key: String,
    pub canonical_fragment_hash: String,
    pub canonical_condition_hash: String,
    pub required: bool,
}

#[derive(Debug, Clone)]
pub struct ClaimComponentCompilerInput {
    pub revision_id: Uuid,
    pub revision_hash: String,
    pub derivation_contract_version: u32,
    pub derivation_contract_digest: String,
    pub claim_clauses: Vec<StructuredClaimComponentSourceV1>,
    pub impact_qualifiers: Vec<StructuredClaimComponentSourceV1>,
    pub trust_boundary_conditions: Vec<StructuredClaimComponentSourceV1>,
    pub identity_conditions: Vec<StructuredClaimComponentSourceV1>,
}
