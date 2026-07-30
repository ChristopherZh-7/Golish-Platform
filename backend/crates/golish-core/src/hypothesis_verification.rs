//! Host-sealed revision verification plans and aggregate adjudication authority.
//!
//! Campaign completion is evidence for one objective.  It is deliberately not
//! revision authority: the outer proof-path reducer below is the only V1 truth
//! table for `Verified` / `Refuted` / `NonTerminal`.

use crate::hypothesis_semantic_key::validate_sha256;
use crate::verification_contract::VerificationContractV1;
use chrono::{DateTime, Utc};
use serde::Serialize;
use sha2::{Digest, Sha256};
use std::collections::{BTreeMap, BTreeSet};
use uuid::Uuid;

#[derive(Debug, Clone, Copy, PartialEq, Eq, PartialOrd, Ord, Serialize)]
#[serde(rename_all = "snake_case")]
pub enum HypothesisClaimComponentKindV1 {
    ClaimClause,
    ImpactQualifier,
    TrustBoundaryCondition,
    IdentityCondition,
}

impl HypothesisClaimComponentKindV1 {
    pub const ALL: [Self; 4] = [
        Self::ClaimClause,
        Self::ImpactQualifier,
        Self::TrustBoundaryCondition,
        Self::IdentityCondition,
    ];

    pub const fn as_str(self) -> &'static str {
        match self {
            Self::ClaimClause => "claim_clause",
            Self::ImpactQualifier => "impact_qualifier",
            Self::TrustBoundaryCondition => "trust_boundary_condition",
            Self::IdentityCondition => "identity_condition",
        }
    }
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize)]
pub struct HypothesisClaimComponentV1 {
    revision_id: Uuid,
    revision_hash: String,
    component_ordinal: u32,
    component_key: String,
    kind: HypothesisClaimComponentKindV1,
    canonical_fragment_hash: String,
    canonical_condition_hash: String,
    derivation_contract_version: u32,
    derivation_contract_digest: String,
    required: bool,
    member_hash: String,
}

#[derive(Debug, Clone)]
pub struct HypothesisClaimComponentInputV1 {
    pub component_key: String,
    pub kind: HypothesisClaimComponentKindV1,
    pub canonical_fragment_hash: String,
    pub canonical_condition_hash: String,
    pub required: bool,
}

pub fn compile_claim_components_v1(
    revision_id: Uuid,
    revision_hash: String,
    derivation_contract_version: u32,
    derivation_contract_digest: String,
    inputs: Vec<HypothesisClaimComponentInputV1>,
) -> Result<Vec<HypothesisClaimComponentV1>, HypothesisVerificationError> {
    require_uuid("revision_id", revision_id)?;
    require_hash(&revision_hash)?;
    require_nonzero("derivation_contract_version", derivation_contract_version)?;
    require_hash(&derivation_contract_digest)?;
    if inputs.is_empty() {
        return Err(HypothesisVerificationError::ClaimComponentsEmpty);
    }
    let mut inputs = inputs;
    inputs.sort_by(|left, right| left.component_key.cmp(&right.component_key));
    if inputs
        .windows(2)
        .any(|pair| pair[0].component_key == pair[1].component_key)
    {
        return Err(HypothesisVerificationError::ClaimComponentDuplicate);
    }
    inputs
        .into_iter()
        .enumerate()
        .map(|(ordinal, input)| {
            require_nonblank("component_key", &input.component_key)?;
            require_hash(&input.canonical_fragment_hash)?;
            require_hash(&input.canonical_condition_hash)?;
            let ingredients = (
                revision_id,
                &revision_hash,
                ordinal as u32,
                &input.component_key,
                input.kind,
                &input.canonical_fragment_hash,
                &input.canonical_condition_hash,
                derivation_contract_version,
                &derivation_contract_digest,
                input.required,
            );
            let member_hash = hash_value("hypothesis_claim_component_member.v1", &ingredients)?;
            Ok(HypothesisClaimComponentV1 {
                revision_id,
                revision_hash: revision_hash.clone(),
                component_ordinal: ordinal as u32,
                component_key: input.component_key,
                kind: input.kind,
                canonical_fragment_hash: input.canonical_fragment_hash,
                canonical_condition_hash: input.canonical_condition_hash,
                derivation_contract_version,
                derivation_contract_digest: derivation_contract_digest.clone(),
                required: input.required,
                member_hash,
            })
        })
        .collect()
}

impl HypothesisClaimComponentV1 {
    pub const fn revision_id(&self) -> Uuid {
        self.revision_id
    }
    pub fn revision_hash(&self) -> &str {
        &self.revision_hash
    }
    pub const fn component_ordinal(&self) -> u32 {
        self.component_ordinal
    }
    pub fn component_key(&self) -> &str {
        &self.component_key
    }
    pub const fn kind(&self) -> HypothesisClaimComponentKindV1 {
        self.kind
    }
    pub fn canonical_fragment_hash(&self) -> &str {
        &self.canonical_fragment_hash
    }
    pub fn canonical_condition_hash(&self) -> &str {
        &self.canonical_condition_hash
    }
    pub const fn derivation_contract_version(&self) -> u32 {
        self.derivation_contract_version
    }
    pub fn derivation_contract_digest(&self) -> &str {
        &self.derivation_contract_digest
    }
    pub const fn required(&self) -> bool {
        self.required
    }
    pub fn member_hash(&self) -> &str {
        &self.member_hash
    }
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize)]
#[serde(rename_all = "snake_case")]
pub enum HypothesisVerificationObjectiveOutcomeRequirementV1 {
    SatisfyBoundComponents,
    SatisfyOrFalsifyBoundRequiredComponents,
}

impl HypothesisVerificationObjectiveOutcomeRequirementV1 {
    pub const fn as_str(self) -> &'static str {
        match self {
            Self::SatisfyBoundComponents => "satisfy_bound_components",
            Self::SatisfyOrFalsifyBoundRequiredComponents => {
                "satisfy_or_falsify_bound_required_components"
            }
        }
    }
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize)]
pub struct HypothesisVerificationPlanObjectiveV1 {
    objective_id: Uuid,
    objective_hash: String,
    verification_contract_id: Uuid,
    verification_contract_version: u32,
    verification_contract_hash: String,
    claim_component_member_hashes: Vec<String>,
    claim_component_count: u32,
    claim_component_set_hash: String,
    stopping_criteria_hash: String,
    outcome_requirement: HypothesisVerificationObjectiveOutcomeRequirementV1,
    member_hash: String,
}

#[derive(Debug, Clone)]
pub struct HypothesisVerificationPlanObjectiveInputV1 {
    pub objective_hash: String,
    pub verification_contract: VerificationContractV1,
    pub claim_component_member_hashes: Vec<String>,
    pub outcome_requirement: HypothesisVerificationObjectiveOutcomeRequirementV1,
}

impl HypothesisVerificationPlanObjectiveV1 {
    pub const fn objective_id(&self) -> Uuid {
        self.objective_id
    }
    pub fn objective_hash(&self) -> &str {
        &self.objective_hash
    }
    pub const fn verification_contract_id(&self) -> Uuid {
        self.verification_contract_id
    }
    pub const fn verification_contract_version(&self) -> u32 {
        self.verification_contract_version
    }
    pub fn verification_contract_hash(&self) -> &str {
        &self.verification_contract_hash
    }
    pub fn claim_component_member_hashes(&self) -> &[String] {
        &self.claim_component_member_hashes
    }
    pub fn claim_component_set_hash(&self) -> &str {
        &self.claim_component_set_hash
    }
    pub const fn claim_component_count(&self) -> u32 {
        self.claim_component_count
    }
    pub fn stopping_criteria_hash(&self) -> &str {
        &self.stopping_criteria_hash
    }
    pub const fn outcome_requirement(&self) -> HypothesisVerificationObjectiveOutcomeRequirementV1 {
        self.outcome_requirement
    }
    pub fn member_hash(&self) -> &str {
        &self.member_hash
    }
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize)]
#[serde(rename_all = "snake_case")]
pub enum HypothesisVerificationPlanPathMemberRoleV1 {
    RequiredProof,
    RequiredProofAndPathFalsifier,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize)]
pub struct HypothesisVerificationPlanPathMemberV1 {
    member_ordinal: u32,
    plan_objective_member_hash: String,
    verification_contract_hash: String,
    claim_component_set_hash: String,
    role: HypothesisVerificationPlanPathMemberRoleV1,
    falsifier_claim_component_member_hashes: Vec<String>,
    falsifier_claim_component_set_hash: String,
    member_hash: String,
}

#[derive(Debug, Clone)]
pub struct HypothesisVerificationPlanPathMemberInputV1 {
    pub objective_id: Uuid,
    pub role: HypothesisVerificationPlanPathMemberRoleV1,
    pub falsifier_claim_component_member_hashes: Vec<String>,
}

impl HypothesisVerificationPlanPathMemberV1 {
    pub const fn member_ordinal(&self) -> u32 {
        self.member_ordinal
    }
    pub fn plan_objective_member_hash(&self) -> &str {
        &self.plan_objective_member_hash
    }
    pub fn verification_contract_hash(&self) -> &str {
        &self.verification_contract_hash
    }
    pub fn claim_component_set_hash(&self) -> &str {
        &self.claim_component_set_hash
    }
    pub const fn role(&self) -> HypothesisVerificationPlanPathMemberRoleV1 {
        self.role
    }
    pub fn falsifier_claim_component_member_hashes(&self) -> &[String] {
        &self.falsifier_claim_component_member_hashes
    }
    pub fn falsifier_claim_component_set_hash(&self) -> &str {
        &self.falsifier_claim_component_set_hash
    }
    pub fn member_hash(&self) -> &str {
        &self.member_hash
    }
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize)]
pub struct HypothesisVerificationPlanPathV1 {
    path_ordinal: u32,
    path_key: String,
    members: Vec<HypothesisVerificationPlanPathMemberV1>,
    member_count: u32,
    member_set_hash: String,
    path_hash: String,
}

#[derive(Debug, Clone)]
pub struct HypothesisVerificationPlanPathInputV1 {
    pub path_key: String,
    pub members: Vec<HypothesisVerificationPlanPathMemberInputV1>,
}

impl HypothesisVerificationPlanPathV1 {
    pub const fn path_ordinal(&self) -> u32 {
        self.path_ordinal
    }
    pub fn path_key(&self) -> &str {
        &self.path_key
    }
    pub fn members(&self) -> &[HypothesisVerificationPlanPathMemberV1] {
        &self.members
    }
    pub const fn member_count(&self) -> u32 {
        self.member_count
    }
    pub fn member_set_hash(&self) -> &str {
        &self.member_set_hash
    }
    pub fn path_hash(&self) -> &str {
        &self.path_hash
    }
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize)]
pub struct HypothesisVerificationPlanV1 {
    plan_id: Uuid,
    plan_version: u32,
    revision_id: Uuid,
    revision_hash: String,
    revision_ingredients_hash: String,
    required_claim_components: Vec<HypothesisClaimComponentV1>,
    required_claim_component_count: u32,
    required_claim_component_set_hash: String,
    objectives: Vec<HypothesisVerificationPlanObjectiveV1>,
    objective_count: u32,
    objective_set_hash: String,
    proof_paths: Vec<HypothesisVerificationPlanPathV1>,
    proof_path_count: u32,
    proof_path_set_hash: String,
    outer_aggregation_policy_version: u32,
    outer_aggregation_policy_digest: String,
    plan_hash: String,
}

#[derive(Debug, Clone)]
pub struct HypothesisVerificationPlanBuildInputV1 {
    pub revision_id: Uuid,
    pub revision_hash: String,
    pub revision_ingredients_hash: String,
    pub required_claim_components: Vec<HypothesisClaimComponentV1>,
    pub objectives: Vec<HypothesisVerificationPlanObjectiveInputV1>,
    pub proof_paths: Vec<HypothesisVerificationPlanPathInputV1>,
    pub outer_aggregation_policy_version: u32,
    pub outer_aggregation_policy_digest: String,
}

