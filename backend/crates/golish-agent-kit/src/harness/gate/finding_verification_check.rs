//! P2 · config-driven "trustworthy conclusion" check (deliverable structural
//! layer). Reads `StageSpec.finding_verification` / `min_findings` /
//! `min_claims` and enforces exactly what the stage JSON declares — no
//! hardcoded per-stage criteria.
//!
//! The evidence-KIND layer (`required_evidence_kinds` and a rule's
//! `require_evidence_kinds`) needs a ledger lookup and is enforced caller-side
//! (see `execute.rs::enforce_evidence_kinds`); this check is the pure,
//! DB-free, deliverable-only half so it stays unit-testable.

use crate::harness::stage_spec::StageSpec;
use crate::harness::types::{HarnessRecoveryActions, StageDeliverable};

use super::GateCheckOutcome;

pub fn run(deliverable: &StageDeliverable, spec: &StageSpec) -> GateCheckOutcome {
    let mut reasons = Vec::new();
    let mut recovery = HarnessRecoveryActions::default();

    if let Some(rule) = &spec.finding_verification {
        let threshold = rule.min_severity.rank();
        let mut any_unverified = false;
        for f in &deliverable.findings {
            if f.severity.rank() >= threshold && f.evidence_refs.is_empty() {
                any_unverified = true;
                reasons.push(format!(
                    "finding {} ({}) at severity {:?} (>= required {:?}) has no evidence — \
                     unverified conclusions do not pass this stage",
                    f.finding_id, f.subject, f.severity, rule.min_severity
                ));
            }
        }
        if any_unverified && !rule.require_evidence_kinds.is_empty() {
            recovery
                .missing_evidence_kinds
                .extend(rule.require_evidence_kinds.iter().cloned());
            recovery.hints.push(format!(
                "Re-run verification tooling so each high/critical finding carries one of these \
                 evidence kinds: {}",
                rule.require_evidence_kinds.join(", ")
            ));
        }
    }

    if let Some(min) = spec.min_findings {
        if (deliverable.findings.len() as u32) < min {
            reasons.push(format!(
                "stage requires at least {} findings, got {}",
                min,
                deliverable.findings.len()
            ));
        }
    }
    if let Some(min) = spec.min_claims {
        if (deliverable.claims.len() as u32) < min {
            reasons.push(format!(
                "stage requires at least {} claims, got {}",
                min,
                deliverable.claims.len()
            ));
        }
    }

    if reasons.is_empty() {
        GateCheckOutcome::Pass
    } else {
        GateCheckOutcome::Block { reasons, recovery }
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::harness::stage_spec::FindingVerificationRule;
    use crate::harness::types::{FindingSeverity, HarnessFinding, StageKind};
    use golish_pentest::evidence_ledger::EvidenceAuditId;
    use uuid::Uuid;

    fn spec_with(rule: Option<FindingVerificationRule>) -> StageSpec {
        StageSpec {
            id: "verification".to_string(),
            kind: StageKind::Verification,
            risk_level: crate::harness::types::RiskLevel::High,
            requires_stages: vec![],
            allowed_next_stages: vec![],
            allowed_tool_types: vec![],
            deliverable_schema: "StageDeliverable".to_string(),
            gate_validator: "validate_stage_gate".to_string(),
            min_invocations: Default::default(),
            max_other_skips: None,
            human_approval: None,
            agent_continuity: crate::harness::types::AgentContinuity::SingleSession,
            inherits_evidence_from: vec![],
            required_evidence_kinds: vec![],
            finding_verification: rule,
            min_findings: None,
            min_claims: None,
            gate_rules: vec![],
            expected_techniques: vec![],
        }
    }

    fn finding(sev: FindingSeverity, refs: Vec<i64>) -> HarnessFinding {
        HarnessFinding {
            finding_id: Uuid::new_v4(),
            kind: "rce".to_string(),
            subject: "api.example.com".to_string(),
            severity: sev,
            evidence_refs: refs.into_iter().map(EvidenceAuditId::new).collect(),
        }
    }

    fn deliverable(findings: Vec<HarnessFinding>) -> StageDeliverable {
        StageDeliverable {
            stage_id: "verification".to_string(),
            stage_run_id: Uuid::new_v4(),
            claims: vec![],
            evidence_refs: vec![],
            skipped_checks: vec![],
            findings,
            required_checks_done: vec![],
            coverage: vec![],
        }
    }

    #[test]
    fn no_rule_passes() {
        let d = deliverable(vec![finding(FindingSeverity::Critical, vec![])]);
        assert!(run(&d, &spec_with(None)).is_pass());
    }

    #[test]
    fn critical_without_evidence_blocks_when_rule_set() {
        let rule = FindingVerificationRule {
            min_severity: FindingSeverity::High,
            require_evidence_kinds: vec!["poc".to_string()],
        };
        let d = deliverable(vec![finding(FindingSeverity::Critical, vec![])]);
        match run(&d, &spec_with(Some(rule))) {
            GateCheckOutcome::Block { reasons, recovery } => {
                assert!(reasons[0].contains("no evidence"));
                assert!(recovery.missing_evidence_kinds.contains(&"poc".to_string()));
            }
            GateCheckOutcome::Pass => panic!("expected BLOCK for unverified critical finding"),
        }
    }

    #[test]
    fn critical_with_evidence_passes() {
        let rule = FindingVerificationRule {
            min_severity: FindingSeverity::High,
            require_evidence_kinds: vec!["poc".to_string()],
        };
        let d = deliverable(vec![finding(FindingSeverity::Critical, vec![1])]);
        assert!(run(&d, &spec_with(Some(rule))).is_pass());
    }

    #[test]
    fn low_severity_without_evidence_is_ignored() {
        let rule = FindingVerificationRule {
            min_severity: FindingSeverity::High,
            require_evidence_kinds: vec![],
        };
        let d = deliverable(vec![finding(FindingSeverity::Low, vec![])]);
        assert!(run(&d, &spec_with(Some(rule))).is_pass());
    }

    #[test]
    fn gate_rule_reproduces_finding_verification_block() {
        use crate::harness::gate::rule_engine::{self, GateRule};

        // 现状路径：finding_verification min_severity=high，一个无证据的 critical → Block。
        let legacy_rule = FindingVerificationRule {
            min_severity: FindingSeverity::High,
            require_evidence_kinds: vec![],
        };
        let d = deliverable(vec![finding(FindingSeverity::Critical, vec![])]);
        let legacy = run(&d, &spec_with(Some(legacy_rule)));

        // 等价 gate_rule：for_all findings where severity>=high require non_empty evidence_refs。
        let gr: GateRule = serde_json::from_str(
            r#"{ "op":"for_all","over":"findings",
                 "where":{"pred":"severity_at_least","min":"high"},
                 "require":{"pred":"non_empty","field":"evidence_refs"},
                 "on_fail":{"reason":"high+ finding needs evidence"} }"#,
        )
        .unwrap();
        let engine = &rule_engine::eval(&d, &spec_with(None), &[gr])[0];

        // 两者结论一致：都 Block。
        assert!(
            !legacy.is_pass(),
            "legacy finding_verification should Block"
        );
        assert!(!engine.is_pass(), "equivalent gate_rule should Block");
    }
}
