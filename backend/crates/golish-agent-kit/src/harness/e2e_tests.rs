//! Phase 1d Task 1d.1 · external_attack_surface demo e2e 单测.
//!
//! 模拟 assessment profile + L2 active_recon + 1 个 stage 完整跑通:
//!   1. mock DB-truth evidence facts (LIVENESS / PORT / SERVICE-FINGERPRINT)
//!   2. mock deliverable 含 claims + findings; model-authored ids are optional
//!   3. validate_external_attack_surface_gate (含 contract_check.run_with_skeleton
//!      + freshness_check.run_with_freshness 完整路径)
//!   4. 断言 allowed=true / blocked + recovery_actions
//!
//! 这套 demo 是 Phase 1d e2e 验收的最小集合; Playwright UI e2e (Task 1d.2) 推
//! 用户手动验证.

#![cfg(test)]

use std::collections::HashMap;
use std::time::Duration as StdDuration;

use golish_pentest::evidence_ledger::{EvidenceAuditId, SkipReason};
use uuid::Uuid;

use super::gate::contract_check::run_with_skeleton as contract_check_with_skeleton;
use super::gate::freshness_check::run_with_freshness as freshness_check_with_freshness;
use super::gate::rule_engine::{EvidenceFact, EvidenceOutcome, GateContext};
use super::gate::{GateCheckOutcome, GateContextBuilder};
use super::profile::load_profile_from_json;
use super::sprint_contract::{
    DefaultSprintContractGenerator, SprintContractGenerator, SprintSkeleton,
};
use super::stage_harness::StageHarness;
use super::stage_spec::load_stage_spec_from_json;
use super::types::{
    CoverageCell, CoverageStatus, ExternalAttackSurfaceDeliverable, FindingSeverity,
    HarnessFinding, SkippedCheckRecord, StageClaim, StageKind,
};

const ASSESSMENT_PROFILE_JSON: &str =
    include_str!("../../../../../resources/harness/profiles/assessment.json");
const ASSESSMENT_SKELETON_JSON: &str =
    include_str!("../../../../../resources/harness/profiles/assessment.sprint_skeleton.json");
const STAGE_JSON: &str =
    include_str!("../../../../../resources/harness/stages/external_attack_surface/spec.json");

fn build_harness() -> StageHarness {
    let profile = load_profile_from_json(ASSESSMENT_PROFILE_JSON).expect("profile");
    let spec = load_stage_spec_from_json(STAGE_JSON).expect("spec");
    StageHarness::for_stage(StageKind::ExternalAttackSurface, profile, spec).expect("harness")
}

/// 构造 surface claim/findings 的 "happy path" deliverable.
fn happy_deliverable(stage_run_id: Uuid) -> ExternalAttackSurfaceDeliverable {
    let dns_eid = EvidenceAuditId::new(1);
    let http_eid = EvidenceAuditId::new(2);
    let ct_eid = EvidenceAuditId::new(3);

    let mut d = ExternalAttackSurfaceDeliverable {
        stage_id: "external_attack_surface".to_string(),
        stage_run_id,
        claims: vec![StageClaim {
            kind: "http_service_observed".to_string(),
            subject: "api.example.com".to_string(),
            summary: "HTTP/1.1 200 OK".to_string(),
            evidence_ids: vec![http_eid],
            technique: None,
        }],
        evidence_refs: vec![dns_eid, http_eid, ct_eid],
        skipped_checks: vec![],
        findings: vec![],
        // EAS now closes from DB-truth coverage; required_checks_done is no longer
        // a hard tool floor.
        required_checks_done: vec![
            "scope_status_present".to_string(),
            "evidence_non_empty".to_string(),
        ],
        coverage: vec![],
        candidates: vec![],
    };
    // 1 subdomain + 1 http_service (覆盖 sprint_skeleton 的两类 expected_findings)
    d.findings.push(HarnessFinding {
        finding_id: Uuid::new_v4(),
        kind: "subdomain".to_string(),
        subject: "api.example.com".to_string(),
        severity: FindingSeverity::Info,
        evidence_refs: vec![dns_eid, ct_eid],
        technique: None,
    });
    d.findings.push(HarnessFinding {
        finding_id: Uuid::new_v4(),
        kind: "http_service".to_string(),
        subject: "api.example.com:443".to_string(),
        severity: FindingSeverity::Info,
        evidence_refs: vec![http_eid],
        technique: None,
    });
    // JS/API finding: 满足 surface_coverage_check 的 JsApi 硬要求
    // (design doc 2026-06-01 §D2 · re-anchor 到 Target Surface Workbench).
    d.findings.push(HarnessFinding {
        finding_id: Uuid::new_v4(),
        kind: "api_endpoint".to_string(),
        subject: "api.example.com/v1/login".to_string(),
        severity: FindingSeverity::Info,
        evidence_refs: vec![http_eid],
        technique: None,
    });
    d
}

