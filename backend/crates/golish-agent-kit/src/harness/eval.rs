//! P2-c · doer-quality eval (deterministic, rule-based).
//!
//! Borrows Heartbit's eval shape (`heartbit-core/src/eval/mod.rs`: an
//! `EvalScorer` trait returning `(score, notes)` + an aggregate summary), but
//! adapted to score a **harness deliverable + its gate outcome** rather than a
//! free-text LLM answer — so "判 doer" is a pure function over the structured
//! deliverable, no LLM judge needed (LLM-judge scoring can layer on later).
//!
//! Each scorer returns a 0.0..=1.0 score; the scorecard's `overall` is the
//! mean. This lets us rank stage runs / agents by evidence-backing, finding
//! verification, and gate outcome from the data the harness already records.

use super::gate::GateResult;
use super::types::{FindingSeverity, StageDeliverable};

/// A pluggable, deterministic quality scorer for a doer's stage deliverable.
pub trait DoerScorer: Send + Sync {
    fn name(&self) -> &'static str;
    /// Returns `(score in 0.0..=1.0, notes)`.
    fn score(&self, deliverable: &StageDeliverable, gate: &GateResult) -> (f64, Vec<String>);
}

/// One scorer's contribution to a scorecard.
#[derive(Debug, Clone)]
pub struct ScoreEntry {
    pub scorer: String,
    pub score: f64,
    pub notes: Vec<String>,
}

/// Aggregate quality verdict for one deliverable.
#[derive(Debug, Clone)]
pub struct DoerScorecard {
    /// Mean of the per-scorer scores (0.0..=1.0). 0.0 when there are no scorers.
    pub overall: f64,
    pub entries: Vec<ScoreEntry>,
}

/// Run a set of scorers over a deliverable + its gate result.
pub fn score_deliverable(
    deliverable: &StageDeliverable,
    gate: &GateResult,
    scorers: &[Box<dyn DoerScorer>],
) -> DoerScorecard {
    let entries: Vec<ScoreEntry> = scorers
        .iter()
        .map(|s| {
            let (score, notes) = s.score(deliverable, gate);
            ScoreEntry {
                scorer: s.name().to_string(),
                score: score.clamp(0.0, 1.0),
                notes,
            }
        })
        .collect();
    let overall = if entries.is_empty() {
        0.0
    } else {
        entries.iter().map(|e| e.score).sum::<f64>() / entries.len() as f64
    };
    DoerScorecard { overall, entries }
}

/// The built-in deterministic scorer set.
pub fn default_scorers() -> Vec<Box<dyn DoerScorer>> {
    vec![
        Box::new(GateOutcomeScorer),
        Box::new(EvidenceBackingScorer),
        Box::new(FindingVerificationScorer),
    ]
}

/// 1.0 if the gate passed, else 0.0 — the single strongest quality signal.
pub struct GateOutcomeScorer;
impl DoerScorer for GateOutcomeScorer {
    fn name(&self) -> &'static str {
        "gate_outcome"
    }
    fn score(&self, _d: &StageDeliverable, gate: &GateResult) -> (f64, Vec<String>) {
        if gate.allowed {
            (1.0, vec![])
        } else {
            (
                0.0,
                vec![format!("gate BLOCKED ({} reasons)", gate.reasons.len())],
            )
        }
    }
}

/// Fraction of claims + findings that cite at least one evidence id.
pub struct EvidenceBackingScorer;
impl DoerScorer for EvidenceBackingScorer {
    fn name(&self) -> &'static str {
        "evidence_backing"
    }
    fn score(&self, d: &StageDeliverable, _gate: &GateResult) -> (f64, Vec<String>) {
        let total = d.claims.len() + d.findings.len();
        if total == 0 {
            return (1.0, vec!["no claims/findings to back".to_string()]);
        }
        let backed = d
            .claims
            .iter()
            .filter(|c| !c.evidence_ids.is_empty())
            .count()
            + d.findings
                .iter()
                .filter(|f| !f.evidence_refs.is_empty())
                .count();
        let frac = backed as f64 / total as f64;
        let notes = if backed < total {
            vec![format!(
                "{}/{} claims+findings cite evidence",
                backed, total
            )]
        } else {
            vec![]
        };
        (frac, notes)
    }
}

