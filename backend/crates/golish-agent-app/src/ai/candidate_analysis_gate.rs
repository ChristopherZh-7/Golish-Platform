//! Atomic application boundary for a Candidate Registry Gate pass.
//!
//! The model-facing runtime can persist analysis artifacts, but it cannot
//! construct [`CandidateGateSnapshot`].  Only a repository adapter that has
//! locked and reloaded the frozen rows may implement
//! [`CandidateGateSnapshotSource`].  This keeps the production path fail
//! closed until the complete opaque material loader is installed.

use std::collections::BTreeMap;
use std::sync::Arc;

use async_trait::async_trait;
use golish_agent_kit::db_traits::*;
use golish_agent_kit::harness::hypothesis_registry::{
    validate_candidate_gate, CandidateGatePass, CandidateGateSnapshot,
    InputHypothesisRelationKindV1 as GateRelationKind,
    InputProcessingDispositionV1 as GateDisposition, RevisionSourceRef,
};
use uuid::Uuid;

#[async_trait]
pub trait CandidateGateSnapshotSource: HypothesisRegistryRepository {
    /// Reload complete, locked authority rows and construct the opaque Gate
    /// snapshot. Implementations must never deserialize this value from an
    /// agent artifact or accept caller-supplied authority fields.
    async fn load_candidate_gate_snapshot(
        &self,
        request: LoadCandidateGateMaterial,
    ) -> Result<CandidateGateSnapshot, HypothesisRegistryError>;
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct CandidateHostCompilationAuthority {
    pub stable_compilation_request_id: Uuid,
    pub stable_apply_request_id: Uuid,
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
    repository: Arc<dyn CandidateGateSnapshotSource>,
}

impl AtomicCandidateFinalizer {
    pub fn new(repository: Arc<dyn CandidateGateSnapshotSource>) -> Self {
        Self { repository }
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

        // The opaque material is loaded before the compiler seal only to obtain
        // server-derived objects. The aggregate material is reloaded after the
        // seal, and apply performs the final transactional CAS revalidation.
        let snapshot = self
            .repository
            .load_candidate_gate_snapshot(load.clone())
            .await?;
        let pass = validate_candidate_gate(&snapshot).map_err(|block| {
            HypothesisRegistryError::AuthorityMismatch(format!("{}: {block}", block.code()))
        })?;

        let sealed = self
            .repository
            .seal_candidate_compilation(SealCandidateCompilation {
                fence: request.fence.clone(),
                stable_compilation_request_id: request.host.stable_compilation_request_id,
                mutation_set_hash: pass.mutation_set_hash.clone(),
                claim_component_set_hash: pass.hypothesis_claim_component_set_hash.clone(),
                verification_contract_set_hash: pass.verification_contract_set_hash.clone(),
                verification_plan_set_hash: pass.hypothesis_verification_plan_set_hash.clone(),
                generation_transition_set_hash: pass.generation_transition_set_hash.clone(),
            })
            .await?;

        let material = self.repository.load_candidate_gate_material(load).await?;
        validate_reloaded_material(&request.fence, &pass, &sealed, &material)?;
        let gate_pass = to_repository_gate_pass(pass, &request.host)?;
        self.repository
            .apply_candidate_gate_pass(ApplyCandidateGatePass {
                fence: request.fence,
                stable_apply_request_id: request.host.stable_apply_request_id,
                gate_pass,
                expected_source_head_version: request.expected_source_head_version,
            })
            .await
    }
}

fn validate_reloaded_material(
    fence: &CandidateRepositoryWriteFenceV1,
    pass: &CandidateGatePass,
    seal: &CandidateCompilationSealView,
    material: &CandidateGateMaterial,
) -> Result<(), HypothesisRegistryError> {
    let valid = material.snapshot.snapshot_id == fence.snapshot_id
        && material.snapshot.operation_id == fence.operation_id
        && material.snapshot.scope_snapshot_id == fence.scope_snapshot_id
        && material.snapshot.organization_id == fence.organization_id
        && material.active_analysis_attempt_id == fence.analysis_attempt_id
        && material.active_analysis_attempt_ordinal == fence.analysis_attempt_ordinal
        && material.snapshot_row_version == fence.expected_snapshot_row_version
        && material.attempt_row_version == fence.expected_attempt_row_version
        && material.snapshot.snapshot_hash == pass.snapshot_hash
        && material.snapshot.candidate_snapshot_authority_hash
            == pass.candidate_snapshot_authority_hash
        && material.gate_temporal_reevaluation_hash == pass.gate_temporal_reevaluation_hash
        && material.gate_knowledge_feed_reevaluation_hash
            == pass.gate_knowledge_feed_reevaluation_hash
        && material.prior_terminal_attempt_chain_hash == pass.prior_terminal_attempt_chain_hash
        && material.proposal_census_hash == pass.proposal_census_hash
        && material.critic_census_hash == pass.critic_census_hash
        && material.controller_decision_set_hash == pass.controller_decision_set_hash
        && material.input_chunk_census_set_hash == pass.input_chunk_census_set_hash
        && material.coverage_subreview_census_set_hash
            == pass.hypothesis_coverage_subreview_census_set_hash
        && material.coverage_synthesis_census_set_hash
            == pass.hypothesis_coverage_synthesis_census_set_hash
        && material.coverage_global_semantic_root_hash
            == pass.hypothesis_coverage_global_semantic_root_hash
        && material.coverage_global_review_hash == pass.hypothesis_coverage_global_review_hash
        && material.coverage_review_set_hash == pass.hypothesis_coverage_review_set_hash
        && material.coverage_checklist_set_hash == pass.hypothesis_coverage_checklist_set_hash
        && material.mutation_set_hash == pass.mutation_set_hash
        && material.claim_component_set_hash == pass.hypothesis_claim_component_set_hash
        && material.verification_contract_set_hash == pass.verification_contract_set_hash
        && material.verification_plan_set_hash == pass.hypothesis_verification_plan_set_hash
        && material.generation_transition_set_hash == pass.generation_transition_set_hash
        && material.final_submitter_worker_run_id == pass.final_submitter_worker_run_id
        && seal.mutation_set_hash == material.mutation_set_hash
        && seal.claim_component_set_hash == material.claim_component_set_hash
        && seal.verification_contract_set_hash == material.verification_contract_set_hash
        && seal.verification_plan_set_hash == material.verification_plan_set_hash
        && seal.generation_transition_set_hash == material.generation_transition_set_hash
        && seal.compiler_seal_hash == material.compiler_seal_hash;
    if valid {
        Ok(())
    } else {
        Err(HypothesisRegistryError::AuthorityMismatch(
            "candidate Gate material drifted between opaque validation and apply".to_owned(),
        ))
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
    use golish_core::hypothesis_semantic_key::CandidateMutationEpistemicState;

    use super::*;

    fn hash(byte: char) -> String {
        format!("sha256:{}", byte.to_string().repeat(64))
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
