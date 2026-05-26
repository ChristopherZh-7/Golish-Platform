//! Doc 3 §8.2 scope_check · claim.evidence_refs[*] 当前 label = InScope.
//!
//! Phase 1c.2 skeleton · 仅做 evidence_refs 非空 sanity + claims 引用一致性.
//! Task 1c.5 真实 scope label 检查 (查 evidence_classifications.current).

use super::super::types::{ExternalAttackSurfaceDeliverable, HarnessRecoveryActions};
use super::GateCheckOutcome;

pub fn run(deliverable: &ExternalAttackSurfaceDeliverable) -> GateCheckOutcome {
    let mut reasons = Vec::new();

    // Sanity 1: 每个 claim 必有非空 evidence_ids (Doc 3 §4.3 "必须非空 + 全 InScope")
    for (idx, claim) in deliverable.claims.iter().enumerate() {
        if claim.evidence_ids.is_empty() {
            reasons.push(format!(
                "claim[{}] (kind={}, subject={}) has empty evidence_ids",
                idx, claim.kind, claim.subject
            ));
        }
    }

    // Sanity 2: 每个 finding 必有非空 evidence_refs
    for (idx, f) in deliverable.findings.iter().enumerate() {
        if f.evidence_refs.is_empty() {
            reasons.push(format!(
                "finding[{}] (kind={}, subject={}) has empty evidence_refs",
                idx, f.kind, f.subject
            ));
        }
    }

    if reasons.is_empty() {
        GateCheckOutcome::Pass
    } else {
        let mut recovery = HarnessRecoveryActions::default();
        recovery
            .hints
            .push("add evidence_refs to each claim/finding via prior tool calls".to_string());
        GateCheckOutcome::Block { reasons, recovery }
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use super::super::super::types::{
        ExternalAttackSurfaceDeliverable, Finding, FindingSeverity, StageClaim,
    };
    use golish_pentest::evidence_ledger::EvidenceAuditId;
    use uuid::Uuid;

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
    fn empty_deliverable_passes_scope_check() {
        // 无 claim 无 finding → 无规则可违反 → Pass.
        // Vacuous check 会单独拦 empty deliverable.
        let d = empty_deliverable();
        assert!(matches!(run(&d), GateCheckOutcome::Pass));
    }

    #[test]
    fn claim_with_evidence_passes() {
        let mut d = empty_deliverable();
        d.claims.push(StageClaim {
            kind: "http_service_observed".to_string(),
            subject: "api.example.com".to_string(),
            summary: "200 OK".to_string(),
            evidence_ids: vec![EvidenceAuditId::new(1)],
        });
        assert!(matches!(run(&d), GateCheckOutcome::Pass));
    }

    #[test]
    fn claim_without_evidence_blocks() {
        let mut d = empty_deliverable();
        d.claims.push(StageClaim {
            kind: "http_service_observed".to_string(),
            subject: "api.example.com".to_string(),
            summary: "200 OK".to_string(),
            evidence_ids: vec![],
        });
        match run(&d) {
            GateCheckOutcome::Block { reasons, .. } => {
                assert!(reasons[0].contains("empty evidence_ids"));
            }
            _ => panic!("expected Block"),
        }
    }

    #[test]
    fn finding_without_evidence_refs_blocks() {
        let mut d = empty_deliverable();
        d.findings.push(Finding {
            finding_id: Uuid::new_v4(),
            kind: "open_port".to_string(),
            subject: "api.example.com:443".to_string(),
            severity: FindingSeverity::Info,
            evidence_refs: vec![],
        });
        match run(&d) {
            GateCheckOutcome::Block { reasons, .. } => {
                assert!(reasons[0].contains("empty evidence_refs"));
            }
            _ => panic!("expected Block"),
        }
    }
}
