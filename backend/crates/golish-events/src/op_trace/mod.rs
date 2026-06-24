//! Operation-scoped, self-discoverable trace.
//!
//! The unified, AI-debuggable view of one run: it merges the main-agent
//! transcript and every sub-agent transcript into a single time-ordered
//! timeline, each line tagged with an `agent_path` (`main`, `main>pentester`,
//! `main>pentester>reporter`), and derives a one-glance [`OperationManifest`].
//!
//! Design: `docs/design/2026-06-05-unified-ai-harness-observability.md` §4.C.
//!
//! P1 choices (resolving the design's open question #1 toward the simplest
//! discoverable layout):
//! - **Session-keyed**: artifacts live next to the existing `transcript.json`
//!   under `{base}/{session_id}/` (the bridge/tool/CLI already know the chat
//!   session string). The `operation_id` is recorded *inside* the manifest.
//! - **Lazy**: the timeline/manifest are computed on read from the transcripts
//!   the existing system already writes — no new write path during the run, so
//!   nothing can block the agent loop. The emitted `AiEvent::HarnessTrace`
//!   decisions land in `transcript.json` via the normal event path.

use std::fs;
use std::path::{Path, PathBuf};
use std::sync::RwLock;

use chrono::{DateTime, Utc};
use golish_core::events::{build_agent_path, AiEvent};
use golish_core::jsonl::TimestampedEntry;
use serde::{Deserialize, Serialize};

/// One merged timeline line: a transcript event plus the correlation spine.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct TraceRecord {
    pub ts: String,
    pub seq: u64,
    pub agent_path: String,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub stage: Option<String>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub operation_id: Option<String>,
    pub event: AiEvent,
}

/// One-glance summary of a run — the entry point an AI reads first.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct OperationManifest {
    pub operation_id: Option<String>,
    pub chat_session: String,
    pub title: String,
    pub status: String,
    pub current_stage: Option<String>,
    pub stages: Vec<String>,
    pub agent_paths: Vec<String>,
    pub last_decision: Option<serde_json::Value>,
    pub record_count: usize,
    pub updated_at: String,
}

/// `{base}/{session_id}/` — same directory as `transcript.json`.
pub fn session_dir(base: &Path, session_id: &str) -> PathBuf {
    base.join(session_id)
}

/// `~/.golish/transcripts` (env `HOME`/`USERPROFILE`), the home-only fallback.
fn home_transcript_base() -> PathBuf {
    let home = std::env::var_os("HOME")
        .or_else(|| std::env::var_os("USERPROFILE"))
        .map(PathBuf::from)
        .unwrap_or_else(|| PathBuf::from("."));
    home.join(".golish").join("transcripts")
}

/// Resolve the transcripts base **the same way the app writes them** (see
/// `golish-agent-app` session init): `VT_TRANSCRIPT_DIR` env override, else a
/// real workspace's `{workspace}/.golish/transcripts`, else
/// `~/.golish/transcripts`.
///
/// Read-side resolution (the `harness_trace` tool, op_trace) MUST stay in
/// lockstep with the write side: a home-only default silently misses every run
/// launched from a real workspace (the common case), which is exactly the
/// "no logs" symptom.
pub fn resolve_transcript_base(workspace: Option<&Path>) -> PathBuf {
    if let Some(dir) = std::env::var_os("VT_TRANSCRIPT_DIR") {
        return PathBuf::from(dir);
    }
    if let Some(ws) = workspace {
        let s = ws.as_os_str();
        if !s.is_empty() && s != "." {
            return ws.join(".golish").join("transcripts");
        }
    }
    home_transcript_base()
}

/// Home/env-only base (no workspace context); equals
/// [`resolve_transcript_base(None)`](resolve_transcript_base). Kept as the
/// zero-context default for callers that genuinely have no workspace.
pub fn default_transcript_base() -> PathBuf {
    resolve_transcript_base(None)
}

/// Process-global "active" transcripts base, set by the app at session init so
/// out-of-band writers (e.g. the per-run tracing log layer in
/// `golish::telemetry::session_log`) co-locate their `run.log` next to
/// `transcript.json` without re-resolving the workspace. Falls back to the home
/// base when unset.
static ACTIVE_TRANSCRIPT_BASE: RwLock<Option<PathBuf>> = RwLock::new(None);

/// Record the transcripts base the app is actively writing to (call at session
/// init, right after [`resolve_transcript_base`]).
pub fn set_active_transcript_base(base: PathBuf) {
    if let Ok(mut guard) = ACTIVE_TRANSCRIPT_BASE.write() {
        *guard = Some(base);
    }
}

