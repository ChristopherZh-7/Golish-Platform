//! Candidate/Hypothesis Registry host boundary.

mod reducer;
mod rollout;
mod semantic_key;
mod types;
mod verification_contract_compiler;
mod verification_plan_compiler;

pub use reducer::{
    reduce_proposals, ReducerCatalog, ReducerDecision, ReducerError, ReducerMutationSet,
    ReducerOperatorInputV1, ReducerProposal,
};
pub use rollout::{
    candidate_mutation_state, freeze_candidate_authority_bundle,
    CandidateAuthorityBundleSnapshotV1, CandidateAuthorityRootSnapshotV1,
    CandidateAuthoritySnapshotDispositionV1, CandidateMutationError,
};
pub use semantic_key::{derive_root_id, initial_root_id, merge_root_id, split_root_id};
pub use types::{CandidateProposal, ClaimComponentCompilerInput, StructuredClaimComponentSourceV1};
pub use verification_contract_compiler::{
    compile_verification_contract, PredicateRegistryEntry, VerificationContractCompilerInput,
};
pub use verification_plan_compiler::{
    compile_claim_components, compile_verification_plan, VerificationPlanCompilerInput,
};

pub use golish_core::hypothesis_semantic_key::{
    AtTimeSubjectIdentity, CandidateMutationEpistemicState, ClaimPolarity, HypothesisSemanticKeyV1,
    PredicateIdentity,
};