fn eas_fact(asset: &str, technique: &str) -> EvidenceFact {
    EvidenceFact {
        asset: asset.to_string(),
        technique: technique.to_string(),
        outcome: EvidenceOutcome::Found,
        evidence_id: 2,
    }
}

fn eas_db_truth_context() -> GateContext {
    GateContextBuilder::new()
        .in_scope_assets(vec!["api.example.com".to_string()])
        .extend_evidence_facts(vec![
            eas_fact("api.example.com", "GOLISH-EAS-LIVENESS"),
            eas_fact("api.example.com", "GOLISH-EAS-PORT"),
            eas_fact("api.example.com", "GOLISH-EAS-SERVICE-FINGERPRINT"),
        ])
        .build()
}

#[test]
fn e2e_happy_path_external_attack_surface_passes_gate() {
    let harness = build_harness();
    let d = happy_deliverable(Uuid::new_v4());
    let ctx = eas_db_truth_context();
    let decision = harness.validate_gate_with_context(&d, None, &ctx);
    assert!(
        decision.allowed,
        "happy deliverable should pass gate: reasons={:?}",
        decision.reasons
    );
    assert!(decision.reasons.is_empty());
    assert!(decision.recovery_actions.is_none());
    // Doc 4 占位字段当前不填 (Phase 2 落 Observability)
    assert!(decision.gate_result_id.is_none());
    assert!(decision.blocking_reason_id.is_none());
}

#[test]
fn e2e_external_attack_surface_allows_found_coverage_without_model_ids() {
    let harness = build_harness();
    let mut d = happy_deliverable(Uuid::new_v4());
    d.coverage.push(CoverageCell {
        asset: "api.example.com".to_string(),
        technique: "GOLISH-EAS-PORT".to_string(),
        status: CoverageStatus::Found,
        evidence_refs: vec![],
        note: None,
        reason_kind: None,
        tested_units: 1,
        total_units: 1,
        sampling_rationale: None,
    });

    let ctx = eas_db_truth_context();
    let decision = harness.validate_gate_with_context(&d, None, &ctx);
    assert!(
        decision.allowed,
        "found coverage model evidence_refs are optional; DB truth decides: {:?}",
        decision.reasons
    );
}

#[test]
fn e2e_external_attack_surface_db_truth_ignores_hand_copied_denominator() {
    let harness = build_harness();
    let mut d = happy_deliverable(Uuid::new_v4());
    d.coverage.push(CoverageCell {
        asset: "api.example.com".to_string(),
        technique: "GOLISH-EAS-SERVICE-FINGERPRINT".to_string(),
        status: CoverageStatus::Found,
        evidence_refs: vec![EvidenceAuditId::new(2)],
        note: None,
        reason_kind: None,
        tested_units: 1,
        total_units: 2,
        sampling_rationale: None,
    });

    let ctx = eas_db_truth_context();
    let decision = harness.validate_gate_with_context(&d, None, &ctx);
    assert!(
        decision.allowed,
        "DB-truth EAS denominator should be no-op: {:?}",
        decision.reasons
    );
}