/// The active transcripts base if the app registered one, else the home base.
pub fn active_transcript_base_or_home() -> PathBuf {
    ACTIVE_TRANSCRIPT_BASE
        .read()
        .ok()
        .and_then(|guard| guard.clone())
        .unwrap_or_else(home_transcript_base)
}

/// Pick the transcripts base that actually holds `session`, for callers with no
/// running bridge to read the exact base from (`golish --replay`). Honors an
/// explicit `VT_TRANSCRIPT_DIR`; otherwise tries the passed workspace, the
/// current dir, then home, and returns the first whose `{base}/{session}`
/// directory exists (falling back to home).
pub fn resolve_transcript_base_for_session(session: &str, workspace: Option<&Path>) -> PathBuf {
    if let Some(dir) = std::env::var_os("VT_TRANSCRIPT_DIR") {
        return PathBuf::from(dir);
    }
    let mut candidates: Vec<PathBuf> = Vec::new();
    if let Some(ws) = workspace {
        let s = ws.as_os_str();
        if !s.is_empty() && s != "." {
            candidates.push(ws.join(".golish").join("transcripts"));
        }
    }
    if let Ok(cwd) = std::env::current_dir() {
        candidates.push(cwd.join(".golish").join("transcripts"));
    }
    let home = home_transcript_base();
    candidates.push(home.clone());
    candidates
        .into_iter()
        .find(|base| session_dir(base, session).is_dir())
        .unwrap_or(home)
}

/// Read a JSONL transcript file synchronously into timestamped AI events.
/// Tolerates the legacy whole-file JSON array form and skips unparseable lines.
fn read_jsonl(path: &Path) -> Vec<TimestampedEntry<AiEvent>> {
    let Ok(content) = fs::read_to_string(path) else {
        return Vec::new();
    };
    let trimmed = content.trim_start();
    if trimmed.is_empty() {
        return Vec::new();
    }
    if trimmed.starts_with('[') {
        return serde_json::from_str(&content).unwrap_or_default();
    }
    content
        .lines()
        .filter_map(|line| {
            let l = line.trim();
            if l.is_empty() {
                None
            } else {
                serde_json::from_str::<TimestampedEntry<AiEvent>>(l).ok()
            }
        })
        .collect()
}

/// Pull a string field out of an event's JSON representation (e.g. `agent_id`,
/// `operation_id`, `stage`, `task_id`, `status`).
fn event_field(event: &AiEvent, key: &str) -> Option<String> {
    serde_json::to_value(event)
        .ok()?
        .get(key)?
        .as_str()
        .map(str::to_string)
}

fn record_stage(event: &AiEvent) -> Option<String> {
    event_field(event, "stage").or_else(|| event_field(event, "stage_kind"))
}

fn record_operation_id(event: &AiEvent) -> Option<String> {
    event_field(event, "operation_id").or_else(|| event_field(event, "task_id"))
}

/// Merge the main transcript + every sub-agent transcript for a session into one
/// time-ordered list of [`TraceRecord`]s. Main-agent events get `agent_path =
/// "main"`; sub-agent events get `main>{agent_id}` (the agent id stamped on the
/// sub-agent event), falling back to the sub-agent directory name.
pub fn collect_records(base: &Path, session_id: &str) -> Vec<TraceRecord> {
    let dir = session_dir(base, session_id);
    let mut rows: Vec<(DateTime<Utc>, String, AiEvent)> = Vec::new();

    for entry in read_jsonl(&dir.join("transcript.json")) {
        rows.push((entry._timestamp, "main".to_string(), entry.event));
    }

    let subagents_dir = dir.join("subagents");
    if let Ok(read_dir) = fs::read_dir(&subagents_dir) {
        for sub in read_dir.flatten() {
            let sub_path = sub.path();
            if !sub_path.is_dir() {
                continue;
            }
            let dir_label = sub.file_name().to_string_lossy().to_string();
            for entry in read_jsonl(&sub_path.join("transcript.json")) {
                let agent = event_field(&entry.event, "agent_id").unwrap_or_else(|| {
                    // dir name is `{agent_id}-{parent_request_id}`; take the head.
                    dir_label
                        .split('-')
                        .next()
                        .unwrap_or(&dir_label)
                        .to_string()
                });
                let agent_path = build_agent_path(Some("main"), &agent);
                rows.push((entry._timestamp, agent_path, entry.event));
            }
        }
    }

    rows.sort_by_key(|r| r.0);
    rows.into_iter()
        .enumerate()
        .map(|(i, (ts, agent_path, event))| TraceRecord {
            ts: ts.to_rfc3339(),
            seq: i as u64,
            stage: record_stage(&event),
            operation_id: record_operation_id(&event),
            agent_path,
            event,
        })
        .collect()
}

