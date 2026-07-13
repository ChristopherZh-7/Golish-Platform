use super::{
    canonical_plan_hash, classifier_recipe_for, classify_candidate, decide_review_barrier,
    transition_attempt, validate_terminal_result, AttemptDisposition, AttemptEvent, AttemptStatus,
    CandidateAttemptResult, CandidateBudget, CandidateClassificationInput, CandidateTargetClass,
    FactDeltaDraft, FindingSeverity, ReviewBarrierAction, ReviewBarrierSnapshot,
    VerificationRiskClass, VerifiedFindingDraft,
};
use uuid::Uuid;

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
    CandidateClassificationInput {
        candidate_id: Uuid::from_u128(7),
        target_identity_hash: "sha256:target".to_string(),
        target_class: CandidateTargetClass::Url,
        target_value: "https://example.com/login".to_string(),
        hypothesis: "The username parameter is SQL injectable".to_string(),
        technique: "WSTG-INPV-05".to_string(),
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
