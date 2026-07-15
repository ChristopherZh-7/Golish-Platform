//! Tests for the operation-scoped trace merge/manifest/render.

use super::*;
use std::fs;

fn write_session(base: &Path, session: &str) {
    let dir = session_dir(base, session);
    fs::create_dir_all(&dir).unwrap();
    // Main transcript: a user message, then a gate BLOCK harness trace.
    let main = "\
{\"_timestamp\":\"2026-06-05T00:00:02Z\",\"type\":\"user_message\",\"content\":\"recon example.com\"}
{\"_timestamp\":\"2026-06-05T00:00:04Z\",\"type\":\"harness_trace\",\"operation_id\":\"op-1\",\"stage\":\"target_intel\",\"agent_path\":\"main\",\"kind\":\"gate_decision\",\"gate\":\"BLOCK\",\"findings\":0,\"fabricated_evidence_refs\":[1,2,3],\"available_real_ids\":[86,88,90]}
";
    fs::write(dir.join("transcript.json"), main).unwrap();

    // One sub-agent transcript (earlier ts so ordering interleaves correctly).
    let sub_dir = dir.join("subagents").join("pentester-req1");
    fs::create_dir_all(&sub_dir).unwrap();
    let sub = "\
{\"_timestamp\":\"2026-06-05T00:00:03Z\",\"type\":\"sub_agent_tool_result\",\"agent_id\":\"pentester\",\"tool_name\":\"nmap\",\"success\":true,\"result\":null,\"request_id\":\"r1\",\"parent_request_id\":\"req1\"}
";
    fs::write(sub_dir.join("transcript.json"), sub).unwrap();
}

fn write_candidate_pipeline_session(base: &Path, session: &str) {
    let dir = session_dir(base, session);
    fs::create_dir_all(&dir).unwrap();
    let transcript = "\
{\"_timestamp\":\"2026-07-13T00:00:01Z\",\"type\":\"harness_trace\",\"operation_id\":\"op-1\",\"stage\":\"verification\",\"agent_path\":\"main\",\"kind\":\"candidate_attempt_terminalized\",\"scope_snapshot_id\":\"scope-1\",\"wave_run_id\":\"wave-1\",\"wave_unit_id\":\"unit-1\",\"organization_id\":\"org-1\",\"candidate_id\":\"candidate-1\",\"attempt_id\":\"attempt-1\",\"finding_id\":\"finding-1\",\"status\":\"verified\",\"evidence_count\":3,\"fact_delta_count\":1,\"replayed\":false}
{\"_timestamp\":\"2026-07-13T00:00:02Z\",\"type\":\"harness_trace\",\"operation_id\":\"op-1\",\"stage\":\"verification\",\"agent_path\":\"main\",\"kind\":\"attack_wave_consolidated\",\"scope_snapshot_id\":\"scope-1\",\"consolidation_id\":\"consolidation-1\",\"source_wave_run_id\":\"wave-1\",\"target_wave_run_id\":\"wave-2\",\"decision_kind\":\"opened_next_wave\",\"accepted_fact_delta_count\":1,\"rejected_fact_delta_count\":2,\"residual_risk_count\":0,\"replayed\":true}
";
    fs::write(dir.join("transcript.json"), transcript).unwrap();
}

#[test]
fn collect_records_merges_and_orders_with_agent_path() {
    let tmp = tempfile::tempdir().unwrap();
    write_session(tmp.path(), "sess-1");

    let records = collect_records(tmp.path(), "sess-1");
    assert_eq!(records.len(), 3, "main(2) + subagent(1)");
    // Sorted by ts: user(02) < sub(03) < gate(04)
    assert_eq!(records[0].agent_path, "main");
    assert_eq!(records[1].agent_path, "main>pentester");
    assert_eq!(records[2].agent_path, "main");
    assert_eq!(records[2].operation_id.as_deref(), Some("op-1"));
    assert_eq!(records[2].stage.as_deref(), Some("target_intel"));
    // seq is assigned in order
    assert_eq!(records[0].seq, 0);
    assert_eq!(records[2].seq, 2);
}

#[test]
fn build_manifest_summarizes_blocked_run() {
    let tmp = tempfile::tempdir().unwrap();
    write_session(tmp.path(), "sess-1");

    let records = collect_records(tmp.path(), "sess-1");
    let m = build_manifest(&records, "sess-1");
    assert_eq!(m.operation_id.as_deref(), Some("op-1"));
    assert_eq!(m.status, "blocked");
    assert!(m.agent_paths.contains(&"main".to_string()));
    assert!(m.agent_paths.contains(&"main>pentester".to_string()));
    assert!(m.stages.contains(&"target_intel".to_string()));
    let last = m.last_decision.expect("a last decision");
    assert_eq!(last["kind"], "gate_decision");
    assert_eq!(last["gate"], "BLOCK");
}

#[test]
fn render_timeline_shows_decision_and_agent() {
    let tmp = tempfile::tempdir().unwrap();
    write_session(tmp.path(), "sess-1");

    let out = render_timeline(tmp.path(), "sess-1");
    assert!(out.contains("gate BLOCK"), "render: {out}");
    assert!(out.contains("main>pentester"), "render: {out}");
    assert!(out.contains("status: blocked"), "render: {out}");
}

#[test]
fn render_timeline_summarizes_candidate_pipeline_traces_without_sensitive_material() {
    let tmp = tempfile::tempdir().unwrap();
    write_candidate_pipeline_session(tmp.path(), "candidate-pipeline");

    let out = render_timeline(tmp.path(), "candidate-pipeline");
    assert!(
        out.contains("candidate_attempt attempt-1 verified evidence=3 deltas=1 replayed=false"),
        "render: {out}"
    );
    assert!(
        out.contains(
            "attack_wave wave-1 -> wave-2 opened_next_wave accepted=1 rejected=2 residuals=0 pending=0 replayed=true"
        ),
        "render: {out}"
    );
    let rendered = out.to_ascii_lowercase();
    for forbidden in ["payload", "lease", "plan", "exploit"] {
        assert!(
            !rendered.contains(forbidden),
            "timeline leaked forbidden material marker {forbidden}: {out}"
        );
    }
}

#[test]
fn decision_records_filter_by_kind() {
    let tmp = tempfile::tempdir().unwrap();
    write_session(tmp.path(), "sess-1");

    let only_gate = decision_records_json(tmp.path(), "sess-1", 10, &["harness_trace".to_string()]);
    assert_eq!(only_gate.len(), 1);
    assert_eq!(only_gate[0]["kind"], "harness_trace");
    assert!(only_gate[0]["summary"]
        .as_str()
        .unwrap()
        .contains("gate BLOCK"));
}

#[test]
fn write_trace_artifacts_creates_files() {
    let tmp = tempfile::tempdir().unwrap();
    write_session(tmp.path(), "sess-1");

    let (timeline, manifest) = write_trace_artifacts(tmp.path(), "sess-1").unwrap();
    assert!(timeline.exists());
    assert!(manifest.exists());
    let manifest_str = fs::read_to_string(&manifest).unwrap();
    assert!(manifest_str.contains("\"status\": \"blocked\""));
}

// Regression: reads must resolve the transcripts base the SAME way the writer
// does. A real workspace writes to `{workspace}/.golish/transcripts`; a home-only
// read default silently missed those runs ("no logs"). (nextest isolates each
// test in its own process, so env mutation here is safe.)

#[test]
fn resolve_base_prefers_workspace_relative() {
    std::env::remove_var("VT_TRANSCRIPT_DIR");
    let ws = Path::new("/tmp/some-workspace");
    assert_eq!(
        resolve_transcript_base(Some(ws)),
        PathBuf::from("/tmp/some-workspace/.golish/transcripts")
    );
}

#[test]
fn resolve_base_dot_or_none_falls_back_home() {
    std::env::remove_var("VT_TRANSCRIPT_DIR");
    let dot = resolve_transcript_base(Some(Path::new(".")));
    let none = resolve_transcript_base(None);
    assert_eq!(dot, none, "\".\" must not be treated as workspace-relative");
    assert!(dot.ends_with("transcripts"));
}

#[test]
fn resolve_base_env_override_wins_over_workspace() {
    std::env::set_var("VT_TRANSCRIPT_DIR", "/tmp/explicit-transcripts");
    let got = resolve_transcript_base(Some(Path::new("/tmp/some-workspace")));
    std::env::remove_var("VT_TRANSCRIPT_DIR");
    assert_eq!(got, PathBuf::from("/tmp/explicit-transcripts"));
}

#[test]
fn for_session_finds_workspace_holding_session() {
    std::env::remove_var("VT_TRANSCRIPT_DIR");
    let tmp = tempfile::tempdir().unwrap();
    let ws = tmp.path();
    let session = "sess-find-me";
    let base = ws.join(".golish").join("transcripts");
    fs::create_dir_all(session_dir(&base, session)).unwrap();
    assert_eq!(resolve_transcript_base_for_session(session, Some(ws)), base);
}