impl HypothesisVerificationPlanV1 {
    pub fn compile(
        mut input: HypothesisVerificationPlanBuildInputV1,
    ) -> Result<Self, HypothesisVerificationError> {
        require_uuid("revision_id", input.revision_id)?;
        require_hash(&input.revision_hash)?;
        require_hash(&input.revision_ingredients_hash)?;
        require_nonzero(
            "outer_aggregation_policy_version",
            input.outer_aggregation_policy_version,
        )?;
        require_hash(&input.outer_aggregation_policy_digest)?;

        input
            .required_claim_components
            .retain(HypothesisClaimComponentV1::required);
        input
            .required_claim_components
            .sort_by(|left, right| left.member_hash.cmp(&right.member_hash));
        if input.required_claim_components.is_empty() {
            return Err(HypothesisVerificationError::RequiredClaimComponentsEmpty);
        }
        for component in &input.required_claim_components {
            if component.revision_id != input.revision_id
                || component.revision_hash != input.revision_hash
            {
                return Err(HypothesisVerificationError::ClaimComponentRevisionMismatch);
            }
        }
        ensure_unique_hashes(
            input
                .required_claim_components
                .iter()
                .map(|component| component.member_hash.as_str()),
            HypothesisVerificationError::ClaimComponentDuplicate,
        )?;
        let required_set = input
            .required_claim_components
            .iter()
            .map(|component| component.member_hash.clone())
            .collect::<BTreeSet<_>>();
        let required_claim_component_set_hash = exact_set_hash(
            "hypothesis_verification_plan_required_components.v1",
            required_set.iter().map(String::as_str),
        )?;

        input
            .objectives
            .sort_by_key(|objective| objective.verification_contract.objective_id());
        if input.objectives.is_empty() {
            return Err(HypothesisVerificationError::ObjectivesEmpty);
        }
        if input.objectives.windows(2).any(|pair| {
            pair[0].verification_contract.objective_id()
                == pair[1].verification_contract.objective_id()
        }) {
            return Err(HypothesisVerificationError::ObjectiveDuplicate);
        }
        let mut objectives = Vec::with_capacity(input.objectives.len());
        for mut objective in input.objectives {
            let objective_id = objective.verification_contract.objective_id();
            require_uuid("objective_id", objective_id)?;
            require_hash(&objective.objective_hash)?;
            if objective.verification_contract.revision_id() != input.revision_id
                || objective.verification_contract.revision_hash() != input.revision_hash
            {
                return Err(HypothesisVerificationError::VerificationContractBindingMismatch);
            }
            objective.claim_component_member_hashes.sort();
            ensure_unique_hashes(
                objective
                    .claim_component_member_hashes
                    .iter()
                    .map(String::as_str),
                HypothesisVerificationError::ClaimComponentDuplicate,
            )?;
            if objective.claim_component_member_hashes.is_empty()
                || !objective
                    .claim_component_member_hashes
                    .iter()
                    .all(|hash| required_set.contains(hash))
            {
                return Err(HypothesisVerificationError::ObjectiveComponentSubsetInvalid);
            }
            let component_set_hash = exact_set_hash(
                "hypothesis_verification_plan_objective_components.v1",
                objective
                    .claim_component_member_hashes
                    .iter()
                    .map(String::as_str),
            )?;
            let member_ingredients = (
                objective_id,
                &objective.objective_hash,
                objective.verification_contract.contract_id(),
                objective.verification_contract.contract_version(),
                objective.verification_contract.contract_hash(),
                &component_set_hash,
                objective.verification_contract.stopping_criteria_hash(),
                objective.outcome_requirement,
            );
            let member_hash = hash_value(
                "hypothesis_verification_plan_objective_member.v1",
                &member_ingredients,
            )?;
            objectives.push(HypothesisVerificationPlanObjectiveV1 {
                objective_id,
                objective_hash: objective.objective_hash,
                verification_contract_id: objective.verification_contract.contract_id(),
                verification_contract_version: objective.verification_contract.contract_version(),
                verification_contract_hash: objective
                    .verification_contract
                    .contract_hash()
                    .to_owned(),
                claim_component_count: objective.claim_component_member_hashes.len() as u32,
                claim_component_member_hashes: objective.claim_component_member_hashes,
                claim_component_set_hash: component_set_hash,
                stopping_criteria_hash: objective
                    .verification_contract
                    .stopping_criteria_hash()
                    .to_owned(),
                outcome_requirement: objective.outcome_requirement,
                member_hash,
            });
        }
        let objective_by_id = objectives
            .iter()
            .map(|objective| (objective.objective_id, objective))
            .collect::<BTreeMap<_, _>>();
        let objective_set_hash = exact_set_hash(
            "hypothesis_verification_plan_objectives.v1",
            objectives
                .iter()
                .map(|objective| objective.member_hash.as_str()),
        )?;

        input
            .proof_paths
            .sort_by(|left, right| left.path_key.cmp(&right.path_key));
        if input.proof_paths.is_empty() {
            return Err(HypothesisVerificationError::ProofPathsEmpty);
        }
        if input
            .proof_paths
            .windows(2)
            .any(|pair| pair[0].path_key == pair[1].path_key)
        {
            return Err(HypothesisVerificationError::ProofPathDuplicate);
        }
        let mut referenced_objectives = BTreeSet::new();
        let mut proof_paths = Vec::with_capacity(input.proof_paths.len());
        for (path_ordinal, mut path) in input.proof_paths.into_iter().enumerate() {
            require_nonblank("path_key", &path.path_key)?;
            path.members.sort_by_key(|member| member.objective_id);
            if path.members.is_empty()
                || path
                    .members
                    .windows(2)
                    .any(|pair| pair[0].objective_id == pair[1].objective_id)
            {
                return Err(HypothesisVerificationError::ProofPathMembersInvalid);
            }
            let mut path_component_union = BTreeSet::new();
            let mut has_required_falsifier = false;
            let mut members = Vec::with_capacity(path.members.len());
            for (member_ordinal, mut member) in path.members.into_iter().enumerate() {
                let objective = objective_by_id
                    .get(&member.objective_id)
                    .ok_or(HypothesisVerificationError::PathObjectiveUnknown)?;
                referenced_objectives.insert(member.objective_id);
                path_component_union
                    .extend(objective.claim_component_member_hashes.iter().cloned());
                member.falsifier_claim_component_member_hashes.sort();
                ensure_unique_hashes(
                    member
                        .falsifier_claim_component_member_hashes
                        .iter()
                        .map(String::as_str),
                    HypothesisVerificationError::PathFalsifierInvalid,
                )?;
                let falsifier_set = member
                    .falsifier_claim_component_member_hashes
                    .iter()
                    .cloned()
                    .collect::<BTreeSet<_>>();
                match member.role {
                    HypothesisVerificationPlanPathMemberRoleV1::RequiredProof
                        if !falsifier_set.is_empty() =>
                    {
                        return Err(HypothesisVerificationError::PathFalsifierInvalid)
                    }
                    HypothesisVerificationPlanPathMemberRoleV1::RequiredProofAndPathFalsifier
                        if falsifier_set.is_empty()
                            || !falsifier_set.iter().all(|hash| {
                                required_set.contains(hash)
                                    && objective.claim_component_member_hashes.contains(hash)
                            }) =>
                    {
                        return Err(HypothesisVerificationError::PathFalsifierInvalid)
                    }
                    HypothesisVerificationPlanPathMemberRoleV1::RequiredProofAndPathFalsifier => {
                        has_required_falsifier = true;
                    }
                    _ => {}
                }
                let falsifier_set_hash = exact_set_hash(
                    "hypothesis_verification_plan_path_falsifiers.v1",
                    falsifier_set.iter().map(String::as_str),
                )?;
                let ingredients = (
                    member_ordinal as u32,
                    &objective.member_hash,
                    &objective.verification_contract_hash,
                    &objective.claim_component_set_hash,
                    member.role,
                    &falsifier_set_hash,
                );
                let member_hash =
                    hash_value("hypothesis_verification_plan_path_member.v1", &ingredients)?;
                members.push(HypothesisVerificationPlanPathMemberV1 {
                    member_ordinal: member_ordinal as u32,
                    plan_objective_member_hash: objective.member_hash.clone(),
                    verification_contract_hash: objective.verification_contract_hash.clone(),
                    claim_component_set_hash: objective.claim_component_set_hash.clone(),
                    role: member.role,
                    falsifier_claim_component_member_hashes: member
                        .falsifier_claim_component_member_hashes,
                    falsifier_claim_component_set_hash: falsifier_set_hash,
                    member_hash,
                });
            }
            if path_component_union != required_set {
                return Err(HypothesisVerificationError::ClaimComponentUncovered);
            }
            if !has_required_falsifier {
                return Err(HypothesisVerificationError::PathFalsifierMissing);
            }
            let member_set_hash = exact_set_hash(
                "hypothesis_verification_plan_path_members.v1",
                members.iter().map(|member| member.member_hash.as_str()),
            )?;
            let path_hash = hash_value(
                "hypothesis_verification_plan_path.v1",
                &(path_ordinal as u32, &path.path_key, &member_set_hash),
            )?;
            proof_paths.push(HypothesisVerificationPlanPathV1 {
                path_ordinal: path_ordinal as u32,
                path_key: path.path_key,
                member_count: members.len() as u32,
                members,
                member_set_hash,
                path_hash,
            });
        }
        if referenced_objectives.len() != objectives.len() {
            return Err(HypothesisVerificationError::ObjectiveNotInProofPath);
        }
        let proof_path_set_hash = exact_set_hash(
            "hypothesis_verification_plan_paths.v1",
            proof_paths.iter().map(|path| path.path_hash.as_str()),
        )?;
        let plan_hash = hash_value(
            "hypothesis_verification_plan.v1",
            &(
                1_u32,
                input.revision_id,
                &input.revision_hash,
                &input.revision_ingredients_hash,
                &required_claim_component_set_hash,
                &objective_set_hash,
                &proof_path_set_hash,
                input.outer_aggregation_policy_version,
                &input.outer_aggregation_policy_digest,
            ),
        )?;
        let plan_id = Uuid::new_v5(
            &input.revision_id,
            format!("hypothesis_verification_plan.v1:{plan_hash}").as_bytes(),
        );
        Ok(Self {
            plan_id,
            plan_version: 1,
            revision_id: input.revision_id,
            revision_hash: input.revision_hash,
            revision_ingredients_hash: input.revision_ingredients_hash,
            required_claim_component_count: input.required_claim_components.len() as u32,
            required_claim_components: input.required_claim_components,
            required_claim_component_set_hash,
            objective_count: objectives.len() as u32,
            objectives,
            objective_set_hash,
            proof_path_count: proof_paths.len() as u32,
            proof_paths,
            proof_path_set_hash,
            outer_aggregation_policy_version: input.outer_aggregation_policy_version,
            outer_aggregation_policy_digest: input.outer_aggregation_policy_digest,
            plan_hash,
        })
    }

    pub const fn plan_id(&self) -> Uuid {
        self.plan_id
    }
    pub const fn plan_version(&self) -> u32 {
        self.plan_version
    }
    pub const fn revision_id(&self) -> Uuid {
        self.revision_id
    }
    pub fn revision_hash(&self) -> &str {
        &self.revision_hash
    }
    pub fn revision_ingredients_hash(&self) -> &str {
        &self.revision_ingredients_hash
    }
    pub fn required_claim_components(&self) -> &[HypothesisClaimComponentV1] {
        &self.required_claim_components
    }
    pub const fn required_claim_component_count(&self) -> u32 {
        self.required_claim_component_count
    }
    pub fn required_claim_component_set_hash(&self) -> &str {
        &self.required_claim_component_set_hash
    }
    pub fn objectives(&self) -> &[HypothesisVerificationPlanObjectiveV1] {
        &self.objectives
    }
    pub const fn objective_count(&self) -> u32 {
        self.objective_count
    }
    pub fn objective_set_hash(&self) -> &str {
        &self.objective_set_hash
    }
    pub fn proof_paths(&self) -> &[HypothesisVerificationPlanPathV1] {
        &self.proof_paths
    }
    pub const fn proof_path_count(&self) -> u32 {
        self.proof_path_count
    }
    pub fn proof_path_set_hash(&self) -> &str {
        &self.proof_path_set_hash
    }
    pub const fn outer_aggregation_policy_version(&self) -> u32 {
        self.outer_aggregation_policy_version
    }
    pub fn outer_aggregation_policy_digest(&self) -> &str {
        &self.outer_aggregation_policy_digest
    }
    pub fn plan_hash(&self) -> &str {
        &self.plan_hash
    }
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize)]
#[serde(rename_all = "snake_case")]
pub enum HypothesisClaimComponentOutcomeKindV1 {
    Satisfied,
    Refuted,
    Inconclusive,
    Blocked,
    Unassigned,
    Invalidated,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize)]
#[serde(rename_all = "snake_case")]
pub enum HypothesisVerificationObjectiveOutcomeKindV1 {
    Satisfied,
    Refuted,
    Inconclusive,
    Blocked,
    ExhaustedWithResiduals,
    Unassigned,
    Invalidated,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize)]
pub struct HypothesisComponentProofRefV1 {
    claim_component_member_hash: String,
    predicate_component_member_hash: String,
    oracle_receipt_id: Uuid,
    oracle_receipt_hash: String,
    coverage_receipt_hash: String,
    fact_delta_consumption_set_hash: String,
    member_hash: String,
}

#[derive(Debug, Clone)]
pub struct HypothesisComponentProofRefInputV1 {
    pub claim_component_member_hash: String,
    pub predicate_component_member_hash: String,
    pub oracle_receipt_id: Uuid,
    pub oracle_receipt_hash: String,
    pub coverage_receipt_hash: String,
    pub fact_delta_consumption_set_hash: String,
}

impl HypothesisComponentProofRefV1 {
    pub fn from_server_receipt(
        input: HypothesisComponentProofRefInputV1,
    ) -> Result<Self, HypothesisVerificationError> {
        require_uuid("oracle_receipt_id", input.oracle_receipt_id)?;
        for hash in [
            &input.claim_component_member_hash,
            &input.predicate_component_member_hash,
            &input.oracle_receipt_hash,
            &input.coverage_receipt_hash,
            &input.fact_delta_consumption_set_hash,
        ] {
            require_hash(hash)?;
        }
        let member_hash = hash_value(
            "hypothesis_component_proof_ref.v1",
            &(
                &input.claim_component_member_hash,
                &input.predicate_component_member_hash,
                input.oracle_receipt_id,
                &input.oracle_receipt_hash,
                &input.coverage_receipt_hash,
                &input.fact_delta_consumption_set_hash,
            ),
        )?;
        Ok(Self {
            claim_component_member_hash: input.claim_component_member_hash,
            predicate_component_member_hash: input.predicate_component_member_hash,
            oracle_receipt_id: input.oracle_receipt_id,
            oracle_receipt_hash: input.oracle_receipt_hash,
            coverage_receipt_hash: input.coverage_receipt_hash,
            fact_delta_consumption_set_hash: input.fact_delta_consumption_set_hash,
            member_hash,
        })
    }
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize)]
pub struct HypothesisComponentRefutationRefV1 {
    claim_component_member_hash: String,
    predicate_component_member_hash: String,
    required_control_set_hash: String,
    oracle_receipt_id: Uuid,
    oracle_receipt_hash: String,
    coverage_receipt_hash: String,
    fact_delta_consumption_set_hash: String,
    member_hash: String,
}

