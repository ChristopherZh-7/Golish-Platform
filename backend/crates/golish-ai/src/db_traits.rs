//! Trait abstractions and local model types for database operations.
//!
//! These types decouple `golish-ai` from `golish-db`. The application layer
//! provides concrete implementations via [`DbRepoProvider`].
//!
//! # Migration checklist
//!
//! Files that still reference `golish_db::` directly (replace with these traits):
//!
//! - [ ] `db_tracking/mod.rs` — `DbReadyGate`, `Embedder`
//! - [ ] `db_tracking/helpers.rs` — `DbReadyGate`
//! - [ ] `agent_bridge/config.rs` — `DbReadyGate`, `Embedder`
//! - [ ] `db_tracking/memory/store.rs` — gatekeeper, `ToolcallStatus`
//! - [ ] `tool_executors/knowledge_base/save.rs` — wiki_kb repo, models
//! - [ ] `tool_executors/knowledge_base/search.rs` — wiki_kb repo
//! - [ ] `tool_executors/knowledge_base/query.rs` — wiki_kb repo
//! - [ ] `tool_executors/security.rs` — audit/security repos
//! - [ ] `planner/manager.rs` — execution_plans repo, models
//! - [ ] `task_orchestrator/orchestrator.rs` — tasks repo
//! - [ ] `task_orchestrator/subtask_phases.rs` — tasks/subtasks/message_chains repos
//! - [ ] `task_orchestrator/helpers.rs` — `AgentType`

use async_trait::async_trait;
use serde::{Deserialize, Serialize};
use uuid::Uuid;

// ============================================================================
// Database readiness gate
// ============================================================================

/// Readiness gate for the database connection pool.
///
/// Replaces `golish_db::DbReadyGate`. The application layer provides a
/// concrete implementation that wraps the embedded-PG startup signal.
#[async_trait]
pub trait DbReadinessGate: Send + Sync + Clone {
    fn is_ready(&self) -> bool;
    fn is_failed(&self) -> bool;
    async fn wait(&mut self) -> bool;
}

// ============================================================================
// Embedder
// ============================================================================

/// Text embedding for semantic search.
///
/// Replaces `golish_db::embeddings::Embedder`.
#[async_trait]
pub trait TextEmbedder: Send + Sync {
    async fn embed(&self, text: &str) -> anyhow::Result<Vec<f32>>;
    async fn embed_batch(&self, texts: &[&str]) -> anyhow::Result<Vec<Vec<f32>>> {
        let mut results = Vec::with_capacity(texts.len());
        for text in texts {
            results.push(self.embed(text).await?);
        }
        Ok(results)
    }
    fn dimension(&self) -> usize;
    fn model_name(&self) -> &str;
}

// ============================================================================
// Model enums (mirrors of golish_db::models enums without sqlx derives)
// ============================================================================

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

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "lowercase")]
pub enum MemoryType {
    Observation,
    Conclusion,
    Technique,
    Vulnerability,
    ToolUsage,
}

// ============================================================================
// Model structs (input types for repository operations)
// ============================================================================

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

// ============================================================================
// Memory gatekeeper (pure logic, no DB dependency)
// ============================================================================

#[derive(Debug, Clone, PartialEq, Eq)]
pub enum StoreDecision {
    Store(MemoryType),
    StoreSummary(MemoryType),
    Skip,
}

/// Determine whether a tool's result should be stored.
pub fn should_store(tool_name: &str, status: ToolcallStatus) -> StoreDecision {
    match (tool_name, status) {
        ("run_command", ToolcallStatus::Finished) => StoreDecision::Store(MemoryType::Technique),
        ("bash" | "shell", ToolcallStatus::Finished) => StoreDecision::Store(MemoryType::Technique),
        ("web_search" | "tavily_search" | "web_fetch", _) => {
            StoreDecision::Store(MemoryType::Observation)
        }
        ("write_file" | "edit_file" | "create_file", ToolcallStatus::Finished) => {
            StoreDecision::StoreSummary(MemoryType::Technique)
        }
        ("nmap" | "nikto" | "sqlmap" | "nuclei" | "ffuf" | "gobuster" | "dirsearch",
         ToolcallStatus::Finished) => {
            StoreDecision::Store(MemoryType::Observation)
        }
        _ if tool_name.starts_with("pentest_") && status == ToolcallStatus::Finished => {
            StoreDecision::Store(MemoryType::Vulnerability)
        }
        _ => StoreDecision::Skip,
    }
}

const MIN_CONTENT_LEN: usize = 50;
const MAX_CONTENT_LEN: usize = 8192;
const TRUNCATION_KEEP: usize = 3072;

