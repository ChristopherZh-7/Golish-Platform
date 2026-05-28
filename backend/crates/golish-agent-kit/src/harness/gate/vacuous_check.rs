//! Doc 3 §8.3 vacuous detector · 防 agent 不调任何工具就提交.
//!
//! Phase 1c.5 完整版本:
//!   (a) NoToolInvocation: deliverable 完全空 (no claim, no finding,
//!       no skipped_check)
//!   (b) FakePattern: stage_spec.required_checks 中每个 check 没有对应
//!       evidence (deliverable.evidence_refs 长度 < min_invocations sum)
//!   (c) SkipPattern: Other-type skip > spec.max_other_skips 或 evidence_ref
//!       不在 deliverable.evidence_refs (来自 §8.3 检查)
//!
//! 关键: detector 以 `StageSpec.required_checks` 为准绳, **不**读
//! `deliverable.required_checks_done` (agent 可清空该字段绕过) (Doc 3 §8.3).

use golish_pentest::evidence_ledger::SkipReason;

use super::super::stage_spec::StageSpec;
use super::super::types::{ExternalAttackSurfaceDeliverable, HarnessRecoveryActions};
use super::GateCheckOutcome;

pub fn run(deliverable: &ExternalAttackSurfaceDeliverable, spec: &StageSpec) -> GateCheckOutcome {
    let mut reasons = Vec::new();
    let mut missing_kinds = Vec::new();

    // (a) Vacuous · 完全空交付 (无 claim 无 finding 无 skipped_checks)
    if deliverable.claims.is_empty()
        && deliverable.findings.is_empty()
        && deliverable.skipped_checks.is_empty()
    {
        reasons.push("deliverable vacuous: no claims, no findings, no skipped_checks".to_string());
        missing_kinds.push("dns_a".to_string());
        missing_kinds.push("http_probe".to_string());
    }

    // (b) FakePattern · stage_spec.required_checks 中每个 check 验 evidence_refs 数量
    //     Doc 3 §8.3 简化版: total evidence_refs 必须 >= sum(min_invocations).
    //     真正每个 check 的 tool_call_count 推 Phase 2 接 EvidenceLedger.
    if !spec.required_checks.is_empty() {
        let required_total: u32 = spec.min_invocations.values().sum();
        let actual_total = deliverable.evidence_refs.len() as u32;
        if required_total > 0 && actual_total < required_total {
            reasons.push(format!(
                "FakePattern: total evidence_refs ({}) below required_checks min_invocations sum ({})",
                actual_total, required_total
            ));
            // missing_kinds 提示从 min_invocations.keys() 来
            for k in spec.min_invocations.keys() {
                missing_kinds.push(k.clone());
            }
        }
    }

    // (c) SkipPattern · Other-type skip 超阈值
    let other_count = deliverable
        .skipped_checks
        .iter()
        .filter(|s| matches!(s.reason, SkipReason::Other { .. }))
        .count() as u32;
    let max_other = spec.max_other_skips.unwrap_or(2);
    if other_count > max_other {
        reasons.push(format!(
            "SkipPattern: Other-type skip count ({}) exceeds max_other_skips ({})",
            other_count, max_other
        ));
    }

    if reasons.is_empty() {
        tracing::info!(
            target: "harness::gate::vacuous_check",
            stage_id = %deliverable.stage_id,
            stage_run_id = %deliverable.stage_run_id,
            claims = deliverable.claims.len(),
            findings = deliverable.findings.len(),
            skipped_checks = deliverable.skipped_checks.len(),
            evidence_refs = deliverable.evidence_refs.len(),
            outcome = "pass",
            "vacuous_check pass"
        );
        GateCheckOutcome::Pass
    } else {
        tracing::info!(
            target: "harness::gate::vacuous_check",
            stage_id = %deliverable.stage_id,
            stage_run_id = %deliverable.stage_run_id,
            claims = deliverable.claims.len(),
            findings = deliverable.findings.len(),
            skipped_checks = deliverable.skipped_checks.len(),
            evidence_refs = deliverable.evidence_refs.len(),
            outcome = "block",
            reasons_count = reasons.len(),
            first_reason = %reasons[0],
            "vacuous_check block"
        );
        let mut recovery = HarnessRecoveryActions::default();
        recovery.hints.push(
            "invoke at least one tool from stage_spec.allowed_tools and submit findings"
                .to_string(),
        );
        for kind in missing_kinds {
            recovery.missing_evidence_kinds.push(kind);
        }
        GateCheckOutcome::Block { reasons, recovery }
    }
}

#[cfg(test)]
mod tests {
    use super::super::super::stage_spec::load_stage_spec_from_json;
    use super::super::super::types::{
        ExternalAttackSurfaceDeliverable, Finding, FindingSeverity, SkippedCheckRecord, StageClaim,
    };
    use super::*;
    use golish_pentest::evidence_ledger::EvidenceAuditId;
    use uuid::Uuid;

