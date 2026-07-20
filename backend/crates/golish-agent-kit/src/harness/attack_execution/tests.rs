use super::{
    canonical_plan_hash, classifier_recipe_for, classify_candidate, decide_review_barrier,
    select_attack_read, transition_attempt, validate_terminal_result, AttackDecisionSemantic,
    AttackDecisionSemanticKind, AttackReadSource, AttackReviewCounts, AttackShadowComparison,
    AttemptDisposition, AttemptEvent, AttemptStatus, CandidateAttemptResult, CandidateBudget,
    CandidateClassificationInput, CandidateExecutionPlan, CandidateTargetClass, CompleteAttackRead,
    FactDeltaDraft, FactDeltaKind, FindingSeverity, ReviewBarrierAction, ReviewBarrierSnapshot,
    V2AttackRead, VerificationRiskClass, VerifiedFindingDraft,
    CANDIDATE_EXECUTOR_CONTRACT_LEGACY_GENERIC_V1, CANDIDATE_RECIPE_VERSION_LEGACY_GENERIC_V1,
};
use golish_core::AttackExecutionContract;
use sha2::{Digest, Sha256};
use uuid::Uuid;

fn observation_hash(observation: &serde_json::Value) -> String {
    let digest = Sha256::digest(serde_json::to_vec(observation).expect("serialize observation"));
    let hex = digest
        .iter()
        .map(|byte| format!("{byte:02x}"))
        .collect::<String>();
    format!("sha256:{hex}")
}

fn finding_fixture() -> VerifiedFindingDraft {
    VerifiedFindingDraft {
        title: "SQL injection".to_string(),
        severity: FindingSeverity::High,
        cvss: Some(8.1),
        affected_target: "https://example.com/login".to_string(),
        description: "A bounded verifier confirmed SQL injection.".to_string(),
        reproduction_steps: vec!["Replay the evidence-linked request.".to_string()],
        remediation: "Use parameterized queries.".to_string(),
    }
}

fn result_fixture(disposition: AttemptDisposition) -> CandidateAttemptResult {
    CandidateAttemptResult {
        attempt_id: Uuid::from_u128(41),
        candidate_plan_hash: "sha256:plan".to_string(),
        disposition,
        proof_evidence_ids: Vec::new(),
        refutation_evidence_ids: Vec::new(),
        blocker_evidence_ids: Vec::new(),
        blocker_reason_code: None,
        finding: None,
        fact_deltas: Vec::<FactDeltaDraft>::new(),
    }
}

fn candidate_fixture_with_refs(prior_refs: Vec<&str>) -> CandidateClassificationInput {
    let observation = serde_json::json!({
        "schema": "nuclei_match_v1",
        "source_mode": "general",
        "target_id": Uuid::from_u128(8),
        "matched_url": "https://example.com/login",
        "template_id": "fixture-sqli",
        "matcher_name": "body",
        "severity": "high",
        "technique": "WSTG-INPV-05",
    });
    CandidateClassificationInput {
        candidate_id: Uuid::from_u128(7),
        target_live_id: Some(Uuid::from_u128(8)),
        target_identity_hash: "sha256:target".to_string(),
        target_class: CandidateTargetClass::Url,
        target_value: "https://example.com:443".to_string(),
        hypothesis: "The username parameter is SQL injectable".to_string(),
        technique: "WSTG-INPV-05".to_string(),
        observation_hash: observation_hash(&observation),
        observation,
        prior_refs: prior_refs.into_iter().map(str::to_string).collect(),
    }
}

fn candidate_fixture() -> CandidateClassificationInput {
    candidate_fixture_with_refs(vec!["CVE-2025-1", "evidence:42"])
}

fn candidate_fixture_with_reordered_prior_refs() -> CandidateClassificationInput {
    candidate_fixture_with_refs(vec!["evidence:42", "CVE-2025-1"])
}

fn unsupported_candidate_fixture() -> CandidateClassificationInput {
    CandidateClassificationInput {
        technique: "CUSTOM-UNSUPPORTED".to_string(),
        ..candidate_fixture()
    }
}