#[derive(Debug, Clone)]
pub struct HypothesisComponentRefutationRefInputV1 {
    pub claim_component_member_hash: String,
    pub predicate_component_member_hash: String,
    pub required_control_set_hash: String,
    pub oracle_receipt_id: Uuid,
    pub oracle_receipt_hash: String,
    pub coverage_receipt_hash: String,
    pub fact_delta_consumption_set_hash: String,
}

impl HypothesisComponentRefutationRefV1 {
    pub fn from_server_receipt(
        input: HypothesisComponentRefutationRefInputV1,
    ) -> Result<Self, HypothesisVerificationError> {
        require_uuid("oracle_receipt_id", input.oracle_receipt_id)?;
        for hash in [
            &input.claim_component_member_hash,
            &input.predicate_component_member_hash,
            &input.required_control_set_hash,
            &input.oracle_receipt_hash,
            &input.coverage_receipt_hash,
            &input.fact_delta_consumption_set_hash,
        ] {
            require_hash(hash)?;
        }
        let member_hash = hash_value(
            "hypothesis_component_refutation_ref.v1",
            &(
                &input.claim_component_member_hash,
                &input.predicate_component_member_hash,
                &input.required_control_set_hash,
                input.oracle_receipt_id,
                &input.oracle_receipt_hash,
                &input.coverage_receipt_hash,
                &input.fact_delta_consumption_set_hash,
            ),
        )?;
        Ok(Self {
            claim_component_member_hash: input.claim_component_member_hash,
            predicate_component_member_hash: input.predicate_component_member_hash,
            required_control_set_hash: input.required_control_set_hash,
            oracle_receipt_id: input.oracle_receipt_id,
            oracle_receipt_hash: input.oracle_receipt_hash,
            coverage_receipt_hash: input.coverage_receipt_hash,
            fact_delta_consumption_set_hash: input.fact_delta_consumption_set_hash,
            member_hash,
        })
    }
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize)]
pub enum HypothesisClaimComponentOutcomeLineageV1 {
    Satisfied {
        proof_members: Vec<HypothesisComponentProofRefV1>,
        proof_member_count: u32,
        proof_member_set_hash: String,
    },
    Refuted {
        refutation_members: Vec<HypothesisComponentRefutationRefV1>,
        refutation_member_count: u32,
        refutation_member_set_hash: String,
    },
    NonTerminal {
        residual_risk_set_hash: String,
    },
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize)]
pub struct HypothesisClaimComponentOutcomeV1 {
    claim_component_member_hash: String,
    outcome: HypothesisClaimComponentOutcomeKindV1,
    lineage: HypothesisClaimComponentOutcomeLineageV1,
    outcome_hash: String,
}

impl HypothesisClaimComponentOutcomeV1 {
    pub fn satisfied(
        claim_component_member_hash: String,
        mut proof_members: Vec<HypothesisComponentProofRefV1>,
    ) -> Result<Self, HypothesisVerificationError> {
        require_hash(&claim_component_member_hash)?;
        if proof_members.is_empty()
            || proof_members
                .iter()
                .any(|member| member.claim_component_member_hash != claim_component_member_hash)
        {
            return Err(HypothesisVerificationError::ComponentLineageMismatch);
        }
        proof_members.sort_by(|left, right| left.member_hash.cmp(&right.member_hash));
        ensure_unique_hashes(
            proof_members
                .iter()
                .map(|member| member.member_hash.as_str()),
            HypothesisVerificationError::ComponentLineageMismatch,
        )?;
        let proof_member_set_hash = exact_set_hash(
            "hypothesis_component_proof_set.v1",
            proof_members
                .iter()
                .map(|member| member.member_hash.as_str()),
        )?;
        let lineage = HypothesisClaimComponentOutcomeLineageV1::Satisfied {
            proof_member_count: proof_members.len() as u32,
            proof_members,
            proof_member_set_hash,
        };
        let outcome_hash = hash_value(
            "hypothesis_claim_component_outcome.v1",
            &(
                &claim_component_member_hash,
                HypothesisClaimComponentOutcomeKindV1::Satisfied,
                &lineage,
            ),
        )?;
        Ok(Self {
            claim_component_member_hash,
            outcome: HypothesisClaimComponentOutcomeKindV1::Satisfied,
            lineage,
            outcome_hash,
        })
    }

    pub fn refuted(
        claim_component_member_hash: String,
        mut refutation_members: Vec<HypothesisComponentRefutationRefV1>,
    ) -> Result<Self, HypothesisVerificationError> {
        require_hash(&claim_component_member_hash)?;
        if refutation_members.is_empty()
            || refutation_members
                .iter()
                .any(|member| member.claim_component_member_hash != claim_component_member_hash)
        {
            return Err(HypothesisVerificationError::ComponentLineageMismatch);
        }
        refutation_members.sort_by(|left, right| left.member_hash.cmp(&right.member_hash));
        ensure_unique_hashes(
            refutation_members
                .iter()
                .map(|member| member.member_hash.as_str()),
            HypothesisVerificationError::ComponentLineageMismatch,
        )?;
        let refutation_member_set_hash = exact_set_hash(
            "hypothesis_component_refutation_set.v1",
            refutation_members
                .iter()
                .map(|member| member.member_hash.as_str()),
        )?;
        let lineage = HypothesisClaimComponentOutcomeLineageV1::Refuted {
            refutation_member_count: refutation_members.len() as u32,
            refutation_members,
            refutation_member_set_hash,
        };
        let outcome_hash = hash_value(
            "hypothesis_claim_component_outcome.v1",
            &(
                &claim_component_member_hash,
                HypothesisClaimComponentOutcomeKindV1::Refuted,
                &lineage,
            ),
        )?;
        Ok(Self {
            claim_component_member_hash,
            outcome: HypothesisClaimComponentOutcomeKindV1::Refuted,
            lineage,
            outcome_hash,
        })
    }

    pub fn nonterminal(
        claim_component_member_hash: String,
        outcome: HypothesisClaimComponentOutcomeKindV1,
        residual_risk_set_hash: String,
    ) -> Result<Self, HypothesisVerificationError> {
        if matches!(
            outcome,
            HypothesisClaimComponentOutcomeKindV1::Satisfied
                | HypothesisClaimComponentOutcomeKindV1::Refuted
        ) {
            return Err(HypothesisVerificationError::ComponentLineageMismatch);
        }
        require_hash(&claim_component_member_hash)?;
        require_hash(&residual_risk_set_hash)?;
        let lineage = HypothesisClaimComponentOutcomeLineageV1::NonTerminal {
            residual_risk_set_hash,
        };
        let outcome_hash = hash_value(
            "hypothesis_claim_component_outcome.v1",
            &(&claim_component_member_hash, outcome, &lineage),
        )?;
        Ok(Self {
            claim_component_member_hash,
            outcome,
            lineage,
            outcome_hash,
        })
    }

    pub fn claim_component_member_hash(&self) -> &str {
        &self.claim_component_member_hash
    }
    pub const fn outcome(&self) -> HypothesisClaimComponentOutcomeKindV1 {
        self.outcome
    }
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize)]
pub struct CampaignTerminalReceiptRefV1 {
    receipt_id: Uuid,
    receipt_version: u32,
    receipt_hash: String,
    plan_objective_member_hash: String,
    claim_component_member_hashes: Vec<String>,
    claim_component_count: u32,
    claim_component_set_hash: String,
    all_fresh_authority_binding_hash: String,
    member_hash: String,
}

impl CampaignTerminalReceiptRefV1 {
    pub fn from_server_receipt(
        receipt_id: Uuid,
        receipt_version: u32,
        receipt_hash: String,
        plan_objective_member_hash: String,
        mut claim_component_member_hashes: Vec<String>,
        all_fresh_authority_binding_hash: String,
    ) -> Result<Self, HypothesisVerificationError> {
        require_uuid("campaign_terminal_receipt_id", receipt_id)?;
        require_nonzero("campaign_terminal_receipt_version", receipt_version)?;
        for hash in [
            &receipt_hash,
            &plan_objective_member_hash,
            &all_fresh_authority_binding_hash,
        ] {
            require_hash(hash)?;
        }
        canonicalize_hash_set(&mut claim_component_member_hashes)?;
        if claim_component_member_hashes.is_empty() {
            return Err(HypothesisVerificationError::ObjectiveReceiptLineageIncomplete);
        }
        let claim_component_set_hash = exact_set_hash(
            "campaign_terminal_receipt_claim_components.v1",
            claim_component_member_hashes.iter().map(String::as_str),
        )?;
        let member_hash = hash_value(
            "campaign_terminal_receipt_ref.v1",
            &(
                receipt_id,
                receipt_version,
                &receipt_hash,
                &plan_objective_member_hash,
                &claim_component_set_hash,
                &all_fresh_authority_binding_hash,
            ),
        )?;
        Ok(Self {
            receipt_id,
            receipt_version,
            receipt_hash,
            plan_objective_member_hash,
            claim_component_count: claim_component_member_hashes.len() as u32,
            claim_component_member_hashes,
            claim_component_set_hash,
            all_fresh_authority_binding_hash,
            member_hash,
        })
    }
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize)]
pub struct OracleCensusReceiptRefV1 {
    receipt_id: Uuid,
    receipt_version: u32,
    receipt_hash: String,
    plan_objective_member_hash: String,
    claim_component_member_hashes: Vec<String>,
    claim_component_count: u32,
    claim_component_set_hash: String,
    oracle_member_set_hash: String,
    all_fresh_authority_binding_hash: String,
    member_hash: String,
}

impl OracleCensusReceiptRefV1 {
    #[allow(clippy::too_many_arguments)]
    pub fn from_server_receipt(
        receipt_id: Uuid,
        receipt_version: u32,
        receipt_hash: String,
        plan_objective_member_hash: String,
        mut claim_component_member_hashes: Vec<String>,
        oracle_member_set_hash: String,
        all_fresh_authority_binding_hash: String,
    ) -> Result<Self, HypothesisVerificationError> {
        require_uuid("oracle_census_receipt_id", receipt_id)?;
        require_nonzero("oracle_census_receipt_version", receipt_version)?;
        for hash in [
            &receipt_hash,
            &plan_objective_member_hash,
            &oracle_member_set_hash,
            &all_fresh_authority_binding_hash,
        ] {
            require_hash(hash)?;
        }
        canonicalize_hash_set(&mut claim_component_member_hashes)?;
        if claim_component_member_hashes.is_empty() {
            return Err(HypothesisVerificationError::ObjectiveReceiptLineageIncomplete);
        }
        let claim_component_set_hash = exact_set_hash(
            "oracle_census_receipt_claim_components.v1",
            claim_component_member_hashes.iter().map(String::as_str),
        )?;
        let member_hash = hash_value(
            "oracle_census_receipt_ref.v1",
            &(
                receipt_id,
                receipt_version,
                &receipt_hash,
                &plan_objective_member_hash,
                &claim_component_set_hash,
                &oracle_member_set_hash,
                &all_fresh_authority_binding_hash,
            ),
        )?;
        Ok(Self {
            receipt_id,
            receipt_version,
            receipt_hash,
            plan_objective_member_hash,
            claim_component_count: claim_component_member_hashes.len() as u32,
            claim_component_member_hashes,
            claim_component_set_hash,
            oracle_member_set_hash,
            all_fresh_authority_binding_hash,
            member_hash,
        })
    }
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize)]
pub struct CampaignCoverageReceiptRefV1 {
    receipt_id: Uuid,
    receipt_hash: String,
    plan_objective_member_hash: String,
    claim_component_set_hash: String,
    denominator_member_set_hash: String,
    member_hash: String,
}

impl CampaignCoverageReceiptRefV1 {
    pub fn from_server_receipt(
        receipt_id: Uuid,
        receipt_hash: String,
        plan_objective_member_hash: String,
        claim_component_set_hash: String,
        denominator_member_set_hash: String,
    ) -> Result<Self, HypothesisVerificationError> {
        require_uuid("coverage_receipt_id", receipt_id)?;
        for hash in [
            &receipt_hash,
            &plan_objective_member_hash,
            &claim_component_set_hash,
            &denominator_member_set_hash,
        ] {
            require_hash(hash)?;
        }
        let member_hash = hash_value(
            "campaign_coverage_receipt_ref.v1",
            &(
                receipt_id,
                &receipt_hash,
                &plan_objective_member_hash,
                &claim_component_set_hash,
                &denominator_member_set_hash,
            ),
        )?;
        Ok(Self {
            receipt_id,
            receipt_hash,
            plan_objective_member_hash,
            claim_component_set_hash,
            denominator_member_set_hash,
            member_hash,
        })
    }
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize)]
pub struct FactDeltaConsumptionReceiptRefV1 {
    receipt_id: Uuid,
    receipt_hash: String,
    plan_objective_member_hash: String,
    claim_component_set_hash: String,
    delta_set_hash: String,
    member_hash: String,
}