/// Filter and clean content for memory storage.
pub fn filter_content(result: &str) -> Option<String> {
    let trimmed = result.trim();
    if trimmed.is_empty() || trimmed.len() < MIN_CONTENT_LEN {
        return None;
    }
    let cleaned = strip_ansi(trimmed);
    if cleaned.len() <= MAX_CONTENT_LEN {
        return Some(cleaned);
    }
    let head = &cleaned[..TRUNCATION_KEEP];
    let tail = &cleaned[cleaned.len() - 512..];
    Some(format!(
        "{}\n\n... [{} bytes omitted] ...\n\n{}",
        head,
        cleaned.len() - TRUNCATION_KEEP - 512,
        tail
    ))
}

fn strip_ansi(s: &str) -> String {
    let mut out = String::with_capacity(s.len());
    let mut chars = s.chars().peekable();
    while let Some(c) = chars.next() {
        if c == '\x1b' {
            if chars.peek() == Some(&'[') {
                chars.next();
                for nc in chars.by_ref() {
                    if nc.is_ascii_alphabetic() || nc == 'm' {
                        break;
                    }
                }
            }
        } else {
            out.push(c);
        }
    }
    out
}

/// Build a search-friendly markdown document from tool invocation details.
pub fn build_memory_content(
    tool_name: &str,
    args: &serde_json::Value,
    result: &str,
) -> String {
    match tool_name {
        "run_command" | "bash" | "shell" => {
            let cmd = extract_str(args, "command")
                .or_else(|| extract_str(args, "cmd"))
                .unwrap_or_default();
            format!(
                "## Command Execution\n**Command:** `{cmd}`\n\n**Output:**\n```\n{result}\n```"
            )
        }
        "web_search" | "tavily_search" => {
            let query = extract_str(args, "query").unwrap_or_default();
            format!("## Web Search\n**Query:** {query}\n\n**Results:**\n{result}")
        }
        "web_fetch" => {
            let url = extract_str(args, "url").unwrap_or_default();
            format!("## Web Fetch\n**URL:** {url}\n\n**Content:**\n{result}")
        }
        "write_file" | "create_file" => {
            let path = extract_str(args, "path").unwrap_or_default();
            format!("## File Created/Written\n**Path:** `{path}`\n\n**Summary:** {result}")
        }
        "edit_file" => {
            let path = extract_str(args, "path").unwrap_or_default();
            format!("## File Edited\n**Path:** `{path}`\n\n**Change:** {result}")
        }
        _ => {
            let args_preview = truncate_json(args, 300);
            format!("## {tool_name}\n**Args:** {args_preview}\n\n**Result:**\n{result}")
        }
    }
}

fn extract_str<'a>(v: &'a serde_json::Value, key: &str) -> Option<&'a str> {
    v.get(key).and_then(|v| v.as_str())
}

fn truncate_json(v: &serde_json::Value, max_len: usize) -> String {
    let s = v.to_string();
    if s.len() <= max_len {
        s
    } else {
        format!("{}...", &s[..max_len])
    }
}

// ============================================================================
// Repository trait — ALL golish_db::repo::* operations used by golish-ai
// ============================================================================

/// Provides all database repository operations that golish-ai needs.
///
/// The application layer implements this trait using `golish-db` repo
/// functions and the `PgPool`. golish-ai callers access it through
/// `DbTracker::repo()`.
#[async_trait]
pub trait DbRepoProvider: Send + Sync {
    // -- Wiki KB ----------------------------------------------------------

    async fn wiki_upsert_page(&self, page: &NewWikiPage) -> anyhow::Result<()>;
    async fn wiki_link_cve(&self, cve: &str, path: &str) -> anyhow::Result<()>;
    async fn wiki_delete_refs_from(&self, path: &str) -> anyhow::Result<()>;
    async fn wiki_upsert_page_ref(
        &self,
        from_path: &str,
        to_path: &str,
        context: &str,
    ) -> anyhow::Result<()>;
    async fn wiki_add_changelog(&self, entry: &NewWikiChangelog) -> anyhow::Result<()>;
    async fn wiki_search_fts(
        &self,
        query: &str,
        limit: i64,
    ) -> anyhow::Result<serde_json::Value>;
    async fn wiki_search_by_category(
        &self,
        category: &str,
        limit: i64,
    ) -> anyhow::Result<serde_json::Value>;
    async fn wiki_search_by_tag(
        &self,
        tag: &str,
        limit: i64,
    ) -> anyhow::Result<serde_json::Value>;
    async fn wiki_list_cves_with_pocs(&self) -> anyhow::Result<serde_json::Value>;
    async fn wiki_list_unresearched_cves(
        &self,
        limit: i64,
    ) -> anyhow::Result<serde_json::Value>;
    async fn wiki_poc_stats(&self) -> anyhow::Result<serde_json::Value>;
    async fn wiki_upsert_poc_full(
        &self,
        cve_id: &str,
        name: &str,
        poc_type: &str,
        language: &str,
        content: &str,
        source: &str,
        source_url: &str,
        severity: &str,
        description: &str,
        tags: &[String],
    ) -> anyhow::Result<serde_json::Value>;

