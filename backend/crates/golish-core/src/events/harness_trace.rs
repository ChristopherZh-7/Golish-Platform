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

    /// Execution mentor produced guidance after the runtime monitor detected a
    /// repetitive or stalled tool pattern. In `shadow` mode this is trace-only;
    /// in `soft` mode the same advice is also appended to the tool response.
    MentorAdviceRecorded {
        /// `"shadow"` | `"soft"`.
        mode: String,
        /// Monitor reason, e.g. `"execution_monitor"`.
        trigger: String,
        /// Tool name that dominated the recent call pattern.
        tool: String,
        #[ts(type = "number")]
        repeat_count: u32,
        /// Whether this advice was injected into the next model-visible tool
        /// response (`true`) or only recorded as telemetry (`false`).
        injected: bool,
        /// Short preview for transcript/run-tree rendering. Full advice remains
        /// in tracing/tool response depending on mode.
        advice_preview: String,
    },

    /// RuntimeSupervisor produced structured strategy guidance after the runtime
    /// monitor detected a repetitive or stalled tool pattern. This is
    /// PentAGI-style execution monitoring, but the model output is parsed and
    /// policy-sanitized before any text is injected.
    RuntimeSupervisorDecision {
        /// `"shadow"` | `"soft"` | `"hard"`.
        mode: String,
        /// Monitor reason, e.g. `"execution_monitor"`.
        trigger: String,
        /// Tool name that dominated the recent call pattern.
        tool: String,
        #[ts(type = "number")]
        repeat_count: u32,
        /// Whether this directive was injected into the model-visible tool
        /// response.
        injected: bool,
        /// `"strategy_pivot"` / `"wait_for_background"` / etc.
        strategy_kind: String,
        root_cause: String,
        #[ts(type = "number")]
        action_count: u32,
        directive_hash: String,
    },

    /// StageRefiner produced a deterministic repair directive after a submit or
    /// per-org gate failure. The full directive is persisted in
    /// `operation_state.state_blob.agent_run.repair_directive`; this trace keeps
    /// the timeline readable.
    StageRefinerDecision {
        repair_kind: String,
        root_cause: String,
        #[ts(type = "number")]
        action_count: u32,
        #[ts(type = "number")]
        gap_count: u32,
        llm_escalated: bool,
        directive_hash: String,
    },

    /// `stage_run` tool: one organization's live progress for the current stage's
    /// per-org fan-out (design 2026-06-13-stage-run-fanout). The frontend upserts a
    /// row per `org_id` into the `stage_run` tool's detail pane; `stage_label`/
    /// `role_label`/`coverage_axis` let it build the row on the first frame it sees
    /// an org, and `agent_request_id` ties the row to that org's specialist
    /// sub-agent so the UI can drill into the org's own conversation + tool calls.
    StageRunOrgProgress {
        /// Refresh-only pointer to the immutable Stage execution. New durable
        /// Team runs populate this together with `stage_run_unit_id`; legacy
        /// fan-out events omit both. The event remains non-authoritative.
        #[serde(default, skip_serializing_if = "Option::is_none")]
        stage_execution_id: Option<String>,
        /// Refresh-only pointer to this organization's exact StageRunUnit.
        #[serde(default, skip_serializing_if = "Option::is_none")]
        stage_run_unit_id: Option<String>,
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

    /// Wake hint only: the frontend must refresh the exact DB-backed review
    /// state and must never treat this trace as approval authority.
    CandidateReviewRequired {
        wave_run_id: String,
        status: String,
        #[ts(type = "number")]
        resume_version: i64,
        #[ts(type = "number")]
        candidate_count: i64,
        #[ts(type = "number")]
        proposed_candidate_count: i64,
    },

    /// Wake hint emitted after the durable resume CAS and trusted resume service
    /// have started. Reloading the app still derives state from DB.
    CandidateReviewResumed {
        wave_run_id: String,
        #[ts(type = "number")]
        resume_version: i64,
    },

    /// Refresh/progress hint emitted after the authoritative DB terminalizer
    /// has closed one exact CandidateAttempt. This trace deliberately carries
    /// only immutable lineage ids, the terminal DB status (`verified`,
    /// `refuted`, or `blocked`), aggregate counts, and replay state. It must
    /// never contain operationally sensitive execution data.
    CandidateAttemptTerminalized {
        scope_snapshot_id: String,
        wave_run_id: String,
        wave_unit_id: String,
        organization_id: String,
        candidate_id: String,
        attempt_id: String,
        #[serde(default, skip_serializing_if = "Option::is_none")]
        finding_id: Option<String>,
        status: String,
        #[ts(type = "number")]
        evidence_count: u32,
        #[ts(type = "number")]
        fact_delta_count: u32,
        replayed: bool,
    },

    /// Refresh/progress hint emitted after the operation-level Wave
    /// consolidation transaction commits. It contains only immutable ids,
    /// the deterministic decision (`opened_next_wave`, `closed_no_delta`,
    /// `pending_enrichment`, or `exhausted`), aggregate counts, and replay
    /// state.
    AttackWaveConsolidated {
        scope_snapshot_id: String,
        consolidation_id: String,
        source_wave_run_id: String,
        #[serde(default, skip_serializing_if = "Option::is_none")]
        target_wave_run_id: Option<String>,
        decision_kind: String,
        #[ts(type = "number")]
        accepted_fact_delta_count: u32,
        #[ts(type = "number")]
        rejected_fact_delta_count: u32,
        #[ts(type = "number")]
        residual_risk_count: u32,
        #[serde(default)]
        #[ts(type = "number")]
        pending_enrichment_count: u32,
        replayed: bool,
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
    fn candidate_review_hints_serialize_without_becoming_authority() {
        let required = HarnessTraceKind::CandidateReviewRequired {
            wave_run_id: "wave-1".into(),
            status: "open".into(),
            resume_version: 2,
            candidate_count: 3,
            proposed_candidate_count: 1,
        };
        let resumed = HarnessTraceKind::CandidateReviewResumed {
            wave_run_id: "wave-1".into(),
            resume_version: 4,
        };
        assert_eq!(
            serde_json::to_value(required).unwrap()["kind"],
            "candidate_review_required"
        );
        assert_eq!(
            serde_json::to_value(resumed).unwrap()["kind"],
            "candidate_review_resumed"
        );
    }

    #[test]
    fn candidate_pipeline_traces_serialize_only_safe_fields() {
        use std::collections::BTreeSet;

        let attempt = HarnessTraceKind::CandidateAttemptTerminalized {
            scope_snapshot_id: "scope-1".into(),
            wave_run_id: "wave-1".into(),
            wave_unit_id: "unit-1".into(),
            organization_id: "org-1".into(),
            candidate_id: "candidate-1".into(),
            attempt_id: "attempt-1".into(),
            finding_id: Some("finding-1".into()),
            status: "verified".into(),
            evidence_count: 3,
            fact_delta_count: 1,
            replayed: false,
        };
        let consolidated = HarnessTraceKind::AttackWaveConsolidated {
            scope_snapshot_id: "scope-1".into(),
            consolidation_id: "consolidation-1".into(),
            source_wave_run_id: "wave-1".into(),
            target_wave_run_id: Some("wave-2".into()),
            decision_kind: "opened_next_wave".into(),
            accepted_fact_delta_count: 1,
            rejected_fact_delta_count: 2,
            residual_risk_count: 0,
            pending_enrichment_count: 0,
            replayed: true,
        };

        let attempt_json = serde_json::to_value(attempt).expect("attempt trace serializes");
        let consolidated_json =
            serde_json::to_value(consolidated).expect("consolidation trace serializes");
        fn keys(value: &serde_json::Value) -> BTreeSet<&str> {
            value
                .as_object()
                .expect("trace is an object")
                .keys()
                .map(String::as_str)
                .collect::<BTreeSet<_>>()
        }

        assert_eq!(
            keys(&attempt_json),
            BTreeSet::from([
                "attempt_id",
                "candidate_id",
                "evidence_count",
                "fact_delta_count",
                "finding_id",
                "kind",
                "organization_id",
                "replayed",
                "scope_snapshot_id",
                "status",
                "wave_run_id",
                "wave_unit_id",
            ])
        );
        assert_eq!(
            keys(&consolidated_json),
            BTreeSet::from([
                "accepted_fact_delta_count",
                "consolidation_id",
                "decision_kind",
                "kind",
                "pending_enrichment_count",
                "rejected_fact_delta_count",
                "replayed",
                "residual_risk_count",
                "scope_snapshot_id",
                "source_wave_run_id",
                "target_wave_run_id",
            ])
        );

        for value in [attempt_json, consolidated_json] {
            let encoded = serde_json::to_string(&value)
                .expect("trace encodes")
                .to_ascii_lowercase();
            for forbidden in ["payload", "lease", "plan", "exploit"] {
                assert!(
                    !encoded.contains(forbidden),
                    "trace leaked forbidden material marker {forbidden}: {encoded}"
                );
            }
        }
    }

    #[test]
    fn mentor_advice_recorded_roundtrips() {
        let k = HarnessTraceKind::MentorAdviceRecorded {
            mode: "shadow".into(),
            trigger: "execution_monitor".into(),
            tool: "pentest_run".into(),
            repeat_count: 3,
            injected: false,
            advice_preview: "check the previous background output first".into(),
        };
        let v = serde_json::to_value(&k).unwrap();
        assert_eq!(v["kind"], "mentor_advice_recorded");
        assert_eq!(v["mode"], "shadow");
        assert_eq!(v["tool"], "pentest_run");
        assert_eq!(v["repeat_count"], 3);
        assert_eq!(v["injected"], false);

        let back: HarnessTraceKind = serde_json::from_value(v).expect("round-trips");
        assert!(matches!(
            back,
            HarnessTraceKind::MentorAdviceRecorded {
                mode,
                injected: false,
                ..
            } if mode == "shadow"
        ));
    }

    #[test]
    fn stage_refiner_decision_serializes() {
        let k = HarnessTraceKind::StageRefinerDecision {
            repair_kind: "coverage_gap".into(),
            root_cause: "deterministic gate found 3 gaps".into(),
            action_count: 3,
            gap_count: 3,
            llm_escalated: false,
            directive_hash: "abc123".into(),
        };
        let v = serde_json::to_value(&k).unwrap();
        assert_eq!(v["kind"], "stage_refiner_decision");
        assert_eq!(v["repair_kind"], "coverage_gap");
        assert_eq!(v["action_count"], 3);
        let back: HarnessTraceKind = serde_json::from_value(v).expect("round-trips");
        assert!(matches!(
            back,
            HarnessTraceKind::StageRefinerDecision {
                gap_count: 3,
                llm_escalated: false,
                ..
            }
        ));
    }

    #[test]
    fn runtime_supervisor_decision_serializes() {
        let k = HarnessTraceKind::RuntimeSupervisorDecision {
            mode: "hard".into(),
            trigger: "execution_monitor".into(),
            tool: "whatweb".into(),
            repeat_count: 3,
            injected: true,
            strategy_kind: "strategy_pivot".into(),
            root_cause: "tool repeated without closing coverage".into(),
            action_count: 1,
            directive_hash: "abc123".into(),
        };
        let v = serde_json::to_value(&k).unwrap();
        assert_eq!(v["kind"], "runtime_supervisor_decision");
        assert_eq!(v["strategy_kind"], "strategy_pivot");
        assert_eq!(v["action_count"], 1);
        let back: HarnessTraceKind = serde_json::from_value(v).expect("round-trips");
        assert!(matches!(
            back,
            HarnessTraceKind::RuntimeSupervisorDecision {
                injected: true,
                repeat_count: 3,
                ..
            }
        ));
    }

    #[test]
    fn stage_run_org_progress_serializes_with_kind_and_coverage() {
        let k = HarnessTraceKind::StageRunOrgProgress {
            stage_execution_id: Some("stage-execution-1".into()),
            stage_run_unit_id: Some("stage-unit-1".into()),
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
        assert_eq!(v["stage_execution_id"], "stage-execution-1");
        assert_eq!(v["stage_run_unit_id"], "stage-unit-1");
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
            stage_execution_id: None,
            stage_run_unit_id: None,
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
        assert!(v.get("stage_execution_id").is_none());
        assert!(v.get("stage_run_unit_id").is_none());
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