impl FactDeltaConsumptionReceiptRefV1 {
    pub fn from_server_receipt(
        receipt_id: Uuid,
        receipt_hash: String,
        plan_objective_member_hash: String,
        claim_component_set_hash: String,
        delta_set_hash: String,
    ) -> Result<Self, HypothesisVerificationError> {
        require_uuid("fact_delta_consumption_receipt_id", receipt_id)?;
        for hash in [
            &receipt_hash,
            &plan_objective_member_hash,
            &claim_component_set_hash,
            &delta_set_hash,
        ] {
            require_hash(hash)?;
        }
        let member_hash = hash_value(
            "fact_delta_consumption_receipt_ref.v1",
            &(
                receipt_id,
                &receipt_hash,
                &plan_objective_member_hash,
                &claim_component_set_hash,
                &delta_set_hash,
            ),
        )?;
        Ok(Self {
            receipt_id,
            receipt_hash,
            plan_objective_member_hash,
            claim_component_set_hash,
            delta_set_hash,
            member_hash,
        })
    }
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize)]
pub struct HypothesisVerificationObjectiveOutcomeV1 {
    outcome_receipt_id: Uuid,
    outcome_receipt_version: u32,
    outcome_ordinal: u32,
    predecessor_outcome_receipt_id: Option<Uuid>,
    predecessor_outcome_receipt_hash: Option<String>,
    campaign_head_hash: String,
    plan_objective_member_hash: String,
    verification_contract_hash: String,
    claim_component_outcomes: Vec<HypothesisClaimComponentOutcomeV1>,
    claim_component_outcome_count: u32,
    claim_component_outcome_set_hash: String,
    outcome: HypothesisVerificationObjectiveOutcomeKindV1,
    campaign_terminal_receipts: Vec<CampaignTerminalReceiptRefV1>,
    campaign_terminal_receipt_count: u32,
    campaign_terminal_receipt_set_hash: String,
    oracle_census_receipts: Vec<OracleCensusReceiptRefV1>,
    oracle_census_receipt_count: u32,
    oracle_census_receipt_set_hash: String,
    coverage_receipts: Vec<CampaignCoverageReceiptRefV1>,
    coverage_receipt_count: u32,
    coverage_receipt_set_hash: String,
    fact_delta_consumption_receipts: Vec<FactDeltaConsumptionReceiptRefV1>,
    fact_delta_consumption_receipt_count: u32,
    fact_delta_consumption_receipt_set_hash: String,
    unassigned_residual_risk_set_hash: String,
    outcome_lineage_hash: String,
    outcome_hash: String,
}

#[derive(Debug, Clone)]
pub struct HypothesisVerificationObjectiveOutcomeBuildInputV1 {
    pub outcome_receipt_id: Uuid,
    pub outcome_receipt_version: u32,
    pub outcome_ordinal: u32,
    pub predecessor_outcome_receipt_id: Option<Uuid>,
    pub predecessor_outcome_receipt_hash: Option<String>,
    pub campaign_head_hash: String,
    pub plan_objective_member_hash: String,
    pub verification_contract_hash: String,
    pub claim_component_outcomes: Vec<HypothesisClaimComponentOutcomeV1>,
    pub outcome: HypothesisVerificationObjectiveOutcomeKindV1,
    pub campaign_terminal_receipts: Vec<CampaignTerminalReceiptRefV1>,
    pub oracle_census_receipts: Vec<OracleCensusReceiptRefV1>,
    pub coverage_receipts: Vec<CampaignCoverageReceiptRefV1>,
    pub fact_delta_consumption_receipts: Vec<FactDeltaConsumptionReceiptRefV1>,
    pub unassigned_residual_risk_set_hash: String,
}

impl HypothesisVerificationObjectiveOutcomeV1 {
    pub fn compile(
        plan_objective: &HypothesisVerificationPlanObjectiveV1,
        mut input: HypothesisVerificationObjectiveOutcomeBuildInputV1,
    ) -> Result<Self, HypothesisVerificationError> {
        require_uuid("outcome_receipt_id", input.outcome_receipt_id)?;
        require_nonzero("outcome_receipt_version", input.outcome_receipt_version)?;
        require_hash(&input.campaign_head_hash)?;
        require_hash(&input.unassigned_residual_risk_set_hash)?;
        if input.plan_objective_member_hash != plan_objective.member_hash
            || input.verification_contract_hash != plan_objective.verification_contract_hash
        {
            return Err(HypothesisVerificationError::ObjectiveOutcomeBindingMismatch);
        }
        match (
            input.outcome_ordinal,
            input.predecessor_outcome_receipt_id,
            input.predecessor_outcome_receipt_hash.as_deref(),
        ) {
            (0, None, None) => {}
            (ordinal, Some(id), Some(hash)) if ordinal > 0 && !id.is_nil() => require_hash(hash)?,
            _ => return Err(HypothesisVerificationError::ObjectiveOutcomePredecessorInvalid),
        }
        input.claim_component_outcomes.sort_by(|left, right| {
            left.claim_component_member_hash
                .cmp(&right.claim_component_member_hash)
        });
        let actual_components = input
            .claim_component_outcomes
            .iter()
            .map(|outcome| outcome.claim_component_member_hash.clone())
            .collect::<Vec<_>>();
        if actual_components != plan_objective.claim_component_member_hashes {
            return Err(HypothesisVerificationError::ObjectiveOutcomeComponentSetMismatch);
        }
        let component_outcome_set_hash = exact_set_hash(
            "hypothesis_objective_component_outcomes.v1",
            input
                .claim_component_outcomes
                .iter()
                .map(|outcome| outcome.outcome_hash.as_str()),
        )?;
        let all_satisfied = input
            .claim_component_outcomes
            .iter()
            .all(|outcome| outcome.outcome == HypothesisClaimComponentOutcomeKindV1::Satisfied);
        let has_refuted = input
            .claim_component_outcomes
            .iter()
            .any(|outcome| outcome.outcome == HypothesisClaimComponentOutcomeKindV1::Refuted);
        if (input.outcome == HypothesisVerificationObjectiveOutcomeKindV1::Satisfied
            && !all_satisfied)
            || (input.outcome == HypothesisVerificationObjectiveOutcomeKindV1::Refuted
                && !has_refuted)
        {
            return Err(HypothesisVerificationError::ObjectiveOutcomeTruthMismatch);
        }
        let is_terminal = matches!(
            input.outcome,
            HypothesisVerificationObjectiveOutcomeKindV1::Satisfied
                | HypothesisVerificationObjectiveOutcomeKindV1::Refuted
        );
        if is_terminal
            && (input.campaign_terminal_receipts.is_empty()
                || input.oracle_census_receipts.is_empty()
                || input.coverage_receipts.is_empty()
                || input.fact_delta_consumption_receipts.is_empty())
        {
            return Err(HypothesisVerificationError::ObjectiveReceiptLineageIncomplete);
        }
        let objective_components = plan_objective
            .claim_component_member_hashes
            .iter()
            .cloned()
            .collect::<BTreeSet<_>>();
        for (objective_hash, component_hashes) in input
            .campaign_terminal_receipts
            .iter()
            .map(|receipt| {
                (
                    receipt.plan_objective_member_hash.as_str(),
                    receipt.claim_component_member_hashes.as_slice(),
                )
            })
            .chain(input.oracle_census_receipts.iter().map(|receipt| {
                (
                    receipt.plan_objective_member_hash.as_str(),
                    receipt.claim_component_member_hashes.as_slice(),
                )
            }))
        {
            if objective_hash != plan_objective.member_hash
                || !component_hashes
                    .iter()
                    .all(|hash| objective_components.contains(hash))
            {
                return Err(HypothesisVerificationError::ObjectiveReceiptBindingMismatch);
            }
        }
        for (objective_hash, component_set_hash) in input
            .coverage_receipts
            .iter()
            .map(|receipt| {
                (
                    receipt.plan_objective_member_hash.as_str(),
                    receipt.claim_component_set_hash.as_str(),
                )
            })
            .chain(input.fact_delta_consumption_receipts.iter().map(|receipt| {
                (
                    receipt.plan_objective_member_hash.as_str(),
                    receipt.claim_component_set_hash.as_str(),
                )
            }))
        {
            if objective_hash != plan_objective.member_hash
                || component_set_hash != plan_objective.claim_component_set_hash
            {
                return Err(HypothesisVerificationError::ObjectiveReceiptBindingMismatch);
            }
        }
        sort_and_validate_receipts(&mut input.campaign_terminal_receipts, |receipt| {
            &receipt.member_hash
        })?;
        sort_and_validate_receipts(&mut input.oracle_census_receipts, |receipt| {
            &receipt.member_hash
        })?;
        sort_and_validate_receipts(&mut input.coverage_receipts, |receipt| &receipt.member_hash)?;
        sort_and_validate_receipts(&mut input.fact_delta_consumption_receipts, |receipt| {
            &receipt.member_hash
        })?;
        let campaign_terminal_receipt_set_hash = exact_set_hash(
            "hypothesis_objective_campaign_terminal_receipts.v1",
            input
                .campaign_terminal_receipts
                .iter()
                .map(|receipt| receipt.member_hash.as_str()),
        )?;
        let oracle_census_receipt_set_hash = exact_set_hash(
            "hypothesis_objective_oracle_census_receipts.v1",
            input
                .oracle_census_receipts
                .iter()
                .map(|receipt| receipt.member_hash.as_str()),
        )?;
        let coverage_receipt_set_hash = exact_set_hash(
            "hypothesis_objective_coverage_receipts.v1",
            input
                .coverage_receipts
                .iter()
                .map(|receipt| receipt.member_hash.as_str()),
        )?;
        let fact_delta_consumption_receipt_set_hash = exact_set_hash(
            "hypothesis_objective_fact_delta_receipts.v1",
            input
                .fact_delta_consumption_receipts
                .iter()
                .map(|receipt| receipt.member_hash.as_str()),
        )?;
        let outcome_lineage_hash = hash_value(
            "hypothesis_verification_objective_outcome_lineage.v1",
            &(
                &component_outcome_set_hash,
                &campaign_terminal_receipt_set_hash,
                &oracle_census_receipt_set_hash,
                &coverage_receipt_set_hash,
                &fact_delta_consumption_receipt_set_hash,
                &input.unassigned_residual_risk_set_hash,
            ),
        )?;
        let outcome_hash = hash_value(
            "hypothesis_verification_objective_outcome.v1",
            &(
                input.outcome_receipt_id,
                input.outcome_receipt_version,
                input.outcome_ordinal,
                input.predecessor_outcome_receipt_id,
                &input.predecessor_outcome_receipt_hash,
                &input.campaign_head_hash,
                &input.plan_objective_member_hash,
                &input.verification_contract_hash,
                input.outcome,
                &outcome_lineage_hash,
            ),
        )?;
        Ok(Self {
            outcome_receipt_id: input.outcome_receipt_id,
            outcome_receipt_version: input.outcome_receipt_version,
            outcome_ordinal: input.outcome_ordinal,
            predecessor_outcome_receipt_id: input.predecessor_outcome_receipt_id,
            predecessor_outcome_receipt_hash: input.predecessor_outcome_receipt_hash,
            campaign_head_hash: input.campaign_head_hash,
            plan_objective_member_hash: input.plan_objective_member_hash,
            verification_contract_hash: input.verification_contract_hash,
            claim_component_outcome_count: input.claim_component_outcomes.len() as u32,
            claim_component_outcomes: input.claim_component_outcomes,
            claim_component_outcome_set_hash: component_outcome_set_hash,
            outcome: input.outcome,
            campaign_terminal_receipt_count: input.campaign_terminal_receipts.len() as u32,
            campaign_terminal_receipts: input.campaign_terminal_receipts,
            campaign_terminal_receipt_set_hash,
            oracle_census_receipt_count: input.oracle_census_receipts.len() as u32,
            oracle_census_receipts: input.oracle_census_receipts,
            oracle_census_receipt_set_hash,
            coverage_receipt_count: input.coverage_receipts.len() as u32,
            coverage_receipts: input.coverage_receipts,
            coverage_receipt_set_hash,
            fact_delta_consumption_receipt_count: input.fact_delta_consumption_receipts.len()
                as u32,
            fact_delta_consumption_receipts: input.fact_delta_consumption_receipts,
            fact_delta_consumption_receipt_set_hash,
            unassigned_residual_risk_set_hash: input.unassigned_residual_risk_set_hash,
            outcome_lineage_hash,
            outcome_hash,
        })
    }

    pub fn outcome_hash(&self) -> &str {
        &self.outcome_hash
    }
}

/// Minimal server-loaded outcome used by the Plan B outer truth reducer.  Plan C
/// will persist the richer receipt DTO above before constructing this view.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct ObjectiveOutcomeViewV1 {
    plan_objective_member_hash: String,
    component_outcomes: BTreeMap<String, HypothesisClaimComponentOutcomeKindV1>,
    outcome: HypothesisVerificationObjectiveOutcomeKindV1,
    outcome_hash: String,
}