#[test]
fn e2e_vacuous_deliverable_is_blocked_with_recovery() {
    let harness = build_harness();
    let d = ExternalAttackSurfaceDeliverable {
        stage_id: "external_attack_surface".to_string(),
        stage_run_id: Uuid::new_v4(),
        claims: vec![],
        evidence_refs: vec![],
        skipped_checks: vec![],
        findings: vec![],
        required_checks_done: vec![],
        coverage: vec![],
        candidates: vec![],
    };
    let decision = harness.validate_gate(&d, None);
    assert!(!decision.allowed);
    let recovery = decision
        .recovery_actions
        .as_ref()
        .expect("recovery_actions should be set on block");
    assert!(!recovery.is_empty());
    // 必含 "vacuous" 类型 block reason
    assert!(decision.reasons.iter().any(|r| r.contains("vacuous")));
}

#[test]
fn e2e_finding_missing_evidence_refs_still_passes_with_db_truth() {
    let harness = build_harness();
    let mut d = happy_deliverable(Uuid::new_v4());
    // 故意把第 0 个 finding 的 evidence_refs 清空。模型侧 ids 不再是 gate 条件；
    // DB truth / fabricated-id checks 才是证据真实性来源。
    d.findings[0].evidence_refs.clear();
    let ctx = eas_db_truth_context();
    let decision = harness.validate_gate_with_context(&d, None, &ctx);
    assert!(
        decision.allowed,
        "missing model evidence_refs should not block: {:?}",
        decision.reasons
    );
}

#[test]
fn e2e_finding_references_are_optional_in_pure_stage_gate() {
    let harness = build_harness();
    let mut d = happy_deliverable(Uuid::new_v4());
    // 加一个 finding 引用 deliverable.evidence_refs 之外的 eid。纯 StageHarness
    // 没有 ledger repo，不在这里判 fabricated；submit tool/runtime 会查真实 ledger。
    d.findings.push(HarnessFinding {
        finding_id: Uuid::new_v4(),
        kind: "subdomain".to_string(),
        subject: "phantom.example.com".to_string(),
        severity: FindingSeverity::Info,
        evidence_refs: vec![EvidenceAuditId::new(9999)],
        technique: None,
    });
    let ctx = eas_db_truth_context();
    let decision = harness.validate_gate_with_context(&d, None, &ctx);
    assert!(
        decision.allowed,
        "pure gate should not require top-level id mirroring: {:?}",
        decision.reasons
    );
}

#[test]
fn e2e_contract_check_with_skeleton_passes_in_range() {
    // happy deliverable 含 1 subdomain (in [1,200]) + 1 http_service (in [0,50]).
    // min_invocations is no longer inferred from model evidence_refs.
    let skeleton_full = SprintSkeleton::from_json(ASSESSMENT_SKELETON_JSON).expect("skeleton");
    let stage_sk = skeleton_full
        .for_stage(StageKind::ExternalAttackSurface)
        .expect("stage skeleton");
    let d = happy_deliverable(Uuid::new_v4());
    let outcome = contract_check_with_skeleton(&d, None, Some(stage_sk));
    assert!(matches!(outcome, GateCheckOutcome::Pass));
}

#[test]
fn e2e_contract_check_below_min_subdomain_blocks() {
    let skeleton_full = SprintSkeleton::from_json(ASSESSMENT_SKELETON_JSON).expect("skeleton");
    let stage_sk = skeleton_full
        .for_stage(StageKind::ExternalAttackSurface)
        .expect("stage skeleton");
    let mut d = happy_deliverable(Uuid::new_v4());
    // 删掉 subdomain finding → count=0 < expected_min=1 → block
    d.findings.retain(|f| f.kind != "subdomain");
    let outcome = contract_check_with_skeleton(&d, None, Some(stage_sk));
    match outcome {
        GateCheckOutcome::Block { reasons, recovery } => {
            assert!(reasons
                .iter()
                .any(|r| r.contains("subdomain") && r.contains("below contract minimum")));
            assert!(!recovery.missing_evidence_kinds.is_empty());
        }
        _ => panic!("expected Block"),
    }
}

