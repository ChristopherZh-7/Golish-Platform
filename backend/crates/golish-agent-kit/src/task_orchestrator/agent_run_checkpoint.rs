//! P2a · fine-grained agent-run checkpoint, persisted inside
//! `operation_state.state_blob.agent_run`.
//!
//! This layer is intentionally small: it records resumable facts at stable
//! boundaries (pending correction, background job ids, last tool identity) while
//! leaving bulky output in transcripts, background-job records, or evidence rows.
//! It complements the existing `graph_flow` checkpoint and must preserve sibling
//! keys such as `stage_run_workers`.

use serde::{Deserialize, Serialize};
use uuid::Uuid;

pub const AGENT_RUN_KEY: &str = "agent_run";

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum AgentRunStatus {
    BeforeLlmTurn,
    WaitingNextTurn,
    AfterLlmResponseParsed,
    BeforeToolDispatch,
    ToolStarted,
    ToolCompleted,
    Backgrounded,
    RuntimeCorrectionQueued,
    SubmitInFlight,
    GateBlocked,
    Abandoned,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum ToolCheckpointState {
    Planned,
    Started,
    Completed,
    Backgrounded,
    Failed,
    Abandoned,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct ToolCheckpoint {
    pub tool_call_id: String,
    pub tool_name: String,
    pub state: ToolCheckpointState,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub result_ref: Option<String>,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct RuntimeCorrectionCheckpoint {
    pub source: String,
    pub kind: String,
    pub message: String,
    #[serde(default, skip_serializing_if = "Vec::is_empty")]
    pub job_ids: Vec<String>,
    #[serde(default, skip_serializing_if = "Vec::is_empty")]
    pub evidence_ids: Vec<i64>,
    pub submit_allowed: bool,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct AgentRunCheckpoint {
    #[serde(default = "default_schema_v")]
    pub schema_v: u32,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub operation_id: Option<Uuid>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub stage: Option<String>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub stage_attempt_id: Option<Uuid>,
    pub agent_path: String,
    pub status: AgentRunStatus,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub llm_turn_index: Option<u64>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub message_chain_ref: Option<String>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub pending_gate_correction: Option<String>,
    #[serde(default)]
    pub pending_submit_only: bool,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub submit_repair_mode: Option<serde_json::Value>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub repair_directive: Option<serde_json::Value>,
    #[serde(default, skip_serializing_if = "Vec::is_empty")]
    pub runtime_corrections: Vec<RuntimeCorrectionCheckpoint>,
    #[serde(default, skip_serializing_if = "Vec::is_empty")]
    pub background_job_ids: Vec<String>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub evidence_watermark: Option<i64>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub last_tool: Option<ToolCheckpoint>,
    pub updated_at: chrono::DateTime<chrono::Utc>,
}

fn default_schema_v() -> u32 {
    1
}

pub fn state_blob_with_agent_run(
    mut existing: serde_json::Value,
    checkpoint: &AgentRunCheckpoint,
) -> serde_json::Value {
    if !existing.is_object() {
        existing = serde_json::json!({});
    }
    existing.as_object_mut().unwrap().insert(
        AGENT_RUN_KEY.to_string(),
        serde_json::to_value(checkpoint).unwrap_or_else(|_| serde_json::json!({})),
    );
    existing
}

pub fn state_blob_without_agent_run(mut existing: serde_json::Value) -> serde_json::Value {
    if !existing.is_object() {
        return serde_json::json!({});
    }
    existing.as_object_mut().unwrap().remove(AGENT_RUN_KEY);
    existing
}

pub fn agent_run_from_state_blob(blob: &serde_json::Value) -> Option<AgentRunCheckpoint> {
    serde_json::from_value(blob.get(AGENT_RUN_KEY)?.clone()).ok()
}

#[cfg(test)]
mod tests {
    use super::*;

    fn checkpoint() -> AgentRunCheckpoint {
        AgentRunCheckpoint {
            schema_v: 1,
            operation_id: Some(Uuid::parse_str("11111111-1111-1111-1111-111111111111").unwrap()),
            stage: Some("external_attack_surface".to_string()),
            stage_attempt_id: None,
            agent_path: "main>stage_run:external_attack_surface>org:abc>prober".to_string(),
            status: AgentRunStatus::GateBlocked,
            llm_turn_index: Some(1),
            message_chain_ref: Some("22222222-2222-2222-2222-222222222222".to_string()),
            pending_gate_correction: Some("wait_for_background_jobs then resubmit".to_string()),
            pending_submit_only: false,
            submit_repair_mode: None,
            repair_directive: None,
            runtime_corrections: vec![RuntimeCorrectionCheckpoint {
                source: "rule".to_string(),
                kind: "submit_needs_fix".to_string(),
                message: "close the named gate gaps".to_string(),
                job_ids: Vec::new(),
                evidence_ids: vec![42],
                submit_allowed: false,
            }],
            background_job_ids: vec!["job_1".to_string()],
            evidence_watermark: Some(42),
            last_tool: Some(ToolCheckpoint {
                tool_call_id: "call_1".to_string(),
                tool_name: "sub_agent_prober".to_string(),
                state: ToolCheckpointState::Completed,
                result_ref: Some("transcript:event_1".to_string()),
            }),
            updated_at: chrono::DateTime::parse_from_rfc3339("2026-06-25T00:00:00Z")
                .unwrap()
                .with_timezone(&chrono::Utc),
        }
    }

    #[test]
    fn agent_run_checkpoint_round_trips_and_preserves_sibling_blob_keys() {
        let existing = serde_json::json!({
            "graph_flow": { "next_node": "external_attack_surface" },
            "stage_run_workers": {
                "external_attack_surface": {
                    "abc": { "chain_id": "22222222-2222-2222-2222-222222222222" }
                }
            }
        });

        let merged = state_blob_with_agent_run(existing, &checkpoint());
        let back = agent_run_from_state_blob(&merged).expect("agent_run checkpoint");

        assert_eq!(back.status, AgentRunStatus::GateBlocked);
        assert_eq!(
            back.pending_gate_correction.as_deref(),
            Some("wait_for_background_jobs then resubmit")
        );
        assert_eq!(merged["graph_flow"]["next_node"], "external_attack_surface");
        assert_eq!(
            merged["stage_run_workers"]["external_attack_surface"]["abc"]["chain_id"],
            "22222222-2222-2222-2222-222222222222"
        );
    }

    #[test]
    fn state_blob_without_agent_run_keeps_graph_flow() {
        let merged = state_blob_with_agent_run(
            serde_json::json!({ "graph_flow": { "next_node": "x" } }),
            &checkpoint(),
        );
        let cleared = state_blob_without_agent_run(merged);

        assert!(cleared.get(AGENT_RUN_KEY).is_none());
        assert_eq!(cleared["graph_flow"]["next_node"], "x");
    }

    #[test]
    fn missing_schema_version_defaults_to_one() {
        let blob = serde_json::json!({
            "agent_run": {
                "agent_path": "main",
                "status": "waiting_next_turn",
                "updated_at": "2026-06-25T00:00:00Z"
            }
        });

        let back = agent_run_from_state_blob(&blob).expect("agent_run");
        assert_eq!(back.schema_v, 1);
        assert_eq!(back.status, AgentRunStatus::WaitingNextTurn);
    }
}
