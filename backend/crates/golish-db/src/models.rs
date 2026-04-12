use chrono::{DateTime, Utc};
use serde::{Deserialize, Serialize};
use sqlx::FromRow;
use uuid::Uuid;

// ============================================================================
// Enums (matching PostgreSQL custom types)
// ============================================================================

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize, sqlx::Type)]
#[sqlx(type_name = "session_status", rename_all = "lowercase")]
pub enum SessionStatus {
    Created,
    Running,
    Waiting,
    Finished,
    Failed,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize, sqlx::Type)]
#[sqlx(type_name = "task_status", rename_all = "lowercase")]
pub enum TaskStatus {
    Created,
    Running,
    Waiting,
    Finished,
    Failed,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize, sqlx::Type)]
#[sqlx(type_name = "subtask_status", rename_all = "lowercase")]
pub enum SubtaskStatus {
    Created,
    Running,
    Waiting,
    Finished,
    Failed,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize, sqlx::Type)]
#[sqlx(type_name = "toolcall_status", rename_all = "lowercase")]
pub enum ToolcallStatus {
    Received,
    Running,
    Finished,
    Failed,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize, sqlx::Type)]
#[sqlx(type_name = "stream_type", rename_all = "lowercase")]
pub enum StreamType {
    Stdin,
    Stdout,
    Stderr,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize, sqlx::Type)]
#[sqlx(type_name = "memory_type", rename_all = "snake_case")]
pub enum MemoryType {
    Observation,
    Conclusion,
    Technique,
    Vulnerability,
    ToolUsage,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize, sqlx::Type)]
#[sqlx(type_name = "agent_type", rename_all = "lowercase")]
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
    Summarizer,
    Assistant,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize, sqlx::Type)]
#[sqlx(type_name = "target_type", rename_all = "lowercase")]
pub enum TargetType {
    Domain,
    Ip,
    Cidr,
    Url,
    Wildcard,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize, sqlx::Type)]
#[sqlx(type_name = "scope_type", rename_all = "lowercase")]
pub enum ScopeType {
    In,
    Out,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize, sqlx::Type)]
#[sqlx(type_name = "severity", rename_all = "lowercase")]
pub enum Severity {
    Critical,
    High,
    Medium,
    Low,
    Info,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize, sqlx::Type)]
#[sqlx(type_name = "finding_status", rename_all = "snake_case")]
pub enum FindingStatus {
    Open,
    Confirmed,
    Fixed,
    FalsePositive,
    Accepted,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize, sqlx::Type)]
#[sqlx(type_name = "vault_entry_type", rename_all = "snake_case")]
pub enum VaultEntryType {
    Password,
    ApiKey,
    Token,
    Certificate,
    SshKey,
    Other,
}

// ============================================================================
// AI Session & Execution Models
// ============================================================================

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

// ============================================================================
// Pentest Data Models (migrated from SQLite)
// ============================================================================

#[derive(Debug, Clone, Serialize, Deserialize, FromRow)]
pub struct Target {
    pub id: Uuid,
    pub name: String,
    pub target_type: TargetType,
    pub value: String,
    pub tags: serde_json::Value,
    pub notes: String,
    pub scope: ScopeType,
    pub grp: String,
    pub project_path: Option<String>,
    pub created_at: DateTime<Utc>,
    pub updated_at: DateTime<Utc>,
}

#[derive(Debug, Clone, Serialize, Deserialize, FromRow)]
pub struct Finding {
    pub id: Uuid,
    pub title: String,
    pub sev: Severity,
    pub cvss: Option<f64>,
    pub url: String,
    pub target: String,
    pub description: String,
    pub steps: String,
    pub remediation: String,
    pub tags: serde_json::Value,
    pub tool: String,
    pub template: String,
    pub refs: serde_json::Value,
    pub evidence: serde_json::Value,
    pub status: FindingStatus,
    pub source: String,
    pub project_path: Option<String>,
    pub created_at: DateTime<Utc>,
    pub updated_at: DateTime<Utc>,
}

#[derive(Debug, Clone, Serialize, Deserialize, FromRow)]
pub struct Note {
    pub id: Uuid,
    pub entity_type: String,
    pub entity_id: String,
    pub content: String,
    pub color: String,
    pub project_path: Option<String>,
    pub created_at: DateTime<Utc>,
    pub updated_at: DateTime<Utc>,
}

#[derive(Debug, Clone, Serialize, Deserialize, FromRow)]
pub struct AuditEntry {
    pub id: i64,
    pub action: String,
    pub category: String,
    pub details: String,
    pub entity_type: Option<String>,
    pub entity_id: Option<String>,
    pub source: String,
    pub project_path: Option<String>,
    pub created_at: DateTime<Utc>,
}

#[derive(Debug, Clone, Serialize, Deserialize, FromRow)]
pub struct VaultEntry {
    pub id: Uuid,
    pub name: String,
    pub entry_type: VaultEntryType,
    pub value: String,
    pub username: String,
    pub notes: String,
    pub project: String,
    pub tags: serde_json::Value,
    pub project_path: Option<String>,
    pub created_at: DateTime<Utc>,
    pub updated_at: DateTime<Utc>,
}

#[derive(Debug, Clone, Serialize, Deserialize, FromRow)]
pub struct TopologyScan {
    pub name: String,
    pub data: serde_json::Value,
    pub project_path: Option<String>,
    pub created_at: DateTime<Utc>,
}

#[derive(Debug, Clone, Serialize, Deserialize, FromRow)]
pub struct MethodologyProject {
    pub id: Uuid,
    pub data: serde_json::Value,
    pub project_path: Option<String>,
    pub updated_at: DateTime<Utc>,
}

#[derive(Debug, Clone, Serialize, Deserialize, FromRow)]
pub struct Pipeline {
    pub id: Uuid,
    pub data: serde_json::Value,
    pub project_path: Option<String>,
    pub updated_at: DateTime<Utc>,
}

// ============================================================================
// Vulnerability Intelligence Models (migrated from JSON files)
// ============================================================================

#[derive(Debug, Clone, Serialize, Deserialize, FromRow)]
pub struct VulnFeed {
    pub id: String,
    pub name: String,
    pub feed_type: String,
    pub url: String,
    pub enabled: bool,
    pub last_fetched: Option<DateTime<Utc>>,
}

#[derive(Debug, Clone, Serialize, Deserialize, FromRow)]
pub struct VulnEntry {
    pub id: Uuid,
    pub cve_id: String,
    pub title: String,
    pub description: String,
    pub sev: String,
    pub cvss_score: Option<f64>,
    pub published: String,
    pub source: String,
    pub refs: serde_json::Value,
    pub affected_products: serde_json::Value,
    pub fetched_at: DateTime<Utc>,
}

// ============================================================================
// Insert structs (for creating new records without auto-generated fields)
// ============================================================================

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
