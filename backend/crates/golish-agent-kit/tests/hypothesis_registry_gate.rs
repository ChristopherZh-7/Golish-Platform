use golish_agent_kit::harness::hypothesis_registry::{
    candidate_mutation_state, initial_root_id, reduce_proposals, CandidateMutationEpistemicState,
    CandidateMutationError, CandidateProposal, ClaimPolarity, HypothesisSemanticKeyV1,
    PredicateIdentity, ReducerCatalog, ReducerDecision, ReducerOperatorInputV1, ReducerProposal,
};
use serde_json::json;
use uuid::Uuid;

#[test]
fn gate_exposes_a_pure_frozen_snapshot_boundary() {
    use golish_agent_kit::harness::hypothesis_registry::{
        validate_candidate_gate, CandidateGateBlock, CandidateGatePass, CandidateGateSnapshot,
    };

    let _gate: fn(&CandidateGateSnapshot) -> Result<CandidateGatePass, CandidateGateBlock> =
        validate_candidate_gate;
}

fn hash(ch: char) -> String {
    format!("sha256:{}", ch.to_string().repeat(64))
}

fn proposal(prose: &str, priority: i32) -> CandidateProposal {
    CandidateProposal {
        operation_id: Uuid::from_u128(1),
        organization_id: Uuid::from_u128(2),
        subject_kind: "service".into(),
        subject_identity_hash: format!("sha256:{}", "1".repeat(64)),
        predicate: PredicateIdentity::new(
            "http.authorization".into(),
            1,
            json!({"resource": {"kind": "invoice", "path": "/v1/invoices/:id"}}),
        )
        .unwrap(),
        trust_boundary: "tenant".into(),
        polarity: ClaimPolarity::Positive,
        prose: prose.into(),
        confidence: 70,
        priority,
        tags: vec!["OWASP-API1".into()],
        evidence_refs: vec!["evidence:1".into()],
        proposer: "analyst".into(),
        requested_state: CandidateMutationEpistemicState::Proposed,
    }
}

fn reducer_catalog() -> ReducerCatalog {
    ReducerCatalog::for_scope(Uuid::from_u128(1), Uuid::from_u128(2))
}

#[test]
fn semantic_nested_json_order_is_canonical() {
    let mut first = proposal("first", 9);
    let mut second = proposal("second", 1);
    first.predicate = PredicateIdentity::new(
        "http.authorization".into(),
        1,
        json!({"z": {"b": 2, "a": 1}, "a": [3, 2, 1]}),
    )
    .unwrap();
    second.predicate = PredicateIdentity::new(
        "http.authorization".into(),
        1,
        json!({"a": [3, 2, 1], "z": {"a": 1, "b": 2}}),
    )
    .unwrap();

    assert_eq!(
        HypothesisSemanticKeyV1::from_claim(&first)
            .unwrap()
            .hash()
            .unwrap(),
        HypothesisSemanticKeyV1::from_claim(&second)
            .unwrap()
            .hash()
            .unwrap()
    );
}

#[test]
fn semantic_identity_axes_are_exact_and_live_fields_are_excluded() {
    let base = proposal("base", 1);
    let base_hash = HypothesisSemanticKeyV1::from_claim(&base)
        .unwrap()
        .hash()
        .unwrap();
    let mut variants = Vec::new();
    let mut changed = base.clone();
    changed.organization_id = Uuid::from_u128(20);
    variants.push(changed);
    let mut changed = base.clone();
    changed.subject_kind = "endpoint".into();
    variants.push(changed);
    let mut changed = base.clone();
    changed.subject_identity_hash = hash('2');
    variants.push(changed);
    let mut changed = base.clone();
    changed.predicate = PredicateIdentity::new(
        "http.authentication".into(),
        1,
        json!({"resource": {"kind": "invoice", "path": "/v1/invoices/:id"}}),
    )
    .unwrap();
    variants.push(changed);
    let mut changed = base.clone();
    changed.predicate = PredicateIdentity::new(
        "http.authorization".into(),
        2,
        json!({"resource": {"kind": "invoice", "path": "/v1/invoices/:id"}}),
    )
    .unwrap();
    variants.push(changed);
    let mut changed = base.clone();
    changed.predicate = PredicateIdentity::new(
        "http.authorization".into(),
        1,
        json!({"resource": {"kind": "invoice", "path": "/v2/invoices/:id"}}),
    )
    .unwrap();
    variants.push(changed);
    let mut changed = base.clone();
    changed.trust_boundary = "organization".into();
    variants.push(changed);
    let mut changed = base;
    changed.polarity = ClaimPolarity::Negative;
    variants.push(changed);

    for changed in variants {
        assert_ne!(
            base_hash,
            HypothesisSemanticKeyV1::from_claim(&changed)
                .unwrap()
                .hash()
                .unwrap()
        );
    }
}

#[test]
fn non_identity_fields_do_not_change_semantic_key_or_initial_root() {
    let first = proposal("first prose", 90);
    let mut second = proposal("second prose", 10);
    second.confidence = 10;
    second.tags = vec!["CWE-639".into()];
    second.evidence_refs = vec!["evidence:other".into()];
    second.proposer = "controller".into();
    let first_key = HypothesisSemanticKeyV1::from_claim(&first).unwrap();
    let second_key = HypothesisSemanticKeyV1::from_claim(&second).unwrap();
    assert_eq!(first_key.hash().unwrap(), second_key.hash().unwrap());
    assert_eq!(
        initial_root_id(first.operation_id, first.organization_id, &first_key).unwrap(),
        initial_root_id(second.operation_id, second.organization_id, &second_key).unwrap()
    );
}

#[test]
fn provider_completion_order_does_not_change_reducer_hash() {
    let proposals = (0..3)
        .map(|ordinal| {
            let mut candidate = proposal(&format!("proposal-{ordinal}"), ordinal);
            candidate.predicate = PredicateIdentity::new(
                "http.authorization".into(),
                1,
                json!({"resource_ordinal": ordinal}),
            )
            .unwrap();
            candidate
        })
        .collect::<Vec<_>>();
    let expected = reduce_proposals(&proposals, &reducer_catalog())
        .unwrap()
        .mutation_set_hash()
        .to_owned();
    for order in [
        [0, 1, 2],
        [0, 2, 1],
        [1, 0, 2],
        [1, 2, 0],
        [2, 0, 1],
        [2, 1, 0],
    ] {
        let permutation = order
            .map(|index| proposals[index].clone())
            .into_iter()
            .collect::<Vec<_>>();
        assert_eq!(
            expected,
            reduce_proposals(&permutation, &reducer_catalog())
                .unwrap()
                .mutation_set_hash()
        );
    }
}

