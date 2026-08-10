//! Deterministic action-oracle and objective-local Campaign adjudication.

use golish_core::verification_contract::{
    ContractCombinatorV1, VerificationContractError, VerificationContractV1,
};
use std::collections::{BTreeMap, BTreeSet};

use super::gate::validate_coverage_results;
use super::types::{
    canonicalize_action_kind, canonicalize_hashes, execution_key_hash, hash_domain, require_hash,
    validate_concurrent_group_receipt, ActionOracleAssessmentV1, ActionOracleContractV1,
    ArtifactAuthorityV1, CampaignAdjudicationV1, ClaimComponentCampaignOutcomeV1,
    ComponentOracleOutcomeV1, ControlValidityV1, CoverageObligationKindV1, CoverageResultStatusV1,
    ExecutionLandingStateV1, ObjectiveCampaignOutcome, ObligationDispositionSetV1,
    ObservationCompletenessV1, OracleCensusV1, OracleLimitationCodeV1,
    OrderedSequenceObservationV1, PairedRelationOutcomeV1, PreconditionStatusV1,
    PredicateOracleAssessmentV1, PreparedActionKindV1, ReconciledExecutionReceiptV1,
};

pub fn reduce_action_oracle(
    contract: &ActionOracleContractV1,
    receipt: &ReconciledExecutionReceiptV1,
) -> Result<ActionOracleAssessmentV1, VerificationContractError> {
    if contract.contract_version != 1 || receipt.receipt_version != 1 {
        return Err(VerificationContractError::InvalidField(
            "action oracle contract version",
        ));
    }
    if contract.prepared_action_id.is_nil()
        || contract.objective_id.is_nil()
        || contract.predicate_component_member_hashes.is_empty()
    {
        return Err(VerificationContractError::InvalidField(
            "action oracle contract identity",
        ));
    }
    let expected_action_contract_hash = hash_domain(
        "action_oracle_contract.v1",
        &(
            contract.contract_version,
            contract.prepared_action_id,
            contract.objective_id,
            &contract.plan_objective_member_hash,
            &contract.verification_contract_hash,
            &contract.predicate_component_member_hashes,
            &contract.required_control_member_hashes,
            &contract.claim_component_bindings,
            &contract.action_kind,
        ),
    )?;
    if expected_action_contract_hash != contract.action_oracle_contract_hash {
        return Err(VerificationContractError::PersistedMismatch(
            "action oracle contract hash",
        ));
    }
    let mut canonical_action_kind = contract.action_kind.clone();
    canonicalize_action_kind(&mut canonical_action_kind)?;
    if canonical_action_kind != contract.action_kind {
        return Err(VerificationContractError::PersistedMismatch(
            "action oracle action kind",
        ));
    }
    let mut predicate_members = contract.predicate_component_member_hashes.clone();
    canonicalize_hashes(&mut predicate_members)?;
    let mut control_members = contract.required_control_member_hashes.clone();
    canonicalize_hashes(&mut control_members)?;
    let predicate_member_set = predicate_members
        .iter()
        .map(String::as_str)
        .collect::<BTreeSet<_>>();
    let binding_set = contract
        .claim_component_bindings
        .iter()
        .map(|binding| {
            (
                binding.predicate_component_member_hash.as_str(),
                binding.claim_component_member_hash.as_str(),
            )
        })
        .collect::<BTreeSet<_>>();
    if binding_set.len() != contract.claim_component_bindings.len()
        || contract.claim_component_bindings.iter().any(|binding| {
            require_hash(&binding.predicate_component_member_hash).is_err()
                || require_hash(&binding.claim_component_member_hash).is_err()
                || !predicate_member_set.contains(binding.predicate_component_member_hash.as_str())
        })
        || predicate_member_set.iter().any(|member| {
            !contract
                .claim_component_bindings
                .iter()
                .any(|binding| binding.predicate_component_member_hash.as_str() == *member)
        })
    {
        return Err(VerificationContractError::InvalidReference(
            "action oracle claim component binding census",
        ));
    }
    if receipt.prepared_action_id != contract.prepared_action_id
        || receipt.execution_key.prepared_action_id != contract.prepared_action_id
        || receipt.verification_contract_hash != contract.verification_contract_hash
    {
        return Err(VerificationContractError::InvalidReference(
            "execution receipt action binding",
        ));
    }
    if receipt.landing_state != ExecutionLandingStateV1::LandedReconciled {
        return Err(VerificationContractError::InvalidReference(
            "execution receipt is not landed and reconciled",
        ));
    }
    require_hash(&contract.plan_objective_member_hash)?;
    require_hash(&contract.verification_contract_hash)?;
    require_hash(&contract.action_oracle_contract_hash)?;
    let execution_key_hash = execution_key_hash(&receipt.execution_key)?;

    let expected_predicates = contract
        .predicate_component_member_hashes
        .iter()
        .map(String::as_str)
        .collect::<BTreeSet<_>>();
    let actual_predicates = receipt
        .predicate_observations
        .iter()
        .map(|observation| observation.predicate_component_member_hash.as_str())
        .collect::<BTreeSet<_>>();
    if actual_predicates != expected_predicates
        || actual_predicates.len() != receipt.predicate_observations.len()
        || receipt.predicate_observations.iter().any(|observation| {
            contract
                .predicate_component_member_hashes
                .get(observation.predicate_ordinal as usize)
                != Some(&observation.predicate_component_member_hash)
        })
    {
        return Err(VerificationContractError::InvalidReference(
            "action predicate observation census",
        ));
    }

    let mut observed_controls = receipt.observed_control_member_hashes.clone();
    canonicalize_hashes(&mut observed_controls)?;
    if observed_controls != control_members {
        return Err(VerificationContractError::InvalidReference(
            "action control observation census",
        ));
    }
    match (
        contract.required_control_member_hashes.is_empty(),
        receipt.control,
    ) {
        (true, ControlValidityV1::NotRequired)
        | (false, ControlValidityV1::Valid | ControlValidityV1::Invalid) => {}
        _ => {
            return Err(VerificationContractError::InvalidField(
                "action control validity",
            ))
        }
    }

    let mut limitation_codes = Vec::new();
    if receipt.precondition != PreconditionStatusV1::Satisfied {
        limitation_codes.push(OracleLimitationCodeV1::PreconditionUnsatisfied);
    }
    if receipt.control == ControlValidityV1::Invalid {
        limitation_codes.push(OracleLimitationCodeV1::ControlInvalid);
    }
    if receipt.completeness == ObservationCompletenessV1::Partial {
        limitation_codes.push(OracleLimitationCodeV1::CoveragePartial);
    }
    if !receipt.cleanup_complete {
        limitation_codes.push(OracleLimitationCodeV1::CleanupIncomplete);
    }

    let concurrent_group_valid = match &contract.action_kind {
        PreparedActionKindV1::SingleActionV1 => receipt.concurrent_group_receipt.is_none(),
        PreparedActionKindV1::ConcurrentActionGroupV1(group) => {
            validate_concurrent_group_receipt(group, receipt.concurrent_group_receipt.as_ref())
        }
    };
    if !concurrent_group_valid {
        limitation_codes.push(OracleLimitationCodeV1::RaceGroupIncomplete);
    }
    limitation_codes.sort();
    limitation_codes.dedup();

    let force_inconclusive = limitation_codes
        .iter()
        .any(|limitation| !matches!(limitation, OracleLimitationCodeV1::CleanupIncomplete));
    let force_blocked = limitation_codes.contains(&OracleLimitationCodeV1::CleanupIncomplete);
    let mut predicate_outcomes = Vec::with_capacity(receipt.predicate_observations.len());
    for observation in &receipt.predicate_observations {
        let mut claims = contract
            .claim_component_bindings
            .iter()
            .filter(|binding| {
                binding.predicate_component_member_hash
                    == observation.predicate_component_member_hash
            })
            .map(|binding| binding.claim_component_member_hash.clone())
            .collect::<Vec<_>>();
        canonicalize_hashes(&mut claims)?;
        if claims.is_empty() {
            return Err(VerificationContractError::InvalidReference(
                "predicate claim component binding",
            ));
        }
        let outcome = if force_blocked {
            ComponentOracleOutcomeV1::Blocked
        } else if force_inconclusive
            || (observation.outcome == ComponentOracleOutcomeV1::Refutation
                && (!observation.deterministic_negative
                    || !observation.observation_window_complete))
        {
            ComponentOracleOutcomeV1::Inconclusive
        } else {
            observation.outcome
        };
        predicate_outcomes.push(PredicateOracleAssessmentV1 {
            predicate_component_member_hash: observation.predicate_component_member_hash.clone(),
            predicate_ordinal: observation.predicate_ordinal,
            claim_component_member_hashes: claims,
            outcome,
        });
    }
    predicate_outcomes.sort_by_key(|outcome| outcome.predicate_ordinal);

    let paired_relation = if force_inconclusive || force_blocked {
        None
    } else {
        receipt.paired_relation.clone()
    };
    let mut ordered_sequence = receipt.ordered_sequence.clone();
    ordered_sequence.sort_by_key(|observation| observation.step_ordinal);
    let assessment_hash = hash_domain(
        "action_oracle_assessment.v1",
        &(
            1_u32,
            contract.prepared_action_id,
            &contract.plan_objective_member_hash,
            &contract.verification_contract_hash,
            &contract.action_oracle_contract_hash,
            &execution_key_hash,
            &predicate_outcomes,
            &contract.required_control_member_hashes,
            receipt.control,
            &paired_relation,
            &ordered_sequence,
            concurrent_group_valid,
            &limitation_codes,
        ),
    )?;
    Ok(ActionOracleAssessmentV1 {
        assessment_version: 1,
        authority: ArtifactAuthorityV1::Canonical,
        prepared_action_id: contract.prepared_action_id,
        plan_objective_member_hash: contract.plan_objective_member_hash.clone(),
        verification_contract_hash: contract.verification_contract_hash.clone(),
        action_oracle_contract_hash: contract.action_oracle_contract_hash.clone(),
        execution_key_hash,
        predicate_outcomes,
        required_control_member_hashes: contract.required_control_member_hashes.clone(),
        control: receipt.control,
        paired_relation,
        ordered_sequence,
        concurrent_group_valid,
        limitation_codes,
        assessment_hash,
    })
}