    const STAGE_JSON: &str =
        include_str!("../../../../../../resources/harness/stages/external_attack_surface.json");

    fn empty_deliverable() -> ExternalAttackSurfaceDeliverable {
        ExternalAttackSurfaceDeliverable {
            stage_id: "external_attack_surface".to_string(),
            stage_run_id: Uuid::new_v4(),
            claims: vec![],
            evidence_refs: vec![],
            skipped_checks: vec![],
            findings: vec![],
            required_checks_done: vec![],
        }
    }

    #[test]
    fn empty_deliverable_is_vacuous() {
        let spec = load_stage_spec_from_json(STAGE_JSON).unwrap();
        let d = empty_deliverable();
        match run(&d, &spec) {
            GateCheckOutcome::Block { reasons, recovery } => {
                assert!(reasons[0].contains("vacuous"));
                assert!(recovery
                    .missing_evidence_kinds
                    .contains(&"dns_a".to_string()));
            }
            _ => panic!("expected Block"),
        }
    }

    #[test]
    fn non_empty_deliverable_passes_when_min_invocations_met() {
        let spec = load_stage_spec_from_json(STAGE_JSON).unwrap();
        let mut d = empty_deliverable();
        d.findings.push(Finding {
            finding_id: Uuid::new_v4(),
            kind: "subdomain".to_string(),
            subject: "api.example.com".to_string(),
            severity: FindingSeverity::Info,
            evidence_refs: vec![EvidenceAuditId::new(1)],
        });
        // 凑 3 个 evidence_refs 满足 min_invocations sum (dns_resolve+http_probe+subdomain_enum_passive)
        d.evidence_refs = (1..=3).map(EvidenceAuditId::new).collect();
        assert!(matches!(run(&d, &spec), GateCheckOutcome::Pass));
    }

    #[test]
    fn other_skip_pattern_over_threshold_blocks() {
        let spec = load_stage_spec_from_json(STAGE_JSON).unwrap();
        let mut d = empty_deliverable();
        // 加一个 finding 让 deliverable 非 vacuous + 凑 3 evidence_refs 避开 FakePattern
        d.findings.push(Finding {
            finding_id: Uuid::new_v4(),
            kind: "subdomain".to_string(),
            subject: "x.example.com".to_string(),
            severity: FindingSeverity::Info,
            evidence_refs: vec![EvidenceAuditId::new(1)],
        });
        d.evidence_refs = (1..=3).map(EvidenceAuditId::new).collect();
        // 加 3 个 Other-type skip (max=2)
        for i in 0..3 {
            d.skipped_checks.push(SkippedCheckRecord {
                check: format!("check_{}", i),
                reason: SkipReason::Other {
                    explanation: "ambiguous".to_string(),
                    evidence_ref: EvidenceAuditId::new(i + 10),
                },
            });
        }
        match run(&d, &spec) {
            GateCheckOutcome::Block { reasons, .. } => {
                assert!(reasons.iter().any(|r| r.contains("SkipPattern")));
            }
            _ => panic!("expected Block"),
        }
    }

    #[test]
    fn claims_only_deliverable_is_not_vacuous_when_evidence_meets_min() {
        let spec = load_stage_spec_from_json(STAGE_JSON).unwrap();
        let mut d = empty_deliverable();
        d.claims.push(StageClaim {
            kind: "http_service_observed".to_string(),
            subject: "x.example.com".to_string(),
            summary: "200 OK".to_string(),
            evidence_ids: vec![EvidenceAuditId::new(1)],
        });
        // 满足 min_invocations sum=3 (dns_resolve+http_probe+subdomain_enum_passive)
        d.evidence_refs = (1..=3).map(EvidenceAuditId::new).collect();
        assert!(matches!(run(&d, &spec), GateCheckOutcome::Pass));
    }

    #[test]
    fn fake_pattern_evidence_refs_below_min_invocations_blocks() {
        let spec = load_stage_spec_from_json(STAGE_JSON).unwrap();
        let mut d = empty_deliverable();
        // 加 finding 让 deliverable 非 vacuous
        d.findings.push(Finding {
            finding_id: Uuid::new_v4(),
            kind: "subdomain".to_string(),
            subject: "x.example.com".to_string(),
            severity: FindingSeverity::Info,
            evidence_refs: vec![EvidenceAuditId::new(1)],
        });
        // 仅 1 个 evidence, min_invocations sum=3 → FakePattern
        d.evidence_refs = vec![EvidenceAuditId::new(1)];
        match run(&d, &spec) {
            GateCheckOutcome::Block { reasons, recovery } => {
                assert!(reasons.iter().any(|r| r.contains("FakePattern")));
                // missing_evidence_kinds 含 min_invocations.keys()
                assert!(recovery
                    .missing_evidence_kinds
                    .iter()
                    .any(|k| k == "dns_resolve"
                        || k == "http_probe"
                        || k == "subdomain_enum_passive"));
            }
            _ => panic!("expected Block"),
        }
    }
}