/// Fraction of high/critical findings that carry evidence (the "verified
/// conclusions" quality signal). No high/critical findings ⇒ 1.0.
pub struct FindingVerificationScorer;
impl DoerScorer for FindingVerificationScorer {
    fn name(&self) -> &'static str {
        "finding_verification"
    }
    fn score(&self, d: &StageDeliverable, _gate: &GateResult) -> (f64, Vec<String>) {
        let high: Vec<_> = d
            .findings
            .iter()
            .filter(|f| f.severity.rank() >= FindingSeverity::High.rank())
            .collect();
        if high.is_empty() {
            return (1.0, vec!["no high/critical findings".to_string()]);
        }
        let verified = high.iter().filter(|f| !f.evidence_refs.is_empty()).count();
        let frac = verified as f64 / high.len() as f64;
        let notes = if verified < high.len() {
            vec![format!(
                "{}/{} high/critical findings carry evidence",
                verified,
                high.len()
            )]
        } else {
            vec![]
        };
        (frac, notes)
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::harness::types::{HarnessFinding, StageClaim};
    use golish_pentest::evidence_ledger::EvidenceAuditId;
    use uuid::Uuid;

    fn deliverable(claims: Vec<StageClaim>, findings: Vec<HarnessFinding>) -> StageDeliverable {
        StageDeliverable {
            stage_id: "s".to_string(),
            stage_run_id: Uuid::new_v4(),
            claims,
            evidence_refs: vec![],
            skipped_checks: vec![],
            findings,
            required_checks_done: vec![],
            coverage: vec![],
            candidates: vec![],
        }
    }

    fn claim(evidence: Vec<i64>) -> StageClaim {
        StageClaim {
            kind: "k".to_string(),
            subject: "s".to_string(),
            summary: "x".to_string(),
            evidence_ids: evidence.into_iter().map(EvidenceAuditId::new).collect(),
            technique: None,
        }
    }

    fn finding(sev: FindingSeverity, evidence: Vec<i64>) -> HarnessFinding {
        HarnessFinding {
            finding_id: Uuid::new_v4(),
            kind: "k".to_string(),
            subject: "s".to_string(),
            severity: sev,
            evidence_refs: evidence.into_iter().map(EvidenceAuditId::new).collect(),
            technique: None,
        }
    }

    #[test]
    fn perfect_deliverable_scores_one() {
        let d = deliverable(
            vec![claim(vec![1])],
            vec![finding(FindingSeverity::Critical, vec![2])],
        );
        let card = score_deliverable(&d, &GateResult::pass(), &default_scorers());
        assert!(
            (card.overall - 1.0).abs() < 1e-9,
            "overall={}",
            card.overall
        );
    }

    #[test]
    fn blocked_gate_and_unbacked_findings_score_low() {
        let d = deliverable(
            vec![claim(vec![])],
            vec![finding(FindingSeverity::Critical, vec![])],
        );
        let gate = GateResult::block(vec!["x".to_string()], Default::default());
        let card = score_deliverable(&d, &gate, &default_scorers());
        // gate_outcome=0, evidence_backing=0, finding_verification=0 → 0.0
        assert!(card.overall < 0.01, "overall={}", card.overall);
    }

    #[test]
    fn no_high_findings_does_not_penalize_verification() {
        let d = deliverable(
            vec![claim(vec![1])],
            vec![finding(FindingSeverity::Low, vec![])],
        );
        let (s, _) = FindingVerificationScorer.score(&d, &GateResult::pass());
        assert_eq!(s, 1.0);
    }

    #[test]
    fn half_backed_claims_scores_half() {
        let d = deliverable(vec![claim(vec![1]), claim(vec![])], vec![]);
        let (s, _) = EvidenceBackingScorer.score(&d, &GateResult::pass());
        assert!((s - 0.5).abs() < 1e-9, "s={s}");
    }
}