#[test]
fn e2e_freshness_check_with_real_evidence_kinds_fresh_passes() {
    let harness = build_harness();
    let d = happy_deliverable(Uuid::new_v4());
    let mut kinds = HashMap::new();
    kinds.insert(EvidenceAuditId::new(1), "dns_a".to_string()); // 1d max
    kinds.insert(EvidenceAuditId::new(2), "http_probe".to_string()); // 6h max
    kinds.insert(EvidenceAuditId::new(3), "ct_log".to_string()); // 7d max
    let mut ages = HashMap::new();
    ages.insert(EvidenceAuditId::new(1), StdDuration::from_secs(60)); // 1 min
    ages.insert(EvidenceAuditId::new(2), StdDuration::from_secs(300)); // 5 min
    ages.insert(EvidenceAuditId::new(3), StdDuration::from_secs(3600)); // 1 hour

    let outcome = freshness_check_with_freshness(&d, &harness.stage_spec, &kinds, &ages);
    assert!(matches!(outcome, GateCheckOutcome::Pass));
}

#[test]
fn e2e_freshness_check_one_expired_blocks_with_repair() {
    let harness = build_harness();
    let d = happy_deliverable(Uuid::new_v4());
    let mut kinds = HashMap::new();
    kinds.insert(EvidenceAuditId::new(1), "dns_a".to_string());
    kinds.insert(EvidenceAuditId::new(2), "http_probe".to_string());
    kinds.insert(EvidenceAuditId::new(3), "ct_log".to_string());
    let mut ages = HashMap::new();
    ages.insert(EvidenceAuditId::new(1), StdDuration::from_secs(60));
    // http_probe max=21600s (6h), 给 24h → hard expired
    ages.insert(EvidenceAuditId::new(2), StdDuration::from_secs(24 * 3600));
    ages.insert(EvidenceAuditId::new(3), StdDuration::from_secs(3600));

    let outcome = freshness_check_with_freshness(&d, &harness.stage_spec, &kinds, &ages);
    match outcome {
        GateCheckOutcome::Block { reasons, recovery } => {
            assert!(reasons
                .iter()
                .any(|r| r.contains("hard-expired") && r.contains("http_probe")));
            assert!(recovery
                .repair_tool_calls
                .iter()
                .any(|c| c.contains("re-acquire fresh http_probe")));
        }
        _ => panic!("expected Block (hard-expired)"),
    }
}

#[test]
fn e2e_skipped_check_other_with_evidence_ref_under_threshold_passes() {
    let harness = build_harness();
    let mut d = happy_deliverable(Uuid::new_v4());
    // 加 1 个 Other-skip (max_other_skips=2 不超), 必带 evidence_ref
    d.skipped_checks.push(SkippedCheckRecord {
        check: "shodan_lookup".to_string(),
        reason: SkipReason::Other {
            explanation: "Shodan API rate limit hit".to_string(),
            evidence_ref: EvidenceAuditId::new(1),
        },
    });
    let ctx = eas_db_truth_context();
    let decision = harness.validate_gate_with_context(&d, None, &ctx);
    assert!(
        decision.allowed,
        "single Other-skip below threshold should not block: reasons={:?}",
        decision.reasons
    );
}

#[tokio::test]
async fn e2e_sprint_contract_default_generator_pipeline() {
    // 验证 SprintContract Generator → validate_gate(contract=Some) 端到端不破.
    let harness = build_harness();
    let skeleton_full = SprintSkeleton::from_json(ASSESSMENT_SKELETON_JSON).expect("skeleton");
    let stage_sk = skeleton_full
        .for_stage(StageKind::ExternalAttackSurface)
        .expect("stage skeleton");
    let gen = DefaultSprintContractGenerator;
    let stage_run_id = Uuid::new_v4();
    let contract = gen
        .generate(
            stage_run_id,
            StageKind::ExternalAttackSurface,
            stage_sk,
            "example.com",
        )
        .await
        .expect("generate contract");
    assert_eq!(contract.planner_llm_id, "deterministic-default");
    assert_eq!(contract.status, "active");

    let d = happy_deliverable(stage_run_id);
    let ctx = eas_db_truth_context();
    let decision = harness.validate_gate_with_context(&d, Some(&contract), &ctx);
    assert!(
        decision.allowed,
        "happy deliverable with active contract should pass: reasons={:?}",
        decision.reasons
    );
}
