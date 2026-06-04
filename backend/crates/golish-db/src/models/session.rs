//! AI session, execution, and tracking models.

use chrono::{DateTime, Utc};
use serde::{Deserialize, Serialize};
use sqlx::FromRow;
use uuid::Uuid;

use super::enums::*;

#[derive(Debug, Clone, Serialize, Deserialize, FromRow)]
pub struct Session {
    pub id: Uuid,
    pub title: Option<String>,
    pub status: SessionStatus,
    pub workspace_path: Option<String>,
    pub workspace_label: Option<String>,
    pub model: Option<String>,
    pub provider: Option<String>,
    pub project_path: Option<String>,
    /// Stable anchor to the chat-panel session that owns this DB session row.
    /// Lets task mode resume the prior operation instead of creating a new
    /// session+task per message (migration 20260604000002).
    pub chat_session_key: Option<String>,
    pub created_at: DateTime<Utc>,
    pub updated_at: DateTime<Utc>,
}

#[derive(Debug, Clone, Serialize, Deserialize, FromRow)]
pub struct Task {
    pub id: Uuid,
    pub session_id: Uuid,
    pub title: Option<String>,
    pub input: String,
    pub result: Option<String>,
    pub status: TaskStatus,
    pub created_at: DateTime<Utc>,
    pub updated_at: DateTime<Utc>,
}

#[derive(Debug, Clone, Serialize, Deserialize, FromRow)]
pub struct Subtask {
    pub id: Uuid,
    pub task_id: Uuid,
    pub session_id: Uuid,
    pub title: Option<String>,
    pub description: Option<String>,
    pub agent: Option<AgentType>,
    pub result: Option<String>,
    pub context: Option<String>,
    pub status: SubtaskStatus,
    pub created_at: DateTime<Utc>,
    pub updated_at: DateTime<Utc>,
}

#[derive(Debug, Clone, Serialize, Deserialize, FromRow)]
pub struct ToolCall {
    pub id: Uuid,
    pub call_id: String,
    pub session_id: Uuid,
    pub task_id: Option<Uuid>,
    pub subtask_id: Option<Uuid>,
    pub agent: Option<AgentType>,
    pub name: String,
    pub args: serde_json::Value,
    pub result: Option<String>,
    pub status: ToolcallStatus,
    pub duration_ms: Option<i32>,
    pub source: String,
    pub created_at: DateTime<Utc>,
    pub updated_at: DateTime<Utc>,
}

#[derive(Debug, Clone, Serialize, Deserialize, FromRow)]
pub struct TerminalLog {
    pub id: Uuid,
    pub session_id: Uuid,
    pub task_id: Option<Uuid>,
    pub subtask_id: Option<Uuid>,
    pub stream: StreamType,
    pub content: String,
    pub created_at: DateTime<Utc>,
    pub project_path: String,
}

#[derive(Debug, Clone, Serialize, Deserialize, FromRow)]
pub struct SearchLog {
    pub id: Uuid,
    pub session_id: Uuid,
    pub task_id: Option<Uuid>,
    pub subtask_id: Option<Uuid>,
    pub initiator: Option<AgentType>,
    pub engine: String,
    pub query: String,
    pub result: Option<String>,
    pub created_at: DateTime<Utc>,
    pub project_path: String,
}

#[derive(Debug, Clone, Serialize, Deserialize, FromRow)]
pub struct MessageChain {
    pub id: Uuid,
    pub session_id: Uuid,
    pub task_id: Option<Uuid>,
    pub subtask_id: Option<Uuid>,
    pub agent: AgentType,
    pub model: Option<String>,
    pub provider: Option<String>,
    pub chain: Option<serde_json::Value>,
    pub tokens_in: i32,
    pub tokens_out: i32,
    pub tokens_cache_in: i32,
    pub cost_in_usd: f64,
    pub cost_out_usd: f64,
    pub duration_ms: i32,
    pub created_at: DateTime<Utc>,
    pub updated_at: DateTime<Utc>,
}

#[derive(Debug, Clone, Serialize, Deserialize, FromRow)]
pub struct Memory {
    pub id: Uuid,
    pub session_id: Option<Uuid>,
    pub task_id: Option<Uuid>,
    pub subtask_id: Option<Uuid>,
    pub agent: Option<AgentType>,
    pub content: String,
    pub mem_type: MemoryType,
    pub tool_name: Option<String>,
    pub doc_type: String,
    pub project_path: Option<String>,
    pub metadata: Option<serde_json::Value>,
    pub created_at: DateTime<Utc>,
}

#[derive(Debug, Clone, Serialize, Deserialize, FromRow)]
pub struct AgentLog {
    pub id: Uuid,
    pub session_id: Uuid,
    pub task_id: Option<Uuid>,
    pub subtask_id: Option<Uuid>,
    pub initiator: AgentType,
    pub executor: AgentType,
    pub task: String,
    pub result: Option<String>,
    pub duration_ms: Option<i32>,
    pub created_at: DateTime<Utc>,
    pub project_path: String,
}

#[derive(Debug)]
pub struct NewAgentLog {
    pub session_id: Uuid,
    pub task_id: Option<Uuid>,
    pub subtask_id: Option<Uuid>,
    pub initiator: AgentType,
    pub executor: AgentType,
    pub task: String,
}

