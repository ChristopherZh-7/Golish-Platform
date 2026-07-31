//! Atomic application boundary for a Candidate Registry Gate pass.
//!
//! The model-facing runtime can persist analysis artifacts, but it cannot
//! construct [`CandidateGateSnapshot`]. Only the Pg adapter below may build
//! the opaque value from freshly revalidated, locked repository authority.

use std::collections::{BTreeMap, BTreeSet};
use std::sync::Arc;

use async_trait::async_trait;
use golish_agent_kit::db_traits::*;
use golish_agent_kit::harness::hypothesis_registry::{
    compile_claim_components, compile_verification_contract, compile_verification_plan,
    exact_set_hash, validate_candidate_gate, CandidateAttemptGateV1, CandidateAuthorityGateV1,
    CandidateAuthorityRootGateV1, CandidateAuthoritySnapshotDispositionV1,
    CandidateCompiledAuthorityV1, CandidateCoverageGateV1, CandidateCoverageOutcomeV1,
    CandidateCoverageSynthesisNodeKindV1, CandidateCoverageSynthesisNodeV1,
    CandidateExactSetSealV1, CandidateGatePass, CandidateGateSnapshot, CandidateHypothesisMutation,
    CandidateKnowledgeFeedGateV1, CandidateKnowledgeFeedMemberV1, CandidateReadGateV1,
    CandidateRepositoryGateHashesV1, ClaimComponentCompilerInput, FrozenCandidateGateMaterialV1,
    InputHypothesisRelationDecision, InputHypothesisRelationKindV1 as GateRelationKind,
    InputProcessingDispositionDecision, InputProcessingDispositionV1 as GateDisposition,
    PredicateRegistryEntry, PriorCandidateAttemptV1, RevisionSourceRef,
    StructuredClaimComponentSourceV1, VerificationContractCompilerInput,
    VerificationPlanCompilerInput,
};
use golish_agent_kit::task_orchestrator::hypothesis_analysis::{
    CandidateControllerDecisionArtifact, CandidateControllerDecisionKind,
    CandidateControllerFinalInput,
};
use golish_core::hypothesis_semantic_key::ClaimPolarity;
use golish_core::hypothesis_verification::{
    HypothesisClaimComponentV1, HypothesisVerificationObjectiveOutcomeRequirementV1,
    HypothesisVerificationPlanBuildInputV1, HypothesisVerificationPlanObjectiveInputV1,
    HypothesisVerificationPlanPathInputV1, HypothesisVerificationPlanPathMemberInputV1,
    HypothesisVerificationPlanPathMemberRoleV1, HypothesisVerificationPlanV1,
};
use golish_core::verification_contract::{
    CanonicalJsonObject, ContractCombinatorV1, VerificationContractV1,
};
use serde_json::Value;
use sqlx::PgPool;
use uuid::Uuid;

#[derive(Debug, Clone)]
pub struct CompiledCandidateHostRecipe {
    pub mutations: Vec<CandidateHypothesisMutation>,
    pub mutation_routes: BTreeMap<Uuid, CandidateRegistryMutationDecisionV1>,
    pub claim_components: Vec<HypothesisClaimComponentV1>,
    pub verification_contracts: Vec<VerificationContractV1>,
    pub verification_plans: Vec<HypothesisVerificationPlanV1>,
    pub mutation_set_hash: String,
    pub claim_component_set_hash: String,
    pub verification_contract_set_hash: String,
    pub verification_plan_set_hash: String,
    pub generation_transition_set_hash: String,
}

fn recipe_text<'a>(value: &'a Value, name: &str) -> Result<&'a str, HypothesisRegistryError> {
    value
        .get(name)
        .and_then(Value::as_str)
        .filter(|value| !value.trim().is_empty())
        .ok_or_else(|| {
            HypothesisRegistryError::AuthorityMismatch(format!(
                "candidate compiler recipe missing {name}"
            ))
        })
}

fn recipe_uuid(value: &Value, name: &str) -> Result<Uuid, HypothesisRegistryError> {
    Uuid::parse_str(recipe_text(value, name)?).map_err(|_| {
        HypothesisRegistryError::AuthorityMismatch(format!(
            "candidate compiler recipe has invalid {name}"
        ))
    })
}

fn validate_server_owned_route_uniqueness(items: &[Value]) -> Result<(), HypothesisRegistryError> {
    let mut semantic_routes = BTreeSet::new();
    for item in items {
        let route = item.get("route").ok_or_else(|| {
            HypothesisRegistryError::AuthorityMismatch(
                "candidate compiler route missing".to_owned(),
            )
        })?;
        let key = (
            recipe_text(item, "semantic_key_hash")?.to_owned(),
            recipe_uuid(route, "root_id")?,
        );
        if !semantic_routes.insert(key) {
            return Err(HypothesisRegistryError::AuthorityMismatch(
                "candidate compiler duplicate semantic route is not closed".to_owned(),
            ));
        }
    }
    Ok(())
}

pub(crate) fn validate_controller_decision_route_confirmation(
    artifact: &CandidateControllerDecisionArtifact,
    routes: &BTreeMap<Uuid, CandidateRegistryMutationDecisionV1>,
) -> Result<(), HypothesisRegistryError> {
    let invalid = |detail: &'static str| {
        HypothesisRegistryError::AuthorityMismatch(format!(
            "HYPOTHESIS_CANDIDATE_CONTROLLER_DECISION_INVALID: {detail}"
        ))
    };
    if artifact.decisions.len() != routes.len() {
        return Err(invalid("proposal exact set is open or drifted"));
    }
    let mut seen = BTreeSet::new();
    for decision in &artifact.decisions {
        if !seen.insert(decision.proposal_id) {
            return Err(invalid("duplicate proposal decision"));
        }
        let route = routes
            .get(&decision.proposal_id)
            .ok_or_else(|| invalid("unknown proposal decision"))?;
        let mut related = BTreeSet::new();
        for related_id in &decision.related_proposal_ids {
            if *related_id == decision.proposal_id
                || !routes.contains_key(related_id)
                || !related.insert(*related_id)
            {
                return Err(invalid("related proposal set is invalid"));
            }
        }
        if !decision.related_proposal_ids.is_empty() || decision.rationale.trim().is_empty() {
            return Err(invalid(
                "implemented route confirmation must be unambiguous",
            ));
        }
        match (&decision.decision, route) {
            (
                CandidateControllerDecisionKind::Accept,
                CandidateRegistryMutationDecisionV1::CreateInitial { .. },
            )
            | (
                CandidateControllerDecisionKind::AttachExisting,
                CandidateRegistryMutationDecisionV1::AttachCurrent { .. },
            ) => {}
            _ => return Err(invalid("decision kind does not confirm compiled route")),
        }
    }
    if seen.len() != routes.len() {
        return Err(invalid("proposal exact set is open or drifted"));
    }
    Ok(())
}

pub(crate) fn validate_controller_proposal_pages(
    input: &CandidateControllerFinalInput,
    routes: &BTreeMap<Uuid, CandidateRegistryMutationDecisionV1>,
) -> Result<(), HypothesisRegistryError> {
    let invalid = |detail: &'static str| {
        HypothesisRegistryError::AuthorityMismatch(format!(
            "HYPOTHESIS_CANDIDATE_CONTROLLER_PROPOSAL_PAGE_INVALID: {detail}"
        ))
    };
    if input.proposal_pages.len() > 4
        || (routes.is_empty() != input.proposal_pages.is_empty())
        || !valid_sha256(&input.proposal_page_set_hash)
    {
        return Err(invalid("page count or set hash is invalid"));
    }
    let mut seen = BTreeSet::new();
    for (ordinal, page) in input.proposal_pages.iter().enumerate() {
        if page.page_ordinal != u32::try_from(ordinal).map_err(|_| invalid("page overflow"))?
            || page.proposals.is_empty()
            || page.proposals.len() > 16
            || page.proposal_count
                != u32::try_from(page.proposals.len())
                    .map_err(|_| invalid("proposal count overflow"))?
            || !valid_sha256(&page.page_hash)
        {
            return Err(invalid("page envelope is invalid"));
        }
        for proposal in &page.proposals {
            if !seen.insert(proposal.proposal_id)
                || !valid_sha256(&proposal.semantic_key_hash)
                || proposal.structured_claim.trim().is_empty()
                || proposal.trust_boundary.trim().is_empty()
                || !matches!(proposal.polarity.as_str(), "positive" | "negative")
            {
                return Err(invalid("proposal summary is invalid or duplicated"));
            }
            let route = routes
                .get(&proposal.proposal_id)
                .ok_or_else(|| invalid("proposal page contains an unknown route"))?;
            if !matches!(
                (proposal.route_kind.as_str(), route),
                (
                    "create_initial",
                    CandidateRegistryMutationDecisionV1::CreateInitial { .. }
                ) | (
                    "attach_current",
                    CandidateRegistryMutationDecisionV1::AttachCurrent { .. }
                )
            ) {
                return Err(invalid("proposal page route kind drifted"));
            }
        }
    }
    if seen != routes.keys().copied().collect() {
        return Err(invalid("proposal page exact set is open or drifted"));
    }
    Ok(())
}

fn valid_sha256(value: &str) -> bool {
    value.len() == 71
        && value.starts_with("sha256:")
        && value[7..]
            .bytes()
            .all(|byte| byte.is_ascii_digit() || (b'a'..=b'f').contains(&byte))
}

