//! Doc 3 §8.2 freshness_check · evidence as_of_timestamp + max_age 比较.
//!
//! Phase 1c.2 skeleton · evidence_refs 仅做 count 检查 (Task 1c.5 完整加入
//! per-evidence freshness lookup via evidence_kinds.json registry +
//! evidence_classifications.valid_from).

use super::super::stage_spec::StageSpec;
use super::super::types::{ExternalAttackSurfaceDeliverable, HarnessRecoveryActions};
use super::GateCheckOutcome;

pub fn run(
    deliverable: &ExternalAttackSurfaceDeliverable,
    _spec: &StageSpec,
) -> GateCheckOutcome {
    // Phase 1c.2 skeleton: 不做真 freshness 查 (需要 EvidenceLedger). 只做
    // sanity: evidence_refs 数量 vs claims/findings 引用 evidence_ids 一致性.
    let referenced_eids: std::collections::HashSet<_> = deliverable
        .claims
        .iter()
        .flat_map(|c| c.evidence_ids.iter().copied())
        .chain(
            deliverable
                .findings
                .iter()
                .flat_map(|f| f.evidence_refs.iter().copied()),
        )
        .collect();

    let registered_eids: std::collections::HashSet<_> =
        deliverable.evidence_refs.iter().copied().collect();

    let mut reasons = Vec::new();
    for eid in &referenced_eids {
        if !registered_eids.contains(eid) {
            reasons.push(format!(
                "evidence_audit_id={} referenced by claim/finding but not declared in deliverable.evidence_refs",
                eid.as_i64()
            ));
        }
    }

    if reasons.is_empty() {
        GateCheckOutcome::Pass
    } else {
        let mut recovery = HarnessRecoveryActions::default();
        recovery
            .hints
            .push("add all claim/finding-referenced evidence ids to deliverable.evidence_refs".to_string());
        GateCheckOutcome::Block { reasons, recovery }
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use super::super::super::stage_spec::load_stage_spec_from_json;
    use super::super::super::types::{
        ExternalAttackSurfaceDeliverable, Finding, FindingSeverity, StageClaim,
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
    fn empty_deliverable_passes_freshness_sanity() {
        let spec = load_stage_spec_from_json(STAGE_JSON).unwrap();
        let d = empty_deliverable();
        assert!(matches!(run(&d, &spec), GateCheckOutcome::Pass));
    }

    #[test]
    fn evidence_refs_complete_passes() {
        let spec = load_stage_spec_from_json(STAGE_JSON).unwrap();
        let mut d = empty_deliverable();
        let eid = EvidenceAuditId::new(7);
        d.evidence_refs = vec![eid];
        d.findings.push(Finding {
            finding_id: Uuid::new_v4(),
            kind: "subdomain".to_string(),
            subject: "x.example.com".to_string(),
            severity: FindingSeverity::Info,
            evidence_refs: vec![eid],
        });
        assert!(matches!(run(&d, &spec), GateCheckOutcome::Pass));
    }

    #[test]
    fn finding_evidence_not_in_deliverable_blocks() {
        let spec = load_stage_spec_from_json(STAGE_JSON).unwrap();
        let mut d = empty_deliverable();
        // 故意不把 eid=42 加到 deliverable.evidence_refs
        d.evidence_refs = vec![];
        d.findings.push(Finding {
            finding_id: Uuid::new_v4(),
            kind: "subdomain".to_string(),
            subject: "x.example.com".to_string(),
            severity: FindingSeverity::Info,
            evidence_refs: vec![EvidenceAuditId::new(42)],
        });
        match run(&d, &spec) {
            GateCheckOutcome::Block { reasons, .. } => {
                assert!(reasons[0].contains("evidence_audit_id=42"));
            }
            _ => panic!("expected Block"),
        }
    }

    #[test]
    fn claim_evidence_not_in_deliverable_blocks() {
        let spec = load_stage_spec_from_json(STAGE_JSON).unwrap();
        let mut d = empty_deliverable();
        d.evidence_refs = vec![];
        d.claims.push(StageClaim {
            kind: "http_service_observed".to_string(),
            subject: "x.example.com".to_string(),
            summary: "200 OK".to_string(),
            evidence_ids: vec![EvidenceAuditId::new(99)],
        });
        match run(&d, &spec) {
            GateCheckOutcome::Block { reasons, .. } => {
                assert!(reasons[0].contains("evidence_audit_id=99"));
            }
            _ => panic!("expected Block"),
        }
    }
}
