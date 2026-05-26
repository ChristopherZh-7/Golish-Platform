//! Doc 3 §8.3 vacuous detector · 防 agent 不调任何工具就提交.
//!
//! Phase 1c.2 skeleton · 仅基础 sanity (deliverable 完全空 / Other-type skip 超
//! `max_other_skips`). Task 1c.5 完整 detector (查 ledger.tool_call_count + min
//! invocations 验证).

use golish_pentest::evidence_ledger::SkipReason;

use super::super::stage_spec::StageSpec;
use super::super::types::{ExternalAttackSurfaceDeliverable, HarnessRecoveryActions};
use super::GateCheckOutcome;

pub fn run(
    deliverable: &ExternalAttackSurfaceDeliverable,
    spec: &StageSpec,
) -> GateCheckOutcome {
    let mut reasons = Vec::new();
    let mut missing_kinds = Vec::new();

    // (a) Vacuous · 完全空交付 (无 claim 无 finding 无 skipped_checks)
    if deliverable.claims.is_empty()
        && deliverable.findings.is_empty()
        && deliverable.skipped_checks.is_empty()
    {
        reasons.push(
            "deliverable vacuous: no claims, no findings, no skipped_checks".to_string(),
        );
        missing_kinds.push("dns_a".to_string());
        missing_kinds.push("http_probe".to_string());
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
            "Other-type skip count ({}) exceeds max_other_skips ({})",
            other_count, max_other
        ));
    }

    if reasons.is_empty() {
        GateCheckOutcome::Pass
    } else {
        let mut recovery = HarnessRecoveryActions::default();
        recovery
            .hints
            .push("invoke at least one tool from stage_spec.allowed_tools and submit findings".to_string());
        for kind in missing_kinds {
            recovery.missing_evidence_kinds.push(kind);
        }
        GateCheckOutcome::Block { reasons, recovery }
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use super::super::super::stage_spec::load_stage_spec_from_json;
    use super::super::super::types::{
        ExternalAttackSurfaceDeliverable, Finding, FindingSeverity, SkippedCheckRecord, StageClaim,
    };
    use golish_pentest::evidence_ledger::EvidenceAuditId;
    use uuid::Uuid;

    const STAGE_JSON: &str = include_str!(
        "../../../../../../resources/harness/stages/external_attack_surface.json"
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

    #[test]
    fn empty_deliverable_is_vacuous() {
        let spec = load_stage_spec_from_json(STAGE_JSON).unwrap();
        let d = empty_deliverable();
        match run(&d, &spec) {
            GateCheckOutcome::Block { reasons, recovery } => {
                assert!(reasons[0].contains("vacuous"));
                assert!(recovery.missing_evidence_kinds.contains(&"dns_a".to_string()));
            }
            _ => panic!("expected Block"),
        }
    }

    #[test]
    fn non_empty_deliverable_passes_basic_vacuous() {
        let spec = load_stage_spec_from_json(STAGE_JSON).unwrap();
        let mut d = empty_deliverable();
        d.findings.push(Finding {
            finding_id: Uuid::new_v4(),
            kind: "subdomain".to_string(),
            subject: "api.example.com".to_string(),
            severity: FindingSeverity::Info,
            evidence_refs: vec![EvidenceAuditId::new(1)],
        });
        // claims 仍空但 findings 非空, 不算 vacuous (sanity)
        assert!(matches!(run(&d, &spec), GateCheckOutcome::Pass));
    }

    #[test]
    fn other_skip_pattern_over_threshold_blocks() {
        let spec = load_stage_spec_from_json(STAGE_JSON).unwrap();
        let mut d = empty_deliverable();
        // 加一个 finding 让 deliverable 非 vacuous
        d.findings.push(Finding {
            finding_id: Uuid::new_v4(),
            kind: "subdomain".to_string(),
            subject: "x.example.com".to_string(),
            severity: FindingSeverity::Info,
            evidence_refs: vec![EvidenceAuditId::new(1)],
        });
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
                assert!(reasons[0].contains("Other-type skip"));
            }
            _ => panic!("expected Block"),
        }
    }

    #[test]
    fn claims_only_deliverable_is_not_vacuous() {
        let spec = load_stage_spec_from_json(STAGE_JSON).unwrap();
        let mut d = empty_deliverable();
        d.claims.push(StageClaim {
            kind: "http_service_observed".to_string(),
            subject: "x.example.com".to_string(),
            summary: "200 OK".to_string(),
            evidence_ids: vec![EvidenceAuditId::new(1)],
        });
        assert!(matches!(run(&d, &spec), GateCheckOutcome::Pass));
    }
}