pub fn compile_candidate_host_recipe(
    recipe: &Value,
) -> Result<CompiledCandidateHostRecipe, HypothesisRegistryError> {
    let items = recipe
        .get("items")
        .and_then(Value::as_array)
        .ok_or_else(|| {
            HypothesisRegistryError::AuthorityMismatch(
                "candidate compiler recipe item set is invalid".to_owned(),
            )
        })?;
    validate_server_owned_route_uniqueness(items)?;
    let organization_id = recipe_uuid(recipe, "organization_id")?;
    let mut mutations = Vec::with_capacity(items.len());
    let mut routes = BTreeMap::new();
    let mut components = Vec::new();
    let mut contracts = Vec::new();
    let mut plans = Vec::new();
    for item in items {
        let proposal_id = recipe_uuid(item, "proposal_id")?;
        let route = item.get("route").ok_or_else(|| {
            HypothesisRegistryError::AuthorityMismatch(
                "candidate compiler route missing".to_owned(),
            )
        })?;
        let route_kind = recipe_text(route, "kind")?;
        let root_id = recipe_uuid(route, "root_id")?;
        let repository_route = match route_kind {
            "create_initial" => CandidateRegistryMutationDecisionV1::CreateInitial { root_id },
            "attach_current" => CandidateRegistryMutationDecisionV1::AttachCurrent {
                root_id,
                revision_id: recipe_uuid(route, "revision_id")?,
            },
            _ => {
                return Err(HypothesisRegistryError::AuthorityMismatch(
                    "candidate compiler route is not server-owned".to_owned(),
                ));
            }
        };
        routes.insert(proposal_id, repository_route);
        let mutation = CandidateHypothesisMutation::parse_controller_artifact(
            serde_json::json!({
                "proposal_id":proposal_id,
                "organization_id":organization_id,
                "semantic_key_hash":recipe_text(item,"semantic_key_hash")?,
                "operator_rank":0,
                "state":recipe_text(item,"state")?,
                "proof_refs":item.get("proof_refs").cloned().unwrap_or_else(||serde_json::json!([])),
                "refutation_refs":item.get("refutation_refs").cloned().unwrap_or_else(||serde_json::json!([])),
                "generation_transition_hash":recipe_text(item,"generation_transition_hash")?,
            }),
        )
        .map_err(|block| {
            HypothesisRegistryError::AuthorityMismatch(format!(
                "{}: {block}",
                block.code()
            ))
        })?;
        mutations.push(mutation);
        if route_kind == "attach_current" {
            continue;
        }
        let revision = item.get("revision").ok_or_else(|| {
            HypothesisRegistryError::AuthorityMismatch(
                "candidate compiler revision missing".to_owned(),
            )
        })?;
        let revision_id = recipe_uuid(revision, "revision_id")?;
        let revision_hash = recipe_text(revision, "revision_hash")?.to_owned();
        let source =
            |key: &'static str,
             hash_name: &'static str|
             -> Result<Vec<StructuredClaimComponentSourceV1>, HypothesisRegistryError> {
                Ok(vec![StructuredClaimComponentSourceV1 {
                    component_key: key.to_owned(),
                    canonical_fragment_hash: recipe_text(revision, hash_name)?.to_owned(),
                    canonical_condition_hash: recipe_text(revision, hash_name)?.to_owned(),
                    required: true,
                }])
            };
        let proposal_components = compile_claim_components(ClaimComponentCompilerInput {
            revision_id,
            revision_hash: revision_hash.clone(),
            derivation_contract_version: 1,
            derivation_contract_digest: recipe_text(revision, "derivation_digest")?.to_owned(),
            claim_clauses: source("claim_clause", "claim_clause_hash")?,
            impact_qualifiers: source("impact", "impact_hash")?,
            trust_boundary_conditions: source("trust_boundary", "trust_boundary_hash")?,
            identity_conditions: source("identity", "identity_hash")?,
        })
        .map_err(|error| HypothesisRegistryError::AuthorityMismatch(error.to_string()))?;
        let objective_id = recipe_uuid(revision, "objective_id")?;
        let predicate_version = item
            .get("predicate_version")
            .and_then(Value::as_u64)
            .and_then(|value| u32::try_from(value).ok())
            .ok_or_else(|| {
                HypothesisRegistryError::AuthorityMismatch(
                    "candidate predicate version invalid".to_owned(),
                )
            })?;
        let polarity = ClaimPolarity::try_from(recipe_text(item, "polarity")?)
            .map_err(|error| HypothesisRegistryError::AuthorityMismatch(error.to_string()))?;
        let contract = compile_verification_contract(VerificationContractCompilerInput {
            revision_id,
            revision_hash: revision_hash.clone(),
            objective_id,
            combinator: ContractCombinatorV1::AllOf,
            predicate_registry_entries: vec![PredicateRegistryEntry {
                semantic_key: recipe_text(item, "semantic_key_hash")?.to_owned(),
                predicate_schema: recipe_text(item, "predicate_schema")?.to_owned(),
                predicate_version,
                normalized_arguments: CanonicalJsonObject::try_from_value(
                    item.get("predicate_arguments")
                        .cloned()
                        .unwrap_or_else(|| serde_json::json!({})),
                )
                .map_err(|error| HypothesisRegistryError::AuthorityMismatch(error.to_string()))?,
                expected_polarity: polarity,
                prerequisite_hash: recipe_text(revision, "identity_hash")?.to_owned(),
            }],
            required_controls: Vec::new(),
            paired_differential_bindings: Vec::new(),
            ordered_steps: Vec::new(),
            stopping_criteria_hash: recipe_text(revision, "stopping_criteria_hash")?.to_owned(),
            compiler_digest: recipe_text(revision, "compiler_digest")?.to_owned(),
            rule_digest: recipe_text(revision, "rule_digest")?.to_owned(),
            policy_snapshot_hash: recipe_text(revision, "policy_snapshot_hash")?.to_owned(),
        })
        .map_err(|error| HypothesisRegistryError::AuthorityMismatch(error.to_string()))?;
        let component_hashes = proposal_components
            .iter()
            .map(|component| component.member_hash().to_owned())
            .collect::<Vec<_>>();
        let plan = compile_verification_plan(VerificationPlanCompilerInput(
            HypothesisVerificationPlanBuildInputV1 {
                revision_id,
                revision_hash,
                revision_ingredients_hash: recipe_text(revision, "revision_ingredients_hash")?
                    .to_owned(),
                required_claim_components: proposal_components.clone(),
                objectives: vec![HypothesisVerificationPlanObjectiveInputV1 {
                    objective_hash: recipe_text(revision, "objective_hash")?.to_owned(),
                    verification_contract: contract.clone(),
                    claim_component_member_hashes: component_hashes.clone(),
                    outcome_requirement:
                        HypothesisVerificationObjectiveOutcomeRequirementV1::SatisfyOrFalsifyBoundRequiredComponents,
                }],
                proof_paths: vec![HypothesisVerificationPlanPathInputV1 {
                    path_key: "candidate_primary_path".to_owned(),
                    members: vec![HypothesisVerificationPlanPathMemberInputV1 {
                        objective_id,
                        role: HypothesisVerificationPlanPathMemberRoleV1::RequiredProofAndPathFalsifier,
                        falsifier_claim_component_member_hashes: component_hashes,
                    }],
                }],
                outer_aggregation_policy_version: 1,
                outer_aggregation_policy_digest: recipe_text(revision, "outer_policy_digest")?
                    .to_owned(),
            },
        ))
        .map_err(|error| HypothesisRegistryError::AuthorityMismatch(error.to_string()))?;
        components.extend(proposal_components);
        contracts.push(contract);
        plans.push(plan);
    }
    if (!mutations.is_empty()
        && (components.is_empty() || contracts.is_empty() || plans.is_empty()))
        || (mutations.is_empty()
            && (!components.is_empty() || !contracts.is_empty() || !plans.is_empty()))
    {
        return Err(HypothesisRegistryError::AuthorityMismatch(
            "candidate compiler produced no new revision authority".to_owned(),
        ));
    }
    mutations.sort_by_key(|mutation| {
        (
            mutation.organization_id,
            mutation.semantic_key_hash.clone(),
            mutation.operator_rank,
            mutation.proposal_id,
        )
    });
    let mutation_hashes = mutations
        .iter()
        .map(|mutation| mutation.mutation_hash.clone())
        .collect::<Vec<_>>();
    let component_hashes = components
        .iter()
        .map(|component| component.member_hash().to_owned())
        .collect::<Vec<_>>();
    let contract_hashes = contracts
        .iter()
        .map(|contract| contract.contract_hash().to_owned())
        .collect::<Vec<_>>();
    let plan_hashes = plans
        .iter()
        .map(|plan| plan.plan_hash().to_owned())
        .collect::<Vec<_>>();
    let transition_hashes = mutations
        .iter()
        .map(|mutation| mutation.generation_transition_hash.clone())
        .collect::<Vec<_>>();
    Ok(CompiledCandidateHostRecipe {
        mutations,
        mutation_routes: routes,
        mutation_set_hash: exact_set_hash("candidate_mutations.v1", &mutation_hashes),
        claim_component_set_hash: exact_set_hash(
            "candidate_claim_components.v1",
            &component_hashes,
        ),
        verification_contract_set_hash: exact_set_hash("candidate_contracts.v1", &contract_hashes),
        verification_plan_set_hash: exact_set_hash("candidate_plans.v1", &plan_hashes),
        generation_transition_set_hash: exact_set_hash(
            "candidate_generation_transitions.v1",
            &transition_hashes,
        ),
        claim_components: components,
        verification_contracts: contracts,
        verification_plans: plans,
    })
}

#[async_trait]
pub trait CandidateGateSnapshotSource: Send + Sync {
    /// Reload complete, locked authority rows and construct the opaque Gate
    /// snapshot. Implementations must never deserialize this value from an
    /// agent artifact or accept caller-supplied authority fields.
    async fn load_candidate_gate_snapshot(
        &self,
        request: LoadCandidateGateMaterial,
    ) -> Result<CandidateGateSnapshot, HypothesisRegistryError>;
}

#[derive(Clone)]
pub struct PgCandidateGateSnapshotSource {
    _repository: Arc<dyn HypothesisRegistryRepository>,
    pool: Arc<PgPool>,
}

impl PgCandidateGateSnapshotSource {
    pub fn new(repository: Arc<dyn HypothesisRegistryRepository>, pool: Arc<PgPool>) -> Self {
        Self {
            _repository: repository,
            pool,
        }
    }
}

fn typed_material_error(detail: impl std::fmt::Display) -> HypothesisRegistryError {
    HypothesisRegistryError::AuthorityMismatch(format!(
        "candidate typed Gate material is unavailable or inconsistent: {detail}"
    ))
}