fn anonymous_candidate_fixture() -> CandidateClassificationInput {
    let observation = serde_json::json!({
        "schema": "anonymous_access_v1",
        "endpoint_id": Uuid::from_u128(81),
        "endpoint_row_sha256": "1".repeat(64),
        "request_plan_sha256": "2".repeat(64),
        "method": "GET",
        "path": "/api/orders",
        "query_bindings": [{"name": "id", "value": "42"}],
        "no_auth": true,
        "network_attempted": true,
        "status_code": 200,
        "redirect": {"present": false, "same_origin": null, "login_like": false},
        "verdict": "suspicious",
        "rationale": "account-scoped API metadata",
        "authority_current_after": true,
    });
    CandidateClassificationInput {
        candidate_id: Uuid::from_u128(71),
        target_live_id: Some(Uuid::from_u128(82)),
        target_identity_hash: "sha256:anonymous-target".to_string(),
        target_class: CandidateTargetClass::Url,
        target_value: "https://example.com:443".to_string(),
        hypothesis: "The exact endpoint may expose account data without authentication".to_string(),
        technique: "WSTG-ATHN-04".to_string(),
        observation_hash: observation_hash(&observation),
        observation,
        prior_refs: vec!["audit:52".to_string()],
    }
}

fn surface_candidate_fixture() -> CandidateClassificationInput {
    let observation = serde_json::json!({
        "schema": "surface_analysis_v1",
        "target_id": Uuid::from_u128(91),
        "target_identity": {
            "type": "url",
            "value": "https://example.com:443",
            "sha256": "sha256:surface-target",
        },
        "formulaic_coverage": [],
        "upstream_query_required": true,
    });
    CandidateClassificationInput {
        candidate_id: Uuid::from_u128(72),
        target_live_id: Some(Uuid::from_u128(91)),
        target_identity_hash: "sha256:surface-target".to_string(),
        target_class: CandidateTargetClass::Url,
        target_value: "https://example.com:443".to_string(),
        hypothesis: "A reflected input may reach a dangerous sink".to_string(),
        technique: "WSTG-INPV-01".to_string(),
        observation_hash: observation_hash(&observation),
        observation,
        prior_refs: vec!["audit:53".to_string()],
    }
}

fn directory_entry_candidate_fixture() -> CandidateClassificationInput {
    let target_id = Uuid::from_u128(91);
    let directory_entry_id = Uuid::from_u128(92);
    let row_material = serde_json::json!({
        "content_length": 74,
        "content_type": "",
        "id": directory_entry_id,
        "status_code": 200,
        "target_id": target_id,
        "tool": "route_probe",
        "url": "https://example.com/README.md",
    });
    let observation = serde_json::json!({
        "schema": "directory_entry_observation_v1",
        "target_id": target_id,
        "directory_entry_id": directory_entry_id,
        "directory_entry_row_sha256": observation_hash(&row_material),
        "url": "https://example.com/README.md",
        "method": "GET",
        "status_code": 200,
        "content_length": 74,
        "content_type": "",
        "source_tool": "route_probe",
        "source_evidence_id": 20,
        "network_attempted": true,
        "authority_current_after": true,
    });
    CandidateClassificationInput {
        candidate_id: Uuid::from_u128(73),
        target_live_id: Some(target_id),
        target_identity_hash: "sha256:directory-target".to_string(),
        target_class: CandidateTargetClass::Url,
        target_value: "https://example.com:443".to_string(),
        hypothesis: "The exact README path may disclose deployment information".to_string(),
        technique: "WSTG-INFO".to_string(),
        observation_hash: observation_hash(&observation),
        observation,
        prior_refs: vec!["audit:20".to_string()],
    }
}

fn directory_entry_set_candidate_fixture() -> CandidateClassificationInput {
    let target_id = Uuid::from_u128(91);
    let directory_entry_id = Uuid::from_u128(92);
    let row = serde_json::json!({
        "content_length": 74,
        "content_type": "application/json",
        "id": directory_entry_id,
        "status_code": 200,
        "target_id": target_id,
        "tool": "route_probe",
        "url": "https://example.com/config.json",
    });
    let observation = serde_json::json!({
        "schema": "directory_entry_set_v1",
        "target_id": target_id,
        "origin": "https://example.com:443",
        "entry_count": 1,
        "entry_set_sha256": observation_hash(&serde_json::json!([row.clone()])),
        "entries_preview": [row],
        "preview_count": 1,
        "preview_truncated": false,
        "method": "GET",
        "source_tool": "route_probe",
        "source_evidence_ids": [20],
        "network_attempted": true,
        "authority_current_after": true,
    });
    CandidateClassificationInput {
        candidate_id: Uuid::from_u128(74),
        target_live_id: Some(target_id),
        target_identity_hash: "sha256:directory-set-target".to_string(),
        target_class: CandidateTargetClass::Url,
        target_value: "https://example.com:443".to_string(),
        hypothesis: "The exact config path may disclose deployment information".to_string(),
        technique: "WSTG-INFO".to_string(),
        observation_hash: observation_hash(&observation),
        observation,
        prior_refs: vec!["audit:20".to_string()],
    }
}