impl From<&HypothesisVerificationObjectiveOutcomeV1> for ObjectiveOutcomeViewV1 {
    fn from(outcome: &HypothesisVerificationObjectiveOutcomeV1) -> Self {
        Self {
            plan_objective_member_hash: outcome.plan_objective_member_hash.clone(),
            component_outcomes: outcome
                .claim_component_outcomes
                .iter()
                .map(|component| {
                    (
                        component.claim_component_member_hash.clone(),
                        component.outcome,
                    )
                })
                .collect(),
            outcome: outcome.outcome,
            outcome_hash: outcome.outcome_hash.clone(),
        }
    }
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize)]
#[serde(rename_all = "snake_case")]
pub enum HypothesisRevisionAdjudicationVerdictV1 {
    Verified,
    Refuted,
    NonTerminal,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, PartialOrd, Ord, Serialize)]
#[serde(rename_all = "snake_case")]
pub enum HypothesisRevisionNonTerminalReasonV1 {
    Inconclusive,
    ExhaustedWithResiduals,
    Unassigned,
    Blocked,
    Invalidated,
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct HypothesisRevisionAggregateV1 {
    verdict: HypothesisRevisionAdjudicationVerdictV1,
    unresolved: Vec<(String, String)>,
    non_decisive_limitations: Vec<(String, String)>,
    reason: Option<HypothesisRevisionNonTerminalReasonV1>,
}

impl HypothesisRevisionAggregateV1 {
    pub const fn verdict(&self) -> HypothesisRevisionAdjudicationVerdictV1 {
        self.verdict
    }
    pub fn unresolved(&self) -> &[(String, String)] {
        &self.unresolved
    }
    pub fn non_decisive_limitations(&self) -> &[(String, String)] {
        &self.non_decisive_limitations
    }
    pub const fn reason(&self) -> Option<HypothesisRevisionNonTerminalReasonV1> {
        self.reason
    }
}

pub fn reduce_verification_plan_v1(
    plan: &HypothesisVerificationPlanV1,
    outcomes: &[ObjectiveOutcomeViewV1],
) -> Result<HypothesisRevisionAggregateV1, HypothesisVerificationError> {
    let expected = plan
        .objectives
        .iter()
        .map(|objective| objective.member_hash.clone())
        .collect::<BTreeSet<_>>();
    let actual = outcomes
        .iter()
        .map(|outcome| outcome.plan_objective_member_hash.clone())
        .collect::<BTreeSet<_>>();
    if expected != actual || actual.len() != outcomes.len() {
        return Err(HypothesisVerificationError::ObjectiveOutcomeSetMismatch);
    }
    for outcome in outcomes {
        require_hash(&outcome.outcome_hash)?;
        let objective = plan
            .objectives
            .iter()
            .find(|objective| objective.member_hash == outcome.plan_objective_member_hash)
            .ok_or(HypothesisVerificationError::ObjectiveOutcomeSetMismatch)?;
        let expected_components = objective
            .claim_component_member_hashes
            .iter()
            .map(String::as_str)
            .collect::<BTreeSet<_>>();
        let actual_components = outcome
            .component_outcomes
            .keys()
            .map(String::as_str)
            .collect::<BTreeSet<_>>();
        if expected_components != actual_components {
            return Err(HypothesisVerificationError::ObjectiveOutcomeComponentSetMismatch);
        }
    }
    let outcomes = outcomes
        .iter()
        .map(|outcome| (outcome.plan_objective_member_hash.as_str(), outcome))
        .collect::<BTreeMap<_, _>>();

    let mut winning_path = None;
    let mut falsified_paths = BTreeSet::new();
    for path in &plan.proof_paths {
        let proved = path.members.iter().all(|member| {
            let objective = plan
                .objectives
                .iter()
                .find(|objective| objective.member_hash == member.plan_objective_member_hash)
                .expect("sealed path objective exists");
            let outcome = outcomes
                .get(member.plan_objective_member_hash.as_str())
                .expect("exact outcome set was checked");
            objective
                .claim_component_member_hashes
                .iter()
                .all(|component| {
                    outcome.component_outcomes.get(component)
                        == Some(&HypothesisClaimComponentOutcomeKindV1::Satisfied)
                })
        });
        if proved {
            winning_path = Some(path.path_key.clone());
            break;
        }
        let falsified = path.members.iter().any(|member| {
            member.role == HypothesisVerificationPlanPathMemberRoleV1::RequiredProofAndPathFalsifier
                && member
                    .falsifier_claim_component_member_hashes
                    .iter()
                    .any(|component| {
                        outcomes
                            .get(member.plan_objective_member_hash.as_str())
                            .and_then(|outcome| outcome.component_outcomes.get(component))
                            == Some(&HypothesisClaimComponentOutcomeKindV1::Refuted)
                    })
        });
        if falsified {
            falsified_paths.insert(path.path_key.clone());
        }
    }

    let mut nonterminal = Vec::new();
    for outcome in outcomes.values() {
        for (component, kind) in &outcome.component_outcomes {
            if !matches!(
                kind,
                HypothesisClaimComponentOutcomeKindV1::Satisfied
                    | HypothesisClaimComponentOutcomeKindV1::Refuted
            ) {
                nonterminal.push((
                    outcome.plan_objective_member_hash.clone(),
                    component.clone(),
                ));
            }
        }
    }
    nonterminal.sort();
    nonterminal.dedup();
    if winning_path.is_some() {
        return Ok(HypothesisRevisionAggregateV1 {
            verdict: HypothesisRevisionAdjudicationVerdictV1::Verified,
            unresolved: Vec::new(),
            non_decisive_limitations: nonterminal,
            reason: None,
        });
    }
    if falsified_paths.len() == plan.proof_paths.len() {
        return Ok(HypothesisRevisionAggregateV1 {
            verdict: HypothesisRevisionAdjudicationVerdictV1::Refuted,
            unresolved: Vec::new(),
            non_decisive_limitations: nonterminal,
            reason: None,
        });
    }
    let live_objectives = plan
        .proof_paths
        .iter()
        .filter(|path| !falsified_paths.contains(&path.path_key))
        .flat_map(|path| {
            path.members
                .iter()
                .map(|member| member.plan_objective_member_hash.as_str())
        })
        .collect::<BTreeSet<_>>();
    let mut unresolved = outcomes
        .values()
        .filter(|outcome| live_objectives.contains(outcome.plan_objective_member_hash.as_str()))
        .flat_map(|outcome| {
            outcome
                .component_outcomes
                .iter()
                .filter(|(_, kind)| **kind != HypothesisClaimComponentOutcomeKindV1::Satisfied)
                .map(|(component, _)| {
                    (
                        outcome.plan_objective_member_hash.clone(),
                        component.clone(),
                    )
                })
        })
        .collect::<Vec<_>>();
    unresolved.sort();
    unresolved.dedup();
    let unresolved_set = unresolved.iter().cloned().collect::<BTreeSet<_>>();
    let non_decisive_limitations = nonterminal
        .into_iter()
        .filter(|member| !unresolved_set.contains(member))
        .collect::<Vec<_>>();
    let reason = outcomes
        .values()
        .filter(|outcome| live_objectives.contains(outcome.plan_objective_member_hash.as_str()))
        .map(|outcome| match outcome.outcome {
            HypothesisVerificationObjectiveOutcomeKindV1::Invalidated => {
                HypothesisRevisionNonTerminalReasonV1::Invalidated
            }
            HypothesisVerificationObjectiveOutcomeKindV1::Blocked => {
                HypothesisRevisionNonTerminalReasonV1::Blocked
            }
            HypothesisVerificationObjectiveOutcomeKindV1::Unassigned => {
                HypothesisRevisionNonTerminalReasonV1::Unassigned
            }
            HypothesisVerificationObjectiveOutcomeKindV1::ExhaustedWithResiduals => {
                HypothesisRevisionNonTerminalReasonV1::ExhaustedWithResiduals
            }
            _ => HypothesisRevisionNonTerminalReasonV1::Inconclusive,
        })
        .max()
        .unwrap_or(HypothesisRevisionNonTerminalReasonV1::Inconclusive);
    Ok(HypothesisRevisionAggregateV1 {
        verdict: HypothesisRevisionAdjudicationVerdictV1::NonTerminal,
        unresolved,
        non_decisive_limitations,
        reason: Some(reason),
    })
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize)]
pub enum HypothesisRevisionAdjudicationLineageV1 {
    Verified {
        finding_id: Uuid,
        finding_lineage_hash: String,
    },
    Refuted {
        refutation_receipt_id: Uuid,
        predicate_component_set_hash: String,
        required_control_set_hash: String,
    },
    NonTerminal {
        reason: HypothesisRevisionNonTerminalReasonV1,
        residual_risk_set_hash: String,
    },
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize)]
pub struct HypothesisUnresolvedOutcomeRefV1 {
    plan_objective_member_hash: String,
    claim_component_member_hash: Option<String>,
    outcome_hash: String,
    member_hash: String,
}

impl HypothesisUnresolvedOutcomeRefV1 {
    fn compile(
        plan_objective_member_hash: String,
        claim_component_member_hash: Option<String>,
        outcome_hash: String,
    ) -> Result<Self, HypothesisVerificationError> {
        require_hash(&plan_objective_member_hash)?;
        if let Some(hash) = &claim_component_member_hash {
            require_hash(hash)?;
        }
        require_hash(&outcome_hash)?;
        let member_hash = hash_value(
            "hypothesis_unresolved_outcome_ref.v1",
            &(
                &plan_objective_member_hash,
                &claim_component_member_hash,
                &outcome_hash,
            ),
        )?;
        Ok(Self {
            plan_objective_member_hash,
            claim_component_member_hash,
            outcome_hash,
            member_hash,
        })
    }
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize)]
pub struct PersistedAllFreshToolTruthAuthorityBindingV1 {
    bundle_seal_id: Uuid,
    relevant_root_set_hash: String,
    bundle_member_set_hash: String,
    receipt_set_hash: String,
    semantic_authority_bundle_hash: String,
    freshness_attestation_bundle_hash: String,
    temporal_validity_bundle_hash: String,
    temporal_validity_policy_hash: String,
    temporal_validity_decision_set_hash: String,
    observation_window_hash: String,
    target_state_epoch_set_hash: String,
    earliest_effective_valid_until: DateTime<Utc>,
    binding_hash: String,
}

impl PersistedAllFreshToolTruthAuthorityBindingV1 {
    #[allow(clippy::too_many_arguments)]
    pub fn from_server_bundle(
        bundle_seal_id: Uuid,
        relevant_root_set_hash: String,
        bundle_member_set_hash: String,
        receipt_set_hash: String,
        semantic_authority_bundle_hash: String,
        freshness_attestation_bundle_hash: String,
        temporal_validity_bundle_hash: String,
        temporal_validity_policy_hash: String,
        temporal_validity_decision_set_hash: String,
        observation_window_hash: String,
        target_state_epoch_set_hash: String,
        earliest_effective_valid_until: DateTime<Utc>,
    ) -> Result<Self, HypothesisVerificationError> {
        require_uuid("bundle_seal_id", bundle_seal_id)?;
        for hash in [
            &relevant_root_set_hash,
            &bundle_member_set_hash,
            &receipt_set_hash,
            &semantic_authority_bundle_hash,
            &freshness_attestation_bundle_hash,
            &temporal_validity_bundle_hash,
            &temporal_validity_policy_hash,
            &temporal_validity_decision_set_hash,
            &observation_window_hash,
            &target_state_epoch_set_hash,
        ] {
            require_hash(hash)?;
        }
        let binding_hash = hash_value(
            "persisted_all_fresh_tool_truth_authority_binding.v1",
            &(
                bundle_seal_id,
                &relevant_root_set_hash,
                &bundle_member_set_hash,
                &receipt_set_hash,
                &semantic_authority_bundle_hash,
                &freshness_attestation_bundle_hash,
                &temporal_validity_bundle_hash,
                &temporal_validity_policy_hash,
                &temporal_validity_decision_set_hash,
                &observation_window_hash,
                &target_state_epoch_set_hash,
                earliest_effective_valid_until,
            ),
        )?;
        Ok(Self {
            bundle_seal_id,
            relevant_root_set_hash,
            bundle_member_set_hash,
            receipt_set_hash,
            semantic_authority_bundle_hash,
            freshness_attestation_bundle_hash,
            temporal_validity_bundle_hash,
            temporal_validity_policy_hash,
            temporal_validity_decision_set_hash,
            observation_window_hash,
            target_state_epoch_set_hash,
            earliest_effective_valid_until,
            binding_hash,
        })
    }

    pub fn binding_hash(&self) -> &str {
        &self.binding_hash
    }
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize)]
pub struct HypothesisRevisionAdjudication {
    adjudication_id: Uuid,
    adjudication_version: u32,
    revision_id: Uuid,
    revision_hash: String,
    verification_plan_id: Uuid,
    verification_plan_hash: String,
    objective_outcomes: Vec<HypothesisVerificationObjectiveOutcomeV1>,
    objective_outcome_count: u32,
    objective_outcome_set_hash: String,
    unresolved_outcomes: Vec<HypothesisUnresolvedOutcomeRefV1>,
    unresolved_outcome_count: u32,
    unresolved_outcome_set_hash: String,
    non_decisive_limitations: Vec<HypothesisUnresolvedOutcomeRefV1>,
    non_decisive_limitation_count: u32,
    non_decisive_limitation_set_hash: String,
    all_fresh_authority_binding: PersistedAllFreshToolTruthAuthorityBindingV1,
    verdict: HypothesisRevisionAdjudicationVerdictV1,
    adjudication_lineage: HypothesisRevisionAdjudicationLineageV1,
    adjudication_hash: String,
}

