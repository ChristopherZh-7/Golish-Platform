//! App-layer implementation of `DbRepoProvider` backed by `golish-db` repo
//! functions and a `PgPool`.
//!
//! The per-domain method bodies live in sibling inherent-impl modules
//! (`wiki` / `recon` / `tasks` / `orchestration`) with a `_impl` suffix; this
//! file holds the struct, the `#[async_trait]` trait impl (thin delegation),
//! and shares the `convert` status/type helpers. Splitting keeps each file
//! within the size budget with zero behavior change.

use std::sync::Arc;

use async_trait::async_trait;
use sqlx::PgPool;
use uuid::Uuid;

use golish_agent_kit::db_traits::*;

mod convert;
mod orchestration;
mod recon;
mod tasks;
mod wiki;

pub struct GolishDbRepoProvider {
    pool: Arc<PgPool>,
}

impl GolishDbRepoProvider {
    pub fn new(pool: Arc<PgPool>) -> Self {
        Self { pool }
    }
}

#[async_trait]
impl DbRepoProvider for GolishDbRepoProvider {
    // ── Wiki KB ──────────────────────────────────────────────
    async fn wiki_upsert_page(&self, page: &NewWikiPage) -> anyhow::Result<()> {
        self.wiki_upsert_page_impl(page).await
    }

    async fn wiki_link_cve(&self, cve: &str, path: &str) -> anyhow::Result<()> {
        self.wiki_link_cve_impl(cve, path).await
    }

    async fn wiki_delete_refs_from(&self, path: &str) -> anyhow::Result<()> {
        self.wiki_delete_refs_from_impl(path).await
    }

    async fn wiki_upsert_page_ref(&self, from: &str, to: &str, ctx: &str) -> anyhow::Result<()> {
        self.wiki_upsert_page_ref_impl(from, to, ctx).await
    }

    async fn wiki_add_changelog(&self, entry: &NewWikiChangelog) -> anyhow::Result<()> {
        self.wiki_add_changelog_impl(entry).await
    }

    async fn wiki_search_fts(&self, query: &str, limit: i64) -> anyhow::Result<serde_json::Value> {
        self.wiki_search_fts_impl(query, limit).await
    }

    async fn wiki_search_by_category(
        &self,
        cat: &str,
        limit: i64,
    ) -> anyhow::Result<serde_json::Value> {
        self.wiki_search_by_category_impl(cat, limit).await
    }

    async fn wiki_search_by_tag(&self, tag: &str, limit: i64) -> anyhow::Result<serde_json::Value> {
        self.wiki_search_by_tag_impl(tag, limit).await
    }

    async fn wiki_list_cves_with_pocs(&self) -> anyhow::Result<serde_json::Value> {
        self.wiki_list_cves_with_pocs_impl().await
    }

    async fn wiki_list_unresearched_cves(&self, limit: i64) -> anyhow::Result<serde_json::Value> {
        self.wiki_list_unresearched_cves_impl(limit).await
    }