#[test]
fn verified_requires_proof_and_finding_draft() {
    let mut result = result_fixture(AttemptDisposition::Verified);
    result.finding = Some(finding_fixture());
    assert_eq!(
        validate_terminal_result(&result).unwrap_err().code(),
        "ATTACK_PROOF_REQUIRED"
    );
}

#[test]
fn refuted_requires_refutation_evidence_and_no_finding() {
    let mut result = result_fixture(AttemptDisposition::Refuted);
    result.refutation_evidence_ids.push(11);
    result.finding = Some(finding_fixture());
    assert_eq!(
        validate_terminal_result(&result).unwrap_err().code(),
        "ATTACK_REFUTED_FINDING_FORBIDDEN"
    );
}

#[test]
fn blocked_requires_stable_reason_or_blocker_evidence() {
    let result = result_fixture(AttemptDisposition::Blocked);
    assert_eq!(
        validate_terminal_result(&result).unwrap_err().code(),
        "ATTACK_BLOCK_REASON_REQUIRED"
    );
}

#[test]
fn attempt_has_no_waiting_background_transition() {
    assert!(transition_attempt(AttemptStatus::Running, AttemptEvent::Backgrounded).is_err());
}

#[test]
fn retryable_failure_terminalizes_the_old_attempt_and_retry_never_requeues_it() {
    assert!(AttemptStatus::RetryableFailed.is_terminal());
    assert!(transition_attempt(AttemptStatus::RetryableFailed, AttemptEvent::Retried).is_err());
}

#[test]
fn complete_attempt_status_machine_has_only_the_eight_persisted_states() {
    let encoded = serde_json::to_value(AttemptStatus::ALL).unwrap();
    assert_eq!(
        encoded,
        serde_json::json!([
            "queued",
            "running",
            "submitted",
            "verified",
            "refuted",
            "blocked",
            "retryable_failed",
            "abandoned"
        ])
    );
    assert_eq!(
        transition_attempt(AttemptStatus::Queued, AttemptEvent::Started).unwrap(),
        AttemptStatus::Running
    );
    assert_eq!(
        transition_attempt(AttemptStatus::Running, AttemptEvent::Submitted).unwrap(),
        AttemptStatus::Submitted
    );
    assert_eq!(
        transition_attempt(AttemptStatus::Submitted, AttemptEvent::Verified).unwrap(),
        AttemptStatus::Verified
    );
    assert!(transition_attempt(AttemptStatus::Verified, AttemptEvent::Started).is_err());
}

#[test]
fn valid_terminal_results_keep_evidence_roles_disjoint() {
    let mut verified = result_fixture(AttemptDisposition::Verified);
    verified.proof_evidence_ids.push(10);
    verified.finding = Some(finding_fixture());
    validate_terminal_result(&verified).unwrap();

    let mut refuted = result_fixture(AttemptDisposition::Refuted);
    refuted.refutation_evidence_ids.push(11);
    validate_terminal_result(&refuted).unwrap();

    let mut blocked = result_fixture(AttemptDisposition::Blocked);
    blocked.blocker_reason_code = Some("approval_expired".to_string());
    validate_terminal_result(&blocked).unwrap();
}

#[test]
fn fact_delta_kind_is_a_closed_domain_set() {
    assert_eq!(
        serde_json::to_value(FactDeltaKind::ALL).unwrap(),
        serde_json::json!(["created", "updated", "refuted", "new_surface"])
    );
    assert!(serde_json::from_value::<FactDeltaDraft>(serde_json::json!({
        "fact_kind": "model_invented_prose",
        "canonical_ref_kind": "attack_candidate_work_item",
        "canonical_ref_id": Uuid::from_u128(99),
        "canonical_ref_version": 1,
        "canonical_ref_hash": "sha256:canonical",
        "summary": "untrusted prose must not become a new delta kind",
        "evidence_ids": [17]
    }))
    .is_err());
}