fn db_snapshot_view(
    value: golish_db::repo::candidate_analysis::CandidateSnapshotRowView,
) -> Result<CandidateAnalysisSnapshotView, HypothesisRegistryError> {
    let authority_roots = value
        .authority_roots
        .into_iter()
        .map(|root| {
            let semantic_status = match root.semantic_status.as_str() {
                "consistent" => CandidateSemanticAuthorityStatusV1::Consistent,
                "pending" => CandidateSemanticAuthorityStatusV1::Pending,
                "orphaned" => CandidateSemanticAuthorityStatusV1::Orphaned,
                "superseded" => CandidateSemanticAuthorityStatusV1::Superseded,
                other => {
                    return Err(typed_material_error(format!(
                        "unknown semantic authority status {other}"
                    )))
                }
            };
            Ok(CandidateToolTruthAuthorityRootViewV1 {
                ordinal: u32::try_from(root.ordinal).map_err(typed_material_error)?,
                root_family: root.root_family,
                root_denominator_id: root.root_denominator_id,
                root_denominator_hash: root.root_denominator_hash,
                authority_set_seal_id: root.authority_set_seal_id,
                authority_set_graph_hash: root.authority_set_graph_hash,
                authority_set_semantic_hash: root.authority_set_semantic_hash,
                authority_set_freshness_hash: root.authority_set_freshness_hash,
                temporal_validity_policy_set_hash: root.temporal_validity_policy_set_hash,
                temporal_validity_decision_set_hash: root.temporal_validity_decision_set_hash,
                target_state_epoch_set_hash: root.target_state_epoch_set_hash,
                receipt_count: u32::try_from(root.receipt_count).map_err(typed_material_error)?,
                receipt_set_hash: root.receipt_set_hash,
                semantic_status,
                temporal_status: root.temporal_status,
                temporal_policies: root.temporal_policies,
                member_hash: root.member_hash,
            })
        })
        .collect::<Result<Vec<_>, HypothesisRegistryError>>()?;
    let disposition = match value.disposition {
        golish_db::repo::candidate_analysis::CandidateSnapshotDispositionRow::SealedReady => {
            CandidateAnalysisSnapshotDispositionV1::SealedReady
        }
        golish_db::repo::candidate_analysis::CandidateSnapshotDispositionRow::BlockedAuthorityBundle => {
            CandidateAnalysisSnapshotDispositionV1::BlockedAuthorityBundle
        }
    };
    Ok(CandidateAnalysisSnapshotView {
        snapshot_id: value.snapshot_id,
        stable_consumer_request_id: value.stable_consumer_request_id,
        operation_id: value.operation_id,
        scope_snapshot_id: value.scope_snapshot_id,
        organization_id: value.organization_id,
        disposition,
        snapshot_hash: value.snapshot_hash,
        candidate_snapshot_authority_hash: value.candidate_snapshot_authority_hash,
        tool_truth_authority_bundle_seal_id: value.tool_truth_authority_bundle_seal_id,
        tool_truth_authority_root_count: u32::try_from(value.tool_truth_authority_root_count)
            .map_err(typed_material_error)?,
        tool_truth_authority_root_set_hash: value.tool_truth_authority_root_set_hash,
        tool_truth_authority_bundle_member_count: u32::try_from(
            value.tool_truth_authority_bundle_member_count,
        )
        .map_err(typed_material_error)?,
        tool_truth_authority_bundle_member_set_hash: value
            .tool_truth_authority_bundle_member_set_hash,
        tool_truth_authority_receipt_count: u32::try_from(value.tool_truth_authority_receipt_count)
            .map_err(typed_material_error)?,
        tool_truth_authority_receipt_set_hash: value.tool_truth_authority_receipt_set_hash,
        denominator_graph_bundle_hash: value.denominator_graph_bundle_hash,
        semantic_authority_bundle_hash: value.semantic_authority_bundle_hash,
        freshness_attestation_bundle_hash: value.freshness_attestation_bundle_hash,
        temporal_validity_bundle_hash: value.temporal_validity_bundle_hash,
        temporal_validity_policy_set_hash: value.temporal_validity_policy_set_hash,
        temporal_validity_decision_set_hash: value.temporal_validity_decision_set_hash,
        observation_window_hash: value.observation_window_hash,
        target_state_epoch_set_hash: value.target_state_epoch_set_hash,
        authority_roots,
        knowledge_feed_catalog_policy_seal_hash: value.knowledge_feed_catalog_policy_seal_hash,
        knowledge_feed_required_member_set_hash: value.knowledge_feed_required_member_set_hash,
        knowledge_feed_signature_algorithm_set_hash: value
            .knowledge_feed_signature_algorithm_set_hash,
        knowledge_feed_trust_store_hash: value.knowledge_feed_trust_store_hash,
        knowledge_feed_key_revocation_epoch_hash: value.knowledge_feed_key_revocation_epoch_hash,
        knowledge_feed_snapshot_set_hash: value.knowledge_feed_snapshot_set_hash,
        product_version_census_hash: value.product_version_census_hash,
        knowledge_feed_match_census_hash: value.knowledge_feed_match_census_hash,
        stale_revalidation_obligation_set_hash: value.stale_revalidation_obligation_set_hash,
        knowledge_feed_obligation_set_hash: value.knowledge_feed_obligation_set_hash,
        row_version: value.row_version,
        sealed_at: value.sealed_at,
    })
}

fn json_array<'a>(value: &'a Value, name: &str) -> Result<&'a [Value], HypothesisRegistryError> {
    value
        .as_array()
        .map(Vec::as_slice)
        .ok_or_else(|| typed_material_error(format!("{name} is not an array")))
}

fn json_uuid(value: &Value, name: &str) -> Result<Uuid, HypothesisRegistryError> {
    value
        .get(name)
        .and_then(Value::as_str)
        .and_then(|value| Uuid::parse_str(value).ok())
        .ok_or_else(|| typed_material_error(format!("{name} is not a UUID")))
}

fn json_hash(value: &Value, name: &str) -> Result<String, HypothesisRegistryError> {
    value
        .get(name)
        .and_then(Value::as_str)
        .filter(|value| value.starts_with("sha256:") && value.len() == 71)
        .map(ToOwned::to_owned)
        .ok_or_else(|| typed_material_error(format!("{name} is not a canonical hash")))
}

fn coverage_outcome(value: &str) -> Result<CandidateCoverageOutcomeV1, HypothesisRegistryError> {
    match value {
        "adequate" | "no_local_miss" | "no_composite_miss" => {
            Ok(CandidateCoverageOutcomeV1::Adequate)
        }
        "missed_hypothesis" => Ok(CandidateCoverageOutcomeV1::MissedHypothesis),
        "blocked" => Ok(CandidateCoverageOutcomeV1::Blocked),
        _ => Err(typed_material_error(format!(
            "unknown coverage outcome {value}"
        ))),
    }
}

fn synthesis_kind(
    value: &str,
) -> Result<CandidateCoverageSynthesisNodeKindV1, HypothesisRegistryError> {
    match value {
        "cross_chunk" => Ok(CandidateCoverageSynthesisNodeKindV1::CrossChunk),
        "cross_input_partition" => Ok(CandidateCoverageSynthesisNodeKindV1::CrossInputPartition),
        "cross_input_reduce" => Ok(CandidateCoverageSynthesisNodeKindV1::CrossInputReduce),
        "cross_dimension_reduce" => Ok(CandidateCoverageSynthesisNodeKindV1::CrossDimensionReduce),
        "global_semantic_root" => Ok(CandidateCoverageSynthesisNodeKindV1::GlobalSemanticRoot),
        _ => Err(typed_material_error(format!(
            "unknown synthesis node kind {value}"
        ))),
    }
}

