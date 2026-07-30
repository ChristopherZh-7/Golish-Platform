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

/// Closes the outer Plan B authority sets around already host-compiled plans.
/// Each path's component union/falsifier quantifier and every objective's
/// contract binding were validated by `HypothesisVerificationPlanV1::compile`;
/// this function prevents omission/substitution between those plans and the
/// Gate's frozen exact denominators.
pub(crate) fn validate_compiled_plan_set(
    plans: &[HypothesisVerificationPlanV1],
    expected_plan_hashes: &[String],
    expected_component_hashes: &[String],
    expected_contract_hashes: &[String],
) -> Result<(), golish_core::hypothesis_verification::HypothesisVerificationError> {
    use std::collections::BTreeSet;

    use golish_core::hypothesis_verification::HypothesisVerificationError;

    if plans.is_empty() {
        return Err(HypothesisVerificationError::ProofPathsEmpty);
    }
    let actual_plans = plans
        .iter()
        .map(|plan| plan.plan_hash().to_owned())
        .collect::<BTreeSet<_>>();
    let expected_plans = expected_plan_hashes
        .iter()
        .cloned()
        .collect::<BTreeSet<_>>();
    if actual_plans.len() != plans.len()
        || expected_plans.len() != expected_plan_hashes.len()
        || actual_plans != expected_plans
    {
        return Err(HypothesisVerificationError::ObjectiveDuplicate);
    }

    let expected_components = expected_component_hashes
        .iter()
        .cloned()
        .collect::<BTreeSet<_>>();
    let expected_contracts = expected_contract_hashes
        .iter()
        .cloned()
        .collect::<BTreeSet<_>>();
    let mut actual_components = BTreeSet::new();
    let mut actual_contracts = BTreeSet::new();
    for plan in plans {
        actual_components.extend(
            plan.required_claim_components()
                .iter()
                .map(|component| component.member_hash().to_owned()),
        );
        actual_contracts.extend(
            plan.objectives()
                .iter()
                .map(|objective| objective.verification_contract_hash().to_owned()),
        );
        if plan
            .proof_paths()
            .iter()
            .any(|path| path.members().is_empty())
        {
            return Err(HypothesisVerificationError::ProofPathMembersInvalid);
        }
    }
    if actual_components != expected_components {
        return Err(HypothesisVerificationError::ClaimComponentUncovered);
    }
    if actual_contracts != expected_contracts {
        return Err(HypothesisVerificationError::VerificationContractBindingMismatch);
    }
    Ok(())
}