pub fn adjudicate_campaign(
    contract: &VerificationContractV1,
    census: &OracleCensusV1,
    obligations: &ObligationDispositionSetV1,
) -> Result<CampaignAdjudicationV1, VerificationContractError> {
    if census.authority != ArtifactAuthorityV1::Canonical {
        return Err(VerificationContractError::InvalidReference(
            "shadow oracle census",
        ));
    }
    if census.verification_contract_hash != contract.contract_hash()
        || obligations.denominator.verification_contract_hash != contract.contract_hash()
        || obligations.denominator.objective_id != contract.objective_id()
    {
        return Err(VerificationContractError::InvalidReference(
            "Campaign contract binding",
        ));
    }
    validate_denominator_against_contract(contract, obligations)?;
    validate_coverage_results(&obligations.denominator, &obligations.results).map_err(|_| {
        VerificationContractError::InvalidReference("Campaign coverage result census")
    })?;

    let mut assessment_hashes = census
        .assessments
        .iter()
        .map(|assessment| assessment.assessment_hash.clone())
        .collect::<Vec<_>>();
    canonicalize_hashes(&mut assessment_hashes)?;
    for assessment in &census.assessments {
        if assessment.assessment_version != 1
            || assessment.authority != ArtifactAuthorityV1::Canonical
            || assessment.verification_contract_hash != contract.contract_hash()
            || assessment.plan_objective_member_hash
                != obligations.denominator.plan_objective_member_hash
        {
            return Err(VerificationContractError::InvalidReference(
                "Campaign oracle assessment binding",
            ));
        }
        require_hash(&assessment.assessment_hash)?;
        let expected_assessment_hash = hash_domain(
            "action_oracle_assessment.v1",
            &(
                assessment.assessment_version,
                assessment.prepared_action_id,
                &assessment.plan_objective_member_hash,
                &assessment.verification_contract_hash,
                &assessment.action_oracle_contract_hash,
                &assessment.execution_key_hash,
                &assessment.predicate_outcomes,
                &assessment.required_control_member_hashes,
                assessment.control,
                &assessment.paired_relation,
                &assessment.ordered_sequence,
                assessment.concurrent_group_valid,
                &assessment.limitation_codes,
            ),
        )?;
        if expected_assessment_hash != assessment.assessment_hash {
            return Err(VerificationContractError::PersistedMismatch(
                "action oracle assessment hash",
            ));
        }
        validate_action_assessment_semantics(contract, obligations, assessment)?;
    }
    for result in &obligations.results {
        if let CoverageResultStatusV1::Tested {
            prepared_action_id,
            oracle_assessment_hash,
            ..
        } = &result.status
        {
            let assessment = census.assessments.iter().find(|assessment| {
                assessment.prepared_action_id == *prepared_action_id
                    && assessment.assessment_hash == *oracle_assessment_hash
            });
            let member = obligations
                .denominator
                .members
                .iter()
                .find(|member| member.member_hash == result.denominator_member_hash);
            let binding_valid = match (member, assessment) {
                (Some(member), Some(assessment)) => match member.kind {
                    CoverageObligationKindV1::Predicate => {
                        assessment.predicate_outcomes.iter().any(|outcome| {
                            outcome.predicate_component_member_hash == member.contract_member_hash
                        })
                    }
                    CoverageObligationKindV1::RequiredControl => assessment
                        .required_control_member_hashes
                        .contains(&member.contract_member_hash),
                },
                _ => false,
            };
            if !binding_valid {
                return Err(VerificationContractError::InvalidReference(
                    "tested coverage oracle binding",
                ));
            }
        }
    }

    let expected_predicates = contract
        .predicate_components()
        .iter()
        .map(|component| component.member_hash().to_owned())
        .collect::<Vec<_>>();
    let mut predicate_outcomes = Vec::with_capacity(expected_predicates.len());
    for (ordinal, expected_hash) in expected_predicates.iter().enumerate() {
        let observed = census
            .assessments
            .iter()
            .flat_map(|assessment| assessment.predicate_outcomes.iter())
            .filter(|outcome| &outcome.predicate_component_member_hash == expected_hash)
            .collect::<Vec<_>>();
        let outcome = reduce_component_observations(&observed);
        let mut claims = obligations
            .denominator
            .members
            .iter()
            .filter(|member| member.contract_member_hash == *expected_hash)
            .flat_map(|member| member.claim_component_member_hashes.iter().cloned())
            .collect::<Vec<_>>();
        canonicalize_hashes(&mut claims)?;
        predicate_outcomes.push(PredicateOracleAssessmentV1 {
            predicate_component_member_hash: expected_hash.clone(),
            predicate_ordinal: ordinal as u32,
            claim_component_member_hashes: claims,
            outcome,
        });
    }

    let all_tested = obligations
        .results
        .iter()
        .all(|result| matches!(result.status, CoverageResultStatusV1::Tested { .. }));
    let has_blocked_coverage = obligations
        .results
        .iter()
        .any(|result| matches!(result.status, CoverageResultStatusV1::Blocked { .. }));
    let local_truth = match contract.combinator() {
        ContractCombinatorV1::AllOf => reduce_all_of(&predicate_outcomes),
        ContractCombinatorV1::AnyOf => reduce_any_of(&predicate_outcomes),
        ContractCombinatorV1::PairedDifferential => reduce_paired_differential(contract, census),
        ContractCombinatorV1::OrderedSequence => reduce_ordered_sequence(contract, census),
    };
    let mut outcome = if !all_tested {
        if has_blocked_coverage {
            ObjectiveCampaignOutcome::Blocked
        } else {
            ObjectiveCampaignOutcome::ExhaustedWithResiduals
        }
    } else {
        match local_truth {
            ComponentOracleOutcomeV1::Proof => ObjectiveCampaignOutcome::Proof,
            ComponentOracleOutcomeV1::Refutation => ObjectiveCampaignOutcome::Refutation,
            ComponentOracleOutcomeV1::Blocked => ObjectiveCampaignOutcome::Blocked,
            ComponentOracleOutcomeV1::Inconclusive => ObjectiveCampaignOutcome::Inconclusive,
        }
    };

    let mut claim_component_outcomes = reduce_claim_components(
        &obligations.denominator.claim_component_member_hashes,
        &predicate_outcomes,
    )?;
    if !all_tested {
        let coverage_outcome = if has_blocked_coverage {
            ComponentOracleOutcomeV1::Blocked
        } else {
            ComponentOracleOutcomeV1::Inconclusive
        };
        for claim in &mut claim_component_outcomes {
            claim.outcome = coverage_outcome;
        }
    } else if matches!(
        contract.combinator(),
        ContractCombinatorV1::PairedDifferential | ContractCombinatorV1::OrderedSequence
    ) {
        for claim in &mut claim_component_outcomes {
            claim.outcome = local_truth;
        }
    }
    if outcome == ObjectiveCampaignOutcome::Proof
        && !claim_component_outcomes
            .iter()
            .all(|claim| claim.outcome == ComponentOracleOutcomeV1::Proof)
    {
        outcome = ObjectiveCampaignOutcome::Inconclusive;
    }
    let mut residual_risk_hashes = obligations
        .results
        .iter()
        .filter_map(|result| match &result.status {
            CoverageResultStatusV1::Tested { .. } => None,
            CoverageResultStatusV1::Untested { residual_risk_hash }
            | CoverageResultStatusV1::Degraded { residual_risk_hash }
            | CoverageResultStatusV1::Blocked { residual_risk_hash } => {
                Some(residual_risk_hash.clone())
            }
        })
        .collect::<Vec<_>>();
    for residual in &residual_risk_hashes {
        require_hash(residual)?;
    }
    residual_risk_hashes.sort();
    residual_risk_hashes.dedup();
    let adjudication_hash = hash_domain(
        "campaign_adjudication.v1",
        &(
            1_u32,
            &obligations.denominator.plan_objective_member_hash,
            contract.contract_hash(),
            outcome,
            &predicate_outcomes,
            &claim_component_outcomes,
            &residual_risk_hashes,
        ),
    )?;
    Ok(CampaignAdjudicationV1 {
        adjudication_version: 1,
        plan_objective_member_hash: obligations.denominator.plan_objective_member_hash.clone(),
        verification_contract_hash: contract.contract_hash().to_owned(),
        outcome,
        predicate_outcomes,
        claim_component_outcomes,
        residual_risk_hashes,
        adjudication_hash,
    })
}

