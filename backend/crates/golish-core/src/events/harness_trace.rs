//! Harness decision sub-events carried by [`super::event::AiEvent::HarnessTrace`].
//!
//! One variant per harness decision *kind*; adding a kind extends this enum
//! only — no new `AiEvent` arm, no exhaustive-match churn across the codebase.
//! See `docs/design/2026-06-05-unified-ai-harness-observability.md` §4.B.

use serde::{Deserialize, Serialize};

/// A single harness decision, tagged by `kind`. Flattened into
/// [`super::event::AiEvent::HarnessTrace`] alongside the correlation spine
/// (`operation_id` + `agent_path` + `stage`).
#[derive(Debug, Clone, Serialize, Deserialize, ts_rs::TS)]
#[serde(tag = "kind", rename_all = "snake_case")]
#[ts(
    export,
    export_to = "../../../../frontend/lib/generated/",
    rename = "GeneratedHarnessTraceKind"
)]
pub enum HarnessTraceKind {
    /// Stage-close gate decision. Emitted for both PASS and BLOCK at the single
    /// chokepoint in `consume_gate_outcome`.
    GateDecision {
        /// `"PASS"` | `"BLOCK"`.
        gate: String,
        #[ts(type = "number")]
        findings: u32,
        /// Cited evidence ids that were NOT found in the ledger (fabricated).
        #[serde(default, skip_serializing_if = "Vec::is_empty")]
        #[ts(type = "number[]")]
        fabricated_evidence_refs: Vec<i64>,
        /// Real evidence ids available for this operation at decision time.
        #[serde(default, skip_serializing_if = "Vec::is_empty")]
        #[ts(type = "number[]")]
        available_real_ids: Vec<i64>,
        #[serde(default, skip_serializing_if = "Option::is_none")]
        first_blocking_reason: Option<String>,
    },
    /// An evidence row was appended to the ledger.
    EvidenceBooked {
        tool: String,
        #[ts(type = "number")]
        evidence_id: i64,
        /// `"sync"` (in-turn tool append) | `"background"` (job listener).
        source: String,
    },
    /// `submit_stage_deliverable` produced an outcome.
    DeliverableSubmitted {
        /// `"accepted"` | `"needs_fix"` | `"rejected"` | `"received"`.
        status: String,
        #[serde(default)]
        #[ts(type = "number[]")]
        cited_evidence_refs: Vec<i64>,
        #[serde(default)]
        #[ts(type = "number[]")]
        available_real_ids: Vec<i64>,
    },
    /// Background-job completion notes were drained into the next turn's prompt.
    BackgroundNotesInjected {
        #[ts(type = "number")]
        count: u32,
        #[ts(type = "number[]")]
        evidence_ids: Vec<i64>,
    },
}

/// Build a `>`-joined agent lineage string from an optional parent path and the
/// current agent's name. The top-level agent is `main`; a direct child is
/// `main>pentester`; a nested child is `main>pentester>reporter`. This is the
/// readable key that lets an AI thread the merged timeline by agent.
///
/// `parent` is the *already-built* path of the delegating agent (e.g.
/// `"main>pentester"`), or `None`/`"main"` for a direct child of the root.
pub fn build_agent_path(parent: Option<&str>, current: &str) -> String {
    match parent {
        None | Some("") | Some("main") => format!("main>{current}"),
        Some(p) if p.starts_with("main>") => format!("{p}>{current}"),
        Some(p) => format!("main>{p}>{current}"),
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn gate_decision_serializes_with_kind_tag() {
        let k = HarnessTraceKind::GateDecision {
            gate: "BLOCK".into(),
            findings: 0,
            fabricated_evidence_refs: vec![1, 2, 3],
            available_real_ids: vec![86, 88, 90],
            first_blocking_reason: Some("fabricated evidence ids".into()),
        };
        let v = serde_json::to_value(&k).unwrap();
        assert_eq!(v["kind"], "gate_decision");
        assert_eq!(v["gate"], "BLOCK");
        assert_eq!(v["fabricated_evidence_refs"], serde_json::json!([1, 2, 3]));
        assert_eq!(v["available_real_ids"], serde_json::json!([86, 88, 90]));
    }

    #[test]
    fn gate_decision_omits_empty_id_lists() {
        let k = HarnessTraceKind::GateDecision {
            gate: "PASS".into(),
            findings: 2,
            fabricated_evidence_refs: vec![],
            available_real_ids: vec![],
            first_blocking_reason: None,
        };
        let v = serde_json::to_value(&k).unwrap();
        assert_eq!(v["kind"], "gate_decision");
        assert!(v.get("fabricated_evidence_refs").is_none());
        assert!(v.get("available_real_ids").is_none());
        assert!(v.get("first_blocking_reason").is_none());
    }

    #[test]
    fn evidence_booked_roundtrips() {
        let k = HarnessTraceKind::EvidenceBooked {
            tool: "run_pty_cmd".into(),
            evidence_id: 88,
            source: "background".into(),
        };
        let back: HarnessTraceKind =
            serde_json::from_value(serde_json::to_value(&k).unwrap()).unwrap();
        assert!(matches!(
            back,
            HarnessTraceKind::EvidenceBooked {
                evidence_id: 88,
                ..
            }
        ));
    }

    #[test]
    fn deliverable_submitted_serializes() {
        let k = HarnessTraceKind::DeliverableSubmitted {
            status: "needs_fix".into(),
            cited_evidence_refs: vec![1, 2, 3],
            available_real_ids: vec![86, 88, 90],
        };
        let v = serde_json::to_value(&k).unwrap();
        assert_eq!(v["kind"], "deliverable_submitted");
        assert_eq!(v["status"], "needs_fix");
        assert_eq!(v["cited_evidence_refs"], serde_json::json!([1, 2, 3]));
    }

    #[test]
    fn background_notes_injected_serializes() {
        let k = HarnessTraceKind::BackgroundNotesInjected {
            count: 57,
            evidence_ids: vec![86, 88, 90],
        };
        let v = serde_json::to_value(&k).unwrap();
        assert_eq!(v["kind"], "background_notes_injected");
        assert_eq!(v["count"], 57);
        assert_eq!(v["evidence_ids"], serde_json::json!([86, 88, 90]));
    }

    #[test]
    fn agent_path_builds_lineage() {
        assert_eq!(build_agent_path(None, "pentester"), "main>pentester");
        assert_eq!(build_agent_path(Some(""), "pentester"), "main>pentester");
        assert_eq!(
            build_agent_path(Some("main"), "pentester"),
            "main>pentester"
        );
        assert_eq!(
            build_agent_path(Some("main>pentester"), "reporter"),
            "main>pentester>reporter"
        );
        assert_eq!(
            build_agent_path(Some("pentester"), "reporter"),
            "main>pentester>reporter"
        );
    }
}
