//! Local model enums and structs for database operations.
//!
//! These types mirror the DB schema without pulling in `sqlx` or `golish-db`.

use serde::{Deserialize, Serialize};
use uuid::Uuid;

// ── Status enums ────────────────────────────────────────────────────────

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "lowercase")]
pub enum TaskStatus {
    Created,
    Running,
    Waiting,
    Finished,
    Failed,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "lowercase")]
pub enum SubtaskStatus {
    Created,
    Running,
    Waiting,
    Finished,
    Failed,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "lowercase")]
pub enum ToolcallStatus {
    Received,
    Running,
    Finished,
    Failed,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "lowercase")]
pub enum AgentType {
    Primary,
    Pentester,
    Coder,
    Searcher,
    Memorist,
    Reporter,
    Adviser,
    Reflector,
    Enricher,
    Installer,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum PlanStatus {
    Planning,
    InProgress,
    Paused,
    Completed,
    Failed,
    Cancelled,
}

/// Lifecycle status of a sub-agent dispatch.
///
/// Mirrors the Postgres `sub_agent_dispatch_status` ENUM defined in the
/// `20260517000001_sub_agent_dispatches` migration.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum DispatchStatus {
    Running,
    Completed,
    Failed,
    Cancelled,
}

impl DispatchStatus {
    pub fn as_str(&self) -> &'static str {
        match self {
            Self::Running => "running",
            Self::Completed => "completed",
            Self::Failed => "failed",
            Self::Cancelled => "cancelled",
        }
    }
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "lowercase")]
pub enum MemoryType {
    Observation,
    Conclusion,
    Technique,
    Vulnerability,
    ToolUsage,
}

// ── Input structs ───────────────────────────────────────────────────────

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
}

#[derive(Debug)]
pub struct NewTask {
    pub session_id: Uuid,
    pub title: Option<String>,
    pub input: String,
}

#[derive(Debug)]
pub struct NewWikiPage {
    pub path: String,
    pub title: String,
    pub category: String,
    pub tags: Vec<String>,
    pub status: String,
    pub content: String,
}

#[derive(Debug)]
pub struct NewWikiChangelog {
    pub page_path: String,
    pub action: String,
    pub title: String,
    pub category: String,
    pub actor: String,
    pub summary: String,
}

// ── View structs (read-side projections) ────────────────────────────────

/// Memory hit row returned by search/fetch operations.
#[derive(Debug, Clone, serde::Serialize, serde::Deserialize)]
pub struct MemoryHit {
    pub id: Uuid,
    pub content: String,
    pub mem_type: String,
    pub metadata: Option<serde_json::Value>,
    pub created_at: chrono::DateTime<chrono::Utc>,
}

/// Scored memory hit with optional tool name attribution.
#[derive(Debug, Clone, serde::Serialize)]
pub struct ScoredMemoryHit {
    pub hit: MemoryHit,
    pub tool_name: Option<String>,
    pub score: f32,
}

/// Execution plan summary used in briefings.
#[derive(Debug, Clone, serde::Serialize)]
pub struct BriefingPlan {
    pub title: String,
    pub description: Option<String>,
    pub steps: serde_json::Value,
    pub current_step: i32,
    pub status: String,
}

/// Minimal view of a DB subtask (only fields accessed by golish-ai).
#[derive(Debug, Clone)]
pub struct SubtaskView {
    pub id: Uuid,
    pub status: SubtaskStatus,
    pub title: Option<String>,
    pub description: Option<String>,
    pub agent: Option<AgentType>,
    pub result: Option<String>,
}

/// Minimal view of a DB task (only fields accessed by golish-ai).
#[derive(Debug, Clone)]
pub struct TaskView {
    pub id: Uuid,
    pub input: String,
    pub status: TaskStatus,
    pub result: Option<String>,
}

/// Minimal view of a DB execution plan.
#[derive(Debug, Clone)]
pub struct ExecutionPlanView {
    pub id: Uuid,
    pub title: String,
    pub description: String,
    pub steps: serde_json::Value,
    pub status: PlanStatus,
    pub current_step: i32,
}

/// Message chain record.
#[derive(Debug, Clone)]
pub struct MessageChainView {
    pub id: Uuid,
}

/// Minimal view of an `operation_state` cursor row (harness stage cursor, Doc 1 §3.4).
///
/// Only the fields the agent runtime reads back: which profile + which stage the
/// operation is currently on.
#[derive(Debug, Clone)]
pub struct OperationStateView {
    pub operation_id: Uuid,
    pub profile: String,
    pub current_stage: String,
    /// Harness-private resume state (JSONB). Carries `HarnessResumeState`
    /// (current stage run id + queue titles + completed count) for kill→resume.
    pub state_blob: serde_json::Value,
}

/// Minimal view of a sub-agent dispatch row, exposed to higher layers
/// (Tauri command + frontend) for the "resume after restart" feature.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct SubAgentDispatchView {
    pub id: Uuid,
    pub parent_dispatch_id: Option<Uuid>,
    pub agent_id: String,
    pub tool_call_id: Option<String>,
    pub depth: i32,
    pub args: serde_json::Value,
    pub started_at: chrono::DateTime<chrono::Utc>,
}