#[async_trait]
impl CandidateGateSnapshotSource for PgCandidateGateSnapshotSource {
    async fn load_candidate_gate_snapshot(
        &self,
        request: LoadCandidateGateMaterial,
    ) -> Result<CandidateGateSnapshot, HypothesisRegistryError> {
        let mut tx = self.pool.begin().await.map_err(typed_material_error)?;
        sqlx::query("SET TRANSACTION ISOLATION LEVEL REPEATABLE READ")
            .execute(&mut *tx)
            .await
            .map_err(typed_material_error)?;
        let aggregate = golish_db::repo::candidate_analysis::load_candidate_pre_gate_material_on(
            &mut tx,
            golish_db::repo::candidate_analysis::LoadCandidateGateMaterialInput {
                operation_id: request.operation_id,
                scope_snapshot_id: request.scope_snapshot_id,
                organization_id: request.organization_id,
                snapshot_id: request.snapshot_id,
                analysis_attempt_id: request.analysis_attempt_id,
                analysis_attempt_ordinal: i32::try_from(request.analysis_attempt_ordinal)
                    .map_err(typed_material_error)?,
                expected_snapshot_row_version: request.expected_snapshot_row_version,
                expected_attempt_row_version: request.expected_attempt_row_version,
            },
        )
        .await
        .map_err(typed_material_error)?;
        let exact_closure = &aggregate.exact_closure;
        if !exact_closure.gate_eligible
            || exact_closure.proposal_census_hash != aggregate.proposal_census_hash
            || exact_closure.critic_census_hash.as_deref()
                != Some(aggregate.critic_census_hash.as_str())
            || exact_closure.coverage_subreview_census_set_hash
                != aggregate.coverage_subreview_census_set_hash
            || exact_closure.coverage_checklist_set_hash != aggregate.coverage_checklist_set_hash
        {
            return Err(typed_material_error(
                "canonical Candidate closure differs from aggregate authority",
            ));
        }
        let snapshot = db_snapshot_view(aggregate.snapshot.clone())?;
        let compiler_material: (Value, Value, Value) = sqlx::query_as(
            r#"SELECT compiler_recipe,input_dispositions,input_relations
                 FROM candidate_analysis_host_compilation_materials
                WHERE analysis_attempt_id=$1 AND snapshot_id=$2
                  AND operation_id=$3 AND organization_id=$4
                  AND final_submitter_worker_run_id=$5"#,
        )
        .bind(request.analysis_attempt_id)
        .bind(request.snapshot_id)
        .bind(request.operation_id)
        .bind(request.organization_id)
        .bind(aggregate.final_submitter_worker_run_id)
        .fetch_optional(&mut *tx)
        .await
        .map_err(typed_material_error)?
        .ok_or_else(|| typed_material_error("host compilation material is absent"))?;
        let compiled = compile_candidate_host_recipe(&compiler_material.0)?;
        let controller_body: Value = sqlx::query_scalar(
            r#"SELECT provider.artifact_body
                 FROM candidate_analysis_provider_attempts provider
                WHERE provider.analysis_attempt_id=$1
                  AND provider.worker_run_id=$2
                  AND provider.artifact_kind='controller_decision.v1'"#,
        )
        .bind(request.analysis_attempt_id)
        .bind(aggregate.final_submitter_worker_run_id)
        .fetch_optional(&mut *tx)
        .await
        .map_err(typed_material_error)?
        .ok_or_else(|| typed_material_error("final Controller decision is absent"))?;
        let controller_artifact: CandidateControllerDecisionArtifact =
            serde_json::from_value(controller_body).map_err(typed_material_error)?;
        validate_controller_decision_route_confirmation(
            &controller_artifact,
            &compiled.mutation_routes,
        )?;

        let root_member_hashes = snapshot
            .authority_roots
            .iter()
            .map(|root| root.member_hash.clone())
            .collect::<Vec<_>>();
        let receipt_hashes = snapshot
            .authority_roots
            .iter()
            .map(|root| root.receipt_set_hash.clone())
            .collect::<Vec<_>>();
        let temporal_hashes = snapshot
            .authority_roots
            .iter()
            .map(|root| root.temporal_validity_decision_set_hash.clone())
            .collect::<Vec<_>>();
        let authority_roots = snapshot
            .authority_roots
            .iter()
            .map(|root| CandidateAuthorityRootGateV1 {
                root_family: root.root_family,
                graph_hash: root.authority_set_graph_hash.clone(),
                semantic_hash: root.authority_set_semantic_hash.clone(),
                freshness_hash: root.authority_set_freshness_hash.clone(),
                temporal_hash: root.temporal_validity_decision_set_hash.clone(),
                target_state_epoch_hash: root.target_state_epoch_set_hash.clone(),
                temporal_status: root.temporal_status,
                member_hash: root.member_hash.clone(),
            })
            .collect();

        let feed_rows: Vec<(String, String)> = sqlx::query_as(
            r#"SELECT member_hash,disposition
                 FROM candidate_analysis_knowledge_feed_snapshot_members
                WHERE snapshot_id=$1 ORDER BY ordinal"#,
        )
        .bind(request.snapshot_id)
        .fetch_all(&mut *tx)
        .await
        .map_err(typed_material_error)?;
        let feed_hashes = feed_rows
            .iter()
            .map(|row| row.0.clone())
            .collect::<Vec<_>>();
        let feed_members = feed_rows
            .into_iter()
            .map(|(member_hash, disposition)| {
                let current = disposition == "current";
                CandidateKnowledgeFeedMemberV1 {
                    member_hash,
                    product_version_known: current,
                    signature_valid: current,
                    provenance_valid: current,
                    age_valid_at_gate: current,
                    key_current_and_not_revoked: current,
                }
            })
            .collect();
        let product_hashes: Vec<String> = sqlx::query_scalar(
            r#"SELECT member_hash FROM candidate_analysis_product_version_census_members
                WHERE snapshot_id=$1 ORDER BY ordinal"#,
        )
        .bind(request.snapshot_id)
        .fetch_all(&mut *tx)
        .await
        .map_err(typed_material_error)?;
        let match_hashes: Vec<String> = sqlx::query_scalar(
            r#"SELECT member_hash FROM candidate_analysis_feed_match_census_members
                WHERE snapshot_id=$1 ORDER BY ordinal"#,
        )
        .bind(request.snapshot_id)
        .fetch_all(&mut *tx)
        .await
        .map_err(typed_material_error)?;
        let signature_algorithms: Vec<String> = sqlx::query_scalar(
            r#"SELECT DISTINCT signature_algorithm
                 FROM candidate_analysis_knowledge_feed_snapshot_members
                WHERE snapshot_id=$1 AND signature_algorithm IS NOT NULL
                ORDER BY signature_algorithm"#,
        )
        .bind(request.snapshot_id)
        .fetch_all(&mut *tx)
        .await
        .map_err(typed_material_error)?;

        let prior_rows: Vec<(Uuid, i32, String)> = sqlx::query_as(
            r#"SELECT attempt.analysis_attempt_id,attempt.attempt_ordinal,event.event_hash
                 FROM candidate_analysis_attempts attempt
                 JOIN candidate_analysis_attempt_state_events event
                   ON event.analysis_attempt_id=attempt.analysis_attempt_id
                WHERE attempt.snapshot_id=$1 AND attempt.attempt_ordinal<$2
                  AND event.event_kind IN ('superseded_missed_hypothesis','sealed','blocked')
                ORDER BY attempt.attempt_ordinal"#,
        )
        .bind(request.snapshot_id)
        .bind(i32::try_from(request.analysis_attempt_ordinal).map_err(typed_material_error)?)
        .fetch_all(&mut *tx)
        .await
        .map_err(typed_material_error)?;
        let prior_hashes = prior_rows
            .iter()
            .map(|row| row.2.clone())
            .collect::<Vec<_>>();
        let prior_attempts = prior_rows
            .into_iter()
            .map(|(attempt_id, ordinal, member_hash)| {
                Ok(PriorCandidateAttemptV1 {
                    attempt_id,
                    ordinal: u32::try_from(ordinal).map_err(typed_material_error)?,
                    terminal: true,
                    member_hash,
                })
            })
            .collect::<Result<Vec<_>, HypothesisRegistryError>>()?;

        let input_hashes: Vec<String> = sqlx::query_scalar(
            "SELECT input_hash FROM candidate_analysis_snapshot_inputs WHERE snapshot_id=$1 ORDER BY stable_input_key",
        )
        .bind(request.snapshot_id)
        .fetch_all(&mut *tx)
        .await
        .map_err(typed_material_error)?;
        let chunk_hashes: Vec<String> = sqlx::query_scalar(
            "SELECT census_hash FROM candidate_analysis_input_chunk_censuses WHERE snapshot_id=$1 ORDER BY snapshot_input_id",
        )
        .bind(request.snapshot_id)
        .fetch_all(&mut *tx)
        .await
        .map_err(typed_material_error)?;
        let page_hashes: Vec<String> = sqlx::query_scalar(
            "SELECT page_hash FROM candidate_analysis_page_receipts WHERE analysis_attempt_id=$1 ORDER BY page_receipt_id",
        )
        .bind(request.analysis_attempt_id)
        .fetch_all(&mut *tx)
        .await
        .map_err(typed_material_error)?;
        let blocked_input_count: i64 = sqlx::query_scalar(
            r#"SELECT count(*) FROM candidate_analysis_snapshot_inputs
                WHERE snapshot_id=$1 AND server_chunking_disposition NOT IN ('complete','source_empty')"#,
        )
        .bind(request.snapshot_id)
        .fetch_one(&mut *tx)
        .await
        .map_err(typed_material_error)?;
        let context_truncated: bool = sqlx::query_scalar(
            r#"SELECT EXISTS(
                 SELECT 1 FROM candidate_analysis_hypothesis_coverage_subreviews
                  WHERE analysis_attempt_id=$1 AND context_truncated
                 UNION ALL
                 SELECT 1 FROM candidate_analysis_hypothesis_coverage_synthesis_reviews
                  WHERE analysis_attempt_id=$1 AND context_truncated)"#,
        )
        .bind(request.analysis_attempt_id)
        .fetch_one(&mut *tx)
        .await
        .map_err(typed_material_error)?;

