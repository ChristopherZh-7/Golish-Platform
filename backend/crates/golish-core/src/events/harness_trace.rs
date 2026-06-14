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

    /// `stage_run` tool: one organization's live progress for the current stage's
    /// per-org fan-out (design 2026-06-13-stage-run-fanout). The frontend upserts a
    /// row per `org_id` into the `stage_run` tool's detail pane; `stage_label`/
    /// `role_label`/`coverage_axis` let it build the row on the first frame it sees
    /// an org, and `agent_request_id` ties the row to that org's specialist
    /// sub-agent so the UI can drill into the org's own conversation + tool calls.
    StageRunOrgProgress {
        /// The organization this progress row is for.
        org_id: String,
        org_name: String,
        /// The per-org specialist sub-agent's `parent_request_id`. Lets the UI
        /// link this org row to its sub-agent (its AI conversation / tool calls /
        /// reasoning) so each org is independently drill-in-able. `None` when the
        /// row is emitted before/without a dispatched sub-agent.
        #[serde(default, skip_serializing_if = "Option::is_none")]
        agent_request_id: Option<String>,
        /// Direct-ownership percentage of this org under the engagement parent
        /// (root org / unknown → `None`).
        #[serde(default, skip_serializing_if = "Option::is_none")]
        ownership_percent: Option<f64>,
        /// `"passed"` | `"running"` | `"queued"` | `"blocked"` | `"pending"`.
        status: String,
        /// Per-technique terminal state on the coverage axis:
        /// `(technique, "found"|"checked_empty"|"blocked"|"pending")`.
        #[serde(default)]
        #[ts(type = "[string, string][]")]
        coverage: Vec<(String, String)>,
        /// Evidence rows this org's specialist has booked into the ledger.
        #[ts(type = "number")]
        evidence_count: u32,
        /// Live one-liner while running (e.g. `"subfinder · pingan.com.cn"`).
        #[serde(default, skip_serializing_if = "Option::is_none")]
        activity: Option<String>,
        /// Stage display name (e.g. `"Target Intel"`) — for first-frame card build.
        stage_label: String,
        /// Specialist role label (e.g. `"Recon"`) — for first-frame card build.
        role_label: String,
        /// Coverage technique columns for this stage (config-driven) — first frame.
        #[serde(default)]
        coverage_axis: Vec<String>,
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
    fn stage_run_org_progress_serializes_with_kind_and_coverage() {
        let k = HarnessTraceKind::StageRunOrgProgress {
            org_id: "org-1".into(),
            org_name: "平安科技".into(),
            agent_request_id: Some("req-1::org::org-1".into()),
            ownership_percent: Some(100.0),
            status: "running".into(),
            coverage: vec![
                ("DNS".into(), "found".into()),
                ("CT".into(), "pending".into()),
            ],
            evidence_count: 3,
            activity: Some("subfinder · pingan.com.cn".into()),
            stage_label: "Target Intel".into(),
            role_label: "Recon".into(),
            coverage_axis: vec!["DNS".into(), "CT".into()],
        };
        let v = serde_json::to_value(&k).unwrap();
        assert_eq!(v["kind"], "stage_run_org_progress");
        assert_eq!(v["org_id"], "org-1");
        assert_eq!(v["status"], "running");
        assert_eq!(v["evidence_count"], 3);
        assert_eq!(
            v["coverage"],
            serde_json::json!([["DNS", "found"], ["CT", "pending"]])
        );
        assert_eq!(v["role_label"], "Recon");
        assert_eq!(v["agent_request_id"], "req-1::org::org-1");
        // Round-trips back to the same variant.
        let back: HarnessTraceKind = serde_json::from_value(v).expect("round-trips");
        assert!(matches!(
            back,
            HarnessTraceKind::StageRunOrgProgress {
                evidence_count: 3,
                ..
            }
        ));
    }

    #[test]
    fn stage_run_org_progress_omits_empty_optionals() {
        let k = HarnessTraceKind::StageRunOrgProgress {
            org_id: "org-2".into(),
            org_name: "root".into(),
            agent_request_id: None,
            ownership_percent: None,
            status: "queued".into(),
            coverage: vec![],
            evidence_count: 0,
            activity: None,
            stage_label: "Target Intel".into(),
            role_label: "Recon".into(),
            coverage_axis: vec![],
        };
        let v = serde_json::to_value(&k).unwrap();
        assert!(v.get("ownership_percent").is_none());
        assert!(v.get("activity").is_none());
        assert!(v.get("agent_request_id").is_none());
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