#[derive(Debug, Clone)]
pub enum HypothesisRevisionAdjudicationLineageInputV1 {
    Verified {
        finding_id: Uuid,
        finding_lineage_hash: String,
    },
    Refuted {
        refutation_receipt_id: Uuid,
        predicate_component_set_hash: String,
        required_control_set_hash: String,
    },
    NonTerminal {
        residual_risk_set_hash: String,
    },
}

impl HypothesisRevisionAdjudication {
    pub fn compile(
        adjudication_id: Uuid,
        plan: &HypothesisVerificationPlanV1,
        mut objective_outcomes: Vec<HypothesisVerificationObjectiveOutcomeV1>,
        all_fresh_authority_binding: PersistedAllFreshToolTruthAuthorityBindingV1,
        lineage_input: HypothesisRevisionAdjudicationLineageInputV1,
    ) -> Result<Self, HypothesisVerificationError> {
        require_uuid("adjudication_id", adjudication_id)?;
        objective_outcomes.sort_by(|left, right| {
            left.plan_objective_member_hash
                .cmp(&right.plan_objective_member_hash)
        });
        ensure_unique_hashes(
            objective_outcomes
                .iter()
                .map(|outcome| outcome.outcome_hash.as_str()),
            HypothesisVerificationError::ObjectiveOutcomeSetMismatch,
        )?;
        let views = objective_outcomes
            .iter()
            .map(ObjectiveOutcomeViewV1::from)
            .collect::<Vec<_>>();
        let aggregate = reduce_verification_plan_v1(plan, &views)?;
        let adjudication_lineage = match (aggregate.verdict, lineage_input) {
            (
                HypothesisRevisionAdjudicationVerdictV1::Verified,
                HypothesisRevisionAdjudicationLineageInputV1::Verified {
                    finding_id,
                    finding_lineage_hash,
                },
            ) => {
                require_uuid("finding_id", finding_id)?;
                require_hash(&finding_lineage_hash)?;
                HypothesisRevisionAdjudicationLineageV1::Verified {
                    finding_id,
                    finding_lineage_hash,
                }
            }
            (
                HypothesisRevisionAdjudicationVerdictV1::Refuted,
                HypothesisRevisionAdjudicationLineageInputV1::Refuted {
                    refutation_receipt_id,
                    predicate_component_set_hash,
                    required_control_set_hash,
                },
            ) => {
                require_uuid("refutation_receipt_id", refutation_receipt_id)?;
                require_hash(&predicate_component_set_hash)?;
                require_hash(&required_control_set_hash)?;
                HypothesisRevisionAdjudicationLineageV1::Refuted {
                    refutation_receipt_id,
                    predicate_component_set_hash,
                    required_control_set_hash,
                }
            }
            (
                HypothesisRevisionAdjudicationVerdictV1::NonTerminal,
                HypothesisRevisionAdjudicationLineageInputV1::NonTerminal {
                    residual_risk_set_hash,
                },
            ) => {
                require_hash(&residual_risk_set_hash)?;
                HypothesisRevisionAdjudicationLineageV1::NonTerminal {
                    reason: aggregate
                        .reason
                        .ok_or(HypothesisVerificationError::AdjudicationLineageMismatch)?,
                    residual_risk_set_hash,
                }
            }
            _ => return Err(HypothesisVerificationError::AdjudicationLineageMismatch),
        };
        let outcome_hash_by_objective = objective_outcomes
            .iter()
            .map(|outcome| {
                (
                    outcome.plan_objective_member_hash.clone(),
                    outcome.outcome_hash.clone(),
                )
            })
            .collect::<BTreeMap<_, _>>();
        let mut unresolved_outcomes = aggregate
            .unresolved
            .into_iter()
            .map(|(objective, component)| {
                let outcome_hash = outcome_hash_by_objective
                    .get(&objective)
                    .ok_or(HypothesisVerificationError::ObjectiveOutcomeSetMismatch)?;
                HypothesisUnresolvedOutcomeRefV1::compile(
                    objective,
                    Some(component),
                    outcome_hash.clone(),
                )
            })
            .collect::<Result<Vec<_>, _>>()?;
        let mut non_decisive_limitations = aggregate
            .non_decisive_limitations
            .into_iter()
            .map(|(objective, component)| {
                let outcome_hash = outcome_hash_by_objective
                    .get(&objective)
                    .ok_or(HypothesisVerificationError::ObjectiveOutcomeSetMismatch)?;
                HypothesisUnresolvedOutcomeRefV1::compile(
                    objective,
                    Some(component),
                    outcome_hash.clone(),
                )
            })
            .collect::<Result<Vec<_>, _>>()?;
        unresolved_outcomes.sort_by(|left, right| left.member_hash.cmp(&right.member_hash));
        non_decisive_limitations.sort_by(|left, right| left.member_hash.cmp(&right.member_hash));
        let objective_outcome_set_hash = exact_set_hash(
            "hypothesis_revision_objective_outcomes.v1",
            objective_outcomes
                .iter()
                .map(|outcome| outcome.outcome_hash.as_str()),
        )?;
        let unresolved_outcome_set_hash = exact_set_hash(
            "hypothesis_revision_unresolved_outcomes.v1",
            unresolved_outcomes
                .iter()
                .map(|outcome| outcome.member_hash.as_str()),
        )?;
        let non_decisive_limitation_set_hash = exact_set_hash(
            "hypothesis_revision_non_decisive_limitations.v1",
            non_decisive_limitations
                .iter()
                .map(|outcome| outcome.member_hash.as_str()),
        )?;
        let adjudication_hash = hash_value(
            "hypothesis_revision_adjudication.v1",
            &(
                adjudication_id,
                1_u32,
                plan.revision_id,
                &plan.revision_hash,
                plan.plan_id,
                &plan.plan_hash,
                &objective_outcome_set_hash,
                &unresolved_outcome_set_hash,
                &non_decisive_limitation_set_hash,
                &all_fresh_authority_binding.binding_hash,
                aggregate.verdict,
                &adjudication_lineage,
            ),
        )?;
        Ok(Self {
            adjudication_id,
            adjudication_version: 1,
            revision_id: plan.revision_id,
            revision_hash: plan.revision_hash.clone(),
            verification_plan_id: plan.plan_id,
            verification_plan_hash: plan.plan_hash.clone(),
            objective_outcome_count: objective_outcomes.len() as u32,
            objective_outcomes,
            objective_outcome_set_hash,
            unresolved_outcome_count: unresolved_outcomes.len() as u32,
            unresolved_outcomes,
            unresolved_outcome_set_hash,
            non_decisive_limitation_count: non_decisive_limitations.len() as u32,
            non_decisive_limitations,
            non_decisive_limitation_set_hash,
            all_fresh_authority_binding,
            verdict: aggregate.verdict,
            adjudication_lineage,
            adjudication_hash,
        })
    }

    pub const fn adjudication_id(&self) -> Uuid {
        self.adjudication_id
    }
    pub const fn verdict(&self) -> HypothesisRevisionAdjudicationVerdictV1 {
        self.verdict
    }
    pub fn adjudication_hash(&self) -> &str {
        &self.adjudication_hash
    }
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize)]
#[serde(rename_all = "snake_case")]
pub enum HypothesisTerminalEpistemicStateV1 {
    Verified,
    Refuted,
    Invalid,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize)]
pub struct HypothesisRevisionTransitionDecisionV1 {
    decision_id: Uuid,
    predecessor_revision_id: Uuid,
    successor_epistemic_state: HypothesisTerminalEpistemicStateV1,
    verification_plan_id: Uuid,
    verification_plan_hash: String,
    adjudication_id: Uuid,
    adjudication_hash: String,
    objective_outcome_set_hash: String,
    all_fresh_authority_binding_hash: String,
    decision_hash: String,
}

impl HypothesisRevisionTransitionDecisionV1 {
    pub fn compile(
        decision_id: Uuid,
        adjudication: &HypothesisRevisionAdjudication,
    ) -> Result<Self, HypothesisVerificationError> {
        require_uuid("transition_decision_id", decision_id)?;
        let successor_epistemic_state = match adjudication.verdict {
            HypothesisRevisionAdjudicationVerdictV1::Verified => {
                HypothesisTerminalEpistemicStateV1::Verified
            }
            HypothesisRevisionAdjudicationVerdictV1::Refuted => {
                HypothesisTerminalEpistemicStateV1::Refuted
            }
            HypothesisRevisionAdjudicationVerdictV1::NonTerminal => {
                return Err(HypothesisVerificationError::TerminalDecisionForNonTerminal)
            }
        };
        let decision_hash = hash_value(
            "hypothesis_revision_transition_decision.v1",
            &(
                decision_id,
                adjudication.revision_id,
                successor_epistemic_state,
                adjudication.verification_plan_id,
                &adjudication.verification_plan_hash,
                adjudication.adjudication_id,
                &adjudication.adjudication_hash,
                &adjudication.objective_outcome_set_hash,
                &adjudication.all_fresh_authority_binding.binding_hash,
            ),
        )?;
        Ok(Self {
            decision_id,
            predecessor_revision_id: adjudication.revision_id,
            successor_epistemic_state,
            verification_plan_id: adjudication.verification_plan_id,
            verification_plan_hash: adjudication.verification_plan_hash.clone(),
            adjudication_id: adjudication.adjudication_id,
            adjudication_hash: adjudication.adjudication_hash.clone(),
            objective_outcome_set_hash: adjudication.objective_outcome_set_hash.clone(),
            all_fresh_authority_binding_hash: adjudication
                .all_fresh_authority_binding
                .binding_hash
                .clone(),
            decision_hash,
        })
    }

    pub const fn decision_id(&self) -> Uuid {
        self.decision_id
    }
    pub fn decision_hash(&self) -> &str {
        &self.decision_hash
    }
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize)]
pub struct HypothesisRevisionTransitionReceiptV1 {
    receipt_id: Uuid,
    transition_decision_id: Uuid,
    transition_decision_hash: String,
    successor_revision_id: Uuid,
    successor_revision_hash: String,
    state_event_hash: String,
    receipt_hash: String,
}

impl HypothesisRevisionTransitionReceiptV1 {
    pub fn compile(
        receipt_id: Uuid,
        decision: &HypothesisRevisionTransitionDecisionV1,
        successor_revision_id: Uuid,
        successor_revision_hash: String,
        state_event_hash: String,
    ) -> Result<Self, HypothesisVerificationError> {
        require_uuid("transition_receipt_id", receipt_id)?;
        require_uuid("successor_revision_id", successor_revision_id)?;
        require_hash(&successor_revision_hash)?;
        require_hash(&state_event_hash)?;
        let receipt_hash = hash_value(
            "hypothesis_revision_transition_receipt.v1",
            &(
                receipt_id,
                decision.decision_id,
                &decision.decision_hash,
                successor_revision_id,
                &successor_revision_hash,
                &state_event_hash,
            ),
        )?;
        Ok(Self {
            receipt_id,
            transition_decision_id: decision.decision_id,
            transition_decision_hash: decision.decision_hash.clone(),
            successor_revision_id,
            successor_revision_hash,
            state_event_hash,
            receipt_hash,
        })
    }

    pub fn receipt_hash(&self) -> &str {
        &self.receipt_hash
    }
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize)]
pub struct HypothesisRevisionAdjudicationAuthorityV1 {
    verification_plan: HypothesisVerificationPlanV1,
    adjudication: HypothesisRevisionAdjudication,
    transition_decision: HypothesisRevisionTransitionDecisionV1,
    transition_receipt: HypothesisRevisionTransitionReceiptV1,
}

impl HypothesisRevisionAdjudicationAuthorityV1 {
    pub fn compile(
        verification_plan: HypothesisVerificationPlanV1,
        adjudication: HypothesisRevisionAdjudication,
        transition_decision: HypothesisRevisionTransitionDecisionV1,
        transition_receipt: HypothesisRevisionTransitionReceiptV1,
    ) -> Result<Self, HypothesisVerificationError> {
        let authority = Self {
            verification_plan,
            adjudication,
            transition_decision,
            transition_receipt,
        };
        authority.validate()?;
        Ok(authority)
    }

    pub fn validate_absent() -> Result<Self, HypothesisVerificationError> {
        Err(HypothesisVerificationError::RevisionAuthorityIncomplete)
    }

    pub fn validate(&self) -> Result<(), HypothesisVerificationError> {
        if self.adjudication.verification_plan_id != self.verification_plan.plan_id
            || self.adjudication.verification_plan_hash != self.verification_plan.plan_hash
            || self.transition_decision.verification_plan_id != self.verification_plan.plan_id
            || self.transition_decision.verification_plan_hash != self.verification_plan.plan_hash
            || self.transition_decision.adjudication_id != self.adjudication.adjudication_id
            || self.transition_decision.adjudication_hash != self.adjudication.adjudication_hash
            || self.transition_receipt.transition_decision_id
                != self.transition_decision.decision_id
            || self.transition_receipt.transition_decision_hash
                != self.transition_decision.decision_hash
        {
            return Err(HypothesisVerificationError::RevisionAuthorityBindingMismatch);
        }
        Ok(())
    }
}