// ── Execution Plans ─────────────────────────────────────────────────────

#[derive(Debug, Clone, Serialize, Deserialize, FromRow)]
pub struct ExecutionPlan {
    pub id: Uuid,
    pub session_id: Option<Uuid>,
    pub project_path: Option<String>,
    pub title: String,
    pub description: String,
    pub steps: serde_json::Value,
    pub status: PlanStatus,
    pub current_step: i32,
    pub stage_id: Option<String>,
    pub context: serde_json::Value,
    pub created_at: DateTime<Utc>,
    pub updated_at: DateTime<Utc>,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct PlanStep {
    pub id: String,
    pub title: String,
    #[serde(default)]
    pub description: String,
    pub status: String,
    #[serde(default)]
    pub agent: Option<String>,
    #[serde(default)]
    pub result: Option<String>,
    #[serde(default)]
    pub started_at: Option<String>,
    #[serde(default)]
    pub completed_at: Option<String>,
}

#[derive(Debug)]
pub struct NewExecutionPlan {
    pub session_id: Option<Uuid>,
    pub project_path: Option<String>,
    pub title: String,
    pub description: String,
    pub steps: serde_json::Value,
    pub stage_id: Option<String>,
}

// ── Sub-agent Dispatches (P0-4) ─────────────────────────────────────────

#[derive(Debug, Clone, Serialize, Deserialize, FromRow)]
pub struct SubAgentDispatch {
    pub id: Uuid,
    pub session_id: Option<Uuid>,
    pub parent_dispatch_id: Option<Uuid>,
    pub agent_id: String,
    pub tool_call_id: Option<String>,
    pub depth: i32,
    pub status: SubAgentDispatchStatus,
    pub args: serde_json::Value,
    pub result: Option<serde_json::Value>,
    pub error_message: Option<String>,
    pub started_at: DateTime<Utc>,
    pub finished_at: Option<DateTime<Utc>>,
}

// ── Insert structs ──────────────────────────────────────────────────────

#[derive(Debug)]
pub struct NewSession {
    pub title: Option<String>,
    pub workspace_path: Option<String>,
    pub workspace_label: Option<String>,
    pub model: Option<String>,
    pub provider: Option<String>,
    pub project_path: Option<String>,
}

#[derive(Debug)]
pub struct NewTask {
    pub session_id: Uuid,
    pub title: Option<String>,
    pub input: String,
}

#[derive(Debug)]
pub struct NewToolCall {
    pub call_id: String,
    pub session_id: Uuid,
    pub task_id: Option<Uuid>,
    pub subtask_id: Option<Uuid>,
    pub agent: Option<AgentType>,
    pub name: String,
    pub args: serde_json::Value,
    pub source: String,
}

#[derive(Debug)]
pub struct NewMemory {
    pub session_id: Option<Uuid>,
    pub task_id: Option<Uuid>,
    pub subtask_id: Option<Uuid>,
    pub agent: Option<AgentType>,
    pub content: String,
    pub mem_type: MemoryType,
    pub tool_name: Option<String>,
    pub doc_type: String,
    pub project_path: Option<String>,
    pub embedding: Option<Vec<f32>>,
    pub metadata: Option<serde_json::Value>,
}

#[derive(Debug, Clone, Serialize, Deserialize, FromRow)]
pub struct MsgLog {
    pub id: Uuid,
    pub session_id: Uuid,
    pub task_id: Option<Uuid>,
    pub subtask_id: Option<Uuid>,
    pub agent: Option<AgentType>,
    pub msg_type: MsgLogType,
    pub message: String,
    pub result: String,
    pub result_format: MsgLogResultFormat,
    pub thinking: Option<String>,
    pub project_path: Option<String>,
    pub created_at: DateTime<Utc>,
}

#[derive(Debug, Clone, Serialize, Deserialize, FromRow)]
pub struct Screenshot {
    pub id: Uuid,
    pub session_id: Uuid,
    pub task_id: Option<Uuid>,
    pub subtask_id: Option<Uuid>,
    pub name: String,
    pub url: String,
    pub file_path: Option<String>,
    pub content_type: String,
    pub size_bytes: Option<i32>,
    pub project_path: Option<String>,
    pub created_at: DateTime<Utc>,
}

#[derive(Debug, Clone, Serialize, Deserialize, FromRow)]
pub struct VectorStoreLog {
    pub id: Uuid,
    pub session_id: Uuid,
    pub task_id: Option<Uuid>,
    pub subtask_id: Option<Uuid>,
    pub initiator: Option<AgentType>,
    pub executor: Option<AgentType>,
    pub action: VecStoreAction,
    pub query: String,
    pub filter: serde_json::Value,
    pub result: String,
    pub result_count: i32,
    pub project_path: Option<String>,
    pub created_at: DateTime<Utc>,
}

#[derive(Debug, Clone, Serialize, Deserialize, FromRow)]
pub struct PromptTemplate {
    pub id: Uuid,
    pub template_name: String,
    pub content: String,
    pub description: String,
    pub is_active: bool,
    pub project_path: Option<String>,
    pub created_at: DateTime<Utc>,
    pub updated_at: DateTime<Utc>,
}
