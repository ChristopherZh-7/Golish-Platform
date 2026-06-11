//! Doc 3 §8.2 schema_check · deliverable 字段齐全 + JSON schema 合.
//!
//! Phase 1c.2 skeleton · 仅基础 sanity (stage_id 非空, stage_run_id 非 nil).
//! Task 1c.5 完整 schema 比对.

use super::super::stage_spec::StageSpec;
use super::super::technique_taxonomy;
use super::super::types::{ExternalAttackSurfaceDeliverable, HarnessRecoveryActions};
use super::GateCheckOutcome;

pub fn run(deliverable: &ExternalAttackSurfaceDeliverable, spec: &StageSpec) -> GateCheckOutcome {
    let mut reasons = Vec::new();

    if deliverable.stage_id.trim().is_empty() {
        reasons.push("deliverable.stage_id is empty".to_string());
    }
    if deliverable.stage_run_id.is_nil() {
        reasons.push("deliverable.stage_run_id is nil uuid".to_string());
    }
    if deliverable.stage_id != spec.id {
        reasons.push(format!(
            "deliverable.stage_id ({}) does not match stage_spec.id ({})",
            deliverable.stage_id, spec.id
        ));
    }

    // P5 (2026-06-11) fail-closed: a claim/finding `technique` (when set) must be a
    // registered technique_taxonomy id. A typo'd id would silently never match any
    // coverage cell (derive/corroborate), so reject it at the schema layer rather
    // than leave a permanent invisible gap. Same philosophy as the spec-side
    // `all_embedded_expected_techniques_are_recognized` guard.
    for c in &deliverable.claims {
        if let Some(t) = c.technique.as_deref() {
            if !technique_taxonomy::is_recognized(t) {
                reasons.push(format!(
                    "claim '{}' carries unregistered technique '{}' — use a registered id from technique_taxonomy.json (e.g. GOLISH-INTEL-DNS / WSTG-INPV-05) or omit the field",
                    c.kind, t
                ));
            }
        }
    }
    for f in &deliverable.findings {
        if let Some(t) = f.technique.as_deref() {
            if !technique_taxonomy::is_recognized(t) {
                reasons.push(format!(
                    "finding '{}' carries unregistered technique '{}' — use a registered id from technique_taxonomy.json or omit the field",
                    f.kind, t
                ));
            }
        }
    }

    if reasons.is_empty() {
        tracing::info!(
            target: "harness::gate::schema_check",
            stage_id = %deliverable.stage_id,
            stage_run_id = %deliverable.stage_run_id,
            outcome = "pass",
            "schema_check pass"
        );
        GateCheckOutcome::Pass
    } else {
        tracing::info!(
            target: "harness::gate::schema_check",
            stage_id = %deliverable.stage_id,
            stage_run_id = %deliverable.stage_run_id,
            outcome = "block",
            reasons_count = reasons.len(),
            first_reason = %reasons[0],
            "schema_check block"
        );
        let mut recovery = HarnessRecoveryActions::default();
        recovery.hints.push(
            "rebuild deliverable with non-empty stage_id, valid stage_run_id and matching schema"
                .to_string(),
        );
        GateCheckOutcome::Block { reasons, recovery }
    }
}

#[cfg(test)]
mod tests {
    use super::super::super::stage_spec::load_stage_spec_from_json;
    use super::super::super::types::{ExternalAttackSurfaceDeliverable, StageClaim};
    use super::*;
    use uuid::Uuid;

    const STAGE_JSON: &str =
        include_str!("../../../../../../resources/harness/stages/external_attack_surface.json");

    fn make_deliverable(stage_id: &str) -> ExternalAttackSurfaceDeliverable {
        ExternalAttackSurfaceDeliverable {
            stage_id: stage_id.to_string(),
            stage_run_id: Uuid::new_v4(),
            claims: Vec::<StageClaim>::new(),
            evidence_refs: vec![],
            skipped_checks: vec![],
            findings: vec![],
            required_checks_done: vec![],
            coverage: vec![],
        }
    }

    #[test]
    fn passes_when_stage_id_matches() {
        let spec = load_stage_spec_from_json(STAGE_JSON).unwrap();
        let d = make_deliverable("external_attack_surface");
        assert!(matches!(run(&d, &spec), GateCheckOutcome::Pass));
    }

    #[test]
    fn blocks_when_stage_id_mismatch() {
        let spec = load_stage_spec_from_json(STAGE_JSON).unwrap();
        let d = make_deliverable("wrong_stage_id");
        let outcome = run(&d, &spec);
        match outcome {
            GateCheckOutcome::Block { reasons, .. } => {
                assert!(reasons.iter().any(|r| r.contains("does not match")));
            }
            _ => panic!("expected Block"),
        }
    }

    #[test]
    fn blocks_when_stage_run_id_nil() {
        let spec = load_stage_spec_from_json(STAGE_JSON).unwrap();
        let mut d = make_deliverable("external_attack_surface");
        d.stage_run_id = Uuid::nil();
        let outcome = run(&d, &spec);
        match outcome {
            GateCheckOutcome::Block { reasons, .. } => {
                assert!(reasons.iter().any(|r| r.contains("nil uuid")));
            }
            _ => panic!("expected Block"),
        }
    }

    #[test]
    fn blocks_unregistered_claim_or_finding_technique() {
        // P5: a typo'd technique id is rejected at the schema layer (fail-closed).
        let spec = load_stage_spec_from_json(STAGE_JSON).unwrap();
        let mut d = make_deliverable("external_attack_surface");
        d.claims.push(StageClaim {
            kind: "dns_a_record".to_string(),
            subject: "example.com".to_string(),
            summary: "A 1.2.3.4".to_string(),
            evidence_ids: vec![],
            technique: Some("GOLISH-INTEL-TYPO".to_string()),
        });
        match run(&d, &spec) {
            GateCheckOutcome::Block { reasons, .. } => {
                assert!(reasons.iter().any(|r| r.contains("GOLISH-INTEL-TYPO")));
            }
            GateCheckOutcome::Pass => panic!("unregistered technique must Block"),
        }
    }

    #[test]
    fn passes_registered_or_absent_technique() {
        // P5: a registered id passes; an untagged (None) claim stays legal.
        let spec = load_stage_spec_from_json(STAGE_JSON).unwrap();
        let mut d = make_deliverable("external_attack_surface");
        d.claims.push(StageClaim {
            kind: "dns_a_record".to_string(),
            subject: "example.com".to_string(),
            summary: "A 1.2.3.4".to_string(),
            evidence_ids: vec![],
            technique: Some("GOLISH-INTEL-DNS".to_string()),
        });
        d.claims.push(StageClaim {
            kind: "note".to_string(),
            subject: "example.com".to_string(),
            summary: "untagged claim stays legal".to_string(),
            evidence_ids: vec![],
            technique: None,
        });
        assert!(matches!(run(&d, &spec), GateCheckOutcome::Pass));
    }
}