fn validate_action_assessment_semantics(
    contract: &VerificationContractV1,
    obligations: &ObligationDispositionSetV1,
    assessment: &ActionOracleAssessmentV1,
) -> Result<(), VerificationContractError> {
    let expected_predicates = contract
        .predicate_components()
        .iter()
        .map(|component| component.member_hash())
        .collect::<BTreeSet<_>>();
    let actual_predicates = assessment
        .predicate_outcomes
        .iter()
        .map(|outcome| outcome.predicate_component_member_hash.as_str())
        .collect::<BTreeSet<_>>();
    if actual_predicates.is_empty()
        || actual_predicates.len() != assessment.predicate_outcomes.len()
        || !actual_predicates.is_subset(&expected_predicates)
    {
        return Err(VerificationContractError::InvalidReference(
            "action oracle predicate census",
        ));
    }
    for outcome in &assessment.predicate_outcomes {
        let denominator_claims = obligations
            .denominator
            .members
            .iter()
            .find(|member| member.contract_member_hash == outcome.predicate_component_member_hash)
            .map(|member| {
                member
                    .claim_component_member_hashes
                    .iter()
                    .map(String::as_str)
                    .collect::<BTreeSet<_>>()
            })
            .ok_or(VerificationContractError::InvalidReference(
                "action oracle predicate denominator binding",
            ))?;
        let actual_claims = outcome
            .claim_component_member_hashes
            .iter()
            .map(String::as_str)
            .collect::<BTreeSet<_>>();
        if denominator_claims != actual_claims
            || actual_claims.len() != outcome.claim_component_member_hashes.len()
        {
            return Err(VerificationContractError::InvalidReference(
                "action oracle claim component census",
            ));
        }
    }
    let expected_controls = contract
        .required_controls()
        .iter()
        .map(|control| control.member_hash())
        .collect::<BTreeSet<_>>();
    let actual_controls = assessment
        .required_control_member_hashes
        .iter()
        .map(String::as_str)
        .collect::<BTreeSet<_>>();
    if expected_controls != actual_controls
        || actual_controls.len() != assessment.required_control_member_hashes.len()
        || !matches!(
            (expected_controls.is_empty(), assessment.control),
            (true, ControlValidityV1::NotRequired)
                | (false, ControlValidityV1::Valid | ControlValidityV1::Invalid)
        )
    {
        return Err(VerificationContractError::InvalidReference(
            "action oracle control census",
        ));
    }
    let limitations = assessment
        .limitation_codes
        .iter()
        .copied()
        .collect::<BTreeSet<_>>();
    if limitations.len() != assessment.limitation_codes.len()
        || (assessment.control == ControlValidityV1::Invalid
            && !limitations.contains(&OracleLimitationCodeV1::ControlInvalid))
        || (!assessment.concurrent_group_valid
            && !limitations.contains(&OracleLimitationCodeV1::RaceGroupIncomplete))
    {
        return Err(VerificationContractError::InvalidReference(
            "action oracle limitation census",
        ));
    }
    let expected_outcome = if limitations.contains(&OracleLimitationCodeV1::CleanupIncomplete) {
        Some(ComponentOracleOutcomeV1::Blocked)
    } else if !limitations.is_empty() {
        Some(ComponentOracleOutcomeV1::Inconclusive)
    } else {
        None
    };
    if expected_outcome.is_some_and(|expected| {
        assessment
            .predicate_outcomes
            .iter()
            .any(|outcome| outcome.outcome != expected)
    }) {
        return Err(VerificationContractError::InvalidReference(
            "action oracle limitation truth",
        ));
    }
    Ok(())
}

