//! Pure Plan C -> Plan B consolidation boundary.
//!
//! Campaign terminality is objective-local. Only an exact set of eligible,
//! current objective outcome receipts may reach Plan B's proof-path reducer.

use std::collections::BTreeSet;

use golish_core::hypothesis_verification::{
    reduce_verification_plan_v1, HypothesisRevisionAggregateV1,
    HypothesisVerificationObjectiveOutcomeV1, HypothesisVerificationPlanV1, ObjectiveOutcomeViewV1,
};
use serde::{Deserialize, Serialize};
use sha2::{Digest, Sha256};
use uuid::Uuid;

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum FactDeltaConsumptionDispositionV1 {
    Applied,
    NoSemanticChange,
    QuarantinedInvalidAuthority,
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct FactDeltaBundleCandidateV1 {
    pub fact_delta_bundle_id: Uuid,
    pub fact_delta_bundle_hash: String,
    pub source_objective_outcome_hash: String,
    pub source_authority_valid: bool,
    pub semantic_material_hash: String,
    pub current_semantic_material_hash: String,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize)]
pub struct FactDeltaConsumptionDecisionV1 {
    pub fact_delta_bundle_id: Uuid,
    pub disposition: FactDeltaConsumptionDispositionV1,
    pub source_objective_outcome_hash: String,
    pub decision_hash: String,
}

pub fn decide_fact_delta_consumption(
    candidate: &FactDeltaBundleCandidateV1,
) -> Result<FactDeltaConsumptionDecisionV1, ConsolidationError> {
    if candidate.fact_delta_bundle_id.is_nil()
        || !valid_hash(&candidate.fact_delta_bundle_hash)
        || !valid_hash(&candidate.source_objective_outcome_hash)
        || !valid_hash(&candidate.semantic_material_hash)
        || !valid_hash(&candidate.current_semantic_material_hash)
    {
        return Err(ConsolidationError::IdentityInvalid);
    }
    let disposition = if !candidate.source_authority_valid {
        FactDeltaConsumptionDispositionV1::QuarantinedInvalidAuthority
    } else if candidate.semantic_material_hash == candidate.current_semantic_material_hash {
        FactDeltaConsumptionDispositionV1::NoSemanticChange
    } else {
        FactDeltaConsumptionDispositionV1::Applied
    };
    let decision_hash = hash_material(
        "verification_fact_delta_consumption.v1",
        &format!(
            "{}:{}:{}:{disposition:?}",
            candidate.fact_delta_bundle_id,
            candidate.fact_delta_bundle_hash,
            candidate.source_objective_outcome_hash,
        ),
    );
    Ok(FactDeltaConsumptionDecisionV1 {
        fact_delta_bundle_id: candidate.fact_delta_bundle_id,
        disposition,
        source_objective_outcome_hash: candidate.source_objective_outcome_hash.clone(),
        decision_hash,
    })
}

#[derive(Debug, Clone, Copy)]
pub struct CurrentObjectiveOutcome<'a> {
    pub outcome: &'a HypothesisVerificationObjectiveOutcomeV1,
    pub selected_current_head: bool,
    pub quarantined: bool,
    pub temporal_authority_fresh: bool,
}

