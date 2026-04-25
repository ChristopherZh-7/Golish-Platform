//! All status / type enums used as sqlx::Type columns.

use serde::{Deserialize, Serialize};


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

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize, sqlx::Type)]
#[sqlx(type_name = "plan_status", rename_all = "snake_case")]
pub enum PlanStatus {
    Planning,
    InProgress,
    Paused,
    Completed,
    Failed,
    Cancelled,
}

// ============================================================================
// Observability: msg_logs, screenshots, vector_store_logs
// ============================================================================

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize, sqlx::Type)]
#[sqlx(type_name = "msglog_type", rename_all = "snake_case")]
pub enum MsgLogType {
    UserMessage,
    AssistantMessage,
    ToolCall,
    ToolResult,
    SystemHook,
    PlanUpdate,
    Error,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize, sqlx::Type)]
#[sqlx(type_name = "msglog_result_format", rename_all = "lowercase")]
pub enum MsgLogResultFormat {
    Text,
    Json,
    Markdown,
    Html,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize, sqlx::Type)]
#[sqlx(type_name = "vecstore_action", rename_all = "lowercase")]
pub enum VecStoreAction {
    Store,
    Search,
    Delete,
    Update,
}