fn validate_denominator_against_contract(
    contract: &VerificationContractV1,
    obligations: &ObligationDispositionSetV1,
) -> Result<(), VerificationContractError> {
    let denominator = &obligations.denominator;
    let expected_contract_members = contract
        .predicate_components()
        .iter()
        .map(|component| (CoverageObligationKindV1::Predicate, component.member_hash()))
        .chain(contract.required_controls().iter().map(|control| {
            (
                CoverageObligationKindV1::RequiredControl,
                control.member_hash(),
            )
        }))
        .collect::<Vec<_>>();
    if denominator.seal_version != 1
        || denominator.members.len() != expected_contract_members.len()
        || denominator
            .members
            .iter()
            .zip(&expected_contract_members)
            .enumerate()
            .any(|(ordinal, (actual, expected))| {
                actual.ordinal != ordinal as u32
                    || actual.kind != expected.0
                    || actual.contract_member_hash != expected.1
                    || (actual.kind == CoverageObligationKindV1::RequiredControl
                        && !actual.claim_component_member_hashes.is_empty())
            })
    {
        return Err(VerificationContractError::InvalidReference(
            "Campaign denominator contract census",
        ));
    }
    for member in &denominator.members {
        let expected_member_hash = hash_domain(
            "campaign_coverage_denominator_member.v1",
            &(
                member.ordinal,
                member.kind,
                &member.contract_member_hash,
                &member.claim_component_member_hashes,
            ),
        )?;
        if expected_member_hash != member.member_hash {
            return Err(VerificationContractError::PersistedMismatch(
                "Campaign denominator member hash",
            ));
        }
    }
    let mut member_hashes = denominator
        .members
        .iter()
        .map(|member| member.member_hash.clone())
        .collect::<Vec<_>>();
    canonicalize_hashes(&mut member_hashes)?;
    let expected_member_set_hash =
        hash_domain("campaign_coverage_denominator_members.v1", &member_hashes)?;
    if expected_member_set_hash != denominator.member_set_hash {
        return Err(VerificationContractError::PersistedMismatch(
            "Campaign denominator member set hash",
        ));
    }
    let expected_seal_hash = hash_domain(
        "campaign_coverage_denominator.v1",
        &(
            denominator.seal_version,
            denominator.objective_id,
            &denominator.plan_objective_member_hash,
            &denominator.verification_contract_hash,
            &denominator.claim_component_set_hash,
            &denominator.member_set_hash,
        ),
    )?;
    if expected_seal_hash != denominator.seal_hash {
        return Err(VerificationContractError::PersistedMismatch(
            "Campaign denominator seal hash",
        ));
    }
    let actual_claims = denominator
        .members
        .iter()
        .flat_map(|member| member.claim_component_member_hashes.iter().cloned())
        .collect::<BTreeSet<_>>();
    let expected_claims = denominator
        .claim_component_member_hashes
        .iter()
        .cloned()
        .collect::<BTreeSet<_>>();
    if actual_claims != expected_claims
        || expected_claims.len() != denominator.claim_component_member_hashes.len()
    {
        return Err(VerificationContractError::InvalidReference(
            "Campaign denominator claim component exact set",
        ));
    }
    Ok(())
}

