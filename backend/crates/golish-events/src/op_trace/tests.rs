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