pub fn adjudicate_revision_from_current_outcomes(
    plan: &HypothesisVerificationPlanV1,
    outcomes: &[CurrentObjectiveOutcome<'_>],
) -> Result<HypothesisRevisionAggregateV1, ConsolidationError> {
    if outcomes.iter().any(|outcome| {
        !outcome.selected_current_head || outcome.quarantined || !outcome.temporal_authority_fresh
    }) {
        return Err(ConsolidationError::OutcomeAuthorityIneligible);
    }
    let hashes = outcomes
        .iter()
        .map(|outcome| outcome.outcome.outcome_hash())
        .collect::<BTreeSet<_>>();
    if hashes.len() != outcomes.len() {
        return Err(ConsolidationError::ObjectiveOutcomeSetInvalid);
    }
    let views = outcomes
        .iter()
        .map(|outcome| ObjectiveOutcomeViewV1::from(outcome.outcome))
        .collect::<Vec<_>>();
    reduce_verification_plan_v1(plan, &views).map_err(|error| {
        ConsolidationError::PlanReducerRejected {
            detail: error.to_string(),
        }
    })
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct WaveCoveragePartitionV1 {
    pub wave_member_hashes: Vec<String>,
    pub campaign_partition_member_hashes: Vec<String>,
    pub explicit_unassigned_member_hashes: Vec<String>,
}

pub fn validate_wave_partition(
    partition: &WaveCoveragePartitionV1,
) -> Result<String, ConsolidationError> {
    let wave = exact_hash_set(&partition.wave_member_hashes)?;
    let campaign = exact_hash_set(&partition.campaign_partition_member_hashes)?;
    let unassigned = exact_hash_set(&partition.explicit_unassigned_member_hashes)?;
    if !campaign.is_disjoint(&unassigned)
        || campaign
            .union(&unassigned)
            .cloned()
            .collect::<BTreeSet<_>>()
            != wave
    {
        return Err(ConsolidationError::WavePartitionIncomplete);
    }
    Ok(hash_material(
        "verification_wave_partition.v1",
        &wave.into_iter().collect::<Vec<_>>().join("\0"),
    ))
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum GenerationTransitionV1 {
    OpenNextGeneration,
    FixedPoint,
    HoldForQuarantine,
}

pub fn generation_transition(
    consumptions: &[FactDeltaConsumptionDecisionV1],
) -> Result<GenerationTransitionV1, ConsolidationError> {
    if consumptions.is_empty() {
        return Ok(GenerationTransitionV1::FixedPoint);
    }
    let ids = consumptions
        .iter()
        .map(|consumption| consumption.fact_delta_bundle_id)
        .collect::<BTreeSet<_>>();
    if ids.len() != consumptions.len()
        || consumptions
            .iter()
            .any(|consumption| !valid_hash(&consumption.decision_hash))
    {
        return Err(ConsolidationError::FactDeltaConsumptionSetInvalid);
    }
    if consumptions.iter().any(|consumption| {
        consumption.disposition == FactDeltaConsumptionDispositionV1::QuarantinedInvalidAuthority
    }) {
        return Ok(GenerationTransitionV1::HoldForQuarantine);
    }
    if consumptions
        .iter()
        .any(|consumption| consumption.disposition == FactDeltaConsumptionDispositionV1::Applied)
    {
        Ok(GenerationTransitionV1::OpenNextGeneration)
    } else {
        Ok(GenerationTransitionV1::FixedPoint)
    }
}

#[derive(Debug, thiserror::Error, Clone, PartialEq, Eq)]
pub enum ConsolidationError {
    #[error("VERIFICATION_CONSOLIDATION_IDENTITY_INVALID")]
    IdentityInvalid,
    #[error("VERIFICATION_CONSOLIDATION_OUTCOME_AUTHORITY_INELIGIBLE")]
    OutcomeAuthorityIneligible,
    #[error("VERIFICATION_CONSOLIDATION_OBJECTIVE_OUTCOME_SET_INVALID")]
    ObjectiveOutcomeSetInvalid,
    #[error("VERIFICATION_CONSOLIDATION_PLAN_REDUCER_REJECTED: {detail}")]
    PlanReducerRejected { detail: String },
    #[error("VERIFICATION_CONSOLIDATION_WAVE_PARTITION_INCOMPLETE")]
    WavePartitionIncomplete,
    #[error("VERIFICATION_CONSOLIDATION_FACT_DELTA_SET_INVALID")]
    FactDeltaConsumptionSetInvalid,
}

impl ConsolidationError {
    pub const fn code(&self) -> &'static str {
        match self {
            Self::IdentityInvalid => "VERIFICATION_CONSOLIDATION_IDENTITY_INVALID",
            Self::OutcomeAuthorityIneligible => {
                "VERIFICATION_CONSOLIDATION_OUTCOME_AUTHORITY_INELIGIBLE"
            }
            Self::ObjectiveOutcomeSetInvalid => {
                "VERIFICATION_CONSOLIDATION_OBJECTIVE_OUTCOME_SET_INVALID"
            }
            Self::PlanReducerRejected { .. } => "VERIFICATION_CONSOLIDATION_PLAN_REDUCER_REJECTED",
            Self::WavePartitionIncomplete => "VERIFICATION_CONSOLIDATION_WAVE_PARTITION_INCOMPLETE",
            Self::FactDeltaConsumptionSetInvalid => {
                "VERIFICATION_CONSOLIDATION_FACT_DELTA_SET_INVALID"
            }
        }
    }
}

fn exact_hash_set(values: &[String]) -> Result<BTreeSet<String>, ConsolidationError> {
    if values.iter().any(|value| !valid_hash(value)) {
        return Err(ConsolidationError::IdentityInvalid);
    }
    let set = values.iter().cloned().collect::<BTreeSet<_>>();
    if set.len() != values.len() {
        return Err(ConsolidationError::WavePartitionIncomplete);
    }
    Ok(set)
}

fn valid_hash(value: &str) -> bool {
    value.strip_prefix("sha256:").is_some_and(|digest| {
        digest.len() == 64 && digest.bytes().all(|byte| byte.is_ascii_hexdigit())
    })
}

fn hash_material(domain: &str, material: &str) -> String {
    let mut hasher = Sha256::new();
    hasher.update(domain.as_bytes());
    hasher.update([0]);
    hasher.update(material.as_bytes());
    format!(
        "sha256:{}",
        hasher
            .finalize()
            .iter()
            .map(|byte| format!("{byte:02x}"))
            .collect::<String>()
    )
}

#[cfg(test)]
mod tests {
    use super::*;

    fn hash(byte: char) -> String {
        format!("sha256:{}", byte.to_string().repeat(64))
    }

    fn candidate(material: char, current: char, authority: bool) -> FactDeltaBundleCandidateV1 {
        FactDeltaBundleCandidateV1 {
            fact_delta_bundle_id: Uuid::from_u128(material as u128),
            fact_delta_bundle_hash: hash('a'),
            source_objective_outcome_hash: hash('b'),
            source_authority_valid: authority,
            semantic_material_hash: hash(material),
            current_semantic_material_hash: hash(current),
        }
    }

    #[test]
    fn verification_fact_delta_consumption_is_host_derived_and_closed() {
        assert_eq!(
            decide_fact_delta_consumption(&candidate('c', 'd', true))
                .unwrap()
                .disposition,
            FactDeltaConsumptionDispositionV1::Applied
        );
        assert_eq!(
            decide_fact_delta_consumption(&candidate('c', 'c', true))
                .unwrap()
                .disposition,
            FactDeltaConsumptionDispositionV1::NoSemanticChange
        );
        assert_eq!(
            decide_fact_delta_consumption(&candidate('c', 'd', false))
                .unwrap()
                .disposition,
            FactDeltaConsumptionDispositionV1::QuarantinedInvalidAuthority
        );
    }

    #[test]
    fn verification_wave_partition_requires_campaign_or_explicit_unassigned_exact_set() {
        let partition = WaveCoveragePartitionV1 {
            wave_member_hashes: vec![hash('a'), hash('b')],
            campaign_partition_member_hashes: vec![hash('a')],
            explicit_unassigned_member_hashes: vec![hash('b')],
        };
        assert!(valid_hash(&validate_wave_partition(&partition).unwrap()));
        assert_eq!(
            validate_wave_partition(&WaveCoveragePartitionV1 {
                explicit_unassigned_member_hashes: Vec::new(),
                ..partition
            })
            .unwrap_err()
            .code(),
            "VERIFICATION_CONSOLIDATION_WAVE_PARTITION_INCOMPLETE"
        );
    }

    #[test]
    fn hypothesis_generation_transition_is_material_or_fixed_point() {
        let applied = decide_fact_delta_consumption(&candidate('c', 'd', true)).unwrap();
        let unchanged = decide_fact_delta_consumption(&candidate('e', 'e', true)).unwrap();
        assert_eq!(
            generation_transition(&[applied]).unwrap(),
            GenerationTransitionV1::OpenNextGeneration
        );
        assert_eq!(
            generation_transition(&[unchanged]).unwrap(),
            GenerationTransitionV1::FixedPoint
        );
        assert_eq!(
            generation_transition(&[]).unwrap(),
            GenerationTransitionV1::FixedPoint
        );
    }

    #[test]
    fn quarantined_consumption_holds_generation_instead_of_reusing_old_truth() {
        let quarantined = decide_fact_delta_consumption(&candidate('c', 'd', false)).unwrap();
        assert_eq!(
            generation_transition(&[quarantined]).unwrap(),
            GenerationTransitionV1::HoldForQuarantine
        );
    }
}