/// Build the one-glance manifest from merged records.
pub fn build_manifest(records: &[TraceRecord], session_id: &str) -> OperationManifest {
    let operation_id = records.iter().find_map(|r| r.operation_id.clone());

    let title = records
        .iter()
        .find_map(|r| match &r.event {
            AiEvent::UserMessage { content } => Some(content.chars().take(80).collect::<String>()),
            _ => None,
        })
        .unwrap_or_default();

    let mut stages: Vec<String> = Vec::new();
    let mut agent_paths: Vec<String> = Vec::new();
    let mut last_gate: Option<&AiEvent> = None;
    let mut last_decision: Option<&AiEvent> = None;
    let mut completed = false;
    for r in records {
        if let Some(s) = &r.stage {
            if !stages.contains(s) {
                stages.push(s.clone());
            }
        }
        if !agent_paths.contains(&r.agent_path) {
            agent_paths.push(r.agent_path.clone());
        }
        match &r.event {
            AiEvent::HarnessTrace { trace, .. } => {
                last_decision = Some(&r.event);
                if matches!(
                    trace,
                    golish_core::events::HarnessTraceKind::GateDecision { .. }
                ) {
                    last_gate = Some(&r.event);
                }
            }
            AiEvent::Completed { .. } => completed = true,
            _ => {}
        }
    }

    let status = match last_gate {
        Some(AiEvent::HarnessTrace { trace, .. }) => {
            if let golish_core::events::HarnessTraceKind::GateDecision { gate, .. } = trace {
                if gate == "BLOCK" {
                    "blocked".to_string()
                } else if completed {
                    "completed".to_string()
                } else {
                    "running".to_string()
                }
            } else {
                "running".to_string()
            }
        }
        _ if completed => "completed".to_string(),
        _ => "running".to_string(),
    };

    OperationManifest {
        operation_id,
        chat_session: session_id.to_string(),
        title,
        status,
        current_stage: stages.last().cloned(),
        stages,
        agent_paths,
        last_decision: last_decision.and_then(|e| serde_json::to_value(e).ok()),
        record_count: records.len(),
        updated_at: Utc::now().to_rfc3339(),
    }
}

/// A compact, model/AI-facing summary line for one event.
fn summarize_event(event: &AiEvent) -> String {
    use golish_core::events::HarnessTraceKind as K;
    match event {
        AiEvent::HarnessTrace { trace, .. } => match trace {
            K::GateDecision {
                gate,
                fabricated_evidence_refs,
                available_real_ids,
                ..
            } => {
                if fabricated_evidence_refs.is_empty() {
                    format!("gate {gate}")
                } else {
                    format!(
                        "gate {gate} fabricated={fabricated_evidence_refs:?} available={available_real_ids:?}"
                    )
                }
            }
            K::EvidenceBooked {
                tool,
                evidence_id,
                source,
            } => format!("evidence #{evidence_id} ({tool}, {source})"),
            K::DeliverableSubmitted {
                status,
                cited_evidence_refs,
                available_real_ids,
            } => format!(
                "submit {status} cited={cited_evidence_refs:?} available={available_real_ids:?}"
            ),
            K::BackgroundNotesInjected { count, .. } => format!("notes injected x{count}"),
            K::MentorAdviceRecorded {
                mode,
                tool,
                repeat_count,
                injected,
                ..
            } => format!("mentor {mode} {tool} x{repeat_count} injected={injected}"),
            K::StageRunOrgProgress {
                org_name,
                status,
                evidence_count,
                ..
            } => format!("stage_run {org_name}: {status} (evidence x{evidence_count})"),
        },
        AiEvent::TaskProgress {
            status, message, ..
        } => format!("task {status}: {message}"),
        AiEvent::SubtaskCompleted { title, .. } => format!("subtask done: {title}"),
        AiEvent::ToolResult {
            tool_name,
            success,
            result,
            ..
        } => {
            // Surface the submit-deliverable outcome (status + cited/available
            // evidence ids) inline — the "cited placeholders while real ids
            // existed" story then sits one line above the gate decision. The data
            // is already in the tool result JSON (design 2026-06-05, Task 8 via
            // the merge layer instead of a dedicated event).
            if tool_name == "submit_stage_deliverable" {
                let status = result.get("status").and_then(|v| v.as_str()).unwrap_or("?");
                match result.get("fabricated_evidence_refs") {
                    Some(fab) if !fab.is_null() => format!(
                        "submit {status} fabricated={fab} available={}",
                        result
                            .get("available_evidence_ids")
                            .cloned()
                            .unwrap_or(serde_json::Value::Null)
                    ),
                    _ => format!("submit {status}"),
                }
            } else {
                format!(
                    "tool {tool_name} -> {}",
                    if *success { "ok" } else { "err" }
                )
            }
        }
        AiEvent::ToolRequest { tool_name, .. } => format!("tool {tool_name} requested"),
        AiEvent::Error { message, .. } => format!("ERROR: {message}"),
        AiEvent::UserMessage { content } => {
            format!("user: {}", content.chars().take(60).collect::<String>())
        }
        other => other.event_type().to_string(),
    }
}

