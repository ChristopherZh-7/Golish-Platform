//! Doc 3 §8.2 contract_check · findings 数量在 Sprint Contract range 内.
//!
//! Phase 1c.5 完整版:
//!   - contract.status='active' 强制
//!   - 已知 SprintSkeleton 时按 expected_count_range 校验 finding kind 数量
//!   - 已知 min_tool_invocations 时按 deliverable evidence_refs 推断 + 比较
//!     (Phase 1 用 deliverable evidence_refs 长度 ≥ min 的简化路径; Phase 2
//!     接 EvidenceLedger 真 tool_call_counts)
//!
//! 调用方 (StageHarness) 可显式传 `skeleton` 以启用范围检查; 不传 → 仅做
//! contract status 校验.

use std::collections::HashMap;

use super::super::sprint_contract::{SprintContract, StageSkeleton};
use super::super::types::{ExternalAttackSurfaceDeliverable, HarnessRecoveryActions};
use super::GateCheckOutcome;

pub fn run(
    deliverable: &ExternalAttackSurfaceDeliverable,
    contract: Option<&SprintContract>,
) -> GateCheckOutcome {
    run_with_skeleton(deliverable, contract, None)
}

/// 完整版本: 额外接受 `StageSkeleton` 启用 expected_count_range + min_tool_invocations
/// 校验.
pub fn run_with_skeleton(
    deliverable: &ExternalAttackSurfaceDeliverable,
    contract: Option<&SprintContract>,
    skeleton: Option<&StageSkeleton>,
) -> GateCheckOutcome {
    let mut reasons = Vec::new();
    let mut recovery = HarnessRecoveryActions::default();

    // 1. contract status 强制 active.
    if let Some(c) = contract {
        if c.status != "active" {
            reasons.push(format!(
                "sprint_contract {} not active (status={})",
                c.id, c.status
            ));
            recovery.hints.push(format!(
                "active sprint_contract required, found status={}",
                c.status
            ));
        }
    }

    // 2. skeleton 提供 expected_count_range → 按 finding kind 分组计数比较.
    if let Some(sk) = skeleton {
        let counts = count_findings_by_kind(deliverable);
        for ef in &sk.expected_findings {
            let actual = counts.get(&ef.kind).copied().unwrap_or(0);
            let [lo, hi] = ef.expected_count_range;
            if actual < lo {
                reasons.push(format!(
                    "finding kind '{}' count {} below contract minimum {}",
                    ef.kind, actual, lo
                ));
                recovery
                    .repair_tool_calls
                    .push(format!("produce more findings of kind '{}'", ef.kind));
                // 缺少 required_evidence_kinds 中的 kinds 也同时报
                for ek in &ef.required_evidence_kinds {
                    recovery.missing_evidence_kinds.push(ek.clone());
                }
            } else if actual > hi {
                reasons.push(format!(
                    "finding kind '{}' count {} exceeds contract maximum {}",
                    ef.kind, actual, hi
                ));
                recovery.hints.push(format!(
                    "finding count for '{}' exceeds expected ceiling; review for false positives",
                    ef.kind
                ));
            }
        }

        // 3. min_tool_invocations 校验 · Phase 1 MVP 用 deliverable.evidence_refs
        //    数量做下限 (真 tool_call_count 推 Phase 2 接 EvidenceLedger).
        let total_evidence = deliverable.evidence_refs.len() as u32;
        let required_total: u32 = sk.min_tool_invocations.values().sum();
        if total_evidence < required_total {
            reasons.push(format!(
                "total evidence ({}) below sum of min_tool_invocations ({})",
                total_evidence, required_total
            ));
            recovery.hints.push(
                "ensure each min_tool_invocation is satisfied (one evidence per invocation expected)"
                    .to_string(),
            );
        }
    }

    if reasons.is_empty() {
        GateCheckOutcome::Pass
    } else {
        GateCheckOutcome::Block { reasons, recovery }
    }
}

fn count_findings_by_kind(d: &ExternalAttackSurfaceDeliverable) -> HashMap<String, u32> {
    let mut m: HashMap<String, u32> = HashMap::new();
    for f in &d.findings {
        *m.entry(f.kind.clone()).or_insert(0) += 1;
    }
    m
}

#[cfg(test)]
mod tests {
    use super::*;
    use super::super::super::sprint_contract::{SprintContract, SprintSkeleton, StageSkeleton, ExpectedFinding};
    use super::super::super::types::{ExternalAttackSurfaceDeliverable, Finding, FindingSeverity};
    use golish_pentest::evidence_ledger::EvidenceAuditId;
    use uuid::Uuid;

    const ASSESSMENT_SKELETON_JSON: &str = include_str!(
        "../../../../../../resources/harness/profiles/assessment.sprint_skeleton.json"
    );

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

