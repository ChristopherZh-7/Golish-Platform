use super::*;
use golish_core::hypothesis_semantic_key::ClaimPolarity;
use golish_core::hypothesis_verification::{
    compile_claim_components_v1, HypothesisClaimComponentInputV1, HypothesisClaimComponentKindV1,
    HypothesisClaimComponentOutcomeKindV1, HypothesisClaimComponentOutcomeV1,
    HypothesisVerificationObjectiveOutcomeBuildInputV1,
    HypothesisVerificationObjectiveOutcomeKindV1,
    HypothesisVerificationObjectiveOutcomeRequirementV1, HypothesisVerificationObjectiveOutcomeV1,
    HypothesisVerificationPlanBuildInputV1, HypothesisVerificationPlanObjectiveInputV1,
    HypothesisVerificationPlanPathInputV1, HypothesisVerificationPlanPathMemberInputV1,
    HypothesisVerificationPlanPathMemberRoleV1, HypothesisVerificationPlanV1,
    ObjectiveOutcomeViewV1,
};
use golish_core::verification_contract::{
    CanonicalJsonObject, ContractCombinatorV1, OrderedInterleavingPolicyV1, OrderedResetPolicyV1,
    OrderedSequenceStepInputV1, OrderedSessionScopeV1, PairedDifferentialBindingInputV1,
    PredicateComponentInputV1, VerificationContractBuildInputV1, VerificationContractV1,
    VerificationControlInputV1,
};
use std::collections::BTreeSet;
use uuid::Uuid;

fn digest(nibble: char) -> String {
    format!("sha256:{}", nibble.to_string().repeat(64))
}

