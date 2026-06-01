//! Structured database repository operations used by `golish-ai`.
//!
//! [`DbRepoProvider`] covers wiki KB, vulnerability intel, security analysis,
//! tasks/subtasks, message chains, and execution plans.

use async_trait::async_trait;
use uuid::Uuid;

use super::types::*;

/// Provides all database repository operations that golish-ai needs.
///
/// The application layer implements this trait. golish-ai callers access
/// it through `DbTracker::repo()`.
#[async_trait]
pub trait DbRepoProvider: Send + Sync {
    // ── Wiki KB ─────────────────────────────────────────────────────────

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
    async fn wiki_search_fts(&self, query: &str, limit: i64) -> anyhow::Result<serde_json::Value>;
    async fn wiki_search_by_category(
        &self,
        category: &str,
        limit: i64,
    ) -> anyhow::Result<serde_json::Value>;
    async fn wiki_search_by_tag(&self, tag: &str, limit: i64) -> anyhow::Result<serde_json::Value>;
    async fn wiki_list_cves_with_pocs(&self) -> anyhow::Result<serde_json::Value>;
    async fn wiki_list_unresearched_cves(&self, limit: i64) -> anyhow::Result<serde_json::Value>;
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

    // ── Vuln Intel ──────────────────────────────────────────────────────

    async fn vuln_intel_search(
        &self,
        cve_id: &str,
        limit: i64,
    ) -> anyhow::Result<serde_json::Value>;

    // ── Security Analysis ───────────────────────────────────────────────

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

    async fn js_analysis_update_file_path(&self, id: Uuid, file_path: &str) -> anyhow::Result<()>;

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

    // ── Tasks & Subtasks ────────────────────────────────────────────────

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
    async fn subtask_update_status(&self, id: Uuid, status: SubtaskStatus) -> anyhow::Result<()>;
    async fn subtask_set_result(&self, id: Uuid, result: &str) -> anyhow::Result<()>;
    async fn subtask_next_pending(&self, task_id: Uuid) -> anyhow::Result<Option<SubtaskView>>;
    async fn subtask_list_by_task(&self, task_id: Uuid) -> anyhow::Result<Vec<SubtaskView>>;
    async fn subtask_delete_pending(&self, task_id: Uuid) -> anyhow::Result<()>;

    // ── Operation State (harness stage cursor · Doc 1 §3.4) ─────────────

    async fn operation_state_insert(
        &self,
        operation_id: Uuid,
        profile: &str,
        current_stage: &str,
    ) -> anyhow::Result<()>;
    async fn operation_state_get(
        &self,
        operation_id: Uuid,
    ) -> anyhow::Result<Option<OperationStateView>>;
    async fn operation_state_advance_stage(
        &self,
        operation_id: Uuid,
        new_stage: &str,
    ) -> anyhow::Result<()>;

    // ── Message Chains ──────────────────────────────────────────────────

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

    // ── Execution Plans ─────────────────────────────────────────────────

    async fn plan_list_active(&self, project_path: &str) -> anyhow::Result<Vec<ExecutionPlanView>>;

    async fn plan_update_steps(
        &self,
        id: Uuid,
        steps: &serde_json::Value,
        current_step: i32,
        status: PlanStatus,
    ) -> anyhow::Result<()>;

    async fn plan_create(&self, plan: NewExecutionPlan) -> anyhow::Result<ExecutionPlanView>;

    // ── Sub-agent Dispatch Tracking (P0-4) ──────────────────────────────

    async fn dispatch_record_start(
        &self,
        session_id: Uuid,
        parent_dispatch_id: Option<Uuid>,
        agent_id: &str,
        tool_call_id: Option<&str>,
        depth: i32,
        args: &serde_json::Value,
    ) -> anyhow::Result<Uuid>;

    async fn dispatch_record_finish(
        &self,
        id: Uuid,
        status: DispatchStatus,
        result: Option<&serde_json::Value>,
        error_message: Option<&str>,
    ) -> anyhow::Result<()>;

    async fn dispatch_list_running(
        &self,
        session_id: Uuid,
    ) -> anyhow::Result<Vec<SubAgentDispatchView>>;
}
