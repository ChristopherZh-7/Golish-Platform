//! `harness_trace` — self-service run introspection for the agent.
//!
//! The whole point of the 2026-06-05 observability design: an AI debugging a
//! stuck run should find the relevant trace itself, not ask the user to point at
//! log files. This tool returns the current operation's **merged, decision-only
//! timeline** (main agent + every sub-agent, each line tagged with `agent_path`)
//! by reading the transcripts the system already writes — so a stuck agent (or a
//! debugging agent) can call `harness_trace()` with no args and immediately see
//! `submit ... cited=[1,2,3]` followed by `gate BLOCK`.
//!
//! See `docs/design/2026-06-05-unified-ai-harness-observability.md` §4.D.

use std::path::{Path, PathBuf};

use anyhow::Result;
use serde_json::{json, Value};

use golish_core::Tool;

/// Tool that reads the operation-scoped trace (timeline + manifest) for a chat
/// session. P1 keys traces by the chat-session string (the dir that already
/// holds `transcript.json`); `session_id` is injected at registration so the
/// tool defaults to the current run when called with no arguments.
pub struct HarnessTraceTool {
    session_id: Option<String>,
    base_dir: PathBuf,
}

impl Default for HarnessTraceTool {
    fn default() -> Self {
        Self::new()
    }
}

impl HarnessTraceTool {
    pub fn new() -> Self {
        Self {
            session_id: None,
            base_dir: golish_events::op_trace::default_transcript_base(),
        }
    }

    /// Scope the default lookup to the current run's chat session.
    pub fn with_session_id(mut self, session_id: impl Into<String>) -> Self {
        self.session_id = Some(session_id.into());
        self
    }

    /// Override the transcripts base dir (tests / non-default layouts).
    pub fn with_base_dir(mut self, base_dir: PathBuf) -> Self {
        self.base_dir = base_dir;
        self
    }
}

#[async_trait::async_trait]
impl Tool for HarnessTraceTool {
    fn name(&self) -> &'static str {
        "harness_trace"
    }

    fn description(&self) -> &'static str {
        "Inspect THIS run's own decision timeline when you are stuck, looping, or \
         a stage won't pass. Returns the merged, time-ordered trace of the main \
         agent and every sub-agent (each line tagged with agent_path like \
         main>pentester), filtered to decisions: gate PASS/BLOCK, evidence booked, \
         tool results, task progress. Call with no arguments to get the current \
         operation. Use it to see, e.g., that a deliverable cited evidence ids the \
         gate flagged as fabricated while real ids existed."
    }

    fn parameters(&self) -> Value {
        json!({
            "type": "object",
            "properties": {
                "operation_id": {
                    "type": "string",
                    "description": "Chat-session / operation id to inspect. Omit to use the current run."
                },
                "last_n": {
                    "type": "integer",
                    "description": "Max number of most-recent records to return (default 50).",
                    "default": 50
                },
                "kinds": {
                    "type": "array",
                    "items": { "type": "string" },
                    "description": "Filter to these event kinds (e.g. harness_trace, tool_result, task_progress). Omit for the decision default."
                }
            },
            "additionalProperties": false
        })
    }

    async fn execute(&self, args: Value, _workspace: &Path) -> Result<Value> {
        let session = args
            .get("operation_id")
            .and_then(|v| v.as_str())
            .map(str::to_string)
            .or_else(|| self.session_id.clone());

        let Some(session) = session else {
            return Ok(json!({
                "error": "no current operation; pass operation_id (the chat-session id)"
            }));
        };

        let last_n = args
            .get("last_n")
            .and_then(|v| v.as_u64())
            .unwrap_or(50)
            .max(1) as usize;

        let kinds: Vec<String> = args
            .get("kinds")
            .and_then(|v| v.as_array())
            .map(|a| {
                a.iter()
                    .filter_map(|v| v.as_str().map(str::to_string))
                    .collect()
            })
            .unwrap_or_default();

        let records = golish_events::op_trace::decision_records_json(
            &self.base_dir,
            &session,
            last_n,
            &kinds,
        );
        let manifest = {
            let all = golish_events::op_trace::collect_records(&self.base_dir, &session);
            golish_events::op_trace::build_manifest(&all, &session)
        };

        Ok(json!({
            "session": session,
            "status": manifest.status,
            "operation_id": manifest.operation_id,
            "current_stage": manifest.current_stage,
            "agent_paths": manifest.agent_paths,
            "last_decision": manifest.last_decision,
            "count": records.len(),
            "timeline": records,
        }))
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::fs;

    fn seed(base: &Path, session: &str) {
        let dir = golish_events::op_trace::session_dir(base, session);
        fs::create_dir_all(&dir).unwrap();
        let main = "\
{\"_timestamp\":\"2026-06-05T00:00:02Z\",\"type\":\"harness_trace\",\"operation_id\":\"op-1\",\"stage\":\"target_intel\",\"agent_path\":\"main\",\"kind\":\"gate_decision\",\"gate\":\"BLOCK\",\"findings\":0}
{\"_timestamp\":\"2026-06-05T00:00:03Z\",\"type\":\"text_delta\",\"delta\":\"x\",\"accumulated\":\"x\"}
";
        fs::write(dir.join("transcript.json"), main).unwrap();
    }

    #[tokio::test]
    async fn returns_current_session_decisions_with_no_args() {
        let tmp = tempfile::tempdir().unwrap();
        seed(tmp.path(), "sess-x");
        let tool = HarnessTraceTool::new()
            .with_base_dir(tmp.path().to_path_buf())
            .with_session_id("sess-x");

        let out = tool.execute(json!({}), tmp.path()).await.unwrap();
        assert_eq!(out["session"], "sess-x");
        assert_eq!(out["status"], "blocked");
        // text_delta filtered out; only the gate decision remains
        assert_eq!(out["count"], 1);
        assert_eq!(out["timeline"][0]["kind"], "harness_trace");
        assert!(out["timeline"][0]["summary"]
            .as_str()
            .unwrap()
            .contains("gate BLOCK"));
    }

    #[tokio::test]
    async fn errors_without_session() {
        let tmp = tempfile::tempdir().unwrap();
        let tool = HarnessTraceTool::new().with_base_dir(tmp.path().to_path_buf());
        let out = tool.execute(json!({}), tmp.path()).await.unwrap();
        assert!(out.get("error").is_some());
    }
}