#[test]
fn root_existing_current_is_attached_before_create() {
    let proposal = ReducerProposal::from_candidate(&proposal("a", 1)).unwrap();
    let root_id = proposal.initial_root_id;
    let revision_id = Uuid::from_u128(9);
    let catalog =
        reducer_catalog().with_current(proposal.semantic_key_hash.clone(), root_id, revision_id);
    assert_eq!(
        catalog.route(&proposal).unwrap(),
        ReducerDecision::AttachCurrent {
            root_id,
            revision_id
        }
    );
}

#[test]
fn reducer_historical_routes_precede_initial_creation() {
    let proposal = ReducerProposal::from_candidate(&proposal("a", 1)).unwrap();
    let historical_root = Uuid::from_u128(30);
    let historical_revision = Uuid::from_u128(31);
    let reopen = reducer_catalog().with_historical(
        proposal.semantic_key_hash.clone(),
        historical_root,
        historical_revision,
        false,
        true,
    );
    assert_eq!(
        reopen.route(&proposal).unwrap(),
        ReducerDecision::ReopenHistorical {
            root_id: historical_root,
            predecessor_revision_id: historical_revision,
        }
    );
    let evolved = reducer_catalog().with_historical(
        proposal.semantic_key_hash.clone(),
        historical_root,
        historical_revision,
        true,
        true,
    );
    assert_eq!(
        evolved.route(&proposal).unwrap(),
        ReducerDecision::ExplicitTransitionRequired {
            historical_root_id: historical_root,
        }
    );
    let unchanged = reducer_catalog().with_historical(
        proposal.semantic_key_hash.clone(),
        historical_root,
        historical_revision,
        false,
        false,
    );
    assert_eq!(
        unchanged.route(&proposal).unwrap(),
        ReducerDecision::NoSemanticChange {
            root_id: historical_root,
            revision_id: historical_revision,
        }
    );
}

#[test]
fn reducer_all_closed_operators_and_collision_are_reachable() {
    let proposal = ReducerProposal::from_candidate(&proposal("operators", 1)).unwrap();
    assert_eq!(
        reducer_catalog().route(&proposal).unwrap(),
        ReducerDecision::CreateInitial {
            root_id: proposal.initial_root_id,
        }
    );

    let parent_a = Uuid::from_u128(60);
    let parent_b = Uuid::from_u128(61);
    assert!(matches!(
        reducer_catalog()
            .route_with_operator(
                &proposal,
                &ReducerOperatorInputV1::Split {
                    parent_root_id: parent_a,
                },
            )
            .unwrap(),
        ReducerDecision::Split { parent_root_id, child_root_ids }
            if parent_root_id == parent_a && child_root_ids.len() == 1
    ));
    assert!(matches!(
        reducer_catalog()
            .route_with_operator(
                &proposal,
                &ReducerOperatorInputV1::Merge {
                    parent_root_ids: vec![parent_b, parent_a],
                },
            )
            .unwrap(),
        ReducerDecision::Merge { parent_root_ids, .. }
            if parent_root_ids == vec![parent_a, parent_b]
    ));
    assert!(matches!(
        reducer_catalog()
            .route_with_operator(
                &proposal,
                &ReducerOperatorInputV1::Derive {
                    source_root_id: parent_a,
                    source_revision_id: Uuid::from_u128(62),
                    derivation_rule_hash: hash('a'),
                },
            )
            .unwrap(),
        ReducerDecision::Derive { source_root_id, .. } if source_root_id == parent_a
    ));
    assert!(matches!(
        reducer_catalog()
            .route_with_operator(
                &proposal,
                &ReducerOperatorInputV1::NarrowSuccessor {
                    source_root_id: parent_a,
                    source_revision_id: Uuid::from_u128(62),
                    covered_claim_component_set_hash: hash('b'),
                },
            )
            .unwrap(),
        ReducerDecision::NarrowSuccessor { source_root_id, .. } if source_root_id == parent_a
    ));

    assert_eq!(
        reducer_catalog()
            .with_root_ingredients(proposal.initial_root_id, hash('c'))
            .route(&proposal)
            .unwrap(),
        ReducerDecision::RootIdCollision {
            computed_root_id: proposal.initial_root_id,
        }
    );
    let mut wrong_scope = proposal.clone();
    wrong_scope.operation_id = Uuid::from_u128(999);
    assert!(reducer_catalog().route(&wrong_scope).is_err());
}

#[test]
fn root_and_revision_uuid_formulas_are_domain_separated_and_replay_stable() {
    use golish_core::hypothesis_semantic_key::{
        candidate_revision_id, derive_root_id, merge_root_id, split_root_id, terminal_revision_id,
    };

    let candidate = proposal("identity", 1);
    let key = HypothesisSemanticKeyV1::from_claim(&candidate).unwrap();
    let operation_id = candidate.operation_id;
    let initial = initial_root_id(operation_id, candidate.organization_id, &key).unwrap();
    let parent_a = Uuid::from_u128(40);
    let parent_b = Uuid::from_u128(41);
    let split = split_root_id(operation_id, &key, parent_a).unwrap();
    let merge = merge_root_id(operation_id, &key, &[parent_b, parent_a]).unwrap();
    assert_eq!(
        merge,
        merge_root_id(operation_id, &key, &[parent_a, parent_b]).unwrap()
    );
    let derived = derive_root_id(
        operation_id,
        &key,
        parent_a,
        Uuid::from_u128(42),
        &hash('a'),
    )
    .unwrap();
    let roots = [initial, split, merge, derived];
    for (index, left) in roots.iter().enumerate() {
        assert!(roots[index + 1..].iter().all(|right| left != right));
    }

    let semantic_key_hash = key.hash().unwrap();
    let candidate_revision =
        candidate_revision_id(initial, 0, &semantic_key_hash, &hash('b')).unwrap();
    assert_eq!(
        candidate_revision,
        candidate_revision_id(initial, 0, &semantic_key_hash, &hash('b')).unwrap()
    );
    let terminal_revision =
        terminal_revision_id(initial, 1, &semantic_key_hash, &hash('c'), &hash('d')).unwrap();
    assert_ne!(candidate_revision, terminal_revision);
}

