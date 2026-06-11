//! Doc 3 §8.2 scope_check · claim.evidence_refs[*] 当前 label = InScope.
//!
//! Phase 1c.2 skeleton · 仅做 evidence_refs 非空 sanity + claims 引用一致性.
//! Task 1c.5 真实 scope label 检查 (查 evidence_classifications.current).

use super::super::types::{ExternalAttackSurfaceDeliverable, HarnessRecoveryActions};
use super::GateCheckOutcome;

pub fn run(deliverable: &ExternalAttackSurfaceDeliverable) -> GateCheckOutcome {
    let mut reasons = Vec::new();

    // Scoping is an L0 authorization-confirmation stage ("L0-L1 only, no
    // probing"): there is no scan, so its scope claim is backed by the
    // authorization framework rather than a tool run. Skip the
    // evidence-required sanity for scoping so an honest "scope confirmed, no
    // tool evidence" deliverable can pass (the evidence-required rule still
    // applies to every scanning stage).
    let evidence_optional = deliverable.stage_id == "scoping";

    // Sanity 1: 每个 claim 必有非空 evidence_ids (Doc 3 §4.3 "必须非空 + 全 InScope")
    if !evidence_optional {
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
    }

    if reasons.is_empty() {
        tracing::info!(
            target: "harness::gate::scope_check",
            stage_id = %deliverable.stage_id,
            stage_run_id = %deliverable.stage_run_id,
            claims = deliverable.claims.len(),
            findings = deliverable.findings.len(),
            outcome = "pass",
            "scope_check pass"
        );
        GateCheckOutcome::Pass
    } else {
        tracing::info!(
            target: "harness::gate::scope_check",
            stage_id = %deliverable.stage_id,
            stage_run_id = %deliverable.stage_run_id,
            outcome = "block",
            reasons_count = reasons.len(),
            first_reason = %reasons[0],
            "scope_check block"
        );
        let mut recovery = HarnessRecoveryActions::default();
        recovery
            .hints
            .push("add evidence_refs to each claim/finding via prior tool calls".to_string());
        GateCheckOutcome::Block { reasons, recovery }
    }
}

#[cfg(test)]
mod tests {
    use super::super::super::types::{
        ExternalAttackSurfaceDeliverable, FindingSeverity, HarnessFinding, StageClaim,
    };
    use super::*;
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
            coverage: vec![],
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
            technique: None,
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
            technique: None,
        });
        match run(&d) {
            GateCheckOutcome::Block { reasons, .. } => {
                assert!(reasons[0].contains("empty evidence_ids"));
            }
            _ => panic!("expected Block"),
        }
    }

    #[test]
    fn scoping_claim_without_evidence_passes() {
        // Scoping is authz-confirmation (no probing) → a scope_confirmed claim
        // needs no tool evidence; scope_check must NOT block it (it would for any
        // scanning stage — see `claim_without_evidence_blocks`).
        let mut d = empty_deliverable();
        d.stage_id = "scoping".to_string();
        d.claims.push(StageClaim {
            kind: "scope_confirmed".to_string(),
            subject: "example.com".to_string(),
            summary: "authorized target, black-box external".to_string(),
            evidence_ids: vec![],
            technique: None,
        });
        assert!(matches!(run(&d), GateCheckOutcome::Pass));
    }

    #[test]
    fn finding_without_evidence_refs_blocks() {
        let mut d = empty_deliverable();
        d.findings.push(HarnessFinding {
            finding_id: Uuid::new_v4(),
            kind: "open_port".to_string(),
            subject: "api.example.com:443".to_string(),
            severity: FindingSeverity::Info,
            evidence_refs: vec![],
            technique: None,
        });
        match run(&d) {
            GateCheckOutcome::Block { reasons, .. } => {
                assert!(reasons[0].contains("empty evidence_refs"));
            }
            _ => panic!("expected Block"),
        }
    }
}