#[test]
fn classifier_is_canonical_and_foreground_only() {
    let a = classify_candidate(&candidate_fixture()).unwrap();
    let b = classify_candidate(&candidate_fixture_with_reordered_prior_refs()).unwrap();
    assert_eq!(
        canonical_plan_hash(&a).unwrap(),
        canonical_plan_hash(&b).unwrap()
    );
    assert!(a.foreground_only);
    assert!(a
        .actions
        .iter()
        .all(|action| !action.canonical_args["background"]
            .as_bool()
            .unwrap_or(false)));
    assert_eq!(
        a.budget,
        CandidateBudget {
            max_actions: 1,
            max_requests: 8,
            max_runtime_ms: 120_000,
        }
    );
    assert_eq!(
        classifier_recipe_for("WSTG-INPV-05", CandidateTargetClass::Url)
            .unwrap()
            .risk_class,
        VerificationRiskClass::Exploit
    );
    assert_eq!(
        a.actions[0].canonical_args["observation"],
        candidate_fixture().observation
    );
    assert_eq!(
        a.actions[0].canonical_args["observation_hash"],
        candidate_fixture().observation_hash
    );
    let mut changed_observation = candidate_fixture();
    changed_observation.observation["template_id"] = serde_json::json!("fixture-sqli-two");
    changed_observation.observation_hash = observation_hash(&changed_observation.observation);
    assert_ne!(
        canonical_plan_hash(&a).unwrap(),
        canonical_plan_hash(&classify_candidate(&changed_observation).unwrap()).unwrap()
    );
}

#[test]
fn candidate_plan_hash_covers_recipe_and_executor_contract_versions() {
    let plan = classify_candidate(&candidate_fixture()).expect("typed Candidate plan");
    let frozen_hash = canonical_plan_hash(&plan).expect("canonical plan hash");

    let mut changed_plan_recipe = plan.clone();
    changed_plan_recipe.recipe_version.push_str("-drift");
    assert_ne!(
        frozen_hash,
        canonical_plan_hash(&changed_plan_recipe).expect("changed plan recipe hash")
    );

    let mut changed_action_recipe = plan.clone();
    changed_action_recipe.actions[0]
        .recipe_version
        .push_str("-drift");
    assert_ne!(
        frozen_hash,
        canonical_plan_hash(&changed_action_recipe).expect("changed action recipe hash")
    );

    let mut changed_plan_executor = plan.clone();
    changed_plan_executor
        .executor_contract_version
        .push_str("-drift");
    assert_ne!(
        frozen_hash,
        canonical_plan_hash(&changed_plan_executor).expect("changed plan executor hash")
    );

    let mut changed_action_executor = plan;
    changed_action_executor.actions[0]
        .executor_contract_version
        .push_str("-drift");
    assert_ne!(
        frozen_hash,
        canonical_plan_hash(&changed_action_executor).expect("changed action executor hash")
    );
}

#[test]
fn legacy_v1_plan_round_trip_does_not_rewrite_its_approved_hash_input() {
    let legacy_json = serde_json::json!({
        "schema_version": "candidate-plan-v1",
        "classifier_version": "candidate-classifier-v1",
        "candidate_id": Uuid::from_u128(700),
        "target_identity_hash": "sha256:legacy-target",
        "actions": [{
            "ordinal": 0,
            "capability_id": "verify.input_validation",
            "action_kind": "bounded_input_reflection_probe",
            "canonical_args": {
                "background": false,
                "target": "https://legacy.example:443"
            },
            "side_effect_class": "active_probe",
            "required_evidence_role": "proof"
        }],
        "budget": {
            "max_actions": 1,
            "max_requests": 8,
            "max_runtime_ms": 120_000
        },
        "foreground_only": true
    });
    let plan: CandidateExecutionPlan =
        serde_json::from_value(legacy_json.clone()).expect("legacy V1 plan");

    assert_eq!(
        plan.recipe_version,
        CANDIDATE_RECIPE_VERSION_LEGACY_GENERIC_V1
    );
    assert_eq!(
        plan.executor_contract_version,
        CANDIDATE_EXECUTOR_CONTRACT_LEGACY_GENERIC_V1
    );
    assert_eq!(
        serde_json::to_value(plan).expect("serialize legacy V1 plan"),
        legacy_json
    );
}