fn reduce_component_observations(
    observed: &[&PredicateOracleAssessmentV1],
) -> ComponentOracleOutcomeV1 {
    let values = observed
        .iter()
        .map(|outcome| outcome.outcome)
        .collect::<BTreeSet<_>>();
    if values.len() == 1 {
        values
            .iter()
            .next()
            .copied()
            .unwrap_or(ComponentOracleOutcomeV1::Inconclusive)
    } else {
        ComponentOracleOutcomeV1::Inconclusive
    }
}

fn reduce_all_of(outcomes: &[PredicateOracleAssessmentV1]) -> ComponentOracleOutcomeV1 {
    if outcomes
        .iter()
        .all(|outcome| outcome.outcome == ComponentOracleOutcomeV1::Proof)
    {
        ComponentOracleOutcomeV1::Proof
    } else if outcomes
        .iter()
        .any(|outcome| outcome.outcome == ComponentOracleOutcomeV1::Refutation)
    {
        ComponentOracleOutcomeV1::Refutation
    } else if outcomes
        .iter()
        .any(|outcome| outcome.outcome == ComponentOracleOutcomeV1::Blocked)
    {
        ComponentOracleOutcomeV1::Blocked
    } else {
        ComponentOracleOutcomeV1::Inconclusive
    }
}