#[test]
fn candidate_terminal_state_is_rejected_before_mutation() {
    for state in ["proposed", "supported", "contested", "inconclusive"] {
        assert!(candidate_mutation_state(state).is_ok());
    }
    assert_eq!(
        candidate_mutation_state("verified"),
        Err(CandidateMutationError::TerminalStateForbidden)
    );
    assert_eq!(
        candidate_mutation_state("refuted"),
        Err(CandidateMutationError::TerminalStateForbidden)
    );
    assert_eq!(
        candidate_mutation_state("invalid"),
        Err(CandidateMutationError::InvalidStateServerOnly)
    );
}

#[test]
fn candidate_terminal_state_artifact_rejects_host_owned_contract_fields() {
    let mut value = serde_json::to_value(proposal("a", 1)).unwrap();
    value
        .as_object_mut()
        .unwrap()
        .insert("contract_hash".into(), json!(hash('f')));
    assert!(serde_json::from_value::<CandidateProposal>(value).is_err());
}

#[test]
fn verification_contract_is_host_owned() {
    use golish_agent_kit::harness::hypothesis_registry::{
        compile_verification_contract, PredicateRegistryEntry, VerificationContractCompilerInput,
    };
    use golish_core::verification_contract::{CanonicalJsonObject, ContractCombinatorV1};

    let input = VerificationContractCompilerInput {
        revision_id: Uuid::from_u128(3),
        revision_hash: hash('1'),
        objective_id: Uuid::from_u128(4),
        combinator: ContractCombinatorV1::AllOf,
        predicate_registry_entries: vec![PredicateRegistryEntry {
            semantic_key: "predicate:a".into(),
            predicate_schema: "http.authorization".into(),
            predicate_version: 1,
            normalized_arguments: CanonicalJsonObject::try_from_value(json!({"a": 1})).unwrap(),
            expected_polarity: ClaimPolarity::Positive,
            prerequisite_hash: hash('2'),
        }],
        required_controls: vec![],
        paired_differential_bindings: vec![],
        ordered_steps: vec![],
        stopping_criteria_hash: hash('3'),
        compiler_digest: hash('4'),
        rule_digest: hash('5'),
        policy_snapshot_hash: hash('6'),
    };
    let first = compile_verification_contract(input.clone()).unwrap();
    let second = compile_verification_contract(input).unwrap();
    assert_eq!(first.contract_hash(), second.contract_hash());
}

