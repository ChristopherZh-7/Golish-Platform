//! P2 · config-driven "trustworthy conclusion" check (deliverable structural
//! layer). Reads `StageSpec.finding_verification` / `min_findings` /
//! `min_claims` and enforces count-level structural criteria. Evidence ids are
//! ledger internals and are no longer model-required fields; evidence-kind /
//! DB-truth checks belong in the caller-side ledger projection, not in this
//! DB-free deliverable shape check.

use crate::harness::stage_spec::StageSpec;
use crate::harness::types::{HarnessRecoveryActions, StageDeliverable};

use super::GateCheckOutcome;

pub fn run(deliverable: &StageDeliverable, spec: &StageSpec) -> GateCheckOutcome {
    let mut reasons = Vec::new();
    let recovery = HarnessRecoveryActions::default();

    let _ = &spec.finding_verification;

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
            specialist: None,
            coverage_axis: vec![],
            facts_from_db_truth: false,
            freshness_window: false,
            asset_wave_barrier: false,
            host_aware_coverage: false,
            enum_ip_web_coverage: false,
            skip_dead_assets: false,
            coverage_anchor_only: false,
            findings_allowed: true,
        }
    }

    fn finding(sev: FindingSeverity, refs: Vec<i64>) -> HarnessFinding {
        HarnessFinding {
            finding_id: Uuid::new_v4(),
            kind: "rce".to_string(),
            subject: "api.example.com".to_string(),
            severity: sev,
            evidence_refs: refs.into_iter().map(EvidenceAuditId::new).collect(),
            technique: None,
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
            candidates: vec![],
        }
    }

    #[test]
    fn no_rule_passes() {
        let d = deliverable(vec![finding(FindingSeverity::Critical, vec![])]);
        assert!(run(&d, &spec_with(None)).is_pass());
    }

    #[test]
    fn critical_without_model_evidence_ids_passes_shape_check() {
        let rule = FindingVerificationRule {
            min_severity: FindingSeverity::High,
            require_evidence_kinds: vec!["poc".to_string()],
        };
        let d = deliverable(vec![finding(FindingSeverity::Critical, vec![])]);
        assert!(run(&d, &spec_with(Some(rule))).is_pass());
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
    fn gate_rule_evidence_id_requirement_is_compat_noop() {
        use crate::harness::gate::rule_engine::{self, GateRule};

        // Evidence ids are optional model fields; neither the legacy
        // finding_verification shape check nor the compatibility gate_rule should
        // block solely because the model omitted ids.
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
                 "on_fail":{"reason":"high+ finding requires backend evidence truth"} }"#,
        )
        .unwrap();
        let engine = &rule_engine::eval(&d, &spec_with(None), &[gr])[0];

        assert!(legacy.is_pass());
        assert!(engine.is_pass());
    }
}