        let proposal_hashes: Vec<String> = sqlx::query_scalar(
            r#"SELECT proposal_hash FROM candidate_analysis_proposal_census_members
                WHERE analysis_attempt_id=$1 ORDER BY ordinal"#,
        )
        .bind(request.analysis_attempt_id)
        .fetch_all(&mut *tx)
        .await
        .map_err(typed_material_error)?;
        let h1_disposition_hashes: Vec<String> = sqlx::query_scalar(
            r#"SELECT disposition_hash FROM candidate_analysis_input_proposal_dispositions
                WHERE analysis_attempt_id=$1 ORDER BY snapshot_input_id"#,
        )
        .bind(request.analysis_attempt_id)
        .fetch_all(&mut *tx)
        .await
        .map_err(typed_material_error)?;
        let checklist_hashes: Vec<String> = sqlx::query_scalar(
            r#"SELECT member_hash FROM candidate_analysis_hypothesis_coverage_checklist_members
                WHERE analysis_attempt_id=$1 ORDER BY snapshot_input_id,ordinal"#,
        )
        .bind(request.analysis_attempt_id)
        .fetch_all(&mut *tx)
        .await
        .map_err(typed_material_error)?;
        let partition_hashes: Vec<String> = sqlx::query_scalar(
            r#"SELECT partition_hash FROM candidate_analysis_hypothesis_coverage_chunk_partitions
                WHERE analysis_attempt_id=$1 ORDER BY snapshot_input_id,partition_ordinal"#,
        )
        .bind(request.analysis_attempt_id)
        .fetch_all(&mut *tx)
        .await
        .map_err(typed_material_error)?;
        let expected_subreview_hashes: Vec<String> = sqlx::query_scalar(
            r#"SELECT member_hash
                 FROM candidate_analysis_hypothesis_coverage_subreview_census_members
                WHERE analysis_attempt_id=$1 ORDER BY snapshot_input_id,checklist_ordinal,partition_ordinal"#,
        )
        .bind(request.analysis_attempt_id)
        .fetch_all(&mut *tx)
        .await
        .map_err(typed_material_error)?;
        let observed_subreview_hashes: Vec<String> = sqlx::query_scalar(
            r#"SELECT member.member_hash
                 FROM candidate_analysis_hypothesis_coverage_subreview_census_members member
                 JOIN candidate_analysis_hypothesis_coverage_subreviews review
                   ON review.subreview_census_member_id=member.subreview_census_member_id
                WHERE member.analysis_attempt_id=$1
                ORDER BY member.snapshot_input_id,member.checklist_ordinal,member.partition_ordinal"#,
        )
        .bind(request.analysis_attempt_id)
        .fetch_all(&mut *tx)
        .await
        .map_err(typed_material_error)?;
        let synthesis_nodes =
            golish_db::repo::candidate_analysis::load_recomputed_coverage_synthesis_gate_nodes_on(
                &mut tx,
                request.analysis_attempt_id,
            )
            .await
            .map_err(typed_material_error)?
            .into_iter()
            .map(|row| {
                Ok(CandidateCoverageSynthesisNodeV1 {
                    node_hash: row.node_hash,
                    node_kind: synthesis_kind(&row.node_kind)?,
                    expected_child_hashes: row.expected_child_hashes,
                    observed_child_hashes: row.observed_child_hashes,
                    worker_run_id: row.synthesis_worker_run_id,
                    primary_analyst_worker_run_ids: row.primary_analyst_worker_run_ids,
                    transitive_descendant_worker_run_ids: row.transitive_descendant_worker_run_ids,
                    outcome: coverage_outcome(&row.outcome)?,
                })
            })
            .collect::<Result<Vec<_>, HypothesisRegistryError>>()?;
        let synthesis_hashes = synthesis_nodes
            .iter()
            .map(|node| node.node_hash.clone())
            .collect::<Vec<_>>();
        let coverage_review_hashes: Vec<String> = sqlx::query_scalar(
            r#"SELECT review_hash FROM candidate_analysis_hypothesis_coverage_reviews
                WHERE analysis_attempt_id=$1 ORDER BY snapshot_input_id"#,
        )
        .bind(request.analysis_attempt_id)
        .fetch_all(&mut *tx)
        .await
        .map_err(typed_material_error)?;
        let (global_outcome, global_review_hash): (String, String) = sqlx::query_as(
            r#"SELECT outcome,review_hash FROM candidate_analysis_hypothesis_coverage_global_reviews
                WHERE analysis_attempt_id=$1"#,
        )
        .bind(request.analysis_attempt_id)
        .fetch_optional(&mut *tx)
        .await
        .map_err(typed_material_error)?
        .ok_or_else(|| typed_material_error("global coverage review is absent"))?;
        let blocked_checklist_count: i64 = sqlx::query_scalar(
            r#"SELECT count(*) FROM candidate_analysis_hypothesis_coverage_checklist_members
                WHERE analysis_attempt_id=$1 AND applicability_disposition LIKE 'blocked_%'"#,
        )
        .bind(request.analysis_attempt_id)
        .fetch_one(&mut *tx)
        .await
        .map_err(typed_material_error)?;
        let sampling_used: bool = sqlx::query_scalar(
            r#"SELECT EXISTS(
                 SELECT 1 FROM candidate_analysis_hypothesis_coverage_subreview_census_members
                  WHERE analysis_attempt_id=$1 AND disposition='sampling_omitted'
                 UNION ALL SELECT 1 FROM candidate_analysis_hypothesis_coverage_reviews
                  WHERE analysis_attempt_id=$1 AND review_mode<>'full')"#,
        )
        .bind(request.analysis_attempt_id)
        .fetch_one(&mut *tx)
        .await
        .map_err(typed_material_error)?;
        let missed_hypothesis: bool = sqlx::query_scalar(
            r#"SELECT EXISTS(
                 SELECT 1 FROM candidate_analysis_hypothesis_coverage_subreviews
                  WHERE analysis_attempt_id=$1 AND outcome='missed_hypothesis'
                 UNION ALL SELECT 1 FROM candidate_analysis_hypothesis_coverage_synthesis_reviews
                  WHERE analysis_attempt_id=$1 AND outcome='missed_hypothesis'
                 UNION ALL SELECT 1 FROM candidate_analysis_hypothesis_coverage_global_reviews
                  WHERE analysis_attempt_id=$1 AND outcome='missed_hypothesis'
                 UNION ALL SELECT 1 FROM candidate_analysis_hypothesis_coverage_reviews
                  WHERE analysis_attempt_id=$1 AND outcome='missed_hypothesis')"#,
        )
        .bind(request.analysis_attempt_id)
        .fetch_one(&mut *tx)
        .await
        .map_err(typed_material_error)?;
        let retry_limit: i32 = sqlx::query_scalar(
            "SELECT retry_limit FROM candidate_analysis_attempts WHERE analysis_attempt_id=$1",
        )
        .bind(request.analysis_attempt_id)
        .fetch_one(&mut *tx)
        .await
        .map_err(typed_material_error)?;
        let controller_hashes: Vec<String> = sqlx::query_scalar(
            r#"SELECT artifact_hash FROM candidate_analysis_artifacts
                WHERE analysis_attempt_id=$1 AND artifact_kind='controller_decision.v1'
                ORDER BY artifact_hash"#,
        )
        .bind(request.analysis_attempt_id)
        .fetch_all(&mut *tx)
        .await
        .map_err(typed_material_error)?;
        let critic_member_hashes: Vec<String> = sqlx::query_scalar(
            r#"SELECT member_hash FROM candidate_analysis_critic_census_members
                WHERE analysis_attempt_id=$1 ORDER BY ordinal"#,
        )
        .bind(request.analysis_attempt_id)
        .fetch_all(&mut *tx)
        .await
        .map_err(typed_material_error)?;

        let input_dispositions = json_array(&compiler_material.1, "input_dispositions")?
            .iter()
            .map(|value| {
                let disposition = match value.get("disposition").and_then(Value::as_str) {
                    Some("analyzed") => GateDisposition::Analyzed,
                    Some("not_security_relevant") => GateDisposition::NotSecurityRelevant,
                    Some("gap") => GateDisposition::Gap,
                    Some("blocked") => GateDisposition::Blocked,
                    other => {
                        return Err(typed_material_error(format!(
                            "unknown input disposition {other:?}"
                        )));
                    }
                };
                Ok(InputProcessingDispositionDecision {
                    input_id: json_uuid(value, "input_id")?,
                    disposition,
                    decision_hash: json_hash(value, "decision_hash")?,
                })
            })
            .collect::<Result<Vec<_>, HypothesisRegistryError>>()?;
        let input_relations = json_array(&compiler_material.2, "input_relations")?
            .iter()
            .map(|value| {
                let relation = match value.get("relation_kind").and_then(Value::as_str) {
                    Some("creates_hypothesis") => GateRelationKind::CreatesHypothesis,
                    Some("supports_existing") => GateRelationKind::SupportsExisting,
                    Some("contradicts_existing") => GateRelationKind::ContradictsExisting,
                    Some("qualifies_existing") => GateRelationKind::QualifiesExisting,
                    other => {
                        return Err(typed_material_error(format!(
                            "unknown input relation {other:?}"
                        )));
                    }
                };
                Ok(InputHypothesisRelationDecision {
                    input_id: json_uuid(value, "input_id")?,
                    hypothesis_root_id: json_uuid(value, "root_id")?,
                    relation,
                    decision_hash: json_hash(value, "decision_hash")?,
                })
            })
            .collect::<Result<Vec<_>, HypothesisRegistryError>>()?;
        let disposition_hashes = input_dispositions
            .iter()
            .map(|decision| decision.decision_hash.clone())
            .collect();
        let relation_hashes = input_relations
            .iter()
            .map(|decision| decision.decision_hash.clone())
            .collect();
        let mutation_hashes = compiled
            .mutations
            .iter()
            .map(|mutation| mutation.mutation_hash.clone())
            .collect::<Vec<_>>();
        let component_hashes = compiled
            .claim_components
            .iter()
            .map(|component| component.member_hash().to_owned())
            .collect::<Vec<_>>();
        let contract_hashes = compiled
            .verification_contracts
            .iter()
            .map(|contract| contract.contract_hash().to_owned())
            .collect::<Vec<_>>();
        let plan_hashes = compiled
            .verification_plans
            .iter()
            .map(|plan| plan.plan_hash().to_owned())
            .collect::<Vec<_>>();
        let transition_hashes = compiled
            .mutations
            .iter()
            .map(|mutation| mutation.generation_transition_hash.clone())
            .collect::<Vec<_>>();

        let gate_snapshot =
            CandidateGateSnapshot::from_repository_material(FrozenCandidateGateMaterialV1 {
                snapshot_id: request.snapshot_id,
                snapshot_hash: snapshot.snapshot_hash.clone(),
                candidate_snapshot_authority_hash: snapshot
                    .candidate_snapshot_authority_hash
                    .clone(),
                operation_id: request.operation_id,
                organization_id: request.organization_id,
                authority: CandidateAuthorityGateV1 {
                    disposition: CandidateAuthoritySnapshotDispositionV1::SealedReady,
                    bundle_seal_id: snapshot.tool_truth_authority_bundle_seal_id,
                    operation_id: request.operation_id,
                    organization_id: request.organization_id,
                    checked_request_id: snapshot.stable_consumer_request_id,
                    gate_request_id: request.analysis_attempt_id,
                    caller_filtered_or_reused_guard: false,
                    old_consistent_row_used: false,
                    root_set: CandidateExactSetSealV1::seal(
                        "candidate_roots.v1",
                        root_member_hashes.clone(),
                    ),
                    bundle_member_set: CandidateExactSetSealV1::seal(
                        "candidate_bundle_members.v1",
                        root_member_hashes,
                    ),
                    receipt_set: CandidateExactSetSealV1::seal(
                        "candidate_receipts.v1",
                        receipt_hashes,
                    ),
                    temporal_decision_set: CandidateExactSetSealV1::seal(
                        "candidate_temporal_decisions.v1",
                        temporal_hashes,
                    ),
                    roots: authority_roots,
                    current_target_state_epoch_set_hash: snapshot
                        .target_state_epoch_set_hash
                        .clone(),
                    snapshot_target_state_epoch_set_hash: snapshot
                        .target_state_epoch_set_hash
                        .clone(),
                    gate_temporal_reevaluation_hash: aggregate
                        .gate_temporal_reevaluation_hash
                        .clone(),
                },
                knowledge_feed: CandidateKnowledgeFeedGateV1 {
                    required_member_set: CandidateExactSetSealV1::seal(
                        "candidate_feed_required.v1",
                        feed_hashes.clone(),
                    ),
                    signed_snapshot_set: CandidateExactSetSealV1::seal(
                        "candidate_feed_snapshots.v1",
                        feed_hashes,
                    ),
                    product_version_census: CandidateExactSetSealV1::seal(
                        "candidate_products.v1",
                        product_hashes,
                    ),
                    match_census: CandidateExactSetSealV1::seal(
                        "candidate_matches.v1",
                        match_hashes,
                    ),
                    signature_algorithm_set: CandidateExactSetSealV1::seal(
                        "candidate_signature_algorithms.v1",
                        signature_algorithms,
                    ),
                    members: feed_members,
                    catalog_policy_seal_hash: snapshot
                        .knowledge_feed_catalog_policy_seal_hash
                        .clone(),
                    trust_store_hash: snapshot.knowledge_feed_trust_store_hash.clone(),
                    snapshot_trust_store_hash: snapshot.knowledge_feed_trust_store_hash.clone(),
                    key_revocation_epoch_hash: snapshot
                        .knowledge_feed_key_revocation_epoch_hash
                        .clone(),
                    snapshot_key_revocation_epoch_hash: snapshot
                        .knowledge_feed_key_revocation_epoch_hash
                        .clone(),
                    gate_reevaluation_hash: aggregate.gate_knowledge_feed_reevaluation_hash.clone(),
                    obligation_set_hash: snapshot.knowledge_feed_obligation_set_hash.clone(),
                },
                attempt: CandidateAttemptGateV1 {
                    active_attempt_id: request.analysis_attempt_id,
                    active_attempt_ordinal: request.analysis_attempt_ordinal,
                    active_attempt_unique: true,
                    prior_attempts,
                    prior_terminal_attempt_chain_hash: exact_set_hash(
                        "candidate_prior_terminal_attempt_chain.v1",
                        &prior_hashes,
                    ),
                    material_attempt_ids: vec![request.analysis_attempt_id],
                },
                read: CandidateReadGateV1 {
                    input_set: CandidateExactSetSealV1::seal("candidate_inputs.v1", input_hashes),
                    chunk_set: CandidateExactSetSealV1::seal("candidate_chunks.v1", chunk_hashes),
                    page_receipt_set: CandidateExactSetSealV1::seal(
                        "candidate_page_receipts.v1",
                        page_hashes.clone(),
                    ),
                    server_read_receipt_set: CandidateExactSetSealV1::seal(
                        "candidate_server_reads.v1",
                        page_hashes,
                    ),
                    source_bytes_complete: blocked_input_count == 0,
                    context_truncated,
                    caller_claimed_read_complete: false,
                },
                coverage: CandidateCoverageGateV1 {
                    h1_proposal_set: CandidateExactSetSealV1::seal(
                        "candidate_h1.v1",
                        proposal_hashes.clone(),
                    ),
                    per_input_h1_disposition_set: CandidateExactSetSealV1::seal(
                        "candidate_h1_dispositions.v1",
                        h1_disposition_hashes,
                    ),
                    checklist_member_set: CandidateExactSetSealV1::seal(
                        "candidate_checklist.v1",
                        checklist_hashes,
                    ),
                    chunk_partition_set: CandidateExactSetSealV1::seal(
                        "candidate_partitions.v1",
                        partition_hashes,
                    ),
                    expected_subreview_set: CandidateExactSetSealV1::seal(
                        "candidate_subreviews_expected.v1",
                        expected_subreview_hashes,
                    ),
                    observed_subreview_set: CandidateExactSetSealV1::seal(
                        "candidate_subreviews_observed.v1",
                        observed_subreview_hashes,
                    ),
                    synthesis_node_set: CandidateExactSetSealV1::seal(
                        "candidate_synthesis.v1",
                        synthesis_hashes,
                    ),
                    synthesis_nodes,
                    per_input_review_set: CandidateExactSetSealV1::seal(
                        "candidate_reviews.v1",
                        coverage_review_hashes,
                    ),
                    h2_proposal_set: CandidateExactSetSealV1::seal(
                        "candidate_h2.v1",
                        proposal_hashes.clone(),
                    ),
                    global_review_hash,
                    global_review_outcome: coverage_outcome(&global_outcome)?,
                    unresolved_feed_dependent_checklist_members: u32::try_from(
                        blocked_checklist_count,
                    )
                    .map_err(typed_material_error)?,
                    missed_hypothesis,
                    sampling_used,
                    retry_limit_reached: missed_hypothesis
                        && i32::try_from(request.analysis_attempt_ordinal)
                            .map_err(typed_material_error)?
                            >= retry_limit,
                },
                proposal_census: CandidateExactSetSealV1::seal(
                    "candidate_proposals.v1",
                    proposal_hashes,
                ),
                critic_census: CandidateExactSetSealV1::seal(
                    "candidate_critics.v1",
                    critic_member_hashes,
                ),
                controller_decision_set: CandidateExactSetSealV1::seal(
                    "candidate_controller_decisions.v1",
                    controller_hashes,
                ),
                mutations: compiled.mutations,
                mutation_set: CandidateExactSetSealV1::seal(
                    "candidate_mutations.v1",
                    mutation_hashes,
                ),
                compiled: CandidateCompiledAuthorityV1 {
                    claim_components: compiled.claim_components,
                    claim_component_set: CandidateExactSetSealV1::seal(
                        "candidate_claim_components.v1",
                        component_hashes,
                    ),
                    verification_contracts: compiled.verification_contracts,
                    verification_contract_set: CandidateExactSetSealV1::seal(
                        "candidate_contracts.v1",
                        contract_hashes,
                    ),
                    verification_plans: compiled.verification_plans,
                    verification_plan_set: CandidateExactSetSealV1::seal(
                        "candidate_plans.v1",
                        plan_hashes,
                    ),
                },
                repository_hashes: CandidateRepositoryGateHashesV1 {
                    tool_truth_authority_root_set_hash: snapshot
                        .tool_truth_authority_root_set_hash
                        .clone(),
                    tool_truth_authority_bundle_member_set_hash: snapshot
                        .tool_truth_authority_bundle_member_set_hash
                        .clone(),
                    tool_truth_authority_receipt_set_hash: snapshot
                        .tool_truth_authority_receipt_set_hash
                        .clone(),
                    denominator_graph_bundle_hash: snapshot.denominator_graph_bundle_hash.clone(),
                    semantic_authority_bundle_hash: snapshot.semantic_authority_bundle_hash.clone(),
                    freshness_attestation_bundle_hash: snapshot
                        .freshness_attestation_bundle_hash
                        .clone(),
                    temporal_validity_bundle_hash: snapshot.temporal_validity_bundle_hash.clone(),
                    temporal_validity_policy_digest: snapshot
                        .temporal_validity_policy_set_hash
                        .clone(),
                    temporal_validity_decision_set_hash: snapshot
                        .temporal_validity_decision_set_hash
                        .clone(),
                    knowledge_feed_catalog_policy_seal_hash: snapshot
                        .knowledge_feed_catalog_policy_seal_hash
                        .clone(),
                    knowledge_feed_required_member_set_hash: snapshot
                        .knowledge_feed_required_member_set_hash
                        .clone(),
                    knowledge_feed_signature_algorithm_set_hash: snapshot
                        .knowledge_feed_signature_algorithm_set_hash
                        .clone(),
                    knowledge_feed_trust_store_hash: snapshot
                        .knowledge_feed_trust_store_hash
                        .clone(),
                    knowledge_feed_key_revocation_epoch_hash: snapshot
                        .knowledge_feed_key_revocation_epoch_hash
                        .clone(),
                    knowledge_feed_snapshot_set_hash: snapshot
                        .knowledge_feed_snapshot_set_hash
                        .clone(),
                    product_version_census_hash: snapshot.product_version_census_hash.clone(),
                    knowledge_feed_match_census_hash: snapshot
                        .knowledge_feed_match_census_hash
                        .clone(),
                    stale_revalidation_obligation_set_hash: snapshot
                        .stale_revalidation_obligation_set_hash
                        .clone(),
                    knowledge_feed_obligation_set_hash: snapshot
                        .knowledge_feed_obligation_set_hash
                        .clone(),
                    prior_terminal_attempt_chain_hash: aggregate
                        .prior_terminal_attempt_chain_hash
                        .clone(),
                    proposal_census_hash: aggregate.proposal_census_hash.clone(),
                    critic_census_hash: aggregate.critic_census_hash.clone(),
                    controller_decision_set_hash: aggregate.controller_decision_set_hash.clone(),
                    input_chunk_census_set_hash: aggregate.input_chunk_census_set_hash.clone(),
                    coverage_subreview_census_set_hash: aggregate
                        .coverage_subreview_census_set_hash
                        .clone(),
                    coverage_synthesis_census_set_hash: aggregate
                        .coverage_synthesis_census_set_hash
                        .clone(),
                    coverage_global_semantic_root_hash: aggregate
                        .coverage_global_semantic_root_hash
                        .clone(),
                    coverage_global_review_hash: aggregate.coverage_global_review_hash.clone(),
                    coverage_review_set_hash: aggregate.coverage_review_set_hash.clone(),
                    coverage_checklist_set_hash: aggregate.coverage_checklist_set_hash.clone(),
                    generation_transition_set_hash: compiled.generation_transition_set_hash.clone(),
                },
                input_dispositions,
                input_disposition_set: CandidateExactSetSealV1::seal(
                    "candidate_input_dispositions.v1",
                    disposition_hashes,
                ),
                input_relations,
                input_relation_set: CandidateExactSetSealV1::seal(
                    "candidate_input_relations.v1",
                    relation_hashes,
                ),
                generation_transition_set: CandidateExactSetSealV1::seal(
                    "candidate_generation_transitions.v1",
                    transition_hashes,
                ),
                planning_ready: true,
                capability_assessment_present: false,
                final_submitter_worker_run_id: aggregate.final_submitter_worker_run_id,
                controller_worker_run_id: aggregate.final_submitter_worker_run_id,
                controller_dispatch_worker_run_id: aggregate.controller_dispatch_worker_run_id,
            });
        tx.commit().await.map_err(typed_material_error)?;
        Ok(gate_snapshot)
    }
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct CandidateHostCompilationAuthority {
    pub stable_compilation_request_id: Uuid,
    pub stable_apply_request_id: Uuid,
    pub mutation_set_hash: String,
    pub claim_component_set_hash: String,
    pub verification_contract_set_hash: String,
    pub verification_plan_set_hash: String,
    pub generation_transition_set_hash: String,
    pub mutation_routes: BTreeMap<Uuid, CandidateRegistryMutationDecisionV1>,
    pub input_disposition_reason_codes: BTreeMap<Uuid, String>,
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct CandidateAtomicFinalizationRequest {
    pub fence: CandidateRepositoryWriteFenceV1,
    pub expected_source_head_version: i64,
    pub host: CandidateHostCompilationAuthority,
}

#[derive(Clone)]
pub struct AtomicCandidateFinalizer {
    repository: Arc<dyn HypothesisRegistryRepository>,
    snapshot_source: Arc<dyn CandidateGateSnapshotSource>,
}

impl AtomicCandidateFinalizer {
    pub fn new(
        repository: Arc<dyn HypothesisRegistryRepository>,
        snapshot_source: Arc<dyn CandidateGateSnapshotSource>,
    ) -> Self {
        Self {
            repository,
            snapshot_source,
        }
    }

    pub async fn finalize(
        &self,
        request: CandidateAtomicFinalizationRequest,
    ) -> Result<CandidateGenerationSealView, HypothesisRegistryError> {
        let fence = &request.fence;
        let load = LoadCandidateGateMaterial {
            operation_id: fence.operation_id,
            scope_snapshot_id: fence.scope_snapshot_id,
            organization_id: fence.organization_id,
            snapshot_id: fence.snapshot_id,
            analysis_attempt_id: fence.analysis_attempt_id,
            analysis_attempt_ordinal: fence.analysis_attempt_ordinal,
            expected_snapshot_row_version: fence.expected_snapshot_row_version,
            expected_attempt_row_version: fence.expected_attempt_row_version,
        };

        let snapshot = self
            .snapshot_source
            .load_candidate_gate_snapshot(load)
            .await?;
        let pass = validate_candidate_gate(&snapshot).map_err(|block| {
            HypothesisRegistryError::AuthorityMismatch(format!("{}: {block}", block.code()))
        })?;

        let gate_pass = to_repository_gate_pass(pass, &request.host)?;
        self.repository
            .apply_candidate_gate_pass(ApplyCandidateGatePass {
                fence: request.fence,
                stable_compilation_request_id: request.host.stable_compilation_request_id,
                stable_apply_request_id: request.host.stable_apply_request_id,
                gate_pass,
                expected_source_head_version: request.expected_source_head_version,
            })
            .await
    }
}

fn source_ref(value: RevisionSourceRef) -> CandidateRegistryRevisionSourceRefV1 {
    match value {
        RevisionSourceRef::ToolTruthEvidence(value) => {
            CandidateRegistryRevisionSourceRefV1::ToolTruthEvidence(value)
        }
        RevisionSourceRef::Finding(value) => CandidateRegistryRevisionSourceRefV1::Finding(value),
        RevisionSourceRef::VerificationReceipt(value) => {
            CandidateRegistryRevisionSourceRefV1::VerificationReceipt(value)
        }
        RevisionSourceRef::ApplicationContext(value) => {
            CandidateRegistryRevisionSourceRefV1::ApplicationContext(value)
        }
        RevisionSourceRef::KnowledgeSignal(value) => {
            CandidateRegistryRevisionSourceRefV1::KnowledgeSignal(value)
        }
        RevisionSourceRef::Gap(value) => CandidateRegistryRevisionSourceRefV1::Gap(value),
    }
}

fn to_repository_gate_pass(
    pass: CandidateGatePass,
    host: &CandidateHostCompilationAuthority,
) -> Result<CandidateGatePassV1, HypothesisRegistryError> {
    let proposal_ids = pass
        .mutation_set
        .iter()
        .map(|mutation| mutation.proposal_id)
        .collect::<std::collections::BTreeSet<_>>();
    if proposal_ids.len() != pass.mutation_set.len()
        || host
            .mutation_routes
            .keys()
            .copied()
            .collect::<std::collections::BTreeSet<_>>()
            != proposal_ids
    {
        return Err(HypothesisRegistryError::AuthorityMismatch(
            "host reducer route exact set differs from the Gate mutation set".to_owned(),
        ));
    }
    let input_ids = pass
        .input_dispositions
        .iter()
        .map(|decision| decision.input_id)
        .collect::<std::collections::BTreeSet<_>>();
    if input_ids.len() != pass.input_dispositions.len()
        || host
            .input_disposition_reason_codes
            .keys()
            .copied()
            .collect::<std::collections::BTreeSet<_>>()
            != input_ids
        || host
            .input_disposition_reason_codes
            .values()
            .any(|reason| reason.trim().is_empty())
    {
        return Err(HypothesisRegistryError::AuthorityMismatch(
            "host input-disposition reason exact set differs from the Gate".to_owned(),
        ));
    }

    let expected_authority = CandidateGateExpectedAuthorityV1 {
        snapshot_hash: pass.snapshot_hash,
        candidate_snapshot_authority_hash: pass.candidate_snapshot_authority_hash,
        tool_truth_authority_bundle_seal_id: pass.tool_truth_authority_bundle_seal_id,
        tool_truth_authority_root_set_hash: pass.tool_truth_authority_root_set_hash,
        tool_truth_authority_bundle_member_set_hash: pass
            .tool_truth_authority_bundle_member_set_hash,
        tool_truth_authority_receipt_set_hash: pass.tool_truth_authority_receipt_set_hash,
        denominator_graph_bundle_hash: pass.denominator_graph_bundle_hash,
        semantic_authority_bundle_hash: pass.semantic_authority_bundle_hash,
        freshness_attestation_bundle_hash: pass.freshness_attestation_bundle_hash,
        temporal_validity_bundle_hash: pass.temporal_validity_bundle_hash,
        temporal_validity_policy_digest: pass.temporal_validity_policy_digest,
        temporal_validity_decision_set_hash: pass.temporal_validity_decision_set_hash,
        target_state_epoch_set_hash: pass.target_state_epoch_set_hash,
        gate_temporal_reevaluation_hash: pass.gate_temporal_reevaluation_hash,
        knowledge_feed_catalog_policy_seal_hash: pass.knowledge_feed_catalog_policy_seal_hash,
        knowledge_feed_required_member_set_hash: pass.knowledge_feed_required_member_set_hash,
        knowledge_feed_signature_algorithm_set_hash: pass
            .knowledge_feed_signature_algorithm_set_hash,
        knowledge_feed_trust_store_hash: pass.knowledge_feed_trust_store_hash,
        knowledge_feed_key_revocation_epoch_hash: pass.knowledge_feed_key_revocation_epoch_hash,
        knowledge_feed_snapshot_set_hash: pass.knowledge_feed_snapshot_set_hash,
        product_version_census_hash: pass.product_version_census_hash,
        knowledge_feed_match_census_hash: pass.knowledge_feed_match_census_hash,
        gate_knowledge_feed_reevaluation_hash: pass.gate_knowledge_feed_reevaluation_hash,
        stale_revalidation_obligation_set_hash: pass.stale_revalidation_obligation_set_hash,
        knowledge_feed_obligation_set_hash: pass.knowledge_feed_obligation_set_hash,
        prior_terminal_attempt_chain_hash: pass.prior_terminal_attempt_chain_hash,
        proposal_census_hash: pass.proposal_census_hash,
        critic_census_hash: pass.critic_census_hash,
        controller_decision_set_hash: pass.controller_decision_set_hash,
        input_chunk_census_set_hash: pass.input_chunk_census_set_hash,
        coverage_subreview_census_set_hash: pass.hypothesis_coverage_subreview_census_set_hash,
        coverage_synthesis_census_set_hash: pass.hypothesis_coverage_synthesis_census_set_hash,
        coverage_global_semantic_root_hash: pass.hypothesis_coverage_global_semantic_root_hash,
        coverage_global_review_hash: pass.hypothesis_coverage_global_review_hash,
        coverage_review_set_hash: pass.hypothesis_coverage_review_set_hash,
        coverage_checklist_set_hash: pass.hypothesis_coverage_checklist_set_hash,
        generation_transition_set_hash: pass.generation_transition_set_hash.clone(),
    };
    let mutations = pass
        .mutation_set
        .into_iter()
        .map(|mutation| {
            Ok(CandidateRegistryMutationV1 {
                proposal_id: mutation.proposal_id,
                organization_id: mutation.organization_id,
                semantic_key_hash: mutation.semantic_key_hash,
                operator_rank: mutation.operator_rank,
                state: mutation.state,
                proof_refs: mutation.proof_refs.into_iter().map(source_ref).collect(),
                refutation_refs: mutation
                    .refutation_refs
                    .into_iter()
                    .map(source_ref)
                    .collect(),
                generation_transition_hash: mutation.generation_transition_hash,
                mutation_hash: mutation.mutation_hash,
                decision: host
                    .mutation_routes
                    .get(&mutation.proposal_id)
                    .cloned()
                    .ok_or_else(|| {
                        HypothesisRegistryError::AuthorityMismatch(
                            "host reducer route is missing".to_owned(),
                        )
                    })?,
            })
        })
        .collect::<Result<Vec<_>, HypothesisRegistryError>>()?;
    let input_dispositions = pass
        .input_dispositions
        .into_iter()
        .map(|decision| InputProcessingDispositionDecisionV1 {
            input_id: decision.input_id,
            disposition: match decision.disposition {
                GateDisposition::Analyzed => InputProcessingDispositionV1::Analyzed,
                GateDisposition::Informational => InputProcessingDispositionV1::Informational,
                GateDisposition::DuplicateInput => InputProcessingDispositionV1::DuplicateInput,
                GateDisposition::NotSecurityRelevant => {
                    InputProcessingDispositionV1::NotSecurityRelevant
                }
                GateDisposition::Gap => InputProcessingDispositionV1::Gap,
                GateDisposition::Blocked => InputProcessingDispositionV1::Blocked,
            },
            reason_code: host.input_disposition_reason_codes[&decision.input_id].clone(),
        })
        .collect();
    let input_relations = pass
        .input_relations
        .into_iter()
        .map(|decision| InputHypothesisRelationDecisionV1 {
            input_id: decision.input_id,
            hypothesis_root_id: decision.hypothesis_root_id,
            relation: match decision.relation {
                GateRelationKind::CreatesHypothesis => {
                    InputHypothesisRelationKindV1::CreatesHypothesis
                }
                GateRelationKind::SupportsExisting => {
                    InputHypothesisRelationKindV1::SupportsExisting
                }
                GateRelationKind::ContradictsExisting => {
                    InputHypothesisRelationKindV1::ContradictsExisting
                }
                GateRelationKind::QualifiesExisting => {
                    InputHypothesisRelationKindV1::QualifiesExisting
                }
            },
        })
        .collect();
    Ok(CandidateGatePassV1 {
        expected_authority,
        active_analysis_attempt_id: pass.active_analysis_attempt_id,
        active_analysis_attempt_ordinal: pass.active_analysis_attempt_ordinal,
        mutation_set: mutations,
        mutation_set_hash: pass.mutation_set_hash,
        hypothesis_claim_components: pass.hypothesis_claim_components,
        hypothesis_claim_component_set_hash: pass.hypothesis_claim_component_set_hash,
        verification_contracts: pass.verification_contracts,
        verification_contract_set_hash: pass.verification_contract_set_hash,
        hypothesis_verification_plans: pass.hypothesis_verification_plans,
        hypothesis_verification_plan_set_hash: pass.hypothesis_verification_plan_set_hash,
        input_dispositions,
        input_relations,
        final_submitter_worker_run_id: pass.final_submitter_worker_run_id,
    })
}

#[cfg(test)]
mod tests {
    use golish_agent_kit::harness::hypothesis_registry::{
        CandidateHypothesisMutation, InputProcessingDispositionDecision,
    };
    use golish_agent_kit::task_orchestrator::hypothesis_analysis::CandidateControllerDecision;
    use golish_core::hypothesis_semantic_key::CandidateMutationEpistemicState;

    use super::*;

    fn hash(byte: char) -> String {
        format!("sha256:{}", byte.to_string().repeat(64))
    }

    #[test]
    fn duplicate_semantic_routes_fail_closed_before_repository_apply() {
        let root_id = Uuid::new_v4();
        let semantic_key_hash = hash('a');
        let recipe = serde_json::json!({
            "schema":"candidate_host_compiler_recipe.v1",
            "organization_id":Uuid::new_v4(),
            "items":[
                {
                    "proposal_id":Uuid::new_v4(),
                    "semantic_key_hash":semantic_key_hash,
                    "route":{"kind":"create_initial","root_id":root_id},
                },
                {
                    "proposal_id":Uuid::new_v4(),
                    "semantic_key_hash":hash('a'),
                    "route":{"kind":"create_initial","root_id":root_id},
                }
            ],
        });
        let error = compile_candidate_host_recipe(&recipe)
            .expect_err("duplicate semantic routes must fail before apply");
        assert!(error
            .to_string()
            .contains("candidate compiler duplicate semantic route is not closed"));
        assert!(!error.to_string().contains("duplicate key value"));
    }

    #[test]
    fn true_zero_proposal_recipe_and_controller_confirmation_are_closed_empty_sets() {
        let recipe = serde_json::json!({
            "schema":"candidate_host_compiler_recipe.v1",
            "analysis_attempt_id":Uuid::new_v4(),
            "snapshot_id":Uuid::new_v4(),
            "operation_id":Uuid::new_v4(),
            "organization_id":Uuid::new_v4(),
            "items":[],
        });
        let compiled = compile_candidate_host_recipe(&recipe).expect("true zero is compilable");
        assert!(compiled.mutations.is_empty());
        assert!(compiled.mutation_routes.is_empty());
        assert!(compiled.claim_components.is_empty());
        assert!(compiled.verification_contracts.is_empty());
        assert!(compiled.verification_plans.is_empty());

        let input = CandidateControllerFinalInput {
            snapshot_id: Uuid::new_v4(),
            analysis_attempt_id: Uuid::new_v4(),
            proposal_census_hash: hash('a'),
            critic_census_hash: hash('b'),
            coverage_review_set_hash: hash('c'),
            claim_component_set_hash: compiled.claim_component_set_hash,
            verification_contract_set_hash: compiled.verification_contract_set_hash,
            verification_plan_set_hash: compiled.verification_plan_set_hash,
            proposal_pages: Vec::new(),
            proposal_page_set_hash: hash('d'),
        };
        validate_controller_proposal_pages(&input, &compiled.mutation_routes)
            .expect("empty page exact set is valid");
        validate_controller_decision_route_confirmation(
            &CandidateControllerDecisionArtifact {
                decisions: Vec::new(),
            },
            &compiled.mutation_routes,
        )
        .expect("empty controller decision exact set is valid");
    }

    #[test]
    fn controller_decisions_exactly_confirm_only_precompiled_routes() {
        let proposal_id = Uuid::new_v4();
        let root_id = Uuid::new_v4();
        let routes = BTreeMap::from([(
            proposal_id,
            CandidateRegistryMutationDecisionV1::CreateInitial { root_id },
        )]);
        let decision = |proposal_id, decision, related_proposal_ids| CandidateControllerDecision {
            proposal_id,
            decision,
            related_proposal_ids,
            rationale: "typed route confirmation".to_owned(),
        };
        let valid = CandidateControllerDecisionArtifact {
            decisions: vec![decision(
                proposal_id,
                CandidateControllerDecisionKind::Accept,
                Vec::new(),
            )],
        };
        validate_controller_decision_route_confirmation(&valid, &routes)
            .expect("Accept confirms exactly one CreateInitial route");

        let invalid = [
            CandidateControllerDecisionArtifact {
                decisions: Vec::new(),
            },
            CandidateControllerDecisionArtifact {
                decisions: vec![valid.decisions[0].clone(), valid.decisions[0].clone()],
            },
            CandidateControllerDecisionArtifact {
                decisions: vec![decision(
                    Uuid::new_v4(),
                    CandidateControllerDecisionKind::Accept,
                    Vec::new(),
                )],
            },
            CandidateControllerDecisionArtifact {
                decisions: vec![decision(
                    proposal_id,
                    CandidateControllerDecisionKind::Accept,
                    vec![proposal_id],
                )],
            },
            CandidateControllerDecisionArtifact {
                decisions: vec![decision(
                    proposal_id,
                    CandidateControllerDecisionKind::Blocked,
                    Vec::new(),
                )],
            },
            CandidateControllerDecisionArtifact {
                decisions: vec![decision(
                    proposal_id,
                    CandidateControllerDecisionKind::AttachExisting,
                    Vec::new(),
                )],
            },
        ];
        for artifact in invalid {
            let error = validate_controller_decision_route_confirmation(&artifact, &routes)
                .expect_err(
                    "open, duplicate, foreign, related, or mismatched decisions fail closed",
                );
            assert!(error
                .to_string()
                .contains("HYPOTHESIS_CANDIDATE_CONTROLLER_DECISION_INVALID"));
        }
    }

    fn pass() -> CandidateGatePass {
        let proposal_id = Uuid::new_v4();
        let input_id = Uuid::new_v4();
        CandidateGatePass {
            snapshot_id: Uuid::new_v4(),
            snapshot_hash: hash('a'),
            candidate_snapshot_authority_hash: hash('b'),
            tool_truth_authority_bundle_seal_id: Uuid::new_v4(),
            tool_truth_authority_root_set_hash: hash('c'),
            tool_truth_authority_bundle_member_set_hash: hash('d'),
            tool_truth_authority_receipt_set_hash: hash('e'),
            denominator_graph_bundle_hash: hash('f'),
            semantic_authority_bundle_hash: hash('1'),
            freshness_attestation_bundle_hash: hash('2'),
            temporal_validity_bundle_hash: hash('3'),
            temporal_validity_policy_digest: hash('4'),
            temporal_validity_decision_set_hash: hash('5'),
            target_state_epoch_set_hash: hash('6'),
            gate_temporal_reevaluation_hash: hash('7'),
            knowledge_feed_catalog_policy_seal_hash: hash('8'),
            knowledge_feed_required_member_set_hash: hash('9'),
            knowledge_feed_signature_algorithm_set_hash: hash('a'),
            knowledge_feed_trust_store_hash: hash('b'),
            knowledge_feed_key_revocation_epoch_hash: hash('c'),
            knowledge_feed_snapshot_set_hash: hash('d'),
            product_version_census_hash: hash('e'),
            knowledge_feed_match_census_hash: hash('f'),
            gate_knowledge_feed_reevaluation_hash: hash('1'),
            stale_revalidation_obligation_set_hash: hash('2'),
            knowledge_feed_obligation_set_hash: hash('3'),
            active_analysis_attempt_id: Uuid::new_v4(),
            active_analysis_attempt_ordinal: 0,
            prior_terminal_attempt_chain_hash: hash('4'),
            proposal_census_hash: hash('5'),
            critic_census_hash: hash('6'),
            controller_decision_set_hash: hash('7'),
            mutation_set: vec![CandidateHypothesisMutation {
                proposal_id,
                organization_id: Uuid::new_v4(),
                semantic_key_hash: hash('8'),
                operator_rank: 0,
                state: CandidateMutationEpistemicState::Proposed,
                proof_refs: vec![RevisionSourceRef::ToolTruthEvidence(hash('9'))],
                refutation_refs: vec![],
                generation_transition_hash: hash('a'),
                mutation_hash: hash('b'),
            }],
            mutation_set_hash: hash('c'),
            hypothesis_claim_components: vec![],
            hypothesis_claim_component_set_hash: hash('d'),
            verification_contracts: vec![],
            verification_contract_set_hash: hash('e'),
            hypothesis_verification_plans: vec![],
            hypothesis_verification_plan_set_hash: hash('f'),
            input_dispositions: vec![InputProcessingDispositionDecision {
                input_id,
                disposition: GateDisposition::Analyzed,
                decision_hash: hash('1'),
            }],
            input_relations: vec![],
            input_chunk_census_set_hash: hash('2'),
            hypothesis_coverage_subreview_census_set_hash: hash('3'),
            hypothesis_coverage_synthesis_census_set_hash: hash('4'),
            hypothesis_coverage_global_semantic_root_hash: hash('5'),
            hypothesis_coverage_global_review_hash: hash('6'),
            hypothesis_coverage_review_set_hash: hash('7'),
            hypothesis_coverage_checklist_set_hash: hash('8'),
            generation_transition_set_hash: hash('9'),
            final_submitter_worker_run_id: Uuid::new_v4(),
        }
    }

    fn host(pass: &CandidateGatePass) -> CandidateHostCompilationAuthority {
        let mutation = &pass.mutation_set[0];
        let input = &pass.input_dispositions[0];
        CandidateHostCompilationAuthority {
            stable_compilation_request_id: Uuid::new_v4(),
            stable_apply_request_id: Uuid::new_v4(),
            mutation_set_hash: pass.mutation_set_hash.clone(),
            claim_component_set_hash: pass.hypothesis_claim_component_set_hash.clone(),
            verification_contract_set_hash: pass.verification_contract_set_hash.clone(),
            verification_plan_set_hash: pass.hypothesis_verification_plan_set_hash.clone(),
            generation_transition_set_hash: pass.generation_transition_set_hash.clone(),
            mutation_routes: BTreeMap::from([(
                mutation.proposal_id,
                CandidateRegistryMutationDecisionV1::CreateInitial {
                    root_id: Uuid::new_v4(),
                },
            )]),
            input_disposition_reason_codes: BTreeMap::from([(
                input.input_id,
                "analyzed_from_exact_frozen_input".to_owned(),
            )]),
        }
    }

    #[test]
    fn candidate_finalizer_maps_only_exact_host_compiler_sets() {
        let pass = pass();
        let expected_proposal = pass.mutation_set[0].proposal_id;
        let expected_input = pass.input_dispositions[0].input_id;
        let converted = to_repository_gate_pass(pass.clone(), &host(&pass)).unwrap();
        assert_eq!(converted.mutation_set[0].proposal_id, expected_proposal);
        assert_eq!(converted.input_dispositions[0].input_id, expected_input);
        assert!(matches!(
            converted.mutation_set[0].proof_refs.as_slice(),
            [CandidateRegistryRevisionSourceRefV1::ToolTruthEvidence(_)]
        ));
    }

    #[test]
    fn candidate_finalizer_rejects_missing_route_or_disposition_reason() {
        let pass = pass();
        let mut missing_route = host(&pass);
        missing_route.mutation_routes.clear();
        assert!(matches!(
            to_repository_gate_pass(pass.clone(), &missing_route),
            Err(HypothesisRegistryError::AuthorityMismatch(_))
        ));

        let mut missing_reason = host(&pass);
        missing_reason.input_disposition_reason_codes.clear();
        assert!(matches!(
            to_repository_gate_pass(pass, &missing_reason),
            Err(HypothesisRegistryError::AuthorityMismatch(_))
        ));
    }
}
