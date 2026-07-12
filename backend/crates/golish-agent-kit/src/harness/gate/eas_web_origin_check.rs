use super::rule_engine::GateContext;
use super::GateCheckOutcome;
use crate::db_traits::TechniqueOutcomeFact;
use crate::harness::stage_spec::StageSpec;
use crate::harness::{EvidenceFact, EvidenceOutcome};

const WEB_FINGERPRINT: &str = "GOLISH-EAS-WEB-FINGERPRINT";
const TRUSTED_WEB_BLOCKED_SOURCE: &str = "eas_fingerprint_web_stack";

/// Return only exact-origin terminal outcomes whose evidence id, outcome and
/// exact-origin asset all match a guarded current-org ledger fact. Host-level
/// compatibility facts deliberately cannot close a scheme/port/vhost origin.
pub fn completed_from_guarded_outcomes(
    rows: &[TechniqueOutcomeFact],
    evidence_facts: &[EvidenceFact],
) -> std::collections::HashSet<String> {
    rows.iter()
        .filter(|row| row.technique == WEB_FINGERPRINT && row.evidence_id > 0)
        .filter_map(|row| {
            let expected_outcome = match row.outcome.as_str() {
                "found" => EvidenceOutcome::Found,
                "empty" => EvidenceOutcome::Empty,
                "blocked" if row.source.as_deref() == Some(TRUSTED_WEB_BLOCKED_SOURCE) => {
                    EvidenceOutcome::Blocked
                }
                _ => return None,
            };
            let origin = golish_pentest_domain::canonical_web_origin(&row.asset)?;
            let guarded = evidence_facts.iter().any(|fact| {
                fact.technique == WEB_FINGERPRINT
                    && fact.evidence_id == row.evidence_id
                    && fact.outcome == expected_outcome
                    && golish_pentest_domain::canonical_web_origin(&fact.asset)
                        .is_some_and(|fact_origin| fact_origin.key == origin.key)
            });
            guarded.then_some(origin.key)
        })
        .collect()
}

