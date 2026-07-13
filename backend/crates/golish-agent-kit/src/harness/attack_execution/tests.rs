use super::{
    canonical_plan_hash, classifier_recipe_for, classify_candidate, decide_review_barrier,
    select_attack_read, transition_attempt, validate_terminal_result, AttackDecisionSemantic,
    AttackDecisionSemanticKind, AttackReadSource, AttackReviewCounts, AttackShadowComparison,
    AttemptDisposition, AttemptEvent, AttemptStatus, CandidateAttemptResult, CandidateBudget,
    CandidateClassificationInput, CandidateTargetClass, CompleteAttackRead, FactDeltaDraft,
    FactDeltaKind, FindingSeverity, ReviewBarrierAction, ReviewBarrierSnapshot, V2AttackRead,
    VerificationRiskClass, VerifiedFindingDraft,
};
use golish_core::AttackExecutionContract;
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