/// Render a human/AI-readable merged timeline for a session.
pub fn render_timeline(base: &Path, session_id: &str) -> String {
    let records = collect_records(base, session_id);
    let manifest = build_manifest(&records, session_id);
    let mut out = String::new();
    out.push_str(&format!(
        "== operation trace · session {} ==\n",
        manifest.chat_session
    ));
    out.push_str(&format!(
        "operation_id: {}\nstatus: {}\ncurrent_stage: {}\nstages: {}\nagents: {}\nrecords: {}\n",
        manifest.operation_id.as_deref().unwrap_or("?"),
        manifest.status,
        manifest.current_stage.as_deref().unwrap_or("-"),
        manifest.stages.join(", "),
        manifest.agent_paths.join(", "),
        manifest.record_count,
    ));
    out.push_str("---- timeline (oldest first) ----\n");
    for r in &records {
        let hhmmss =
            r.ts.split('T')
                .nth(1)
                .and_then(|t| t.split(['.', '+', 'Z']).next())
                .unwrap_or(&r.ts);
        out.push_str(&format!(
            "[{hhmmss}] {:<24} | {}\n",
            r.agent_path,
            summarize_event(&r.event)
        ));
    }
    out
}

/// Decision-focused records as JSON, newest-last, for the `harness_trace` tool.
/// `kinds` filters by event_type (e.g. `harness_trace`, `tool_result`); empty =
/// a sensible decision default (harness traces + tool results + task progress).
pub fn decision_records_json(
    base: &Path,
    session_id: &str,
    last_n: usize,
    kinds: &[String],
) -> Vec<serde_json::Value> {
    let default_kinds = [
        "harness_trace",
        "tool_result",
        "task_progress",
        "subtask_completed",
        "error",
    ];
    let want = |t: &str| -> bool {
        if kinds.is_empty() {
            default_kinds.contains(&t)
        } else {
            kinds.iter().any(|k| k == t)
        }
    };
    let records = collect_records(base, session_id);
    let mut filtered: Vec<serde_json::Value> = records
        .iter()
        .filter(|r| want(r.event.event_type()))
        .map(|r| {
            serde_json::json!({
                "ts": r.ts,
                "agent_path": r.agent_path,
                "stage": r.stage,
                "kind": r.event.event_type(),
                "summary": summarize_event(&r.event),
            })
        })
        .collect();
    if filtered.len() > last_n {
        filtered = filtered.split_off(filtered.len() - last_n);
    }
    filtered
}

/// Write `timeline.jsonl` + `manifest.json` into the session dir (side effect of
/// `just replay`), returning their paths. Atomic manifest write (temp + rename).
pub fn write_trace_artifacts(base: &Path, session_id: &str) -> std::io::Result<(PathBuf, PathBuf)> {
    let dir = session_dir(base, session_id);
    fs::create_dir_all(&dir)?;
    let records = collect_records(base, session_id);

    let timeline_path = dir.join("timeline.jsonl");
    let mut buf = String::new();
    for r in &records {
        if let Ok(line) = serde_json::to_string(r) {
            buf.push_str(&line);
            buf.push('\n');
        }
    }
    fs::write(&timeline_path, buf)?;

    let manifest = build_manifest(&records, session_id);
    let manifest_path = dir.join("manifest.json");
    let tmp = dir.join("manifest.json.tmp");
    fs::write(
        &tmp,
        serde_json::to_string_pretty(&manifest).unwrap_or_default(),
    )?;
    fs::rename(&tmp, &manifest_path)?;
    Ok((timeline_path, manifest_path))
}

#[cfg(test)]
mod tests;