    fn deliverable_with_findings(subdomain_count: u32, http_count: u32, evidence_count: u32) -> ExternalAttackSurfaceDeliverable {
        let mut d = empty_deliverable();
        for i in 0..subdomain_count {
            d.findings.push(Finding {
                finding_id: Uuid::new_v4(),
                kind: "subdomain".to_string(),
                subject: format!("sub{}.example.com", i),
                severity: FindingSeverity::Info,
                evidence_refs: vec![EvidenceAuditId::new(i as i64 + 1)],
            });
        }
        for i in 0..http_count {
            d.findings.push(Finding {
                finding_id: Uuid::new_v4(),
                kind: "http_service".to_string(),
                subject: format!("http{}.example.com", i),
                severity: FindingSeverity::Info,
                evidence_refs: vec![EvidenceAuditId::new(i as i64 + 100)],
            });
        }
        d.evidence_refs = (1..=(evidence_count as i64)).map(EvidenceAuditId::new).collect();
        d
    }

    fn skeleton() -> StageSkeleton {
        let s = SprintSkeleton::from_json(ASSESSMENT_SKELETON_JSON).expect("parse");
        s.stages
            .get("external_attack_surface")
            .cloned()
            .expect("external_attack_surface stage")
    }

    #[test]
    fn no_contract_passes() {
        assert!(matches!(run(&empty_deliverable(), None), GateCheckOutcome::Pass));
    }

    #[test]
    fn active_contract_passes() {
        let c = SprintContract::new_active(
            Uuid::new_v4(),
            "contract text".to_string(),
            "openai:gpt-4o".to_string(),
        );
        assert!(matches!(run(&empty_deliverable(), Some(&c)), GateCheckOutcome::Pass));
    }

    #[test]
    fn superseded_contract_blocks() {
        let mut c = SprintContract::new_active(
            Uuid::new_v4(),
            "contract text".to_string(),
            "openai:gpt-4o".to_string(),
        );
        c.status = "superseded".to_string();
        match run(&empty_deliverable(), Some(&c)) {
            GateCheckOutcome::Block { reasons, .. } => {
                assert!(reasons[0].contains("not active"));
            }
            _ => panic!("expected Block"),
        }
    }

    #[test]
    fn with_skeleton_finding_count_below_min_blocks() {
        let sk = skeleton();
        // 期望 subdomain 至少 1; 给 0 个 → block.
        let d = deliverable_with_findings(0, 5, 5);
        let outcome = run_with_skeleton(&d, None, Some(&sk));
        match outcome {
            GateCheckOutcome::Block { reasons, recovery } => {
                assert!(reasons.iter().any(|r| r.contains("subdomain") && r.contains("below contract minimum")));
                assert!(recovery
                    .missing_evidence_kinds
                    .iter()
                    .any(|e| e == "dns_a" || e == "ct_log"));
            }
            _ => panic!("expected Block"),
        }
    }

    #[test]
    fn with_skeleton_finding_count_above_max_blocks() {
        let sk = skeleton();
        // 期望 http_service 最多 50; 给 60 个 → block.
        let d = deliverable_with_findings(1, 60, 100);
        match run_with_skeleton(&d, None, Some(&sk)) {
            GateCheckOutcome::Block { reasons, .. } => {
                assert!(reasons.iter().any(|r| r.contains("http_service") && r.contains("exceeds contract maximum")));
            }
            _ => panic!("expected Block"),
        }
    }

    #[test]
    fn with_skeleton_finding_count_in_range_passes() {
        let sk = skeleton();
        // 1 subdomain + 1 http_service + 至少 3 evidence (满足 min_tool_invocations=3)
        let d = deliverable_with_findings(1, 1, 5);
        assert!(matches!(run_with_skeleton(&d, None, Some(&sk)), GateCheckOutcome::Pass));
    }

    #[test]
    fn with_skeleton_min_tool_invocations_below_blocks() {
        let sk = skeleton();
        // dns_resolve+http_probe+subdomain_enum_passive 至少 3; 给 1 个 evidence → block.
        let d = deliverable_with_findings(1, 1, 1);
        match run_with_skeleton(&d, None, Some(&sk)) {
            GateCheckOutcome::Block { reasons, .. } => {
                assert!(reasons.iter().any(|r| r.contains("min_tool_invocations")));
            }
            _ => panic!("expected Block"),
        }
    }

    #[test]
    fn with_custom_skeleton_zero_range_pass() {
        // 自定义 skeleton: subdomain 0-0 → 给 0 个就 pass.
        let sk = StageSkeleton {
            expected_findings: vec![ExpectedFinding {
                kind: "subdomain".to_string(),
                expected_count_range: [0, 0],
                required_evidence_kinds: vec![],
            }],
            time_budget_minutes: 10,
            min_tool_invocations: std::collections::HashMap::new(),
        };
        let d = deliverable_with_findings(0, 0, 0);
        assert!(matches!(run_with_skeleton(&d, None, Some(&sk)), GateCheckOutcome::Pass));
    }
}