fn canonicalize_hash_set(hashes: &mut [String]) -> Result<(), HypothesisVerificationError> {
    hashes.sort();
    ensure_unique_hashes(
        hashes.iter().map(String::as_str),
        HypothesisVerificationError::ObjectiveReceiptBindingMismatch,
    )
}

fn sort_and_validate_receipts<T, F>(
    receipts: &mut [T],
    member_hash: F,
) -> Result<(), HypothesisVerificationError>
where
    F: Fn(&T) -> &str,
{
    receipts.sort_by(|left, right| member_hash(left).cmp(member_hash(right)));
    ensure_unique_hashes(
        receipts.iter().map(member_hash),
        HypothesisVerificationError::ObjectiveReceiptBindingMismatch,
    )
}

fn hash_value<T: Serialize + ?Sized>(
    domain: &'static str,
    value: &T,
) -> Result<String, HypothesisVerificationError> {
    let bytes = serde_json::to_vec(value)
        .map_err(|error| HypothesisVerificationError::Serialization(error.to_string()))?;
    let mut hasher = Sha256::new();
    hasher.update(domain.as_bytes());
    hasher.update([0]);
    hasher.update((bytes.len() as u64).to_be_bytes());
    hasher.update(bytes);
    Ok(format!("sha256:{}", hex_lower(&hasher.finalize())))
}

fn hex_lower(bytes: &[u8]) -> String {
    const HEX: &[u8; 16] = b"0123456789abcdef";
    let mut output = String::with_capacity(bytes.len() * 2);
    for byte in bytes {
        output.push(HEX[(byte >> 4) as usize] as char);
        output.push(HEX[(byte & 0x0f) as usize] as char);
    }
    output
}

fn exact_set_hash<'a>(
    domain: &'static str,
    members: impl IntoIterator<Item = &'a str>,
) -> Result<String, HypothesisVerificationError> {
    let mut members = members.into_iter().collect::<Vec<_>>();
    members.sort_unstable();
    for member in &members {
        require_hash(member)?;
    }
    hash_value(domain, &members)
}

fn ensure_unique_hashes<'a>(
    hashes: impl IntoIterator<Item = &'a str>,
    error: HypothesisVerificationError,
) -> Result<(), HypothesisVerificationError> {
    let mut seen = BTreeSet::new();
    for hash in hashes {
        require_hash(hash)?;
        if !seen.insert(hash) {
            return Err(error);
        }
    }
    Ok(())
}

fn require_hash(hash: &str) -> Result<(), HypothesisVerificationError> {
    validate_sha256(hash).map_err(|_| HypothesisVerificationError::InvalidHash(hash.into()))
}

fn require_uuid(field: &'static str, value: Uuid) -> Result<(), HypothesisVerificationError> {
    if value.is_nil() {
        Err(HypothesisVerificationError::NilUuid(field))
    } else {
        Ok(())
    }
}

fn require_nonzero(field: &'static str, value: u32) -> Result<(), HypothesisVerificationError> {
    if value == 0 {
        Err(HypothesisVerificationError::ZeroVersion(field))
    } else {
        Ok(())
    }
}

fn require_nonblank(field: &'static str, value: &str) -> Result<(), HypothesisVerificationError> {
    if value.trim().is_empty() {
        Err(HypothesisVerificationError::Blank(field))
    } else {
        Ok(())
    }
}

