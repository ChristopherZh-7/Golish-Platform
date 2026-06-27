//! `start_operation` — lead-agent decision handoff to the structured planner.
//!
//! The Task-mode lead turn (a visible `bridge.execute` turn) thinks about the
//! user's request and decides whether it needs the multi-stage planner. If so it
//! calls this tool with a refined `objective` (and optional `analysis`); the
//! handler captures that into the bridge side-channel (`pending_plan_request`),
//! and the Task-mode router reads it after the lead turn to run the planner. If
//! the lead does NOT call this tool, its reply is the answer (no planning).
//!
//! Mirrors the `submit_stage_deliverable` control-plane tool pattern (a typed
//! tool that writes to a bridge side-channel) — see `harness_submit_tool.rs`.

use std::path::Path;
use std::sync::Arc;

use anyhow::Result;
use serde_json::{json, Value};
use tokio::sync::RwLock;

use golish_core::Tool;

/// Captures the lead agent's decision to begin a structured operation into the
/// bridge side-channel (`pending_plan_request`) for the Task-mode router.
pub struct StartOperationTool {
    /// Sink the Task-mode router reads after the lead turn. JSON `{objective, analysis}`.
    pending: Arc<RwLock<Option<String>>>,
}

impl StartOperationTool {
    pub fn new(pending: Arc<RwLock<Option<String>>>) -> Self {
        Self { pending }
    }
}

#[async_trait::async_trait]
impl Tool for StartOperationTool {
    fn name(&self) -> &'static str {
        "start_operation"
    }

    fn description(&self) -> &'static str {
        "Begin a structured multi-stage pentest operation for the user's request. \
         Call this ONLY after you have thought through the request and decided it \
         needs the staged planner (recon -> enumerate -> ...). Pass a refined \
         `objective` (what the operation should achieve / its scope) and a short \
         `analysis`. If the user is answering a continuity question, set \
         `continuity_decision` to `reuse_existing` or `start_fresh`. Otherwise \
         leave it as `ask_before_reuse` so the harness can ask before adopting \
         older DB-backed facts. If the request is a simple question or \
         conversation, do NOT call this — just answer the user directly."
    }

    fn parameters(&self) -> Value {
        json!({
            "type": "object",
            "properties": {
                "objective": {
                    "type": "string",
                    "description": "The refined operation objective / scope to plan for."
                },
                "analysis": {
                    "type": "string",
                    "description": "Your short analysis of the request to seed the planner."
                },
                "continuity_decision": {
                    "type": "string",
                    "enum": ["ask_before_reuse", "reuse_existing", "start_fresh"],
                    "description": "How to handle prior DB-backed progress in a different/older session. Use reuse_existing only when the user explicitly chooses reuse; use start_fresh when the user explicitly rejects reuse; otherwise ask_before_reuse."
                }
            },
            "required": ["objective"]
        })
    }

    async fn execute(&self, args: Value, _workspace: &Path) -> Result<Value> {
        let objective = args
            .get("objective")
            .and_then(|v| v.as_str())
            .unwrap_or("")
            .trim()
            .to_string();
        if objective.is_empty() {
            return Ok(json!({
                "status": "rejected",
                "reason": "objective is required and must be a non-empty string."
            }));
        }
        let analysis = args
            .get("analysis")
            .and_then(|v| v.as_str())
            .unwrap_or("")
            .to_string();
        let continuity_decision = args
            .get("continuity_decision")
            .and_then(|v| v.as_str())
            .filter(|v| matches!(*v, "ask_before_reuse" | "reuse_existing" | "start_fresh"))
            .unwrap_or("ask_before_reuse");
        let payload = json!({
            "objective": objective,
            "analysis": analysis,
            "continuity_decision": continuity_decision
        })
        .to_string();
        *self.pending.write().await = Some(payload);
        Ok(json!({ "status": "planning_started" }))
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[tokio::test]
    async fn captures_objective_into_side_channel() {
        let sink: Arc<RwLock<Option<String>>> = Arc::new(RwLock::new(None));
        let tool = StartOperationTool::new(Arc::clone(&sink));
        let out = tool
            .execute(
                json!({ "objective": "Recon example.com", "analysis": "passive only" }),
                Path::new("/tmp"),
            )
            .await
            .unwrap();
        assert_eq!(out["status"].as_str(), Some("planning_started"));
        let captured = sink.read().await.clone().expect("captured");
        assert!(captured.contains("Recon example.com"));
        assert!(captured.contains("passive only"));
        assert!(captured.contains("ask_before_reuse"));
    }

    #[tokio::test]
    async fn rejects_empty_objective() {
        let sink: Arc<RwLock<Option<String>>> = Arc::new(RwLock::new(None));
        let tool = StartOperationTool::new(Arc::clone(&sink));
        let out = tool
            .execute(json!({ "objective": "  " }), Path::new("/tmp"))
            .await
            .unwrap();
        assert_eq!(out["status"].as_str(), Some("rejected"));
        assert!(sink.read().await.is_none());
    }

    #[tokio::test]
    async fn captures_continuity_decision() {
        let sink: Arc<RwLock<Option<String>>> = Arc::new(RwLock::new(None));
        let tool = StartOperationTool::new(Arc::clone(&sink));
        let out = tool
            .execute(
                json!({
                    "objective": "Continue prior operation",
                    "continuity_decision": "reuse_existing"
                }),
                Path::new("/tmp"),
            )
            .await
            .unwrap();

        assert_eq!(out["status"].as_str(), Some("planning_started"));
        let captured = sink.read().await.clone().expect("captured");
        assert!(captured.contains("reuse_existing"));
    }
}