#[test]
fn changing_the_executor_binding_cannot_retain_the_approved_plan_hash() {
    let approved = classify_candidate(&candidate_fixture()).expect("typed Candidate plan");
    let approved_hash = canonical_plan_hash(&approved).expect("approved plan hash");
    let mut drifted = approved;
    drifted.actions[0].capability_id = "verify.input_validation".to_string();
    drifted.actions[0].action_kind = "bounded_input_reflection_probe".to_string();
    drifted.actions[0].executor_contract_version = "candidate-executor.generic-v2".to_string();

    assert_ne!(
        approved_hash,
        canonical_plan_hash(&drifted).expect("drifted executor hash")
    );
}

#[test]
fn classifier_nuclei_match_freezes_the_exact_template_url_and_target_into_a_replay_action() {
    let candidate = candidate_fixture();
    let plan = classify_candidate(&candidate).expect("typed Nuclei observation should classify");
    let action = &plan.actions[0];

    assert_eq!(action.ordinal, 0);
    assert_eq!(action.capability_id, "verify.nuclei_template_replay");
    assert_eq!(action.action_kind, "nuclei_template_replay");
    assert_eq!(
        action.canonical_args["target_id"],
        candidate.observation["target_id"]
    );
    assert_eq!(
        action.canonical_args["matched_url"],
        candidate.observation["matched_url"]
    );
    assert_eq!(
        action.canonical_args["template_id"],
        candidate.observation["template_id"]
    );
    assert_eq!(
        action.canonical_args["observation_hash"],
        candidate.observation_hash
    );
}

#[test]
fn classifier_anonymous_access_freezes_the_exact_endpoint_row_and_request_plan() {
    let candidate = anonymous_candidate_fixture();
    let plan =
        classify_candidate(&candidate).expect("typed anonymous-access observation should classify");
    let action = &plan.actions[0];

    assert_eq!(action.ordinal, 0);
    assert_eq!(action.capability_id, "verify.anonymous_request_replay");
    assert_eq!(action.action_kind, "anonymous_request_replay");
    for field in [
        "endpoint_id",
        "endpoint_row_sha256",
        "request_plan_sha256",
        "method",
        "path",
        "query_bindings",
    ] {
        assert_eq!(
            action.canonical_args[field], candidate.observation[field],
            "{field} must come from the frozen observation"
        );
    }
    assert_eq!(action.canonical_args["no_auth"], true);
    assert_eq!(action.canonical_args["background"], false);
}

#[test]
fn classifier_quarantines_surface_analysis_without_a_typed_v2_executor() {
    let generic_surface = surface_candidate_fixture();
    let error =
        classify_candidate(&generic_surface).expect_err("new V2 generic recipes must fail closed");

    assert_eq!(error.code(), "ATTACK_EXECUTOR_CONTRACT_UNAVAILABLE");

    let mut information_disclosure_surface = generic_surface;
    information_disclosure_surface.technique = "WSTG-INFO".to_string();
    let error = classify_candidate(&information_disclosure_surface)
        .expect_err("WSTG-INFO still needs an exact typed path observation");
    assert_eq!(error.code(), "ATTACK_EXECUTOR_CONTRACT_UNAVAILABLE");
}

#[test]
fn classifier_directory_entry_observation_freezes_exact_read_only_replay() {
    let candidate = directory_entry_candidate_fixture();
    let plan = classify_candidate(&candidate)
        .expect("target-bound directory observation should have a typed replay plan");
    let action = &plan.actions[0];

    assert_eq!(action.capability_id, "verify.directory_entry_replay");
    assert_eq!(action.action_kind, "directory_entry_replay");
    assert_eq!(action.side_effect_class, super::SideEffectClass::ReadOnly);
    assert_eq!(action.canonical_args["background"], false);
    assert_eq!(action.canonical_args["method"], "GET");
    assert_eq!(
        action.canonical_args["directory_entry_id"],
        candidate.observation["directory_entry_id"]
    );
    assert_eq!(
        action.canonical_args["directory_entry_row_sha256"],
        candidate.observation["directory_entry_row_sha256"]
    );
    assert_eq!(
        action.canonical_args["url"],
        "https://example.com/README.md"
    );
    assert_eq!(
        action.canonical_args["source_evidence_id"],
        candidate.observation["source_evidence_id"]
    );
}

