//! Doc 3 §8.2 scope_check.
//!
//! Evidence ids are ledger internals, not model-authored completion fields.
//! This check intentionally no longer requires every claim/finding to carry
//! ids; fabricated ids are still rejected in the runtime ledger-existence hook
//! when the model supplies them.

use super::super::types::ExternalAttackSurfaceDeliverable;
use super::GateCheckOutcome;

pub fn run(deliverable: &ExternalAttackSurfaceDeliverable) -> GateCheckOutcome {
    tracing::info!(
        target: "harness::gate::scope_check",
        stage_id = %deliverable.stage_id,
        stage_run_id = %deliverable.stage_run_id,
        claims = deliverable.claims.len(),
        findings = deliverable.findings.len(),
        outcome = "pass",
        "scope_check pass (model-authored evidence ids optional)"
    );
    GateCheckOutcome::Pass
}

#[cfg(test)]
mod tests {
    use super::super::super::types::{ExternalAttackSurfaceDeliverable, StageClaim};
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
            candidates: vec![],
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
    fn claim_without_evidence_passes() {
        let mut d = empty_deliverable();
        d.claims.push(StageClaim {
            kind: "http_service_observed".to_string(),
            subject: "api.example.com".to_string(),
            summary: "200 OK".to_string(),
            evidence_ids: vec![],
            technique: None,
        });
        assert!(matches!(run(&d), GateCheckOutcome::Pass));
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
    fn finding_without_evidence_refs_passes() {
        let mut d = empty_deliverable();
        d.findings.push(super::super::super::types::HarnessFinding {
            finding_id: Uuid::new_v4(),
            kind: "open_port".to_string(),
            subject: "api.example.com:443".to_string(),
            severity: super::super::super::types::FindingSeverity::Info,
            evidence_refs: vec![],
            technique: None,
        });
        assert!(matches!(run(&d), GateCheckOutcome::Pass));
    }
}
