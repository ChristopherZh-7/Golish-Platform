//! Database models: row types and inserts.

use chrono::{DateTime, Utc};
use serde::{Deserialize, Serialize};
use sqlx::FromRow;
use uuid::Uuid;

mod enums;

pub use enums::*;


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

// ============================================================================
// Execution Plans (structured task plans for continuation)
// ============================================================================

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
    pub target_id: Option<Uuid>,
    pub session_id: Option<String>,
    pub tool_name: Option<String>,
    pub status: String,
    pub detail: serde_json::Value,
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
// Wiki Knowledge Base Models
// ============================================================================

#[derive(Debug, Clone, Serialize, Deserialize, FromRow)]
pub struct WikiPage {
    pub id: Uuid,
    pub path: String,
    pub title: String,
    pub category: String,
    pub tags: Vec<String>,
    pub status: String,
    pub content: String,
    pub word_count: i32,
    pub created_at: DateTime<Utc>,
    pub updated_at: DateTime<Utc>,
}

#[derive(Debug, Clone, Serialize, Deserialize, FromRow)]
pub struct VulnKbLink {
    pub id: Uuid,
    pub cve_id: String,
    pub wiki_path: String,
    pub created_at: DateTime<Utc>,
}

#[derive(Debug, Clone, Serialize, Deserialize, FromRow)]
pub struct VulnKbPoc {
    pub id: Uuid,
    pub cve_id: String,
    pub name: String,
    pub poc_type: String,
    pub language: String,
    pub content: String,
    pub source: String,
    pub source_url: String,
    pub severity: String,
    pub verified: bool,
    pub description: String,
    pub tags: Vec<String>,
    pub created_at: DateTime<Utc>,
    pub updated_at: DateTime<Utc>,
}

#[derive(Debug, Clone, Serialize, Deserialize, FromRow)]
pub struct CvePocSummary {
    pub cve_id: String,
    pub poc_count: i64,
    pub max_severity: Option<String>,
    pub any_verified: Option<bool>,
    pub has_research: Option<bool>,
    pub has_wiki: Option<bool>,
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

#[derive(Debug, Clone, Serialize, Deserialize, FromRow)]
pub struct KbResearchLog {
    pub id: Uuid,
    pub cve_id: String,
    pub session_id: String,
    pub turns: serde_json::Value,
    pub status: String,
    pub created_at: DateTime<Utc>,
    pub updated_at: DateTime<Utc>,
}

#[derive(Debug, Clone, Serialize, Deserialize, FromRow)]
pub struct VulnScanHistory {
    pub id: Uuid,
    pub cve_id: String,
    pub target: String,
    pub result: String,
    pub details: Option<String>,
    pub scanned_at: DateTime<Utc>,
}

// ============================================================================
// Wiki Cross-References & Changelog
// ============================================================================

#[derive(Debug, Clone, Serialize, Deserialize, FromRow)]
pub struct WikiPageRef {
    pub id: Uuid,
    pub source_path: String,
    pub target_path: String,
    pub context: String,
    pub created_at: DateTime<Utc>,
}

#[derive(Debug, Clone, Serialize, Deserialize, FromRow)]
pub struct WikiChangelog {
    pub id: i64,
    pub page_path: String,
    pub action: String,
    pub title: String,
    pub category: String,
    pub actor: String,
    pub summary: String,
    pub created_at: DateTime<Utc>,
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

/// Lightweight page info returned by category-grouped queries.
#[derive(Debug, Clone, Serialize, Deserialize, FromRow)]
pub struct WikiPageSummary {
    pub path: String,
    pub title: String,
    pub category: String,
    pub tags: Vec<String>,
    pub status: String,
    pub word_count: i32,
    pub updated_at: DateTime<Utc>,
}

// ============================================================================
// Security Analysis Models
// ============================================================================

#[derive(Debug, Clone, Serialize, Deserialize, FromRow)]
pub struct TargetAsset {
    pub id: Uuid,
    pub target_id: Uuid,
    pub project_path: Option<String>,
    pub asset_type: String,
    pub value: String,
    pub port: Option<i32>,
    pub protocol: Option<String>,
    pub service: Option<String>,
    pub version: Option<String>,
    pub metadata: serde_json::Value,
    pub status: String,
    pub discovered_at: DateTime<Utc>,
    pub updated_at: DateTime<Utc>,
}

#[derive(Debug, Clone, Serialize, Deserialize, FromRow)]
pub struct ApiEndpoint {
    pub id: Uuid,
    pub target_id: Uuid,
    pub project_path: Option<String>,
    pub url: String,
    pub method: String,
    pub path: String,
    pub params: serde_json::Value,
    pub headers: serde_json::Value,
    pub auth_type: Option<String>,
    pub response_type: Option<String>,
    pub status_code: Option<i32>,
    pub notes: String,
    pub source: String,
    pub risk_level: String,
    pub tested: bool,
    pub capture_path: Option<String>,
    pub discovered_at: DateTime<Utc>,
    pub updated_at: DateTime<Utc>,
}

#[derive(Debug, Clone, Serialize, Deserialize, FromRow)]
pub struct JsAnalysisResult {
    pub id: Uuid,
    pub target_id: Uuid,
    pub project_path: Option<String>,
    pub url: String,
    pub filename: String,
    pub size_bytes: Option<i64>,
    pub hash_sha256: Option<String>,
    pub frameworks: serde_json::Value,
    pub libraries: serde_json::Value,
    pub endpoints_found: serde_json::Value,
    pub secrets_found: serde_json::Value,
    pub comments: serde_json::Value,
    pub source_maps: bool,
    pub risk_summary: String,
    pub raw_analysis: serde_json::Value,
    pub file_path: Option<String>,
    pub analyzed_at: DateTime<Utc>,
}

#[derive(Debug, Clone, Serialize, Deserialize, FromRow)]
pub struct Fingerprint {
    pub id: Uuid,
    pub target_id: Uuid,
    pub project_path: Option<String>,
    pub category: String,
    pub name: String,
    pub version: Option<String>,
    pub confidence: f32,
    pub evidence: serde_json::Value,
    pub cpe: Option<String>,
    pub source: String,
    pub detected_at: DateTime<Utc>,
}

#[derive(Debug, Clone, Serialize, Deserialize, FromRow)]
pub struct PassiveScanLog {
    pub id: Uuid,
    pub target_id: Uuid,
    pub project_path: Option<String>,
    pub test_type: String,
    pub payload: String,
    pub url: String,
    pub parameter: String,
    pub result: String,
    pub evidence: String,
    pub severity: String,
    pub tool_used: String,
    pub tester: String,
    pub notes: String,
    pub detail: serde_json::Value,
    pub tested_at: DateTime<Utc>,
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