fn reduce_any_of(outcomes: &[PredicateOracleAssessmentV1]) -> ComponentOracleOutcomeV1 {
    if outcomes
        .iter()
        .any(|outcome| outcome.outcome == ComponentOracleOutcomeV1::Proof)
    {
        ComponentOracleOutcomeV1::Proof
    } else if outcomes
        .iter()
        .all(|outcome| outcome.outcome == ComponentOracleOutcomeV1::Refutation)
    {
        ComponentOracleOutcomeV1::Refutation
    } else if outcomes
        .iter()
        .any(|outcome| outcome.outcome == ComponentOracleOutcomeV1::Blocked)
    {
        ComponentOracleOutcomeV1::Blocked
    } else {
        ComponentOracleOutcomeV1::Inconclusive
    }
}

fn reduce_paired_differential(
    contract: &VerificationContractV1,
    census: &OracleCensusV1,
) -> ComponentOracleOutcomeV1 {
    let bindings = contract.paired_differential_bindings();
    if bindings.len() != 1 {
        return ComponentOracleOutcomeV1::Inconclusive;
    }
    let binding = &bindings[0];
    let by_key = contract
        .predicate_components()
        .iter()
        .map(|component| (component.semantic_key(), component.member_hash()))
        .collect::<BTreeMap<_, _>>();
    let Some(baseline_hash) = by_key.get(binding.baseline_component_key()) else {
        return ComponentOracleOutcomeV1::Inconclusive;
    };
    let Some(variant_hash) = by_key.get(binding.variant_component_key()) else {
        return ComponentOracleOutcomeV1::Inconclusive;
    };
    let relations = census
        .assessments
        .iter()
        .filter_map(|assessment| assessment.paired_relation.as_ref())
        .collect::<Vec<_>>();
    if relations.len() != 1 {
        return ComponentOracleOutcomeV1::Inconclusive;
    }
    let relation = relations[0];
    if relation.pair_key != binding.pair_key()
        || relation.baseline_predicate_component_member_hash.as_str() != *baseline_hash
        || relation.variant_predicate_component_member_hash.as_str() != *variant_hash
        || relation.required_control_member_hash != binding.required_control_member_hash()
        || relation.comparator_rule_digest != binding.comparator_rule_digest()
        || require_hash(&relation.baseline_pair_identity_hash).is_err()
        || require_hash(&relation.variant_pair_identity_hash).is_err()
        || relation.baseline_pair_identity_hash != relation.variant_pair_identity_hash
        || census
            .assessments
            .iter()
            .any(|assessment| assessment.control != ControlValidityV1::Valid)
    {
        return ComponentOracleOutcomeV1::Inconclusive;
    }
    match relation.outcome {
        PairedRelationOutcomeV1::Satisfied => ComponentOracleOutcomeV1::Proof,
        PairedRelationOutcomeV1::Refuted => ComponentOracleOutcomeV1::Refutation,
        PairedRelationOutcomeV1::Inconclusive => ComponentOracleOutcomeV1::Inconclusive,
    }
}

