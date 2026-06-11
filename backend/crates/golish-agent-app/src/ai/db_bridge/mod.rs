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
use golish_app_core::ports::pentest::{PentestPlanPort, PgPentestPlanAdapter};
use golish_app_core::ports::recon::{
    PgReconAssetsAdapter, PgReconScansAdapter, PgReconTargetsAdapter, ReconAssetsPort,
    ReconScansPort, ReconTargetsPort,
};
use golish_app_core::ports::vuln::{
    PgVulnIntelAdapter, PgWikiKbAdapter, VulnIntelPort, WikiKbPort,
};

mod convert;
mod evidence;
mod orchestration;
mod recon;
mod tasks;
mod wiki;

pub struct GolishDbRepoProvider {
    pool: Arc<PgPool>,
    // Recon cross-service reads/writes route through the recon service ports
    // (servitization S1-2b) instead of calling `golish_db::repo::<recon>` here.
    recon_scans: Arc<dyn ReconScansPort>,
    recon_assets: Arc<dyn ReconAssetsPort>,
    // In-scope target reads (harness coverage gate) route through the recon
    // targets service port (recon-owned), not the targets repo directly.
    recon_targets: Arc<dyn ReconTargetsPort>,
    // Vuln cross-service reads/writes route through the vuln service ports
    // (servitization S1-2c) instead of calling `golish_db::repo::{vuln_intel,
    // wiki_kb}` here.
    vuln_intel: Arc<dyn VulnIntelPort>,
    wiki_kb: Arc<dyn WikiKbPort>,
    // Pentest execution-plan reads/writes route through the pentest service port
    // (servitization S1-2d) instead of calling the pentest plan repo directly.
    pentest_plan: Arc<dyn PentestPlanPort>,
}