#[test]
fn classifier_directory_entry_set_selects_one_frozen_preview_row_for_exact_replay() {
    let candidate = directory_entry_set_candidate_fixture();
    let plan = classify_candidate(&candidate)
        .expect("target-bound directory set should have a typed exact replay plan");
    let action = &plan.actions[0];

    assert_eq!(action.capability_id, "verify.directory_entry_replay");
    assert_eq!(action.action_kind, "directory_entry_replay");
    assert_eq!(action.canonical_args["background"], false);
    assert_eq!(action.canonical_args["method"], "GET");
    assert_eq!(
        action.canonical_args["directory_entry_id"],
        candidate.observation["entries_preview"][0]["id"]
    );
    assert_eq!(
        action.canonical_args["url"],
        "https://example.com/config.json"
    );
    assert_eq!(action.canonical_args["source_evidence_id"], 20);
    assert_eq!(
        action.canonical_args["observation"],
        candidate.observation
    );
    assert_eq!(
        action.canonical_args["observation_hash"],
        candidate.observation_hash
    );
}

#[test]
fn classifier_directory_entry_set_rejects_preview_or_evidence_drift() {
    let mut missing_ref = directory_entry_set_candidate_fixture();
    missing_ref.prior_refs.clear();
    assert_eq!(
        classify_candidate(&missing_ref).unwrap_err().code(),
        "ATTACK_OBSERVATION_INVALID"
    );

    let mut foreign_origin = directory_entry_set_candidate_fixture();
    foreign_origin.observation["entries_preview"][0]["url"] =
        serde_json::json!("https://foreign.example/config.json");
    foreign_origin.observation_hash = observation_hash(&foreign_origin.observation);
    assert_eq!(
        classify_candidate(&foreign_origin).unwrap_err().code(),
        "ATTACK_OBSERVATION_IDENTITY_MISMATCH"
    );

    let mut count_drift = directory_entry_set_candidate_fixture();
    count_drift.observation["preview_count"] = serde_json::json!(2);
    count_drift.observation_hash = observation_hash(&count_drift.observation);
    assert_eq!(
        classify_candidate(&count_drift).unwrap_err().code(),
        "ATTACK_OBSERVATION_INVALID"
    );
}

#[test]
fn classifier_directory_entry_observation_rejects_foreign_or_drifted_inputs() {
    let mut foreign_origin = directory_entry_candidate_fixture();
    foreign_origin.observation["url"] = serde_json::json!("https://foreign.example/README.md");
    foreign_origin.observation_hash = observation_hash(&foreign_origin.observation);
    assert_eq!(
        classify_candidate(&foreign_origin).unwrap_err().code(),
        "ATTACK_OBSERVATION_IDENTITY_MISMATCH"
    );

    let mut target_drift = directory_entry_candidate_fixture();
    target_drift.observation["target_id"] = serde_json::json!(Uuid::from_u128(999));
    target_drift.observation_hash = observation_hash(&target_drift.observation);
    assert_eq!(
        classify_candidate(&target_drift).unwrap_err().code(),
        "ATTACK_OBSERVATION_IDENTITY_MISMATCH"
    );

    let mut technique_drift = directory_entry_candidate_fixture();
    technique_drift.technique = "WSTG-CONF-05".to_string();
    assert_eq!(
        classify_candidate(&technique_drift).unwrap_err().code(),
        "ATTACK_OBSERVATION_IDENTITY_MISMATCH"
    );

    let mut row_hash_drift = directory_entry_candidate_fixture();
    row_hash_drift.observation["directory_entry_row_sha256"] =
        serde_json::json!(format!("sha256:{}", "0".repeat(64)));
    row_hash_drift.observation_hash = observation_hash(&row_hash_drift.observation);
    assert_eq!(
        classify_candidate(&row_hash_drift).unwrap_err().code(),
        "ATTACK_OBSERVATION_HASH_MISMATCH"
    );

    let mut malformed = directory_entry_candidate_fixture();
    malformed.observation["method"] = serde_json::json!("POST");
    malformed.observation_hash = observation_hash(&malformed.observation);
    assert_eq!(
        classify_candidate(&malformed).unwrap_err().code(),
        "ATTACK_OBSERVATION_INVALID"
    );
}