fn predicate(key: &str, nibble: char) -> PredicateComponentInputV1 {
    PredicateComponentInputV1 {
        semantic_key: key.to_owned(),
        predicate_schema: "verification_campaign.test.v1".to_owned(),
        predicate_version: 1,
        normalized_arguments: CanonicalJsonObject::parse(&format!(r#"{{"key":"{key}"}}"#))
            .expect("fixture JSON is canonicalizable"),
        expected_polarity: ClaimPolarity::Positive,
        prerequisite_hash: digest(nibble),
    }
}

fn contract(combinator: ContractCombinatorV1) -> VerificationContractV1 {
    let mut input = VerificationContractBuildInputV1 {
        revision_id: Uuid::from_u128(0x101),
        revision_hash: digest('1'),
        objective_id: Uuid::from_u128(0x202),
        combinator,
        predicate_components: vec![predicate("alpha", 'a'), predicate("bravo", 'b')],
        required_controls: Vec::new(),
        paired_differential_bindings: Vec::new(),
        ordered_steps: Vec::new(),
        stopping_criteria_hash: digest('2'),
        compiler_digest: digest('3'),
        rule_digest: digest('4'),
        policy_snapshot_hash: digest('5'),
    };
    match combinator {
        ContractCombinatorV1::AllOf | ContractCombinatorV1::AnyOf => {}
        ContractCombinatorV1::PairedDifferential => {
            input.required_controls = vec![VerificationControlInputV1 {
                control_id: "negative-control".to_owned(),
                control_version: 1,
                control_contract_hash: digest('6'),
            }];
            input.paired_differential_bindings = vec![PairedDifferentialBindingInputV1 {
                pair_key: "alpha-vs-bravo".to_owned(),
                baseline_component_key: "alpha".to_owned(),
                variant_component_key: "bravo".to_owned(),
                required_control_id: "negative-control".to_owned(),
                required_control_version: 1,
                required_control_contract_hash: digest('6'),
                comparator_rule_id: "strict-difference".to_owned(),
                comparator_rule_version: 1,
                comparator_rule_digest: digest('7'),
            }];
        }
        ContractCombinatorV1::OrderedSequence => {
            input.ordered_steps = vec![
                OrderedSequenceStepInputV1 {
                    step_ordinal: 0,
                    component_key: "alpha".to_owned(),
                    predecessor_step_ordinal: None,
                    session_binding_key_schema: "execution-session.v1".to_owned(),
                    session_binding_key_version: 1,
                    session_scope: OrderedSessionScopeV1::SameExecutionSession,
                    interleaving_policy: OrderedInterleavingPolicyV1::Forbid,
                    reset_policy: OrderedResetPolicyV1::RestartAtStepZero,
                },
                OrderedSequenceStepInputV1 {
                    step_ordinal: 1,
                    component_key: "bravo".to_owned(),
                    predecessor_step_ordinal: Some(0),
                    session_binding_key_schema: "execution-session.v1".to_owned(),
                    session_binding_key_version: 1,
                    session_scope: OrderedSessionScopeV1::SameExecutionSession,
                    interleaving_policy: OrderedInterleavingPolicyV1::Forbid,
                    reset_policy: OrderedResetPolicyV1::RestartAtStepZero,
                },
            ];
        }
    }
    VerificationContractV1::compile(input).expect("fixture contract compiles")
}

fn plan(contract: VerificationContractV1) -> HypothesisVerificationPlanV1 {
    let components = compile_claim_components_v1(
        contract.revision_id(),
        contract.revision_hash().to_owned(),
        1,
        digest('8'),
        vec![
            HypothesisClaimComponentInputV1 {
                component_key: "claim".to_owned(),
                kind: HypothesisClaimComponentKindV1::ClaimClause,
                canonical_fragment_hash: digest('9'),
                canonical_condition_hash: digest('a'),
                required: true,
            },
            HypothesisClaimComponentInputV1 {
                component_key: "impact".to_owned(),
                kind: HypothesisClaimComponentKindV1::ImpactQualifier,
                canonical_fragment_hash: digest('b'),
                canonical_condition_hash: digest('c'),
                required: true,
            },
        ],
    )
    .expect("claim components compile");
    let component_hashes = components
        .iter()
        .map(|component| component.member_hash().to_owned())
        .collect::<Vec<_>>();
    HypothesisVerificationPlanV1::compile(HypothesisVerificationPlanBuildInputV1 {
        revision_id: contract.revision_id(),
        revision_hash: contract.revision_hash().to_owned(),
        revision_ingredients_hash: digest('d'),
        required_claim_components: components,
        objectives: vec![HypothesisVerificationPlanObjectiveInputV1 {
            objective_hash: digest('e'),
            verification_contract: contract.clone(),
            claim_component_member_hashes: component_hashes.clone(),
            outcome_requirement:
                HypothesisVerificationObjectiveOutcomeRequirementV1::SatisfyOrFalsifyBoundRequiredComponents,
        }],
        proof_paths: vec![HypothesisVerificationPlanPathInputV1 {
            path_key: "primary".to_owned(),
            members: vec![HypothesisVerificationPlanPathMemberInputV1 {
                objective_id: contract.objective_id(),
                role:
                    HypothesisVerificationPlanPathMemberRoleV1::RequiredProofAndPathFalsifier,
                falsifier_claim_component_member_hashes: vec![component_hashes[0].clone()],
            }],
        }],
        outer_aggregation_policy_version: 1,
        outer_aggregation_policy_digest: digest('f'),
    })
    .expect("fixture verification plan compiles")
}

fn bindings(
    contract: &VerificationContractV1,
    plan: &HypothesisVerificationPlanV1,
) -> Vec<ClaimComponentBindingV1> {
    let claims = plan.objectives()[0].claim_component_member_hashes();
    contract
        .predicate_components()
        .iter()
        .zip(claims)
        .map(|(predicate, claim)| ClaimComponentBindingV1 {
            predicate_component_member_hash: predicate.member_hash().to_owned(),
            claim_component_member_hash: claim.clone(),
        })
        .collect()
}

fn action_contract(
    contract: &VerificationContractV1,
    plan: &HypothesisVerificationPlanV1,
    kind: PreparedActionKindV1,
) -> ActionOracleContractV1 {
    ActionOracleContractV1::seal(
        contract,
        plan.objectives()[0].member_hash().to_owned(),
        Uuid::from_u128(0x303),
        bindings(contract, plan),
        kind,
    )
    .expect("action oracle contract seals")
}

fn receipt(
    contract: &ActionOracleContractV1,
    outcomes: &[ComponentOracleOutcomeV1],
) -> ReconciledExecutionReceiptV1 {
    ReconciledExecutionReceiptV1 {
        receipt_version: 1,
        prepared_action_id: contract.prepared_action_id,
        verification_contract_hash: contract.verification_contract_hash.clone(),
        execution_key: ExecutionKeyV1 {
            prepared_action_id: contract.prepared_action_id,
            authorization_receipt_id: Uuid::from_u128(0x404),
            execution_ordinal: 0,
        },
        landing_state: ExecutionLandingStateV1::LandedReconciled,
        precondition: PreconditionStatusV1::Satisfied,
        control: if contract.required_control_member_hashes.is_empty() {
            ControlValidityV1::NotRequired
        } else {
            ControlValidityV1::Valid
        },
        completeness: ObservationCompletenessV1::Complete,
        cleanup_complete: true,
        predicate_observations: contract
            .predicate_component_member_hashes
            .iter()
            .zip(outcomes)
            .enumerate()
            .map(|(ordinal, (member_hash, outcome))| PredicateObservationV1 {
                predicate_component_member_hash: member_hash.clone(),
                predicate_ordinal: ordinal as u32,
                outcome: *outcome,
                deterministic_negative: *outcome == ComponentOracleOutcomeV1::Refutation,
                observation_window_complete: true,
            })
            .collect(),
        observed_control_member_hashes: contract.required_control_member_hashes.clone(),
        paired_relation: None,
        ordered_sequence: Vec::new(),
        concurrent_group_receipt: None,
    }
}

fn assessment_and_obligations(
    combinator: ContractCombinatorV1,
    outcomes: &[ComponentOracleOutcomeV1],
) -> (
    VerificationContractV1,
    OracleCensusV1,
    ObligationDispositionSetV1,
) {
    let contract = contract(combinator);
    let plan = plan(contract.clone());
    let action = action_contract(&contract, &plan, PreparedActionKindV1::SingleActionV1);
    let assessment = reduce_action_oracle(&action, &receipt(&action, outcomes))
        .expect("oracle assessment reduces");
    let denominator = CampaignCoverageDenominatorSealV1::seal(
        &contract,
        &plan.objectives()[0],
        bindings(&contract, &plan),
        false,
    )
    .expect("denominator seals");
    let results = denominator
        .members
        .iter()
        .map(|member| CoverageResultV1 {
            denominator_member_hash: member.member_hash.clone(),
            status: CoverageResultStatusV1::Tested {
                prepared_action_id: action.prepared_action_id,
                capability_receipt_hash: digest('1'),
                oracle_assessment_hash: assessment.assessment_hash.clone(),
            },
        })
        .collect();
    let obligations = ObligationDispositionSetV1 {
        denominator,
        results,
    };
    let census = OracleCensusV1 {
        verification_contract_hash: contract.contract_hash().to_owned(),
        authority: ArtifactAuthorityV1::Canonical,
        assessments: vec![assessment],
    };
    (contract, census, obligations)
}

fn gate_action(disposition: PreparedActionDisposition) -> GateActionTruthV1 {
    GateActionTruthV1 {
        prepared_action_id: Uuid::from_u128(0x505),
        authority: ArtifactAuthorityV1::Canonical,
        disposition,
        authorized: false,
        durable_started: false,
        execution_receipt_hash: None,
        landed_reconciled: false,
        oracle_assessment_hash: None,
        reason_code: None,
        residual_risk_hash: None,
    }
}

#[test]
fn verification_campaign_single_active_action_is_enforced() {
    let mut state = CampaignStateV1::new(Uuid::from_u128(1));
    state.activate_action(Uuid::from_u128(2)).unwrap();
    let err = state
        .activate_action(Uuid::from_u128(3))
        .expect_err("second active action must be rejected");
    assert_eq!(err.code(), "VERIFICATION_CAMPAIGN_ACTIVE_ACTION_CONFLICT");
}

#[test]
fn verification_campaign_no_execution_terminal_dispositions_require_typed_residual() {
    for disposition in [
        PreparedActionDisposition::CompileRejected,
        PreparedActionDisposition::Denied,
        PreparedActionDisposition::Expired,
    ] {
        let mut action = gate_action(disposition);
        assert_eq!(
            validate_gate_action(&action).unwrap_err().code(),
            "VERIFICATION_CAMPAIGN_TERMINAL_RESIDUAL_REQUIRED"
        );
        action.reason_code = Some(ResidualReasonCodeV1::PolicyDenied);
        action.residual_risk_hash = Some(digest('2'));
        validate_gate_action(&action).expect("no fake execution or oracle is required");
        action.execution_receipt_hash = Some(digest('3'));
        assert_eq!(
            validate_gate_action(&action).unwrap_err().code(),
            "VERIFICATION_CAMPAIGN_CONDITIONAL_ARTIFACT_FORBIDDEN"
        );
    }
}

#[test]
fn verification_campaign_authorized_started_without_execution_receipt_blocks() {
    let mut action = gate_action(PreparedActionDisposition::OutcomeUnknown);
    action.authorized = true;
    action.durable_started = true;
    action.reason_code = Some(ResidualReasonCodeV1::OutcomeUnknown);
    action.residual_risk_hash = Some(digest('4'));
    assert_eq!(
        validate_gate_action(&action).unwrap_err().code(),
        "VERIFICATION_CAMPAIGN_EXECUTION_RECEIPT_REQUIRED"
    );
}

#[test]
fn verification_campaign_reconciled_execution_without_oracle_blocks() {
    let mut action = gate_action(PreparedActionDisposition::Succeeded);
    action.authorized = true;
    action.durable_started = true;
    action.execution_receipt_hash = Some(digest('5'));
    action.landed_reconciled = true;
    assert_eq!(
        validate_gate_action(&action).unwrap_err().code(),
        "VERIFICATION_CAMPAIGN_ACTION_ORACLE_REQUIRED"
    );
}

#[test]
fn verification_campaign_invalid_control_partial_coverage_and_unknown_precondition_are_inconclusive(
) {
    for mutation in 0..3 {
        let contract = contract(if mutation == 0 {
            ContractCombinatorV1::PairedDifferential
        } else {
            ContractCombinatorV1::AllOf
        });
        let plan = plan(contract.clone());
        let action = action_contract(&contract, &plan, PreparedActionKindV1::SingleActionV1);
        let mut execution = receipt(
            &action,
            &[
                ComponentOracleOutcomeV1::Proof,
                ComponentOracleOutcomeV1::Proof,
            ],
        );
        match mutation {
            0 => execution.control = ControlValidityV1::Invalid,
            1 => execution.completeness = ObservationCompletenessV1::Partial,
            _ => execution.precondition = PreconditionStatusV1::Unknown,
        }
        let assessment = reduce_action_oracle(&action, &execution).unwrap();
        assert!(assessment
            .predicate_outcomes
            .iter()
            .all(|outcome| outcome.outcome == ComponentOracleOutcomeV1::Inconclusive));
    }
}

#[test]
fn verification_campaign_all_of_and_any_of_use_exact_component_truth_tables() {
    let (all_contract, all_census, all_obligations) = assessment_and_obligations(
        ContractCombinatorV1::AllOf,
        &[
            ComponentOracleOutcomeV1::Proof,
            ComponentOracleOutcomeV1::Inconclusive,
        ],
    );
    assert_eq!(
        adjudicate_campaign(&all_contract, &all_census, &all_obligations)
            .unwrap()
            .outcome,
        ObjectiveCampaignOutcome::Inconclusive
    );
    let (all_contract, all_census, all_obligations) = assessment_and_obligations(
        ContractCombinatorV1::AllOf,
        &[
            ComponentOracleOutcomeV1::Refutation,
            ComponentOracleOutcomeV1::Inconclusive,
        ],
    );
    assert_eq!(
        adjudicate_campaign(&all_contract, &all_census, &all_obligations)
            .unwrap()
            .outcome,
        ObjectiveCampaignOutcome::Refutation
    );
    let (any_contract, any_census, any_obligations) = assessment_and_obligations(
        ContractCombinatorV1::AnyOf,
        &[
            ComponentOracleOutcomeV1::Refutation,
            ComponentOracleOutcomeV1::Inconclusive,
        ],
    );
    assert_eq!(
        adjudicate_campaign(&any_contract, &any_census, &any_obligations)
            .unwrap()
            .outcome,
        ObjectiveCampaignOutcome::Inconclusive
    );
}

#[test]
fn verification_campaign_no_action_compilable_is_adjudicable_without_fake_artifacts() {
    let disposition = RoundDispositionV1::NoActionCompilable {
        reason_code: ResidualReasonCodeV1::AdapterMissing,
        residual_risk_hash: digest('6'),
    };
    validate_round_disposition(&disposition, None, None).unwrap();
    assert_eq!(
        validate_round_disposition(&disposition, Some(&digest('7')), None)
            .unwrap_err()
            .code(),
        "VERIFICATION_CAMPAIGN_CONDITIONAL_ARTIFACT_FORBIDDEN"
    );

    let contract = contract(ContractCombinatorV1::AllOf);
    let plan = plan(contract.clone());
    let denominator = CampaignCoverageDenominatorSealV1::seal(
        &contract,
        &plan.objectives()[0],
        bindings(&contract, &plan),
        false,
    )
    .unwrap();
    let results = denominator
        .members
        .iter()
        .map(|member| CoverageResultV1 {
            denominator_member_hash: member.member_hash.clone(),
            status: CoverageResultStatusV1::Untested {
                residual_risk_hash: digest('8'),
            },
        })
        .collect();
    let adjudication = adjudicate_campaign(
        &contract,
        &OracleCensusV1 {
            verification_contract_hash: contract.contract_hash().to_owned(),
            authority: ArtifactAuthorityV1::Canonical,
            assessments: Vec::new(),
        },
        &ObligationDispositionSetV1 {
            denominator,
            results,
        },
    )
    .unwrap();
    assert_eq!(
        adjudication.outcome,
        ObjectiveCampaignOutcome::ExhaustedWithResiduals
    );
}

#[test]
fn verification_campaign_budget_stop_drains_before_terminal() {
    let mut state = CampaignStateV1::new(Uuid::from_u128(7));
    state
        .request_stop(CampaignStopReasonV1::BudgetExhausted)
        .unwrap();
    assert_eq!(state.phase(), CampaignPhaseV1::Stopping);
    assert_eq!(
        state.terminalize().unwrap_err().code(),
        "VERIFICATION_CAMPAIGN_LOCAL_DRAIN_INCOMPLETE"
    );
    state.begin_draining().unwrap();
    state.complete_drain().unwrap();
    state.terminalize().unwrap();
    assert_eq!(state.phase(), CampaignPhaseV1::Terminal);
}

#[test]
fn verification_campaign_execution_key_and_semantic_no_progress_fingerprint_are_separate() {
    let action_id = Uuid::from_u128(8);
    let first = ExecutionKeyV1 {
        prepared_action_id: action_id,
        authorization_receipt_id: Uuid::from_u128(9),
        execution_ordinal: 0,
    };
    let replay = first.clone();
    let reauthorized = ExecutionKeyV1 {
        authorization_receipt_id: Uuid::from_u128(10),
        ..first.clone()
    };
    assert_eq!(
        execution_key_hash(&first).unwrap(),
        execution_key_hash(&replay).unwrap()
    );
    assert_ne!(
        execution_key_hash(&first).unwrap(),
        execution_key_hash(&reauthorized).unwrap()
    );
    let semantic = SemanticAttemptFingerprintInputV1 {
        objective_id: Uuid::from_u128(11),
        verification_contract_hash: digest('8'),
        required_control_member_hashes: vec![digest('9')],
        action_contract_digest: digest('a'),
        adapter_contract_digest: digest('b'),
        oracle_rule_digest: digest('c'),
        relevant_evidence_member_hashes: vec![digest('d')],
    };
    assert_eq!(
        semantic_attempt_fingerprint(&semantic).unwrap(),
        semantic_attempt_fingerprint(&semantic).unwrap()
    );
}

#[test]
fn verification_campaign_terminal_does_not_wait_for_fact_delta_consumption() {
    let mut state = CampaignStateV1::new(Uuid::from_u128(12));
    state
        .request_stop(CampaignStopReasonV1::ObjectiveDecided)
        .unwrap();
    state.begin_draining().unwrap();
    state.complete_drain().unwrap();
    state.terminalize().unwrap();
    assert_eq!(state.phase(), CampaignPhaseV1::Terminal);

    let (_, _, obligations) = assessment_and_obligations(
        ContractCombinatorV1::AllOf,
        &[
            ComponentOracleOutcomeV1::Proof,
            ComponentOracleOutcomeV1::Proof,
        ],
    );
    let action = GateActionTruthV1 {
        prepared_action_id: Uuid::from_u128(0x303),
        authority: ArtifactAuthorityV1::Canonical,
        disposition: PreparedActionDisposition::Succeeded,
        authorized: true,
        durable_started: true,
        execution_receipt_hash: Some(digest('1')),
        landed_reconciled: true,
        oracle_assessment_hash: Some(digest('2')),
        reason_code: None,
        residual_risk_hash: None,
    };
    validate_campaign_gate(&CampaignGateSnapshotV1 {
        authority: ArtifactAuthorityV1::Canonical,
        phase: CampaignPhaseV1::Terminal,
        actions: vec![action],
        denominator: Some(obligations.denominator),
        coverage_results: obligations.results,
        fact_delta_bundle_count: 1,
        fact_delta_consumed: false,
    })
    .unwrap();
}

#[test]
fn verification_campaign_denominator_is_pre_authorization_and_terminal_results_are_exact() {
    let contract = contract(ContractCombinatorV1::AllOf);
    let plan = plan(contract.clone());
    assert_eq!(
        CampaignCoverageDenominatorSealV1::seal(
            &contract,
            &plan.objectives()[0],
            bindings(&contract, &plan),
            true,
        )
        .unwrap_err()
        .code(),
        "VERIFICATION_CAMPAIGN_DENOMINATOR_SEAL_TOO_LATE"
    );
    let denominator = CampaignCoverageDenominatorSealV1::seal(
        &contract,
        &plan.objectives()[0],
        bindings(&contract, &plan),
        false,
    )
    .unwrap();
    let result = CoverageResultV1 {
        denominator_member_hash: denominator.members[0].member_hash.clone(),
        status: CoverageResultStatusV1::Untested {
            residual_risk_hash: digest('e'),
        },
    };
    assert_eq!(
        validate_coverage_results(&denominator, &[result])
            .unwrap_err()
            .code(),
        "VERIFICATION_CAMPAIGN_COVERAGE_CENSUS_MISMATCH"
    );
}

#[test]
fn verification_campaign_shadow_artifacts_never_enter_authority_gates() {
    let mut action = gate_action(PreparedActionDisposition::Denied);
    action.authority = ArtifactAuthorityV1::Shadow;
    action.reason_code = Some(ResidualReasonCodeV1::PolicyDenied);
    action.residual_risk_hash = Some(digest('f'));
    assert_eq!(
        validate_gate_action(&action).unwrap_err().code(),
        "VERIFICATION_CAMPAIGN_SHADOW_AUTHORITY_FORBIDDEN"
    );
}

#[test]
fn verification_campaign_wave_denominator_requires_exact_campaign_or_unassigned_partition() {
    let plan = plan(contract(ContractCombinatorV1::AllOf));
    let wave = VerificationWaveDenominatorSealV1::seal(&plan, digest('1'), false).unwrap();
    assert_eq!(
        VerificationWaveDenominatorSealV1::seal(&plan, digest('1'), true)
            .unwrap_err()
            .code(),
        "VERIFICATION_CAMPAIGN_WAVE_DENOMINATOR_SEAL_TOO_LATE"
    );
    assert_eq!(
        validate_wave_partition(&wave, &[]).unwrap_err().code(),
        "VERIFICATION_CAMPAIGN_WAVE_PARTITION_MISMATCH"
    );
    validate_wave_partition(
        &wave,
        &[WaveMemberDispositionV1::Unassigned {
            wave_member_hash: wave.members[0].member_hash.clone(),
            residual_risk_hash: digest('2'),
        }],
    )
    .unwrap();
}

#[test]
fn verification_campaign_paired_and_ordered_contracts_fail_closed_on_incomplete_relations() {
    let paired = contract(ContractCombinatorV1::PairedDifferential);
    let paired_plan = plan(paired.clone());
    let paired_action =
        action_contract(&paired, &paired_plan, PreparedActionKindV1::SingleActionV1);
    let paired_assessment = reduce_action_oracle(
        &paired_action,
        &receipt(
            &paired_action,
            &[
                ComponentOracleOutcomeV1::Proof,
                ComponentOracleOutcomeV1::Proof,
            ],
        ),
    )
    .unwrap();
    assert!(paired_assessment.paired_relation.is_none());
    let paired_denominator = CampaignCoverageDenominatorSealV1::seal(
        &paired,
        &paired_plan.objectives()[0],
        bindings(&paired, &paired_plan),
        false,
    )
    .unwrap();
    let paired_results = paired_denominator
        .members
        .iter()
        .map(|member| CoverageResultV1 {
            denominator_member_hash: member.member_hash.clone(),
            status: CoverageResultStatusV1::Tested {
                prepared_action_id: paired_action.prepared_action_id,
                capability_receipt_hash: digest('1'),
                oracle_assessment_hash: paired_assessment.assessment_hash.clone(),
            },
        })
        .collect();
    let paired_census = OracleCensusV1 {
        verification_contract_hash: paired.contract_hash().to_owned(),
        authority: ArtifactAuthorityV1::Canonical,
        assessments: vec![paired_assessment],
    };
    assert_eq!(
        adjudicate_campaign(
            &paired,
            &paired_census,
            &ObligationDispositionSetV1 {
                denominator: paired_denominator,
                results: paired_results,
            },
        )
        .unwrap()
        .outcome,
        ObjectiveCampaignOutcome::Inconclusive
    );

    let ordered = contract(ContractCombinatorV1::OrderedSequence);
    let ordered_plan = plan(ordered.clone());
    let ordered_action = action_contract(
        &ordered,
        &ordered_plan,
        PreparedActionKindV1::SingleActionV1,
    );
    let mut ordered_receipt = receipt(
        &ordered_action,
        &[
            ComponentOracleOutcomeV1::Proof,
            ComponentOracleOutcomeV1::Proof,
        ],
    );
    ordered_receipt.ordered_sequence = vec![
        OrderedSequenceObservationV1 {
            predicate_component_member_hash: ordered_action.predicate_component_member_hashes[1]
                .clone(),
            step_ordinal: 1,
            event_ordinal: 0,
            execution_session_hash: digest('3'),
            causal_chain_hash: digest('4'),
            outcome: ComponentOracleOutcomeV1::Proof,
            deterministic_negative: false,
            observation_window_complete: true,
        },
        OrderedSequenceObservationV1 {
            predicate_component_member_hash: ordered_action.predicate_component_member_hashes[0]
                .clone(),
            step_ordinal: 0,
            event_ordinal: 1,
            execution_session_hash: digest('3'),
            causal_chain_hash: digest('4'),
            outcome: ComponentOracleOutcomeV1::Proof,
            deterministic_negative: false,
            observation_window_complete: true,
        },
    ];
    let assessment = reduce_action_oracle(&ordered_action, &ordered_receipt).unwrap();
    assert_eq!(assessment.ordered_sequence.len(), 2);
    let ordered_denominator = CampaignCoverageDenominatorSealV1::seal(
        &ordered,
        &ordered_plan.objectives()[0],
        bindings(&ordered, &ordered_plan),
        false,
    )
    .unwrap();
    let ordered_results = ordered_denominator
        .members
        .iter()
        .map(|member| CoverageResultV1 {
            denominator_member_hash: member.member_hash.clone(),
            status: CoverageResultStatusV1::Tested {
                prepared_action_id: ordered_action.prepared_action_id,
                capability_receipt_hash: digest('2'),
                oracle_assessment_hash: assessment.assessment_hash.clone(),
            },
        })
        .collect();
    let ordered_census = OracleCensusV1 {
        verification_contract_hash: ordered.contract_hash().to_owned(),
        authority: ArtifactAuthorityV1::Canonical,
        assessments: vec![assessment],
    };
    assert_eq!(
        adjudicate_campaign(
            &ordered,
            &ordered_census,
            &ObligationDispositionSetV1 {
                denominator: ordered_denominator,
                results: ordered_results,
            },
        )
        .unwrap()
        .outcome,
        ObjectiveCampaignOutcome::Inconclusive
    );
}

#[test]
fn verification_campaign_combinators_are_closed_and_set_order_is_stable() {
    assert_eq!(ContractCombinatorV1::ALL.len(), 4);
    assert!(serde_json::from_str::<ContractCombinatorV1>(r#""threshold""#).is_err());
    for combinator in ContractCombinatorV1::ALL {
        let (contract, mut census, obligations) = assessment_and_obligations(
            *combinator,
            &[
                ComponentOracleOutcomeV1::Inconclusive,
                ComponentOracleOutcomeV1::Inconclusive,
            ],
        );
        let expected = adjudicate_campaign(&contract, &census, &obligations)
            .unwrap()
            .outcome;
        census.assessments.reverse();
        assert_eq!(
            adjudicate_campaign(&contract, &census, &obligations)
                .unwrap()
                .outcome,
            expected
        );
    }
    let (base_contract, mut census, obligations) = assessment_and_obligations(
        ContractCombinatorV1::AllOf,
        &[
            ComponentOracleOutcomeV1::Proof,
            ComponentOracleOutcomeV1::Proof,
        ],
    );
    census.assessments.push(census.assessments[0].clone());
    assert_eq!(
        adjudicate_campaign(&base_contract, &census, &obligations)
            .unwrap_err()
            .code(),
        "VERIFICATION_CONTRACT_DUPLICATE_IDENTITY"
    );

    let contract = contract(ContractCombinatorV1::AllOf);
    let plan = plan(contract.clone());
    let all_bindings = bindings(&contract, &plan);
    let assessments = all_bindings
        .iter()
        .enumerate()
        .map(|(ordinal, binding)| {
            let action = ActionOracleContractV1::seal(
                &contract,
                plan.objectives()[0].member_hash().to_owned(),
                Uuid::from_u128(0x700 + ordinal as u128),
                vec![binding.clone()],
                PreparedActionKindV1::SingleActionV1,
            )
            .unwrap();
            reduce_action_oracle(
                &action,
                &receipt(&action, &[ComponentOracleOutcomeV1::Proof]),
            )
            .unwrap()
        })
        .collect::<Vec<_>>();
    let denominator = CampaignCoverageDenominatorSealV1::seal(
        &contract,
        &plan.objectives()[0],
        all_bindings,
        false,
    )
    .unwrap();
    let results = denominator
        .members
        .iter()
        .map(|member| {
            let assessment = assessments
                .iter()
                .find(|assessment| {
                    assessment.predicate_outcomes.iter().any(|outcome| {
                        outcome.predicate_component_member_hash == member.contract_member_hash
                    })
                })
                .unwrap();
            CoverageResultV1 {
                denominator_member_hash: member.member_hash.clone(),
                status: CoverageResultStatusV1::Tested {
                    prepared_action_id: assessment.prepared_action_id,
                    capability_receipt_hash: digest('a'),
                    oracle_assessment_hash: assessment.assessment_hash.clone(),
                },
            }
        })
        .collect::<Vec<_>>();
    let obligations = ObligationDispositionSetV1 {
        denominator,
        results,
    };
    let ordered = adjudicate_campaign(
        &contract,
        &OracleCensusV1 {
            verification_contract_hash: contract.contract_hash().to_owned(),
            authority: ArtifactAuthorityV1::Canonical,
            assessments: assessments.clone(),
        },
        &obligations,
    )
    .unwrap();
    let reversed = adjudicate_campaign(
        &contract,
        &OracleCensusV1 {
            verification_contract_hash: contract.contract_hash().to_owned(),
            authority: ArtifactAuthorityV1::Canonical,
            assessments: assessments.into_iter().rev().collect(),
        },
        &obligations,
    )
    .unwrap();
    assert_eq!(ordered.outcome, ObjectiveCampaignOutcome::Proof);
    assert_eq!(ordered, reversed);
}

#[test]
fn verification_campaign_objective_outcome_is_not_revision_authority() {
    let (contract, census, obligations) = assessment_and_obligations(
        ContractCombinatorV1::AllOf,
        &[
            ComponentOracleOutcomeV1::Proof,
            ComponentOracleOutcomeV1::Proof,
        ],
    );
    let adjudication = adjudicate_campaign(&contract, &census, &obligations).unwrap();
    assert_eq!(adjudication.outcome, ObjectiveCampaignOutcome::Proof);
    assert_eq!(
        HypothesisRevisionOutcome::NonTerminal,
        golish_core::hypothesis_verification::HypothesisRevisionAdjudicationVerdictV1::NonTerminal
    );
}

#[test]
fn verification_campaign_outer_proof_paths_remain_plan_b_authority() {
    let plan = plan(contract(ContractCombinatorV1::AllOf));
    assert_eq!(plan.proof_path_count(), 1);
    assert_eq!(plan.objective_count(), 1);
    let required = plan
        .required_claim_components()
        .iter()
        .map(|component| component.member_hash())
        .collect::<BTreeSet<_>>();
    let planned = plan.objectives()[0]
        .claim_component_member_hashes()
        .iter()
        .map(String::as_str)
        .collect::<BTreeSet<_>>();
    assert_eq!(required, planned);
    assert!(adjudicate_hypothesis_revision(&plan, &[]).is_err());

    let objective = &plan.objectives()[0];
    let claim_component_outcomes = objective
        .claim_component_member_hashes()
        .iter()
        .map(|claim| {
            HypothesisClaimComponentOutcomeV1::nonterminal(
                claim.clone(),
                HypothesisClaimComponentOutcomeKindV1::Blocked,
                digest('6'),
            )
            .unwrap()
        })
        .collect();
    let outcome = HypothesisVerificationObjectiveOutcomeV1::compile(
        objective,
        HypothesisVerificationObjectiveOutcomeBuildInputV1 {
            outcome_receipt_id: Uuid::from_u128(0x909),
            outcome_receipt_version: 1,
            outcome_ordinal: 0,
            predecessor_outcome_receipt_id: None,
            predecessor_outcome_receipt_hash: None,
            campaign_head_hash: digest('7'),
            plan_objective_member_hash: objective.member_hash().to_owned(),
            verification_contract_hash: objective.verification_contract_hash().to_owned(),
            claim_component_outcomes,
            outcome: HypothesisVerificationObjectiveOutcomeKindV1::Blocked,
            campaign_terminal_receipts: Vec::new(),
            oracle_census_receipts: Vec::new(),
            coverage_receipts: Vec::new(),
            fact_delta_consumption_receipts: Vec::new(),
            unassigned_residual_risk_set_hash: digest('8'),
        },
    )
    .unwrap();
    let aggregate =
        adjudicate_hypothesis_revision(&plan, &[ObjectiveOutcomeViewV1::from(&outcome)]).unwrap();
    assert_eq!(aggregate.verdict(), HypothesisRevisionOutcome::NonTerminal);
    assert!(!aggregate.unresolved().is_empty());
}

#[test]
fn verification_campaign_conflict_keys_use_partial_overlap_and_canonical_order() {
    let a = ConflictKeyV1::new(ConflictKeyKindV1::MutableResource, digest('1')).unwrap();
    let b = ConflictKeyV1::new(ConflictKeyKindV1::CredentialSession, digest('2')).unwrap();
    let c = ConflictKeyV1::new(ConflictKeyKindV1::TargetRateBucket, digest('3')).unwrap();
    assert!(conflict_key_sets_overlap(
        &[a.clone(), b.clone()],
        &[b.clone(), c]
    ));
    let same_resource_other_credential =
        ConflictKeyV1::new(ConflictKeyKindV1::MutableResource, a.key_hash.clone()).unwrap();
    assert!(conflict_key_sets_overlap(
        std::slice::from_ref(&a),
        &[same_resource_other_credential]
    ));
    let ordered = canonical_conflict_keys(vec![b.clone(), a, b]).unwrap();
    assert_eq!(ordered.len(), 2);
    assert!(ordered.windows(2).all(|pair| pair[0] < pair[1]));
}

#[test]
fn verification_campaign_race_objective_requires_atomic_concurrent_group() {
    let residual = validate_objective_action_kind(
        ObjectiveExecutionClassV1::RaceClass,
        &PreparedActionKindV1::SingleActionV1,
    )
    .expect_err("sequential action cannot decide a race objective");
    assert_eq!(
        residual.code(),
        "VERIFICATION_CAMPAIGN_CONCURRENT_GROUP_REQUIRED"
    );
    assert_eq!(
        residual.residual_reason(),
        Some(ResidualReasonCodeV1::RaceAdapterMissing)
    );
}

#[test]
fn verification_campaign_claim_component_binding_must_cover_plan_b_exact_set() {
    let contract = contract(ContractCombinatorV1::AllOf);
    let plan = plan(contract.clone());
    let mut incomplete = bindings(&contract, &plan);
    incomplete.pop();
    assert_eq!(
        CampaignCoverageDenominatorSealV1::seal(
            &contract,
            &plan.objectives()[0],
            incomplete,
            false,
        )
        .unwrap_err()
        .code(),
        "VERIFICATION_CAMPAIGN_CLAIM_COMPONENT_BINDING_MISMATCH"
    );
}