fn reduce_ordered_sequence(
    contract: &VerificationContractV1,
    census: &OracleCensusV1,
) -> ComponentOracleOutcomeV1 {
    let mut observations = census
        .assessments
        .iter()
        .flat_map(|assessment| assessment.ordered_sequence.iter().cloned())
        .collect::<Vec<OrderedSequenceObservationV1>>();
    observations.sort_by_key(|observation| observation.step_ordinal);
    let steps = contract.ordered_steps();
    if observations.len() != steps.len()
        || observations
            .windows(2)
            .any(|pair| pair[0].step_ordinal == pair[1].step_ordinal)
    {
        return ComponentOracleOutcomeV1::Inconclusive;
    }
    let by_key = contract
        .predicate_components()
        .iter()
        .map(|component| (component.semantic_key(), component.member_hash()))
        .collect::<BTreeMap<_, _>>();
    if observations.iter().zip(steps).any(|(observation, step)| {
        observation.step_ordinal != step.step_ordinal()
            || by_key.get(step.component_key())
                != Some(&observation.predicate_component_member_hash.as_str())
    }) {
        return ComponentOracleOutcomeV1::Inconclusive;
    }
    let same_session = observations
        .iter()
        .map(|observation| observation.execution_session_hash.as_str())
        .collect::<BTreeSet<_>>()
        .len()
        == 1;
    let same_causal_chain = observations
        .iter()
        .map(|observation| observation.causal_chain_hash.as_str())
        .collect::<BTreeSet<_>>()
        .len()
        == 1;
    let ordered = observations
        .windows(2)
        .all(|pair| pair[0].event_ordinal < pair[1].event_ordinal);
    if !same_session || !same_causal_chain || !ordered {
        return ComponentOracleOutcomeV1::Inconclusive;
    }
    if observations
        .iter()
        .all(|observation| observation.outcome == ComponentOracleOutcomeV1::Proof)
    {
        ComponentOracleOutcomeV1::Proof
    } else if observations.iter().any(|observation| {
        observation.outcome == ComponentOracleOutcomeV1::Refutation
            && observation.deterministic_negative
            && observation.observation_window_complete
    }) {
        ComponentOracleOutcomeV1::Refutation
    } else {
        ComponentOracleOutcomeV1::Inconclusive
    }
}