    // -- Vuln Intel -------------------------------------------------------

    async fn vuln_intel_search(
        &self,
        cve_id: &str,
        limit: i64,
    ) -> anyhow::Result<serde_json::Value>;

    // -- Security Analysis ------------------------------------------------

    async fn audit_log_operation(
        &self,
        summary: &str,
        op_type: &str,
        description: &str,
        project_path: Option<&str>,
        source: &str,
        target_id: Option<Uuid>,
        session_id: Option<&str>,
        tool_name: Option<&str>,
        status: &str,
        detail: &serde_json::Value,
    ) -> anyhow::Result<serde_json::Value>;

    async fn api_endpoints_insert(
        &self,
        target_id: Uuid,
        project_path: Option<&str>,
        url: &str,
        method: &str,
        path: &str,
        params: &serde_json::Value,
        raw_data: &serde_json::Value,
        auth_type: Option<&str>,
        source: &str,
        risk_level: &str,
    ) -> anyhow::Result<serde_json::Value>;

    async fn js_analysis_insert(
        &self,
        target_id: Uuid,
        project_path: &str,
        url: &str,
        filename: &str,
        analysis: &serde_json::Value,
    ) -> anyhow::Result<serde_json::Value>;

    async fn js_analysis_update_file_path(
        &self,
        id: Uuid,
        file_path: &str,
    ) -> anyhow::Result<()>;

    async fn fingerprints_upsert(
        &self,
        target_id: Uuid,
        project_path: &str,
        category: &str,
        name: &str,
        version: Option<&str>,
        confidence: f64,
        raw_data: Option<&serde_json::Value>,
    ) -> anyhow::Result<bool>;

    async fn passive_scans_insert(
        &self,
        target_id: Uuid,
        project_path: &str,
        scan_type: &str,
        tool_name: &str,
        findings: &serde_json::Value,
        raw_output: Option<&str>,
        severity: &str,
    ) -> anyhow::Result<serde_json::Value>;

    async fn query_target_data(
        &self,
        target_id: Uuid,
        sections: &[String],
    ) -> anyhow::Result<serde_json::Value>;

    // -- Tasks & Subtasks -------------------------------------------------

    async fn task_create(&self, task: NewTask) -> anyhow::Result<TaskView>;
    async fn task_get(&self, id: Uuid) -> anyhow::Result<Option<TaskView>>;
    async fn task_update_status(&self, id: Uuid, status: TaskStatus) -> anyhow::Result<()>;
    async fn task_set_result(&self, id: Uuid, result: &str) -> anyhow::Result<()>;

    async fn subtask_create(
        &self,
        task_id: Uuid,
        session_id: Uuid,
        title: &str,
        description: &str,
        agent: Option<AgentType>,
    ) -> anyhow::Result<SubtaskView>;
    async fn subtask_update_status(
        &self,
        id: Uuid,
        status: SubtaskStatus,
    ) -> anyhow::Result<()>;
    async fn subtask_set_result(&self, id: Uuid, result: &str) -> anyhow::Result<()>;
    async fn subtask_next_pending(&self, task_id: Uuid) -> anyhow::Result<Option<SubtaskView>>;
    async fn subtask_list_by_task(&self, task_id: Uuid) -> anyhow::Result<Vec<SubtaskView>>;
    async fn subtask_delete_pending(&self, task_id: Uuid) -> anyhow::Result<()>;

    // -- Message Chains ---------------------------------------------------

    async fn message_chain_create(
        &self,
        session_id: Uuid,
        task_id: Option<Uuid>,
        subtask_id: Option<Uuid>,
        agent_type: AgentType,
        parent_chain_id: Option<Uuid>,
        model: Option<&str>,
    ) -> anyhow::Result<MessageChainView>;

    async fn message_chain_update_chain(
        &self,
        id: Uuid,
        chain_json: &serde_json::Value,
    ) -> anyhow::Result<()>;

    async fn message_chain_update_usage(
        &self,
        id: Uuid,
        input_tokens: i32,
        output_tokens: i32,
        cache_read_tokens: i32,
        input_cost: f64,
        output_cost: f64,
        duration_ms: i32,
    ) -> anyhow::Result<()>;

    // -- Execution Plans --------------------------------------------------

    async fn plan_list_active(
        &self,
        project_path: &str,
    ) -> anyhow::Result<Vec<ExecutionPlanView>>;

    async fn plan_update_steps(
        &self,
        id: Uuid,
        steps: &serde_json::Value,
        current_step: i32,
        status: PlanStatus,
    ) -> anyhow::Result<()>;

    async fn plan_create(&self, plan: NewExecutionPlan) -> anyhow::Result<ExecutionPlanView>;
}