#[derive(Debug, thiserror::Error, Clone, PartialEq, Eq)]
pub enum HypothesisVerificationError {
    #[error("{0} must not be nil")]
    NilUuid(&'static str),
    #[error("{0} must be nonzero")]
    ZeroVersion(&'static str),
    #[error("{0} must not be blank")]
    Blank(&'static str),
    #[error("invalid sha256 hash: {0}")]
    InvalidHash(String),
    #[error("serialization failed: {0}")]
    Serialization(String),
    #[error("claim component set must not be empty")]
    ClaimComponentsEmpty,
    #[error("claim components must be unique")]
    ClaimComponentDuplicate,
    #[error("required claim component set must not be empty")]
    RequiredClaimComponentsEmpty,
    #[error("claim component belongs to a different revision")]
    ClaimComponentRevisionMismatch,
    #[error("verification objective set must not be empty")]
    ObjectivesEmpty,
    #[error("verification objectives must be unique")]
    ObjectiveDuplicate,
    #[error("verification contract does not bind the sealed revision/objective")]
    VerificationContractBindingMismatch,
    #[error("objective component set is not a non-empty revision subset")]
    ObjectiveComponentSubsetInvalid,
    #[error("proof path set must not be empty")]
    ProofPathsEmpty,
    #[error("proof paths must be unique")]
    ProofPathDuplicate,
    #[error("proof path members are empty or duplicate")]
    ProofPathMembersInvalid,
    #[error("proof path references an unknown objective")]
    PathObjectiveUnknown,
    #[error("proof path falsifier is invalid")]
    PathFalsifierInvalid,
    #[error("proof path lacks a required-component falsifier")]
    PathFalsifierMissing,
    #[error("HYPOTHESIS_VERIFICATION_PLAN_CLAIM_COMPONENT_UNCOVERED")]
    ClaimComponentUncovered,
    #[error("a sealed objective is not present in any proof path")]
    ObjectiveNotInProofPath,
    #[error("objective outcome set does not exactly match the plan")]
    ObjectiveOutcomeSetMismatch,
    #[error("component outcome lineage does not bind its component")]
    ComponentLineageMismatch,
    #[error("objective outcome does not bind the sealed plan objective")]
    ObjectiveOutcomeBindingMismatch,
    #[error("objective outcome predecessor chain is invalid")]
    ObjectiveOutcomePredecessorInvalid,
    #[error("objective outcome component set does not exactly match the objective")]
    ObjectiveOutcomeComponentSetMismatch,
    #[error("objective outcome kind does not match its component truth")]
    ObjectiveOutcomeTruthMismatch,
    #[error("objective terminal receipt lineage is incomplete")]
    ObjectiveReceiptLineageIncomplete,
    #[error("objective receipt references another objective or component set")]
    ObjectiveReceiptBindingMismatch,
    #[error("adjudication lineage does not match the proof-path reducer verdict")]
    AdjudicationLineageMismatch,
    #[error("a transition decision cannot be created for a nonterminal adjudication")]
    TerminalDecisionForNonTerminal,
    #[error("revision adjudication authority is incomplete")]
    RevisionAuthorityIncomplete,
    #[error("revision adjudication hash chain does not bind exactly")]
    RevisionAuthorityBindingMismatch,
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::hypothesis_semantic_key::ClaimPolarity;
    use crate::verification_contract::{
        CanonicalJsonObject, ContractCombinatorV1, PredicateComponentInputV1,
        VerificationContractBuildInputV1,
    };
    use proptest::prelude::*;

    fn hash(ch: char) -> String {
        format!("sha256:{}", ch.to_string().repeat(64))
    }

    fn contract(
        revision_id: Uuid,
        revision_hash: &str,
        objective_id: Uuid,
        semantic_key: &str,
        hash_char: char,
    ) -> VerificationContractV1 {
        VerificationContractV1::compile(VerificationContractBuildInputV1 {
            revision_id,
            revision_hash: revision_hash.to_owned(),
            objective_id,
            combinator: ContractCombinatorV1::AllOf,
            predicate_components: vec![PredicateComponentInputV1 {
                semantic_key: semantic_key.to_owned(),
                predicate_schema: "verification.test.v1".into(),
                predicate_version: 1,
                normalized_arguments: CanonicalJsonObject::parse("{\"value\":1}").unwrap(),
                expected_polarity: ClaimPolarity::Positive,
                prerequisite_hash: hash(hash_char),
            }],
            required_controls: Vec::new(),
            paired_differential_bindings: Vec::new(),
            ordered_steps: Vec::new(),
            stopping_criteria_hash: hash('d'),
            compiler_digest: hash('e'),
            rule_digest: hash('f'),
            policy_snapshot_hash: hash('0'),
        })
        .unwrap()
    }

    fn fixture_plan() -> HypothesisVerificationPlanV1 {
        let revision_id = Uuid::from_u128(1);
        let revision_hash = hash('1');
        let components = compile_claim_components_v1(
            revision_id,
            revision_hash.clone(),
            1,
            hash('2'),
            vec![
                HypothesisClaimComponentInputV1 {
                    component_key: "claim".into(),
                    kind: HypothesisClaimComponentKindV1::ClaimClause,
                    canonical_fragment_hash: hash('3'),
                    canonical_condition_hash: hash('4'),
                    required: true,
                },
                HypothesisClaimComponentInputV1 {
                    component_key: "identity".into(),
                    kind: HypothesisClaimComponentKindV1::IdentityCondition,
                    canonical_fragment_hash: hash('5'),
                    canonical_condition_hash: hash('6'),
                    required: true,
                },
            ],
        )
        .unwrap();
        let component_hashes = components
            .iter()
            .map(|component| component.member_hash.clone())
            .collect::<Vec<_>>();
        let objective_id = Uuid::from_u128(2);
        HypothesisVerificationPlanV1::compile(HypothesisVerificationPlanBuildInputV1 {
            revision_id,
            revision_hash: revision_hash.clone(),
            revision_ingredients_hash: hash('7'),
            required_claim_components: components,
            objectives: vec![HypothesisVerificationPlanObjectiveInputV1 {
                objective_hash: hash('8'),
                verification_contract: contract(
                    revision_id,
                    &revision_hash,
                    objective_id,
                    "fixture",
                    '9',
                ),
                claim_component_member_hashes: component_hashes.clone(),
                outcome_requirement:
                    HypothesisVerificationObjectiveOutcomeRequirementV1::SatisfyOrFalsifyBoundRequiredComponents,
            }],
            proof_paths: vec![HypothesisVerificationPlanPathInputV1 {
                path_key: "path-a".into(),
                members: vec![HypothesisVerificationPlanPathMemberInputV1 {
                    objective_id,
                    role: HypothesisVerificationPlanPathMemberRoleV1::RequiredProofAndPathFalsifier,
                    falsifier_claim_component_member_hashes: vec![component_hashes[0].clone()],
                }],
            }],
            outer_aggregation_policy_version: 1,
            outer_aggregation_policy_digest: hash('b'),
        })
        .unwrap()
    }

    fn two_path_plan_input(
        reverse_components: bool,
        reverse_objectives: bool,
        reverse_paths: bool,
    ) -> HypothesisVerificationPlanBuildInputV1 {
        let revision_id = Uuid::from_u128(101);
        let revision_hash = hash('1');
        let mut component_inputs = vec![
            HypothesisClaimComponentInputV1 {
                component_key: "claim".into(),
                kind: HypothesisClaimComponentKindV1::ClaimClause,
                canonical_fragment_hash: hash('2'),
                canonical_condition_hash: hash('3'),
                required: true,
            },
            HypothesisClaimComponentInputV1 {
                component_key: "identity".into(),
                kind: HypothesisClaimComponentKindV1::IdentityCondition,
                canonical_fragment_hash: hash('4'),
                canonical_condition_hash: hash('5'),
                required: true,
            },
        ];
        if reverse_components {
            component_inputs.reverse();
        }
        let components = compile_claim_components_v1(
            revision_id,
            revision_hash.clone(),
            1,
            hash('6'),
            component_inputs,
        )
        .unwrap();
        let component_hashes = components
            .iter()
            .map(|component| component.member_hash.clone())
            .collect::<Vec<_>>();
        let objective_a = Uuid::from_u128(102);
        let objective_b = Uuid::from_u128(103);
        let mut objectives = vec![
            HypothesisVerificationPlanObjectiveInputV1 {
                objective_hash: hash('7'),
                verification_contract: contract(
                    revision_id,
                    &revision_hash,
                    objective_a,
                    "path-a",
                    '8',
                ),
                claim_component_member_hashes: component_hashes.clone(),
                outcome_requirement:
                    HypothesisVerificationObjectiveOutcomeRequirementV1::SatisfyOrFalsifyBoundRequiredComponents,
            },
            HypothesisVerificationPlanObjectiveInputV1 {
                objective_hash: hash('a'),
                verification_contract: contract(
                    revision_id,
                    &revision_hash,
                    objective_b,
                    "path-b",
                    'b',
                ),
                claim_component_member_hashes: component_hashes.clone(),
                outcome_requirement:
                    HypothesisVerificationObjectiveOutcomeRequirementV1::SatisfyOrFalsifyBoundRequiredComponents,
            },
        ];
        if reverse_objectives {
            objectives.reverse();
        }
        let mut proof_paths = vec![
            HypothesisVerificationPlanPathInputV1 {
                path_key: "path-a".into(),
                members: vec![HypothesisVerificationPlanPathMemberInputV1 {
                    objective_id: objective_a,
                    role: HypothesisVerificationPlanPathMemberRoleV1::RequiredProofAndPathFalsifier,
                    falsifier_claim_component_member_hashes: vec![component_hashes[0].clone()],
                }],
            },
            HypothesisVerificationPlanPathInputV1 {
                path_key: "path-b".into(),
                members: vec![HypothesisVerificationPlanPathMemberInputV1 {
                    objective_id: objective_b,
                    role: HypothesisVerificationPlanPathMemberRoleV1::RequiredProofAndPathFalsifier,
                    falsifier_claim_component_member_hashes: vec![component_hashes[1].clone()],
                }],
            },
        ];
        if reverse_paths {
            proof_paths.reverse();
        }
        HypothesisVerificationPlanBuildInputV1 {
            revision_id,
            revision_hash,
            revision_ingredients_hash: hash('d'),
            required_claim_components: components,
            objectives,
            proof_paths,
            outer_aggregation_policy_version: 1,
            outer_aggregation_policy_digest: hash('e'),
        }
    }

    fn outcome_view(
        objective: &HypothesisVerificationPlanObjectiveV1,
        component_kinds: [HypothesisClaimComponentOutcomeKindV1; 2],
        outcome: HypothesisVerificationObjectiveOutcomeKindV1,
        hash_char: char,
    ) -> ObjectiveOutcomeViewV1 {
        ObjectiveOutcomeViewV1 {
            plan_objective_member_hash: objective.member_hash.clone(),
            component_outcomes: objective
                .claim_component_member_hashes
                .iter()
                .cloned()
                .zip(component_kinds)
                .collect(),
            outcome,
            outcome_hash: hash(hash_char),
        }
    }

    #[test]
    fn hypothesis_claim_component_and_verification_plan_hashes_are_stable() {
        assert_eq!(fixture_plan().plan_hash(), fixture_plan().plan_hash());
    }

    proptest! {
        #[test]
        fn hypothesis_verification_plan_property_permutations_are_stable(
            reverse_components in any::<bool>(),
            reverse_objectives in any::<bool>(),
            reverse_paths in any::<bool>(),
        ) {
            let expected = HypothesisVerificationPlanV1::compile(
                two_path_plan_input(false, false, false),
            ).unwrap();
            let actual = HypothesisVerificationPlanV1::compile(two_path_plan_input(
                reverse_components,
                reverse_objectives,
                reverse_paths,
            )).unwrap();
            prop_assert_eq!(actual.plan_hash(), expected.plan_hash());
            prop_assert_eq!(actual.plan_id(), expected.plan_id());
        }
    }

    #[test]
    fn hypothesis_verification_plan_outer_truth_table_tracks_live_paths_exactly() {
        let plan = HypothesisVerificationPlanV1::compile(two_path_plan_input(false, false, false))
            .unwrap();
        let winning = vec![
            outcome_view(
                &plan.objectives[0],
                [
                    HypothesisClaimComponentOutcomeKindV1::Satisfied,
                    HypothesisClaimComponentOutcomeKindV1::Satisfied,
                ],
                HypothesisVerificationObjectiveOutcomeKindV1::Satisfied,
                '1',
            ),
            outcome_view(
                &plan.objectives[1],
                [
                    HypothesisClaimComponentOutcomeKindV1::Unassigned,
                    HypothesisClaimComponentOutcomeKindV1::Blocked,
                ],
                HypothesisVerificationObjectiveOutcomeKindV1::Blocked,
                '2',
            ),
        ];
        let aggregate = reduce_verification_plan_v1(&plan, &winning).unwrap();
        assert_eq!(
            aggregate.verdict(),
            HypothesisRevisionAdjudicationVerdictV1::Verified
        );
        assert!(aggregate.unresolved().is_empty());
        assert_eq!(aggregate.non_decisive_limitations().len(), 2);

        let refuted = vec![
            outcome_view(
                &plan.objectives[0],
                [
                    HypothesisClaimComponentOutcomeKindV1::Refuted,
                    HypothesisClaimComponentOutcomeKindV1::Unassigned,
                ],
                HypothesisVerificationObjectiveOutcomeKindV1::Refuted,
                '3',
            ),
            outcome_view(
                &plan.objectives[1],
                [
                    HypothesisClaimComponentOutcomeKindV1::Blocked,
                    HypothesisClaimComponentOutcomeKindV1::Refuted,
                ],
                HypothesisVerificationObjectiveOutcomeKindV1::Refuted,
                '4',
            ),
        ];
        let aggregate = reduce_verification_plan_v1(&plan, &refuted).unwrap();
        assert_eq!(
            aggregate.verdict(),
            HypothesisRevisionAdjudicationVerdictV1::Refuted
        );
        assert!(aggregate.unresolved().is_empty());
        assert_eq!(aggregate.non_decisive_limitations().len(), 2);

        let nonterminal = vec![
            outcome_view(
                &plan.objectives[0],
                [
                    HypothesisClaimComponentOutcomeKindV1::Refuted,
                    HypothesisClaimComponentOutcomeKindV1::Unassigned,
                ],
                HypothesisVerificationObjectiveOutcomeKindV1::Refuted,
                '5',
            ),
            outcome_view(
                &plan.objectives[1],
                [
                    HypothesisClaimComponentOutcomeKindV1::Satisfied,
                    HypothesisClaimComponentOutcomeKindV1::Blocked,
                ],
                HypothesisVerificationObjectiveOutcomeKindV1::Blocked,
                '6',
            ),
        ];
        let aggregate = reduce_verification_plan_v1(&plan, &nonterminal).unwrap();
        assert_eq!(
            aggregate.verdict(),
            HypothesisRevisionAdjudicationVerdictV1::NonTerminal
        );
        assert_eq!(aggregate.unresolved().len(), 1);
        assert_eq!(aggregate.non_decisive_limitations().len(), 1);
        assert_eq!(
            aggregate.reason(),
            Some(HypothesisRevisionNonTerminalReasonV1::Blocked)
        );
    }

    #[test]
    fn hypothesis_verification_plan_rejects_incomplete_outcome_component_set() {
        let plan = HypothesisVerificationPlanV1::compile(two_path_plan_input(false, false, false))
            .unwrap();
        let mut first = outcome_view(
            &plan.objectives[0],
            [
                HypothesisClaimComponentOutcomeKindV1::Satisfied,
                HypothesisClaimComponentOutcomeKindV1::Satisfied,
            ],
            HypothesisVerificationObjectiveOutcomeKindV1::Satisfied,
            '7',
        );
        first.component_outcomes.pop_first();
        let second = outcome_view(
            &plan.objectives[1],
            [
                HypothesisClaimComponentOutcomeKindV1::Unassigned,
                HypothesisClaimComponentOutcomeKindV1::Unassigned,
            ],
            HypothesisVerificationObjectiveOutcomeKindV1::Unassigned,
            '8',
        );
        assert!(matches!(
            reduce_verification_plan_v1(&plan, &[first, second]),
            Err(HypothesisVerificationError::ObjectiveOutcomeComponentSetMismatch)
        ));
    }

    #[test]
    fn hypothesis_verification_plan_rejects_contract_substitution() {
        let mut input = two_path_plan_input(false, false, false);
        let objective_id = input.objectives[0].verification_contract.objective_id();
        input.objectives[0].verification_contract = contract(
            Uuid::from_u128(999),
            &hash('9'),
            objective_id,
            "stale-revision",
            '8',
        );
        assert!(matches!(
            HypothesisVerificationPlanV1::compile(input),
            Err(HypothesisVerificationError::VerificationContractBindingMismatch)
        ));
    }

    #[test]
    fn hypothesis_revision_adjudication_uses_path_truth_table() {
        let plan = fixture_plan();
        let objective = &plan.objectives[0];
        let satisfied = ObjectiveOutcomeViewV1 {
            plan_objective_member_hash: objective.member_hash.clone(),
            component_outcomes: objective
                .claim_component_member_hashes
                .iter()
                .map(|hash| {
                    (
                        hash.clone(),
                        HypothesisClaimComponentOutcomeKindV1::Satisfied,
                    )
                })
                .collect(),
            outcome: HypothesisVerificationObjectiveOutcomeKindV1::Satisfied,
            outcome_hash: hash('c'),
        };
        assert_eq!(
            reduce_verification_plan_v1(&plan, &[satisfied])
                .unwrap()
                .verdict(),
            HypothesisRevisionAdjudicationVerdictV1::Verified
        );

        let mut component_outcomes = objective
            .claim_component_member_hashes
            .iter()
            .map(|hash| {
                (
                    hash.clone(),
                    HypothesisClaimComponentOutcomeKindV1::Unassigned,
                )
            })
            .collect::<BTreeMap<_, _>>();
        component_outcomes.insert(
            plan.proof_paths[0].members[0].falsifier_claim_component_member_hashes[0].clone(),
            HypothesisClaimComponentOutcomeKindV1::Refuted,
        );
        let refuted = ObjectiveOutcomeViewV1 {
            plan_objective_member_hash: objective.member_hash.clone(),
            component_outcomes,
            outcome: HypothesisVerificationObjectiveOutcomeKindV1::Refuted,
            outcome_hash: hash('d'),
        };
        assert_eq!(
            reduce_verification_plan_v1(&plan, &[refuted])
                .unwrap()
                .verdict(),
            HypothesisRevisionAdjudicationVerdictV1::Refuted
        );
    }

    #[test]
    fn hypothesis_revision_adjudication_hash_chain_is_acyclic_and_replay_stable() {
        let plan = fixture_plan();
        let objective = &plan.objectives[0];
        let authority_binding = PersistedAllFreshToolTruthAuthorityBindingV1::from_server_bundle(
            Uuid::from_u128(50),
            hash('1'),
            hash('2'),
            hash('3'),
            hash('4'),
            hash('5'),
            hash('6'),
            hash('7'),
            hash('8'),
            hash('9'),
            hash('a'),
            Utc::now() + chrono::Duration::minutes(5),
        )
        .unwrap();
        let component_outcomes = objective
            .claim_component_member_hashes
            .iter()
            .enumerate()
            .map(|(index, component)| {
                let proof = HypothesisComponentProofRefV1::from_server_receipt(
                    HypothesisComponentProofRefInputV1 {
                        claim_component_member_hash: component.clone(),
                        predicate_component_member_hash: hash('b'),
                        oracle_receipt_id: Uuid::from_u128(60 + index as u128),
                        oracle_receipt_hash: hash('c'),
                        coverage_receipt_hash: hash('d'),
                        fact_delta_consumption_set_hash: hash('e'),
                    },
                )
                .unwrap();
                HypothesisClaimComponentOutcomeV1::satisfied(component.clone(), vec![proof])
                    .unwrap()
            })
            .collect::<Vec<_>>();
        let campaign = CampaignTerminalReceiptRefV1::from_server_receipt(
            Uuid::from_u128(70),
            1,
            hash('f'),
            objective.member_hash.clone(),
            objective.claim_component_member_hashes.clone(),
            authority_binding.binding_hash().to_owned(),
        )
        .unwrap();
        let oracle = OracleCensusReceiptRefV1::from_server_receipt(
            Uuid::from_u128(71),
            1,
            hash('1'),
            objective.member_hash.clone(),
            objective.claim_component_member_hashes.clone(),
            hash('2'),
            authority_binding.binding_hash().to_owned(),
        )
        .unwrap();
        let coverage = CampaignCoverageReceiptRefV1::from_server_receipt(
            Uuid::from_u128(72),
            hash('3'),
            objective.member_hash.clone(),
            objective.claim_component_set_hash.clone(),
            hash('4'),
        )
        .unwrap();
        let delta = FactDeltaConsumptionReceiptRefV1::from_server_receipt(
            Uuid::from_u128(73),
            hash('5'),
            objective.member_hash.clone(),
            objective.claim_component_set_hash.clone(),
            hash('6'),
        )
        .unwrap();
        let outcome = HypothesisVerificationObjectiveOutcomeV1::compile(
            objective,
            HypothesisVerificationObjectiveOutcomeBuildInputV1 {
                outcome_receipt_id: Uuid::from_u128(74),
                outcome_receipt_version: 1,
                outcome_ordinal: 0,
                predecessor_outcome_receipt_id: None,
                predecessor_outcome_receipt_hash: None,
                campaign_head_hash: hash('7'),
                plan_objective_member_hash: objective.member_hash.clone(),
                verification_contract_hash: objective.verification_contract_hash.clone(),
                claim_component_outcomes: component_outcomes,
                outcome: HypothesisVerificationObjectiveOutcomeKindV1::Satisfied,
                campaign_terminal_receipts: vec![campaign],
                oracle_census_receipts: vec![oracle],
                coverage_receipts: vec![coverage],
                fact_delta_consumption_receipts: vec![delta],
                unassigned_residual_risk_set_hash: hash('8'),
            },
        )
        .unwrap();
        let adjudication = HypothesisRevisionAdjudication::compile(
            Uuid::from_u128(75),
            &plan,
            vec![outcome],
            authority_binding,
            HypothesisRevisionAdjudicationLineageInputV1::Verified {
                finding_id: Uuid::from_u128(76),
                finding_lineage_hash: hash('9'),
            },
        )
        .unwrap();
        let decision =
            HypothesisRevisionTransitionDecisionV1::compile(Uuid::from_u128(77), &adjudication)
                .unwrap();
        let receipt = HypothesisRevisionTransitionReceiptV1::compile(
            Uuid::from_u128(78),
            &decision,
            Uuid::from_u128(79),
            hash('a'),
            hash('b'),
        )
        .unwrap();
        assert!(HypothesisRevisionAdjudicationAuthorityV1::compile(
            plan,
            adjudication,
            decision,
            receipt.clone(),
        )
        .is_ok());
        assert!(receipt.receipt_hash().starts_with("sha256:"));
    }
}
