use super::types::{ClaimComponentCompilerInput, StructuredClaimComponentSourceV1};
use golish_core::hypothesis_verification::{
    compile_claim_components_v1, HypothesisClaimComponentInputV1, HypothesisClaimComponentKindV1,
    HypothesisClaimComponentV1, HypothesisVerificationPlanBuildInputV1,
    HypothesisVerificationPlanV1,
};

fn typed_components(
    kind: HypothesisClaimComponentKindV1,
    sources: Vec<StructuredClaimComponentSourceV1>,
) -> Vec<HypothesisClaimComponentInputV1> {
    sources
        .into_iter()
        .map(|source| HypothesisClaimComponentInputV1 {
            component_key: source.component_key,
            kind,
            canonical_fragment_hash: source.canonical_fragment_hash,
            canonical_condition_hash: source.canonical_condition_hash,
            required: source.required,
        })
        .collect()
}

pub fn compile_claim_components(
    input: ClaimComponentCompilerInput,
) -> Result<
    Vec<HypothesisClaimComponentV1>,
    golish_core::hypothesis_verification::HypothesisVerificationError,
> {
    if input.claim_clauses.is_empty()
        || input.impact_qualifiers.is_empty()
        || input.trust_boundary_conditions.is_empty()
        || input.identity_conditions.is_empty()
    {
        return Err(
            golish_core::hypothesis_verification::HypothesisVerificationError::ClaimComponentsEmpty,
        );
    }
    let mut components = typed_components(
        HypothesisClaimComponentKindV1::ClaimClause,
        input.claim_clauses,
    );
    components.extend(typed_components(
        HypothesisClaimComponentKindV1::ImpactQualifier,
        input.impact_qualifiers,
    ));
    components.extend(typed_components(
        HypothesisClaimComponentKindV1::TrustBoundaryCondition,
        input.trust_boundary_conditions,
    ));
    components.extend(typed_components(
        HypothesisClaimComponentKindV1::IdentityCondition,
        input.identity_conditions,
    ));
    compile_claim_components_v1(
        input.revision_id,
        input.revision_hash,
        input.derivation_contract_version,
        input.derivation_contract_digest,
        components,
    )
}

#[derive(Debug, Clone)]
pub struct VerificationPlanCompilerInput(pub HypothesisVerificationPlanBuildInputV1);

pub fn compile_verification_plan(
    input: VerificationPlanCompilerInput,
) -> Result<
    HypothesisVerificationPlanV1,
    golish_core::hypothesis_verification::HypothesisVerificationError,
> {
    HypothesisVerificationPlanV1::compile(input.0)
}