/// Enforce Web fingerprint completion at exact `scheme://host:port` identity.
/// The check activates only when an authoritative caller supplied the required
/// set; legacy/test contexts with `None` retain their previous behavior.
pub fn run(spec: &StageSpec, ctx: &GateContext) -> GateCheckOutcome {
    if spec.kind != crate::harness::StageKind::ExternalAttackSurface {
        return GateCheckOutcome::Pass;
    }
    let Some(required) = ctx.eas_required_web_origins.as_ref() else {
        return GateCheckOutcome::Pass;
    };
    let completed = ctx.eas_completed_web_origins.as_ref();

    let mut malformed = Vec::new();
    let mut missing = Vec::new();
    for raw in required {
        let Some(origin) = golish_pentest_domain::canonical_web_origin(raw) else {
            malformed.push(raw.clone());
            continue;
        };
        let is_completed = completed.is_some_and(|origins| {
            origins.iter().any(|candidate| {
                golish_pentest_domain::canonical_web_origin(candidate)
                    .is_some_and(|candidate| candidate.key == origin.key)
            })
        });
        if !is_completed {
            missing.push(origin.key);
        }
    }
    malformed.sort();
    malformed.dedup();
    missing.sort();
    missing.dedup();

    if malformed.is_empty() && missing.is_empty() {
        return GateCheckOutcome::Pass;
    }

    let mut reasons = Vec::new();
    if !malformed.is_empty() {
        reasons.push(format!(
            "EAS exact-origin denominator contains malformed authoritative origins: {}",
            malformed.join(", ")
        ));
    }
    if !missing.is_empty() {
        reasons.push(format!(
            "EAS Web fingerprint incomplete for {} exact origin(s): {}",
            missing.len(),
            missing.join(", ")
        ));
    }
    GateCheckOutcome::Block {
        reasons,
        recovery: crate::harness::types::HarnessRecoveryActions {
            hints: vec![
                "run eas_fingerprint_web_stack once for each missing exact scheme://host:port origin; shared IP/port does not merge Host/SNI identities"
                    .to_string(),
            ],
            repair_tool_calls: vec!["eas_fingerprint_web_stack".to_string()],
            coverage_gap_actions: missing
                .into_iter()
                .map(|origin| crate::harness::types::CoverageGapAction {
                    asset: origin,
                    technique: WEB_FINGERPRINT.to_string(),
                    reason: "missing_exact_origin".to_string(),
                    suggested_capabilities: Vec::new(),
                    suggested_tools: vec!["eas_fingerprint_web_stack".to_string()],
                })
                .collect(),
            ..Default::default()
        },
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::db_traits::TechniqueOutcomeFact;
    use crate::harness::stage_spec::load_stage_spec_from_json;
    use crate::harness::{EvidenceFact, EvidenceOutcome};

    fn eas_spec() -> StageSpec {
        load_stage_spec_from_json(
            r#"{"id":"external_attack_surface","kind":"external_attack_surface","risk_level":"medium","deliverable_schema":"StageDeliverable","gate_validator":"validate_stage_gate"}"#,
        )
        .unwrap()
    }

    #[test]
    fn exact_origin_barrier_blocks_each_missing_scheme_host_and_port_identity() {
        let ctx = crate::harness::gate::GateContextBuilder::new()
            .eas_web_origin_barrier(
                [
                    "http://a.example:80".to_string(),
                    "https://a.example:443".to_string(),
                    "https://b.example:443".to_string(),
                ],
                ["http://a.example:80".to_string()],
            )
            .build();

        let GateCheckOutcome::Block { reasons, recovery } = run(&eas_spec(), &ctx) else {
            panic!("missing exact origins must block EAS")
        };
        let joined = reasons.join(" ");
        assert!(joined.contains("https://a.example:443"));
        assert!(joined.contains("https://b.example:443"));
        assert_eq!(recovery.coverage_gap_actions.len(), 2);
        assert_eq!(
            recovery
                .coverage_gap_actions
                .iter()
                .map(|action| action.asset.as_str())
                .collect::<Vec<_>>(),
            vec!["https://a.example:443", "https://b.example:443"]
        );
        assert!(recovery.coverage_gap_actions.iter().all(|action| {
            action.technique == WEB_FINGERPRINT
                && action.reason == "missing_exact_origin"
                && action.suggested_tools == ["eas_fingerprint_web_stack"]
        }));
    }

    #[test]
    fn exact_origin_barrier_passes_only_when_every_required_origin_completed() {
        let origins = [
            "http://a.example:80".to_string(),
            "https://a.example:443".to_string(),
        ];
        let ctx = crate::harness::gate::GateContextBuilder::new()
            .eas_web_origin_barrier(origins.clone(), origins)
            .build();
        assert!(run(&eas_spec(), &ctx).is_pass());
    }

    #[test]
    fn exact_origin_barrier_rejects_malformed_authoritative_origin() {
        let ctx = crate::harness::gate::GateContextBuilder::new()
            .eas_web_origin_barrier(["not-an-origin".to_string()], Vec::<String>::new())
            .build();
        assert!(matches!(
            run(&eas_spec(), &ctx),
            GateCheckOutcome::Block { .. }
        ));
    }

    fn outcome(asset: &str, outcome: &str, evidence_id: i64) -> TechniqueOutcomeFact {
        TechniqueOutcomeFact {
            asset: asset.to_string(),
            technique: "GOLISH-EAS-WEB-FINGERPRINT".to_string(),
            outcome: outcome.to_string(),
            evidence_id,
            source: Some("whatweb".to_string()),
        }
    }

    fn evidence(asset: &str, outcome: EvidenceOutcome, evidence_id: i64) -> EvidenceFact {
        EvidenceFact {
            asset: asset.to_string(),
            technique: "GOLISH-EAS-WEB-FINGERPRINT".to_string(),
            outcome,
            evidence_id,
        }
    }

    #[test]
    fn guarded_completion_preserves_scheme_host_and_port_and_evidence_identity() {
        let rows = vec![
            outcome("http://a.example:80", "found", 11),
            outcome("https://a.example:443", "empty", 12),
            outcome("https://b.example:443", "found", 13),
            outcome("https://a.example:8443", "error", 14),
        ];
        let facts = vec![
            evidence("http://a.example:80", EvidenceOutcome::Found, 11),
            evidence("https://a.example:443", EvidenceOutcome::Empty, 12),
            // Same endpoint/port but the wrong Host/SNI must not authorize b.
            evidence("https://a.example:443", EvidenceOutcome::Found, 13),
            // Host-only compatibility fact must not authorize an exact origin.
            evidence("a.example", EvidenceOutcome::Found, 14),
        ];

        let completed = completed_from_guarded_outcomes(&rows, &facts);
        assert!(completed.contains("http://a.example:80"));
        assert!(completed.contains("https://a.example:443"));
        assert!(!completed.contains("https://b.example:443"));
        assert!(!completed.contains("https://a.example:8443"));
    }

    #[test]
    fn eas_web_blocked_completion_requires_trusted_source_and_exact_fact_identity() {
        let trusted = |asset: &str, evidence_id: i64| TechniqueOutcomeFact {
            asset: asset.to_string(),
            technique: WEB_FINGERPRINT.to_string(),
            outcome: "blocked".to_string(),
            evidence_id,
            source: Some("eas_fingerprint_web_stack".to_string()),
        };
        let mut forged_source = trusted("https://source.example:443", 22);
        forged_source.source = Some("model_authored".to_string());
        let rows = vec![
            trusted("https://ok.example:443", 21),
            forged_source,
            trusted("https://wrong-outcome.example:443", 23),
            trusted("https://wrong-id.example:443", 24),
            trusted("http://wrong-origin.example:80", 25),
        ];
        let facts = vec![
            evidence("https://ok.example:443", EvidenceOutcome::Blocked, 21),
            evidence("https://source.example:443", EvidenceOutcome::Blocked, 22),
            evidence(
                "https://wrong-outcome.example:443",
                EvidenceOutcome::Error,
                23,
            ),
            evidence(
                "https://wrong-id.example:443",
                EvidenceOutcome::Blocked,
                240,
            ),
            evidence(
                "https://wrong-origin.example:443",
                EvidenceOutcome::Blocked,
                25,
            ),
        ];

        let completed = completed_from_guarded_outcomes(&rows, &facts);

        assert_eq!(
            completed,
            std::collections::HashSet::from(["https://ok.example:443".to_string()])
        );
    }
}
