use golish_core::hypothesis_semantic_key::ClaimPolarity;
use golish_core::verification_contract::{
    CanonicalJsonObject, ContractCombinatorV1, OrderedSequenceStepInputV1,
    PairedDifferentialBindingInputV1, PredicateComponentInputV1, VerificationContractBuildInputV1,
    VerificationContractError, VerificationContractV1, VerificationControlInputV1,
};
use uuid::Uuid;

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct PredicateRegistryEntry {
    pub semantic_key: String,
    pub predicate_schema: String,
    pub predicate_version: u32,
    pub normalized_arguments: CanonicalJsonObject,
    pub expected_polarity: ClaimPolarity,
    pub prerequisite_hash: String,
}

impl From<PredicateRegistryEntry> for PredicateComponentInputV1 {
    fn from(value: PredicateRegistryEntry) -> Self {
        Self {
            semantic_key: value.semantic_key,
            predicate_schema: value.predicate_schema,
            predicate_version: value.predicate_version,
            normalized_arguments: value.normalized_arguments,
            expected_polarity: value.expected_polarity,
            prerequisite_hash: value.prerequisite_hash,
        }
    }
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct VerificationContractCompilerInput {
    pub revision_id: Uuid,
    pub revision_hash: String,
    pub objective_id: Uuid,
    pub combinator: ContractCombinatorV1,
    pub predicate_registry_entries: Vec<PredicateRegistryEntry>,
    pub required_controls: Vec<VerificationControlInputV1>,
    pub paired_differential_bindings: Vec<PairedDifferentialBindingInputV1>,
    pub ordered_steps: Vec<OrderedSequenceStepInputV1>,
    pub stopping_criteria_hash: String,
    pub compiler_digest: String,
    pub rule_digest: String,
    pub policy_snapshot_hash: String,
}

pub fn compile_verification_contract(
    input: VerificationContractCompilerInput,
) -> Result<VerificationContractV1, VerificationContractError> {
    VerificationContractV1::compile(VerificationContractBuildInputV1 {
        revision_id: input.revision_id,
        revision_hash: input.revision_hash,
        objective_id: input.objective_id,
        combinator: input.combinator,
        predicate_components: input
            .predicate_registry_entries
            .into_iter()
            .map(Into::into)
            .collect(),
        required_controls: input.required_controls,
        paired_differential_bindings: input.paired_differential_bindings,
        ordered_steps: input.ordered_steps,
        stopping_criteria_hash: input.stopping_criteria_hash,
        compiler_digest: input.compiler_digest,
        rule_digest: input.rule_digest,
        policy_snapshot_hash: input.policy_snapshot_hash,
    })
}