#[test]
fn classifier_unknown_or_mismatched_observations_fail_closed() {
    let mut unknown = surface_candidate_fixture();
    unknown.observation["schema"] = serde_json::json!("future_untrusted_observation_v9");
    unknown.observation_hash = observation_hash(&unknown.observation);
    assert_eq!(
        classify_candidate(&unknown).unwrap_err().code(),
        "ATTACK_OBSERVATION_SCHEMA_UNSUPPORTED"
    );

    let mut technique_drift = candidate_fixture();
    technique_drift.observation["technique"] = serde_json::json!("WSTG-INPV-01");
    technique_drift.observation_hash = observation_hash(&technique_drift.observation);
    assert_eq!(
        classify_candidate(&technique_drift).unwrap_err().code(),
        "ATTACK_OBSERVATION_IDENTITY_MISMATCH"
    );

    let mut surface_identity_drift = surface_candidate_fixture();
    surface_identity_drift.observation["target_identity"]["sha256"] =
        serde_json::json!("sha256:foreign-target");
    surface_identity_drift.observation_hash = observation_hash(&surface_identity_drift.observation);
    assert_eq!(
        classify_candidate(&surface_identity_drift)
            .unwrap_err()
            .code(),
        "ATTACK_OBSERVATION_IDENTITY_MISMATCH"
    );

    let mut declared_hash_drift = anonymous_candidate_fixture();
    declared_hash_drift.observation_hash = "sha256:foreign-observation".to_string();
    assert_eq!(
        classify_candidate(&declared_hash_drift).unwrap_err().code(),
        "ATTACK_OBSERVATION_HASH_MISMATCH"
    );
}

#[test]
fn unsupported_technique_fails_closed_before_review() {
    let err = classify_candidate(&unsupported_candidate_fixture()).unwrap_err();
    assert_eq!(err.code(), "ATTACK_CAPABILITY_UNSUPPORTED");
}

#[test]
fn review_barrier_branches_only_from_the_exact_db_snapshot() {
    let open = ReviewBarrierSnapshot {
        wave_unit_count: 2,
        review_closed_unit_count: 1,
        candidate_count: 2,
        proposed_candidate_count: 1,
        durable_status: "resume_pending".to_string(),
        dispatch_is_stale: false,
    };
    assert_eq!(
        decide_review_barrier(&open).unwrap(),
        ReviewBarrierAction::KeepOpen
    );

    let closed = ReviewBarrierSnapshot {
        review_closed_unit_count: 2,
        proposed_candidate_count: 0,
        durable_status: "open".to_string(),
        ..open.clone()
    };
    assert_eq!(
        decide_review_barrier(&closed).unwrap(),
        ReviewBarrierAction::SetResumePending
    );

    let stale_dispatch = ReviewBarrierSnapshot {
        durable_status: "dispatching".to_string(),
        dispatch_is_stale: true,
        ..closed
    };
    assert_eq!(
        decide_review_barrier(&stale_dispatch).unwrap(),
        ReviewBarrierAction::ResetStaleDispatch
    );
}

fn semantic_decision(
    work_item_key: &str,
    kind: AttackDecisionSemanticKind,
    semantic_hash: &str,
) -> AttackDecisionSemantic {
    AttackDecisionSemantic::try_new(work_item_key, kind, semantic_hash).unwrap()
}

fn complete_attack_read(
    decisions: Vec<AttackDecisionSemantic>,
    counts: AttackReviewCounts,
) -> CompleteAttackRead {
    CompleteAttackRead::try_new(decisions, counts).unwrap()
}

fn legacy_attack_read() -> CompleteAttackRead {
    complete_attack_read(
        vec![
            semantic_decision(
                "legacy-work-1",
                AttackDecisionSemanticKind::Candidate,
                "sha256:legacy-candidate",
            ),
            semantic_decision(
                "legacy-work-2",
                AttackDecisionSemanticKind::NoCandidate,
                "sha256:legacy-no-candidate",
            ),
        ],
        AttackReviewCounts::new(2, 2, 1, 1),
    )
}