#[test]
fn hypothesis_claim_component_and_hypothesis_verification_plan_are_host_derived() {
    use golish_agent_kit::harness::hypothesis_registry::{
        compile_claim_components, compile_verification_contract, compile_verification_plan,
        ClaimComponentCompilerInput, PredicateRegistryEntry, StructuredClaimComponentSourceV1,
        VerificationContractCompilerInput, VerificationPlanCompilerInput,
    };

    use golish_core::hypothesis_verification::{
        HypothesisClaimComponentKindV1, HypothesisVerificationObjectiveOutcomeRequirementV1,
        HypothesisVerificationPlanBuildInputV1, HypothesisVerificationPlanObjectiveInputV1,
        HypothesisVerificationPlanPathInputV1, HypothesisVerificationPlanPathMemberInputV1,
        HypothesisVerificationPlanPathMemberRoleV1,
    };
    use golish_core::verification_contract::{CanonicalJsonObject, ContractCombinatorV1};

    let revision_id = Uuid::from_u128(5);
    let revision_hash = hash('7');
    let source = |key: &str, fragment: char, condition: char| StructuredClaimComponentSourceV1 {
        component_key: key.into(),
        canonical_fragment_hash: hash(fragment),
        canonical_condition_hash: hash(condition),
        required: true,
    };
    let components = compile_claim_components(ClaimComponentCompilerInput {
        revision_id,
        revision_hash: revision_hash.clone(),
        derivation_contract_version: 1,
        derivation_contract_digest: hash('8'),
        claim_clauses: vec![source("claim", '9', 'a')],
        impact_qualifiers: vec![source("impact", 'b', 'c')],
        trust_boundary_conditions: vec![source("trust", 'd', 'e')],
        identity_conditions: vec![source("identity", 'f', '0')],
    })
    .unwrap();
    assert!(HypothesisClaimComponentKindV1::ALL
        .into_iter()
        .all(|kind| components.iter().any(|component| component.kind() == kind)));
    let component_hashes = components
        .iter()
        .map(|component| component.member_hash().to_owned())
        .collect::<Vec<_>>();
    let objective_id = Uuid::from_u128(6);
    let contract = compile_verification_contract(VerificationContractCompilerInput {
        revision_id,
        revision_hash: revision_hash.clone(),
        objective_id,
        combinator: ContractCombinatorV1::AllOf,
        predicate_registry_entries: vec![PredicateRegistryEntry {
            semantic_key: "claim".into(),
            predicate_schema: "verification.test.v1".into(),
            predicate_version: 1,
            normalized_arguments: CanonicalJsonObject::parse("{\"value\":1}").unwrap(),
            expected_polarity: ClaimPolarity::Positive,
            prerequisite_hash: hash('c'),
        }],
        required_controls: Vec::new(),
        paired_differential_bindings: Vec::new(),
        ordered_steps: Vec::new(),
        stopping_criteria_hash: hash('d'),
        compiler_digest: hash('e'),
        rule_digest: hash('f'),
        policy_snapshot_hash: hash('0'),
    })
    .unwrap();
    let plan = compile_verification_plan(VerificationPlanCompilerInput(
        HypothesisVerificationPlanBuildInputV1 {
            revision_id,
            revision_hash,
            revision_ingredients_hash: hash('b'),
            required_claim_components: components,
            objectives: vec![HypothesisVerificationPlanObjectiveInputV1 {
                objective_hash: hash('c'),
                verification_contract: contract,
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
            outer_aggregation_policy_digest: hash('f'),
        },
    ))
    .unwrap();
    assert!(!plan.plan_hash().is_empty());
}

#[test]
fn hypothesis_revision_adjudication_requires_full_authority() {
    use golish_core::hypothesis_verification::HypothesisRevisionAdjudicationAuthorityV1;
    assert!(HypothesisRevisionAdjudicationAuthorityV1::validate_absent().is_err());
}

#[test]
fn verification_contract_authority_adapter_requires_opaque_plan_a_guard() {
    use golish_agent_kit::harness::hypothesis_registry::{
        freeze_candidate_authority_bundle, CandidateAuthorityBundleSnapshotV1,
    };
    use golish_db::repo::capability_execution_receipts::CheckedToolTruthAuthorityBundle;

    fn adapter_accepts_only_opaque_checked_bundle(
        _adapter: for<'guard> fn(
            &CheckedToolTruthAuthorityBundle<'guard>,
        ) -> CandidateAuthorityBundleSnapshotV1,
    ) {
    }

    adapter_accepts_only_opaque_checked_bundle(freeze_candidate_authority_bundle);
}

fn gate_fixture() -> golish_agent_kit::harness::hypothesis_registry::CandidateGateSnapshot {
    use golish_agent_kit::harness::hypothesis_registry::*;
    use golish_core::hypothesis_verification::{
        HypothesisVerificationObjectiveOutcomeRequirementV1,
        HypothesisVerificationPlanBuildInputV1, HypothesisVerificationPlanObjectiveInputV1,
        HypothesisVerificationPlanPathInputV1, HypothesisVerificationPlanPathMemberInputV1,
        HypothesisVerificationPlanPathMemberRoleV1,
    };
    use golish_core::verification_contract::{CanonicalJsonObject, ContractCombinatorV1};
    use golish_pentest_domain::tool_truth::{TemporalValidityStatus, ToolTruthRootFamilyV1};

    let operation_id = Uuid::from_u128(0xb001);
    let organization_id = Uuid::from_u128(0xb002);
    let revision_id = Uuid::from_u128(0xb003);
    let revision_hash = hash('1');
    let source = |key: &str, fragment: char, condition: char| StructuredClaimComponentSourceV1 {
        component_key: key.into(),
        canonical_fragment_hash: hash(fragment),
        canonical_condition_hash: hash(condition),
        required: true,
    };
    let components = compile_claim_components(ClaimComponentCompilerInput {
        revision_id,
        revision_hash: revision_hash.clone(),
        derivation_contract_version: 1,
        derivation_contract_digest: hash('2'),
        claim_clauses: vec![source("claim", '3', '4')],
        impact_qualifiers: vec![source("impact", '5', '6')],
        trust_boundary_conditions: vec![source("boundary", '7', '8')],
        identity_conditions: vec![source("identity", '9', 'a')],
    })
    .unwrap();
    let component_hashes = components
        .iter()
        .map(|component| component.member_hash().to_owned())
        .collect::<Vec<_>>();
    let objective_id = Uuid::from_u128(0xb004);
    let contract = compile_verification_contract(VerificationContractCompilerInput {
        revision_id,
        revision_hash: revision_hash.clone(),
        objective_id,
        combinator: ContractCombinatorV1::AllOf,
        predicate_registry_entries: vec![PredicateRegistryEntry {
            semantic_key: "predicate:authorization".into(),
            predicate_schema: "http.authorization.v1".into(),
            predicate_version: 1,
            normalized_arguments: CanonicalJsonObject::parse("{\"path\":\"/invoice/:id\"}")
                .unwrap(),
            expected_polarity: ClaimPolarity::Positive,
            prerequisite_hash: hash('b'),
        }],
        required_controls: vec![],
        paired_differential_bindings: vec![],
        ordered_steps: vec![],
        stopping_criteria_hash: hash('c'),
        compiler_digest: hash('d'),
        rule_digest: hash('e'),
        policy_snapshot_hash: hash('f'),
    })
    .unwrap();
    let plan = compile_verification_plan(VerificationPlanCompilerInput(
        HypothesisVerificationPlanBuildInputV1 {
            revision_id,
            revision_hash,
            revision_ingredients_hash: hash('0'),
            required_claim_components: components.clone(),
            objectives: vec![HypothesisVerificationPlanObjectiveInputV1 {
                objective_hash: hash('1'),
                verification_contract: contract.clone(),
                claim_component_member_hashes: component_hashes.clone(),
                outcome_requirement:
                    HypothesisVerificationObjectiveOutcomeRequirementV1::SatisfyOrFalsifyBoundRequiredComponents,
            }],
            proof_paths: vec![HypothesisVerificationPlanPathInputV1 {
                path_key: "authorization-proof".into(),
                members: vec![HypothesisVerificationPlanPathMemberInputV1 {
                    objective_id,
                    role:
                        HypothesisVerificationPlanPathMemberRoleV1::RequiredProofAndPathFalsifier,
                    falsifier_claim_component_member_hashes: component_hashes.clone(),
                }],
            }],
            outer_aggregation_policy_version: 1,
            outer_aggregation_policy_digest: hash('2'),
        },
    ))
    .unwrap();

    let transition_hash = hash('3');
    let mut mutation = CandidateHypothesisMutation::parse_controller_artifact(json!({
        "proposal_id": Uuid::from_u128(0xb005),
        "organization_id": organization_id,
        "semantic_key_hash": hash('4'),
        "operator_rank": 0,
        "state": "proposed",
        "proof_refs": [{"kind": "tool_truth_evidence", "id": "evidence:1"}],
        "refutation_refs": [],
        "generation_transition_hash": transition_hash,
    }))
    .unwrap();
    mutation.reseal();

    let roots = ToolTruthRootFamilyV1::ALL
        .iter()
        .enumerate()
        .map(|(ordinal, family)| CandidateAuthorityRootGateV1 {
            root_family: *family,
            graph_hash: hash(char::from_digit((ordinal + 1) as u32, 16).unwrap()),
            semantic_hash: hash(char::from_digit((ordinal + 5) as u32, 16).unwrap()),
            freshness_hash: hash(char::from_digit((ordinal + 9) as u32, 16).unwrap()),
            temporal_hash: hash(char::from_digit((ordinal + 11) as u32, 16).unwrap()),
            target_state_epoch_hash: hash('e'),
            temporal_status: TemporalValidityStatus::Fresh,
            member_hash: hash(char::from_digit((ordinal + 1) as u32, 16).unwrap()),
        })
        .collect::<Vec<_>>();
    let root_members = roots
        .iter()
        .map(|root| root.member_hash.clone())
        .collect::<Vec<_>>();
    let feed_member_hash = hash('5');
    let synthesis_nodes = [
        CandidateCoverageSynthesisNodeKindV1::CrossChunk,
        CandidateCoverageSynthesisNodeKindV1::CrossInputPartition,
        CandidateCoverageSynthesisNodeKindV1::CrossInputReduce,
        CandidateCoverageSynthesisNodeKindV1::CrossDimensionReduce,
        CandidateCoverageSynthesisNodeKindV1::GlobalSemanticRoot,
    ]
    .into_iter()
    .enumerate()
    .map(|(ordinal, node_kind)| {
        let child = if ordinal == 0 {
            hash('6')
        } else {
            hash(char::from_digit((ordinal + 5) as u32, 16).unwrap())
        };
        CandidateCoverageSynthesisNodeV1 {
            node_hash: hash(char::from_digit((ordinal + 6) as u32, 16).unwrap()),
            node_kind,
            expected_child_hashes: vec![child.clone()],
            observed_child_hashes: vec![child],
            worker_run_id: Uuid::from_u128(0xc000 + ordinal as u128),
            primary_analyst_worker_run_ids: vec![Uuid::from_u128(0xd000)],
            transitive_descendant_worker_run_ids: vec![Uuid::from_u128(0xd001 + ordinal as u128)],
            outcome: CandidateCoverageOutcomeV1::Adequate,
        }
    })
    .collect::<Vec<_>>();
    let synthesis_hashes = synthesis_nodes
        .iter()
        .map(|node| node.node_hash.clone())
        .collect::<Vec<_>>();
    let disposition_hash = hash('d');
    let relation_hash = hash('e');
    let mutation_hash = mutation.mutation_hash.clone();
    let contract_hash = contract.contract_hash().to_owned();
    let plan_hash = plan.plan_hash().to_owned();
    let controller_worker_run_id = Uuid::from_u128(0xbeef);

    CandidateGateSnapshot::from_repository_material(FrozenCandidateGateMaterialV1 {
        snapshot_id: Uuid::from_u128(0xb006),
        snapshot_hash: hash('6'),
        candidate_snapshot_authority_hash: hash('7'),
        operation_id,
        organization_id,
        authority: CandidateAuthorityGateV1 {
            disposition: CandidateAuthoritySnapshotDispositionV1::SealedReady,
            bundle_seal_id: Uuid::from_u128(0xb007),
            operation_id,
            organization_id,
            checked_request_id: Uuid::from_u128(0xb008),
            gate_request_id: Uuid::from_u128(0xb009),
            caller_filtered_or_reused_guard: false,
            old_consistent_row_used: false,
            root_set: CandidateExactSetSealV1::seal("candidate_roots.v1", root_members),
            bundle_member_set: CandidateExactSetSealV1::seal(
                "candidate_bundle_members.v1",
                vec![hash('8')],
            ),
            receipt_set: CandidateExactSetSealV1::seal("candidate_receipts.v1", vec![hash('9')]),
            temporal_decision_set: CandidateExactSetSealV1::seal(
                "candidate_temporal_decisions.v1",
                vec![hash('a')],
            ),
            roots,
            current_target_state_epoch_set_hash: hash('b'),
            snapshot_target_state_epoch_set_hash: hash('b'),
            gate_temporal_reevaluation_hash: hash('c'),
        },
        knowledge_feed: CandidateKnowledgeFeedGateV1 {
            required_member_set: CandidateExactSetSealV1::seal(
                "candidate_feed_required.v1",
                vec![feed_member_hash.clone()],
            ),
            signed_snapshot_set: CandidateExactSetSealV1::seal(
                "candidate_feed_snapshots.v1",
                vec![feed_member_hash.clone()],
            ),
            product_version_census: CandidateExactSetSealV1::seal(
                "candidate_products.v1",
                vec![hash('6')],
            ),
            match_census: CandidateExactSetSealV1::seal("candidate_matches.v1", vec![hash('7')]),
            signature_algorithm_set: CandidateExactSetSealV1::seal(
                "candidate_signature_algorithms.v1",
                vec![hash('8')],
            ),
            members: vec![CandidateKnowledgeFeedMemberV1 {
                member_hash: feed_member_hash,
                product_version_known: true,
                signature_valid: true,
                provenance_valid: true,
                age_valid_at_gate: true,
                key_current_and_not_revoked: true,
            }],
            catalog_policy_seal_hash: hash('9'),
            trust_store_hash: hash('a'),
            snapshot_trust_store_hash: hash('a'),
            key_revocation_epoch_hash: hash('b'),
            snapshot_key_revocation_epoch_hash: hash('b'),
            gate_reevaluation_hash: hash('c'),
            obligation_set_hash: hash('d'),
        },
        attempt: CandidateAttemptGateV1 {
            active_attempt_id: Uuid::from_u128(0xb010),
            active_attempt_ordinal: 0,
            active_attempt_unique: true,
            prior_attempts: vec![],
            prior_terminal_attempt_chain_hash: exact_set_hash(
                "candidate_prior_terminal_attempt_chain.v1",
                &[],
            ),
            material_attempt_ids: vec![Uuid::from_u128(0xb010); 8],
        },
        read: CandidateReadGateV1 {
            input_set: CandidateExactSetSealV1::seal("candidate_inputs.v1", vec![hash('1')]),
            chunk_set: CandidateExactSetSealV1::seal("candidate_chunks.v1", vec![hash('2')]),
            page_receipt_set: CandidateExactSetSealV1::seal(
                "candidate_page_receipts.v1",
                vec![hash('3')],
            ),
            server_read_receipt_set: CandidateExactSetSealV1::seal(
                "candidate_server_reads.v1",
                vec![hash('4')],
            ),
            source_bytes_complete: true,
            context_truncated: false,
            caller_claimed_read_complete: false,
        },
        coverage: CandidateCoverageGateV1 {
            h1_proposal_set: CandidateExactSetSealV1::seal("candidate_h1.v1", vec![hash('5')]),
            per_input_h1_disposition_set: CandidateExactSetSealV1::seal(
                "candidate_h1_dispositions.v1",
                vec![hash('6')],
            ),
            checklist_member_set: CandidateExactSetSealV1::seal(
                "candidate_checklist.v1",
                vec![hash('7')],
            ),
            chunk_partition_set: CandidateExactSetSealV1::seal(
                "candidate_partitions.v1",
                vec![hash('8')],
            ),
            expected_subreview_set: CandidateExactSetSealV1::seal(
                "candidate_subreviews_expected.v1",
                vec![hash('9')],
            ),
            observed_subreview_set: CandidateExactSetSealV1::seal(
                "candidate_subreviews_observed.v1",
                vec![hash('9')],
            ),
            synthesis_node_set: CandidateExactSetSealV1::seal(
                "candidate_synthesis.v1",
                synthesis_hashes,
            ),
            synthesis_nodes,
            per_input_review_set: CandidateExactSetSealV1::seal(
                "candidate_reviews.v1",
                vec![hash('a')],
            ),
            h2_proposal_set: CandidateExactSetSealV1::seal("candidate_h2.v1", vec![hash('b')]),
            global_review_hash: hash('c'),
            global_review_outcome: CandidateCoverageOutcomeV1::Adequate,
            unresolved_feed_dependent_checklist_members: 0,
            missed_hypothesis: false,
            sampling_used: false,
            retry_limit_reached: false,
        },
        proposal_census: CandidateExactSetSealV1::seal("candidate_proposals.v1", vec![hash('d')]),
        critic_census: CandidateExactSetSealV1::seal("candidate_critics.v1", vec![hash('e')]),
        controller_decision_set: CandidateExactSetSealV1::seal(
            "candidate_controller_decisions.v1",
            vec![hash('f')],
        ),
        mutations: vec![mutation],
        mutation_set: CandidateExactSetSealV1::seal("candidate_mutations.v1", vec![mutation_hash]),
        compiled: CandidateCompiledAuthorityV1 {
            claim_components: components,
            claim_component_set: CandidateExactSetSealV1::seal(
                "candidate_claim_components.v1",
                component_hashes,
            ),
            verification_contracts: vec![contract],
            verification_contract_set: CandidateExactSetSealV1::seal(
                "candidate_contracts.v1",
                vec![contract_hash],
            ),
            verification_plans: vec![plan],
            verification_plan_set: CandidateExactSetSealV1::seal(
                "candidate_plans.v1",
                vec![plan_hash],
            ),
        },
        repository_hashes: CandidateRepositoryGateHashesV1 {
            tool_truth_authority_root_set_hash: hash('1'),
            tool_truth_authority_bundle_member_set_hash: hash('2'),
            tool_truth_authority_receipt_set_hash: hash('3'),
            denominator_graph_bundle_hash: hash('4'),
            semantic_authority_bundle_hash: hash('5'),
            freshness_attestation_bundle_hash: hash('6'),
            temporal_validity_bundle_hash: hash('7'),
            temporal_validity_policy_digest: hash('8'),
            temporal_validity_decision_set_hash: hash('9'),
            knowledge_feed_catalog_policy_seal_hash: hash('a'),
            knowledge_feed_required_member_set_hash: hash('b'),
            knowledge_feed_signature_algorithm_set_hash: hash('c'),
            knowledge_feed_trust_store_hash: hash('d'),
            knowledge_feed_key_revocation_epoch_hash: hash('e'),
            knowledge_feed_snapshot_set_hash: hash('f'),
            product_version_census_hash: hash('0'),
            knowledge_feed_match_census_hash: hash('1'),
            stale_revalidation_obligation_set_hash: hash('2'),
            knowledge_feed_obligation_set_hash: hash('3'),
            prior_terminal_attempt_chain_hash: exact_set_hash(
                "candidate_prior_terminal_attempt_chain.v1",
                &[],
            ),
            proposal_census_hash: hash('4'),
            critic_census_hash: hash('5'),
            controller_decision_set_hash: hash('6'),
            input_chunk_census_set_hash: hash('7'),
            coverage_subreview_census_set_hash: hash('8'),
            coverage_synthesis_census_set_hash: hash('9'),
            coverage_global_semantic_root_hash: hash('a'),
            coverage_global_review_hash: hash('b'),
            coverage_review_set_hash: hash('c'),
            coverage_checklist_set_hash: hash('d'),
            generation_transition_set_hash: hash('e'),
        },
        input_dispositions: vec![InputProcessingDispositionDecision {
            input_id: Uuid::from_u128(0xb011),
            disposition: InputProcessingDispositionV1::Analyzed,
            decision_hash: disposition_hash.clone(),
        }],
        input_disposition_set: CandidateExactSetSealV1::seal(
            "candidate_input_dispositions.v1",
            vec![disposition_hash],
        ),
        input_relations: vec![InputHypothesisRelationDecision {
            input_id: Uuid::from_u128(0xb011),
            hypothesis_root_id: Uuid::from_u128(0xb012),
            relation: InputHypothesisRelationKindV1::CreatesHypothesis,
            decision_hash: relation_hash.clone(),
        }],
        input_relation_set: CandidateExactSetSealV1::seal(
            "candidate_input_relations.v1",
            vec![relation_hash],
        ),
        generation_transition_set: CandidateExactSetSealV1::seal(
            "candidate_generation_transitions.v1",
            vec![transition_hash],
        ),
        planning_ready: true,
        capability_assessment_present: false,
        final_submitter_worker_run_id: controller_worker_run_id,
        controller_worker_run_id,
        controller_dispatch_worker_run_id: Uuid::from_u128(0xbeee),
    })
}

fn assert_gate_code(
    snapshot: &golish_agent_kit::harness::hypothesis_registry::CandidateGateSnapshot,
    expected: &str,
) {
    use golish_agent_kit::harness::hypothesis_registry::validate_candidate_gate;
    assert_eq!(
        validate_candidate_gate(snapshot).unwrap_err().code(),
        expected
    );
}

#[test]
fn gate_accepts_only_the_closed_frozen_snapshot() {
    use golish_agent_kit::harness::hypothesis_registry::validate_candidate_gate;
    let pass = validate_candidate_gate(&gate_fixture()).unwrap();
    assert_eq!(pass.mutation_set.len(), 1);
    assert_eq!(pass.active_analysis_attempt_ordinal, 0);
}

#[test]
fn candidate_authority_bundle_gate_rejects_scope_root_and_guard_drift() {
    use golish_agent_kit::harness::hypothesis_registry::CandidateAuthoritySnapshotDispositionV1;
    for corrupt in 0..5 {
        let mut snapshot = gate_fixture();
        let material = snapshot.material_for_host_tests_mut();
        match corrupt {
            0 => material.authority.organization_id = Uuid::from_u128(999),
            1 => {
                material.authority.roots.pop();
            }
            2 => material.authority.caller_filtered_or_reused_guard = true,
            3 => material.authority.old_consistent_row_used = true,
            4 => {
                material.authority.disposition =
                    CandidateAuthoritySnapshotDispositionV1::BlockedSemanticInvalid;
            }
            _ => unreachable!(),
        }
        assert_gate_code(&snapshot, "HYPOTHESIS_CANDIDATE_AUTHORITY_BUNDLE_INVALID");
    }
}

#[test]
fn candidate_temporal_validity_gate_rechecks_status_epoch_and_decision_census() {
    use golish_pentest_domain::tool_truth::TemporalValidityStatus;
    for status in [
        TemporalValidityStatus::Expired,
        TemporalValidityStatus::MixedEpoch,
        TemporalValidityStatus::SkewExceeded,
    ] {
        let mut snapshot = gate_fixture();
        snapshot.material_for_host_tests_mut().authority.roots[0].temporal_status = status;
        assert_gate_code(&snapshot, "HYPOTHESIS_CANDIDATE_TEMPORAL_VALIDITY_INVALID");
    }
    let mut snapshot = gate_fixture();
    snapshot
        .material_for_host_tests_mut()
        .authority
        .current_target_state_epoch_set_hash = hash('0');
    assert_gate_code(&snapshot, "HYPOTHESIS_CANDIDATE_TEMPORAL_VALIDITY_INVALID");
}

#[test]
fn candidate_knowledge_feed_gate_rejects_denominator_signature_age_and_epoch_drift() {
    for corrupt in 0..5 {
        let mut snapshot = gate_fixture();
        let feed = &mut snapshot.material_for_host_tests_mut().knowledge_feed;
        match corrupt {
            0 => feed.required_member_set.observed_member_hashes.clear(),
            1 => feed.members[0].signature_valid = false,
            2 => feed.members[0].age_valid_at_gate = false,
            3 => feed.members[0].product_version_known = false,
            4 => feed.snapshot_key_revocation_epoch_hash = hash('0'),
            _ => unreachable!(),
        }
        assert_gate_code(&snapshot, "HYPOTHESIS_CANDIDATE_KNOWLEDGE_FEED_INVALID");
    }
}

#[test]
fn candidate_attempt_chain_gate_rejects_forks_and_cross_attempt_receipts() {
    let mut snapshot = gate_fixture();
    let attempt = &mut snapshot.material_for_host_tests_mut().attempt;
    attempt.active_attempt_ordinal = 1;
    assert_gate_code(&snapshot, "HYPOTHESIS_CANDIDATE_ATTEMPT_CHAIN_INVALID");

    let mut snapshot = gate_fixture();
    snapshot
        .material_for_host_tests_mut()
        .attempt
        .material_attempt_ids[0] = Uuid::from_u128(77);
    assert_gate_code(&snapshot, "HYPOTHESIS_CANDIDATE_ATTEMPT_CHAIN_INVALID");
}

#[test]
fn chunk_census_gate_rejects_silent_truncation_and_caller_read_claims() {
    let mut snapshot = gate_fixture();
    snapshot
        .material_for_host_tests_mut()
        .read
        .chunk_set
        .observed_member_hashes
        .clear();
    assert_gate_code(&snapshot, "HYPOTHESIS_CANDIDATE_CHUNK_CENSUS_INVALID");

    for corrupt in 0..2 {
        let mut snapshot = gate_fixture();
        let read = &mut snapshot.material_for_host_tests_mut().read;
        if corrupt == 0 {
            read.context_truncated = true;
        } else {
            read.caller_claimed_read_complete = true;
        }
        assert_gate_code(&snapshot, "HYPOTHESIS_CANDIDATE_READ_RECEIPT_INVALID");
    }
}

#[test]
fn hypothesis_coverage_subreview_gate_requires_checklist_by_partition_exact_set() {
    let mut snapshot = gate_fixture();
    snapshot
        .material_for_host_tests_mut()
        .coverage
        .observed_subreview_set
        .observed_member_hashes = vec![hash('0')];
    assert_gate_code(&snapshot, "HYPOTHESIS_CANDIDATE_COVERAGE_SUBREVIEW_INVALID");
}

#[test]
fn hypothesis_coverage_synthesis_gate_requires_recursive_children_and_worker_separation() {
    let mut snapshot = gate_fixture();
    snapshot
        .material_for_host_tests_mut()
        .coverage
        .synthesis_nodes[2]
        .observed_child_hashes
        .clear();
    assert_gate_code(&snapshot, "HYPOTHESIS_CANDIDATE_COVERAGE_SYNTHESIS_INVALID");

    let mut snapshot = gate_fixture();
    let node = &mut snapshot
        .material_for_host_tests_mut()
        .coverage
        .synthesis_nodes[1];
    node.transitive_descendant_worker_run_ids = vec![node.worker_run_id];
    assert_gate_code(&snapshot, "HYPOTHESIS_CANDIDATE_COVERAGE_SYNTHESIS_INVALID");
}

#[test]
fn hypothesis_coverage_gate_never_turns_miss_sampling_or_blocker_into_adequate() {
    use golish_agent_kit::harness::hypothesis_registry::CandidateCoverageOutcomeV1;
    for corrupt in 0..4 {
        let mut snapshot = gate_fixture();
        let coverage = &mut snapshot.material_for_host_tests_mut().coverage;
        match corrupt {
            0 => coverage.missed_hypothesis = true,
            1 => coverage.sampling_used = true,
            2 => coverage.unresolved_feed_dependent_checklist_members = 1,
            3 => coverage.global_review_outcome = CandidateCoverageOutcomeV1::Blocked,
            _ => unreachable!(),
        }
        assert_gate_code(&snapshot, "HYPOTHESIS_CANDIDATE_COVERAGE_REVIEW_INVALID");
    }
}

#[test]
fn zero_proposal_special_case_gate_uses_the_same_empty_exact_set_rule() {
    use golish_agent_kit::harness::hypothesis_registry::{
        validate_candidate_gate, CandidateExactSetSealV1,
    };
    let mut snapshot = gate_fixture();
    let material = snapshot.material_for_host_tests_mut();
    material.mutations.clear();
    material.mutation_set = CandidateExactSetSealV1::seal("candidate_mutations.v1", vec![]);
    material.generation_transition_set =
        CandidateExactSetSealV1::seal("candidate_generation_transitions.v1", vec![]);
    material.coverage.h1_proposal_set = CandidateExactSetSealV1::seal("candidate_h1.v1", vec![]);
    material.coverage.h2_proposal_set = CandidateExactSetSealV1::seal("candidate_h2.v1", vec![]);
    material.proposal_census = CandidateExactSetSealV1::seal("candidate_proposals.v1", vec![]);
    material.controller_decision_set =
        CandidateExactSetSealV1::seal("candidate_controller_decisions.v1", vec![]);
    material.compiled.claim_components.clear();
    material.compiled.claim_component_set =
        CandidateExactSetSealV1::seal("candidate_claim_components.v1", vec![]);
    material.compiled.verification_contracts.clear();
    material.compiled.verification_contract_set =
        CandidateExactSetSealV1::seal("candidate_contracts.v1", vec![]);
    material.compiled.verification_plans.clear();
    material.compiled.verification_plan_set =
        CandidateExactSetSealV1::seal("candidate_plans.v1", vec![]);
    material.input_relations.clear();
    material.input_relation_set =
        CandidateExactSetSealV1::seal("candidate_input_relations.v1", vec![]);
    assert!(validate_candidate_gate(&snapshot).is_ok());
}

#[test]
fn verification_contract_gate_rejects_contract_set_substitution() {
    let mut snapshot = gate_fixture();
    snapshot
        .material_for_host_tests_mut()
        .compiled
        .verification_contracts
        .clear();
    assert_gate_code(
        &snapshot,
        "HYPOTHESIS_VERIFICATION_CONTRACT_EXACT_SET_INVALID",
    );
}

#[test]
fn paired_control_binding_gate_is_part_of_the_sealed_contract_authority() {
    let mut snapshot = gate_fixture();
    snapshot
        .material_for_host_tests_mut()
        .compiled
        .verification_contract_set
        .observed_member_hashes = vec![hash('0')];
    assert_gate_code(
        &snapshot,
        "HYPOTHESIS_VERIFICATION_CONTRACT_EXACT_SET_INVALID",
    );
}

#[test]
fn hypothesis_claim_component_gate_rejects_denominator_and_revision_substitution() {
    let mut snapshot = gate_fixture();
    snapshot
        .material_for_host_tests_mut()
        .compiled
        .claim_component_set
        .observed_member_hashes
        .pop();
    assert_gate_code(&snapshot, "HYPOTHESIS_CLAIM_COMPONENT_EXACT_SET_INVALID");
}

#[test]
fn hypothesis_verification_plan_gate_requires_objective_component_path_union() {
    let mut snapshot = gate_fixture();
    snapshot
        .material_for_host_tests_mut()
        .compiled
        .verification_plans
        .clear();
    assert_gate_code(&snapshot, "HYPOTHESIS_VERIFICATION_PLAN_EXACT_SET_INVALID");

    let mut snapshot = gate_fixture();
    snapshot
        .material_for_host_tests_mut()
        .capability_assessment_present = true;
    assert_gate_code(&snapshot, "HYPOTHESIS_VERIFICATION_PLAN_EXACT_SET_INVALID");
}

#[test]
fn gate_blocks_gap_as_refutation_and_application_or_feed_context_as_proof() {
    use golish_agent_kit::harness::hypothesis_registry::RevisionSourceRef;
    let cases = [
        (
            RevisionSourceRef::ApplicationContext("application:item".into()),
            true,
            "HYPOTHESIS_APPLICATION_CONTEXT_IS_NOT_PROOF",
        ),
        (
            RevisionSourceRef::KnowledgeSignal("feed:item".into()),
            true,
            "HYPOTHESIS_KNOWLEDGE_SIGNAL_IS_NOT_PROOF",
        ),
        (
            RevisionSourceRef::Gap("gap:item".into()),
            false,
            "HYPOTHESIS_GAP_IS_NOT_REFUTATION",
        ),
    ];
    for (source, proof, code) in cases {
        let mut snapshot = gate_fixture();
        let material = snapshot.material_for_host_tests_mut();
        if proof {
            material.mutations[0].proof_refs = vec![source];
        } else {
            material.mutations[0].refutation_refs = vec![source];
        }
        material.mutations[0].reseal();
        material.mutation_set =
            golish_agent_kit::harness::hypothesis_registry::CandidateExactSetSealV1::seal(
                "candidate_mutations.v1",
                vec![material.mutations[0].mutation_hash.clone()],
            );
        assert_gate_code(&snapshot, code);
    }
}

#[test]
fn candidate_terminal_state_artifacts_have_stable_forbidden_codes() {
    use golish_agent_kit::harness::hypothesis_registry::CandidateHypothesisMutation;
    for (state, expected) in [
        ("verified", "HYPOTHESIS_CANDIDATE_TERMINAL_STATE_FORBIDDEN"),
        ("refuted", "HYPOTHESIS_CANDIDATE_TERMINAL_STATE_FORBIDDEN"),
        ("invalid", "HYPOTHESIS_INVALID_STATE_SERVER_ONLY"),
    ] {
        let artifact = json!({
            "proposal_id": Uuid::from_u128(1),
            "organization_id": Uuid::from_u128(2),
            "semantic_key_hash": hash('1'),
            "operator_rank": 0,
            "state": state,
            "proof_refs": [],
            "refutation_refs": [],
            "generation_transition_hash": hash('2'),
        });
        assert_eq!(
            CandidateHypothesisMutation::parse_controller_artifact(artifact)
                .unwrap_err()
                .code(),
            expected,
        );
    }
}

#[test]
fn gate_rejects_cross_org_mutation_and_transition_drift() {
    let mut snapshot = gate_fixture();
    snapshot.material_for_host_tests_mut().mutations[0].organization_id = Uuid::from_u128(999);
    assert_gate_code(&snapshot, "HYPOTHESIS_CANDIDATE_SEMANTIC_REDUCER_INVALID");

    let mut snapshot = gate_fixture();
    snapshot
        .material_for_host_tests_mut()
        .generation_transition_set
        .observed_member_hashes = vec![hash('0')];
    assert_gate_code(
        &snapshot,
        "HYPOTHESIS_GENERATION_TRANSITION_EXACT_SET_INVALID",
    );
}