fn reduce_claim_components(
    expected_claims: &[String],
    predicates: &[PredicateOracleAssessmentV1],
) -> Result<Vec<ClaimComponentCampaignOutcomeV1>, VerificationContractError> {
    let mut claims = Vec::with_capacity(expected_claims.len());
    for claim_hash in expected_claims {
        let mut bound = predicates
            .iter()
            .filter(|predicate| predicate.claim_component_member_hashes.contains(claim_hash))
            .collect::<Vec<_>>();
        bound.sort_by(|left, right| {
            left.predicate_component_member_hash
                .cmp(&right.predicate_component_member_hash)
        });
        if bound.is_empty() {
            return Err(VerificationContractError::InvalidReference(
                "claim component outcome binding",
            ));
        }
        let outcome = if bound
            .iter()
            .all(|predicate| predicate.outcome == ComponentOracleOutcomeV1::Proof)
        {
            ComponentOracleOutcomeV1::Proof
        } else if bound
            .iter()
            .any(|predicate| predicate.outcome == ComponentOracleOutcomeV1::Refutation)
        {
            ComponentOracleOutcomeV1::Refutation
        } else if bound
            .iter()
            .any(|predicate| predicate.outcome == ComponentOracleOutcomeV1::Blocked)
        {
            ComponentOracleOutcomeV1::Blocked
        } else {
            ComponentOracleOutcomeV1::Inconclusive
        };
        claims.push(ClaimComponentCampaignOutcomeV1 {
            claim_component_member_hash: claim_hash.clone(),
            outcome,
            predicate_component_member_hashes: bound
                .into_iter()
                .map(|predicate| predicate.predicate_component_member_hash.clone())
                .collect(),
        });
    }
    claims.sort_by(|left, right| {
        left.claim_component_member_hash
            .cmp(&right.claim_component_member_hash)
    });
    Ok(claims)
}