#[test]
fn dual_legacy_returns_whole_legacy_and_reports_v2_mismatch() {
    let legacy = legacy_attack_read();
    let v2 = complete_attack_read(
        vec![
            semantic_decision(
                "legacy-work-1",
                AttackDecisionSemanticKind::Candidate,
                "sha256:v2-candidate-mismatch",
            ),
            semantic_decision(
                "legacy-work-2",
                AttackDecisionSemanticKind::NoCandidate,
                "sha256:legacy-no-candidate",
            ),
        ],
        AttackReviewCounts::new(2, 2, 1, 1),
    );

    let selected = select_attack_read(
        AttackExecutionContract::DualWriteReadLegacy,
        Some(legacy.clone()),
        V2AttackRead::Complete(v2),
    )
    .unwrap();

    assert_eq!(selected.source(), AttackReadSource::Legacy);
    assert_eq!(selected.record(), &legacy);
    assert_eq!(
        selected.shadow_comparison(),
        Some(AttackShadowComparison::Mismatch)
    );
    assert!(!selected.executes_v2_verifier());
}

#[test]
fn dual_shadow_equal_whole_records_report_match_without_running_verifier() {
    for contract in [
        AttackExecutionContract::DualWriteReadLegacy,
        AttackExecutionContract::DualWriteReadV2Fallback,
    ] {
        let legacy = legacy_attack_read();
        let selected = select_attack_read(
            contract,
            Some(legacy.clone()),
            V2AttackRead::Complete(legacy.clone()),
        )
        .unwrap();

        assert_eq!(
            selected.shadow_comparison(),
            Some(AttackShadowComparison::Match),
            "contract={contract:?}"
        );
        assert_eq!(selected.record(), &legacy);
        assert!(!selected.executes_v2_verifier());
    }
}

#[test]
fn dual_v2_fallback_with_missing_child_returns_exact_whole_legacy_record() {
    let legacy = legacy_attack_read();
    let incomplete_v2 = V2AttackRead::from_parts(
        vec![semantic_decision(
            "legacy-work-1",
            AttackDecisionSemanticKind::Candidate,
            "sha256:v2-candidate",
        )],
        // The aggregate says one Candidate and one no-Candidate decision, but
        // the no-Candidate child row is intentionally absent.
        AttackReviewCounts::new(2, 2, 1, 1),
    );
    assert!(matches!(incomplete_v2, V2AttackRead::Incomplete));

    let selected = select_attack_read(
        AttackExecutionContract::DualWriteReadV2Fallback,
        Some(legacy.clone()),
        incomplete_v2,
    )
    .unwrap();

    assert_eq!(selected.source(), AttackReadSource::LegacyFallback);
    assert_eq!(selected.record(), &legacy);
    assert_eq!(
        selected.shadow_comparison(),
        Some(AttackShadowComparison::V2Missing)
    );
    // An atomic record equality assertion makes field-level fallback
    // impossible to hide behind matching review counts.
    assert_eq!(selected.into_record(), legacy);
}

#[test]
fn v2_only_with_incomplete_v2_errors_despite_complete_legacy() {
    let legacy = legacy_attack_read();
    let incomplete_v2 = V2AttackRead::from_parts(
        vec![semantic_decision(
            "legacy-work-1",
            AttackDecisionSemanticKind::Candidate,
            "sha256:v2-candidate",
        )],
        AttackReviewCounts::new(2, 2, 1, 1),
    );

    let error = select_attack_read(AttackExecutionContract::V2Only, Some(legacy), incomplete_v2)
        .unwrap_err();

    assert_eq!(error.code(), "ATTACK_V2_READ_REQUIRED");
}

#[test]
fn only_v2_only_executes_verifier() {
    for contract in AttackExecutionContract::ALL {
        let legacy = legacy_attack_read();
        let v2 = V2AttackRead::Complete(legacy.clone());
        let selected = select_attack_read(contract, Some(legacy), v2).unwrap();
        assert_eq!(
            selected.executes_v2_verifier(),
            contract == AttackExecutionContract::V2Only,
            "contract={contract:?}"
        );
    }
}