impl GolishDbRepoProvider {
    pub fn new(pool: Arc<PgPool>) -> Self {
        let recon_scans: Arc<dyn ReconScansPort> = Arc::new(PgReconScansAdapter::new(pool.clone()));
        let recon_assets: Arc<dyn ReconAssetsPort> =
            Arc::new(PgReconAssetsAdapter::new(pool.clone()));
        let recon_targets: Arc<dyn ReconTargetsPort> =
            Arc::new(PgReconTargetsAdapter::new(pool.clone()));
        let vuln_intel: Arc<dyn VulnIntelPort> = Arc::new(PgVulnIntelAdapter::new(pool.clone()));
        let wiki_kb: Arc<dyn WikiKbPort> = Arc::new(PgWikiKbAdapter::new(pool.clone()));
        let pentest_plan: Arc<dyn PentestPlanPort> =
            Arc::new(PgPentestPlanAdapter::new(pool.clone()));
        Self {
            pool,
            recon_scans,
            recon_assets,
            recon_targets,
            vuln_intel,
            wiki_kb,
            pentest_plan,
        }
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

    async fn in_scope_assets(&self, org_id: Option<Uuid>) -> anyhow::Result<Vec<String>> {
        self.in_scope_assets_impl(org_id).await
    }

    async fn in_scope_targets(&self) -> anyhow::Result<Vec<serde_json::Value>> {
        self.in_scope_targets_impl().await
    }

    async fn in_scope_target_types(&self, org_id: Option<Uuid>) -> anyhow::Result<Vec<String>> {
        self.in_scope_target_types_impl(org_id).await
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

    // ── Operation State (harness stage cursor) ───────────────
    async fn operation_state_insert(
        &self,
        operation_id: Uuid,
        profile: &str,
        current_stage: &str,
    ) -> anyhow::Result<()> {
        self.operation_state_insert_impl(operation_id, profile, current_stage)
            .await
    }

    async fn operation_state_get(
        &self,
        operation_id: Uuid,
    ) -> anyhow::Result<Option<OperationStateView>> {
        self.operation_state_get_impl(operation_id).await
    }

    async fn operation_state_advance_stage(
        &self,
        operation_id: Uuid,
        new_stage: &str,
    ) -> anyhow::Result<()> {
        self.operation_state_advance_stage_impl(operation_id, new_stage)
            .await
    }

    async fn stage_run_insert(
        &self,
        id: Uuid,
        operation_id: Uuid,
        stage_kind: &str,
    ) -> anyhow::Result<()> {
        self.stage_run_insert_impl(id, operation_id, stage_kind)
            .await
    }

    async fn stage_run_mark_terminal(&self, id: Uuid, status: &str) -> anyhow::Result<()> {
        self.stage_run_mark_terminal_impl(id, status).await
    }

    async fn operation_state_write_state_blob(
        &self,
        operation_id: Uuid,
        state_blob: serde_json::Value,
    ) -> anyhow::Result<()> {
        self.operation_state_write_state_blob_impl(operation_id, state_blob)
            .await
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

    // ── Evidence Ledger (P0) ─────────────────────────────────
    async fn evidence_append(
        &self,
        operation_id: Uuid,
        stage_run_id: Option<Uuid>,
        session_id: Option<&str>,
        project_path: Option<&str>,
        tool_name: &str,
        kind: &str,
        subject: &str,
        raw_output: &str,
        facts: Option<(&str, &str, &str)>,
    ) -> anyhow::Result<i64> {
        self.evidence_append_impl(
            operation_id,
            stage_run_id,
            session_id,
            project_path,
            tool_name,
            kind,
            subject,
            raw_output,
            facts,
        )
        .await
    }

    async fn evidence_facts_for_session(
        &self,
        session_id: &str,
    ) -> anyhow::Result<Vec<(String, String, String, i64)>> {
        self.evidence_facts_for_session_impl(session_id).await
    }

    async fn evidence_existing_ids(
        &self,
        ids: &[i64],
    ) -> anyhow::Result<std::collections::HashSet<i64>> {
        self.evidence_existing_ids_impl(ids).await
    }

    async fn recent_evidence_ids(&self, session_id: &str, limit: i64) -> anyhow::Result<Vec<i64>> {
        self.recent_evidence_ids_impl(session_id, limit).await
    }

    async fn evidence_kinds_for(
        &self,
        ids: &[i64],
    ) -> anyhow::Result<std::collections::HashMap<i64, String>> {
        self.evidence_kinds_for_impl(ids).await
    }

    async fn evidence_ages_for(
        &self,
        ids: &[i64],
    ) -> anyhow::Result<std::collections::HashMap<i64, std::time::Duration>> {
        self.evidence_ages_for_impl(ids).await
    }

    async fn scoping_actions_for_session(
        &self,
        session_id: Uuid,
    ) -> anyhow::Result<Option<ScopingActionsSeen>> {
        let (total, unit_review_invoked, organization_created) =
            golish_db::repo::tool_calls::scoping_actions_for_session(&self.pool, session_id)
                .await?;
        // No recorded tool calls for this session ⇒ verification impossible
        // (tracking disabled / nothing ran) ⇒ fail open (do not block).
        if total == 0 {
            return Ok(None);
        }
        Ok(Some(ScopingActionsSeen {
            unit_review_invoked,
            organization_created,
        }))
    }
}

/// DB-backed integration tests for the harness `operation_state` stage cursor.
///
/// These exercise the **full wired stack** for Phase 2 gate-driven transitions:
/// `decide_transition` (golish-agent-kit) → `DbRepoProvider::operation_state_*`
/// (this app impl) → `golish_db::repo::operation_state` (real Postgres).
///
/// They are **opt-in**: set `GOLISH_TEST_DATABASE_URL` to a migrated Postgres
/// (e.g. the running app's embedded PG) to run them. Without it the tests skip
/// so the default `cargo nextest` stays green in DB-less environments.
#[cfg(test)]
mod operation_state_integration_tests {
    use super::GolishDbRepoProvider;
    use golish_agent_kit::db_traits::DbRepoProvider;
    use golish_agent_kit::harness::{
        base_operation_graph, decide_transition, load_profile_from_json, StageKind,
    };
    use std::sync::Arc;
    use std::time::Duration;
    use uuid::Uuid;

    const ASSESSMENT_JSON: &str =
        include_str!("../../../../../../resources/harness/profiles/assessment.json");

    async fn connect_test_pool() -> Option<Arc<sqlx::PgPool>> {
        let url = std::env::var("GOLISH_TEST_DATABASE_URL").ok()?;
        let pool = sqlx::postgres::PgPoolOptions::new()
            .max_connections(2)
            .acquire_timeout(Duration::from_secs(10))
            .connect(&url)
            .await
            .ok()?;
        Some(Arc::new(pool))
    }

    /// assessment 投影子图: external_attack_surface gate 过 → 应推进到 enumeration.
    fn assessment_advance_target(current: StageKind, gate_allowed: bool) -> Option<StageKind> {
        let dag = base_operation_graph().expect("base graph").project(
            &load_profile_from_json(ASSESSMENT_JSON)
                .expect("assessment profile")
                .allowed_stage_set(),
        );
        decide_transition(current, gate_allowed, &dag).advance_target()
    }

    /// 闭环 (Doc 3 §6.2): insert(external_attack_surface) → 真决策(gate 过)选下一格
    /// → advance_stage → 读回 current_stage == enumeration.
    #[tokio::test]
    async fn gate_pass_advances_operation_state_cursor_end_to_end() {
        let Some(pool) = connect_test_pool().await else {
            eprintln!(
                "skip gate_pass_advances_operation_state_cursor_end_to_end: \
                 set GOLISH_TEST_DATABASE_URL to a migrated Postgres to run it"
            );
            return;
        };
        let repo = GolishDbRepoProvider::new(pool);
        let op = Uuid::new_v4();

        repo.operation_state_insert(op, "assessment", StageKind::ExternalAttackSurface.as_str())
            .await
            .expect("insert operation_state");
        let row = repo
            .operation_state_get(op)
            .await
            .expect("get")
            .expect("row exists after insert");
        assert_eq!(row.current_stage, "external_attack_surface");
        assert_eq!(row.profile, "assessment");

        let next = assessment_advance_target(StageKind::ExternalAttackSurface, true)
            .expect("gate pass should yield an advance target");
        assert_eq!(next, StageKind::Enumeration);

        repo.operation_state_advance_stage(op, next.as_str())
            .await
            .expect("advance_stage");
        let row = repo
            .operation_state_get(op)
            .await
            .expect("get")
            .expect("row exists after advance");
        assert_eq!(row.current_stage, "enumeration");
    }

    /// gate 没过 → 决策 Hold (无 advance target) → 不推进, 游标保持原 stage.
    #[tokio::test]
    async fn gate_block_holds_operation_state_cursor() {
        let Some(pool) = connect_test_pool().await else {
            eprintln!("skip gate_block_holds_operation_state_cursor: set GOLISH_TEST_DATABASE_URL");
            return;
        };
        let repo = GolishDbRepoProvider::new(pool);
        let op = Uuid::new_v4();

        repo.operation_state_insert(op, "assessment", StageKind::ExternalAttackSurface.as_str())
            .await
            .expect("insert operation_state");

        assert!(
            assessment_advance_target(StageKind::ExternalAttackSurface, false).is_none(),
            "blocked gate must not yield an advance target"
        );

        // 没调 advance_stage → 游标仍在 external_attack_surface.
        let row = repo
            .operation_state_get(op)
            .await
            .expect("get")
            .expect("row exists");
        assert_eq!(row.current_stage, "external_attack_surface");
    }
}