    async fn wiki_poc_stats(&self) -> anyhow::Result<serde_json::Value> {
        self.wiki_poc_stats_impl().await
    }

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
    ) -> anyhow::Result<serde_json::Value> {
        self.wiki_upsert_poc_full_impl(
            cve_id,
            name,
            poc_type,
            language,
            content,
            source,
            source_url,
            severity,
            description,
            tags,
        )
        .await
    }

    // ── Vuln Intel / Security Analysis ───────────────────────
    async fn vuln_intel_search(
        &self,
        cve_id: &str,
        limit: i64,
    ) -> anyhow::Result<serde_json::Value> {
        self.vuln_intel_search_impl(cve_id, limit).await
    }

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
    ) -> anyhow::Result<serde_json::Value> {
        self.audit_log_operation_impl(
            summary,
            op_type,
            description,
            project_path,
            source,
            target_id,
            session_id,
            tool_name,
            status,
            detail,
        )
        .await
    }

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
    ) -> anyhow::Result<serde_json::Value> {
        self.api_endpoints_insert_impl(
            target_id,
            project_path,
            url,
            method,
            path,
            params,
            raw_data,
            auth_type,
            source,
            risk_level,
        )
        .await
    }

    async fn js_analysis_insert(
        &self,
        target_id: Uuid,
        project_path: &str,
        url: &str,
        filename: &str,
        _analysis: &serde_json::Value,
    ) -> anyhow::Result<serde_json::Value> {
        self.js_analysis_insert_impl(target_id, project_path, url, filename, _analysis)
            .await
    }

    async fn js_analysis_update_file_path(&self, id: Uuid, file_path: &str) -> anyhow::Result<()> {
        self.js_analysis_update_file_path_impl(id, file_path).await
    }

    async fn fingerprints_upsert(
        &self,
        target_id: Uuid,
        project_path: &str,
        category: &str,
        name: &str,
        version: Option<&str>,
        confidence: f64,
        raw_data: Option<&serde_json::Value>,
    ) -> anyhow::Result<bool> {
        self.fingerprints_upsert_impl(
            target_id,
            project_path,
            category,
            name,
            version,
            confidence,
            raw_data,
        )
        .await
    }

    async fn passive_scans_insert(
        &self,
        target_id: Uuid,
        project_path: &str,
        scan_type: &str,
        tool_name: &str,
        _findings: &serde_json::Value,
        raw_output: Option<&str>,
        severity: &str,
    ) -> anyhow::Result<serde_json::Value> {
        self.passive_scans_insert_impl(
            target_id,
            project_path,
            scan_type,
            tool_name,
            _findings,
            raw_output,
            severity,
        )
        .await
    }

    async fn query_target_data(
        &self,
        target_id: Uuid,
        sections: &[String],
    ) -> anyhow::Result<serde_json::Value> {
        self.query_target_data_impl(target_id, sections).await
    }

    // ── Tasks / Subtasks ─────────────────────────────────────
    async fn task_create(&self, task: NewTask) -> anyhow::Result<TaskView> {
        self.task_create_impl(task).await
    }

    async fn task_get(&self, id: Uuid) -> anyhow::Result<Option<TaskView>> {
        self.task_get_impl(id).await
    }

    async fn task_update_status(&self, id: Uuid, status: TaskStatus) -> anyhow::Result<()> {
        self.task_update_status_impl(id, status).await
    }

    async fn task_set_result(&self, id: Uuid, result: &str) -> anyhow::Result<()> {
        self.task_set_result_impl(id, result).await
    }

    async fn subtask_create(
        &self,
        task_id: Uuid,
        session_id: Uuid,
        title: &str,
        description: &str,
        agent: Option<AgentType>,
    ) -> anyhow::Result<SubtaskView> {
        self.subtask_create_impl(task_id, session_id, title, description, agent)
            .await
    }

    async fn subtask_update_status(&self, id: Uuid, status: SubtaskStatus) -> anyhow::Result<()> {
        self.subtask_update_status_impl(id, status).await
    }

    async fn subtask_set_result(&self, id: Uuid, result: &str) -> anyhow::Result<()> {
        self.subtask_set_result_impl(id, result).await
    }

    async fn subtask_next_pending(&self, task_id: Uuid) -> anyhow::Result<Option<SubtaskView>> {
        self.subtask_next_pending_impl(task_id).await
    }

    async fn subtask_list_by_task(&self, task_id: Uuid) -> anyhow::Result<Vec<SubtaskView>> {
        self.subtask_list_by_task_impl(task_id).await
    }

    async fn subtask_delete_pending(&self, task_id: Uuid) -> anyhow::Result<()> {
        self.subtask_delete_pending_impl(task_id).await
    }

    // ── Message Chains / Execution Plans / Dispatch ──────────
    async fn message_chain_create(
        &self,
        session_id: Uuid,
        task_id: Option<Uuid>,
        subtask_id: Option<Uuid>,
        agent_type: AgentType,
        _parent_chain_id: Option<Uuid>,
        model: Option<&str>,
    ) -> anyhow::Result<MessageChainView> {
        self.message_chain_create_impl(
            session_id,
            task_id,
            subtask_id,
            agent_type,
            _parent_chain_id,
            model,
        )
        .await
    }

    async fn message_chain_update_chain(
        &self,
        id: Uuid,
        chain_json: &serde_json::Value,
    ) -> anyhow::Result<()> {
        self.message_chain_update_chain_impl(id, chain_json).await
    }

    async fn message_chain_update_usage(
        &self,
        id: Uuid,
        input_tokens: i32,
        output_tokens: i32,
        cache_read_tokens: i32,
        input_cost: f64,
        output_cost: f64,
        duration_ms: i32,
    ) -> anyhow::Result<()> {
        self.message_chain_update_usage_impl(
            id,
            input_tokens,
            output_tokens,
            cache_read_tokens,
            input_cost,
            output_cost,
            duration_ms,
        )
        .await
    }

    async fn plan_list_active(&self, project_path: &str) -> anyhow::Result<Vec<ExecutionPlanView>> {
        self.plan_list_active_impl(project_path).await
    }

    async fn plan_update_steps(
        &self,
        id: Uuid,
        steps: &serde_json::Value,
        current_step: i32,
        status: PlanStatus,
    ) -> anyhow::Result<()> {
        self.plan_update_steps_impl(id, steps, current_step, status)
            .await
    }

    async fn plan_create(&self, plan: NewExecutionPlan) -> anyhow::Result<ExecutionPlanView> {
        self.plan_create_impl(plan).await
    }

    async fn dispatch_record_start(
        &self,
        session_id: Uuid,
        parent_dispatch_id: Option<Uuid>,
        agent_id: &str,
        tool_call_id: Option<&str>,
        depth: i32,
        args: &serde_json::Value,
    ) -> anyhow::Result<Uuid> {
        self.dispatch_record_start_impl(
            session_id,
            parent_dispatch_id,
            agent_id,
            tool_call_id,
            depth,
            args,
        )
        .await
    }

    async fn dispatch_record_finish(
        &self,
        id: Uuid,
        status: DispatchStatus,
        result: Option<&serde_json::Value>,
        error_message: Option<&str>,
    ) -> anyhow::Result<()> {
        self.dispatch_record_finish_impl(id, status, result, error_message)
            .await
    }

    async fn dispatch_list_running(
        &self,
        session_id: Uuid,
    ) -> anyhow::Result<Vec<SubAgentDispatchView>> {
        self.dispatch_list_running_impl(session_id).await
    }
}
