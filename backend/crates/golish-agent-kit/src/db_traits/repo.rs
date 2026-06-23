//! Structured database repository operations used by `golish-ai`.
//!
//! [`DbRepoProvider`] covers wiki KB, vulnerability intel, security analysis,
//! tasks/subtasks, message chains, and execution plans.

use async_trait::async_trait;
use uuid::Uuid;

use super::types::*;

/// Real, persisted red_team scoping actions observed for a session (read from
/// `tool_calls`). The scoping gate uses this to verify the model actually
/// performed the unit-candidate + organization-creation flow instead of merely
/// asserting a `scope_human_approved` claim (which a weak model can fabricate).
#[derive(Debug, Clone, Copy, Default)]
pub struct ScopingActionsSeen {
    /// The model invoked `ask_human(input_type="unit_review")` this run.
    pub unit_review_invoked: bool,
    /// The model invoked `manage_organizations(action="create")` this run.
    pub organization_created: bool,
}

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

    /// In-scope recon assets (`targets.scope='in'` values) for the current
    /// operation. The harness coverage gate injects these into
    /// `GateContext.in_scope_assets` so `coverage_complete` measures against the
    /// authoritative asset set (populated by organization recon / manual
    /// target-add) instead of the agent's self-reported coverage.
    ///
    /// `org_id` narrows the axis to the operation's organization (coverage
    /// asset-axis isolation, design 2026-06-09) so a persistent DB carrying
    /// residue from other orgs/runs cannot explode the denominator; `None` =
    /// legacy whole-DB set.
    ///
    /// Default empty so test doubles keep the prior self-reported behavior; the
    /// gate hook only overrides the asset axis when this returns a non-empty set
    /// (an empty set must never vacuously satisfy `coverage_complete`).
    async fn in_scope_assets(&self, org_id: Option<Uuid>) -> anyhow::Result<Vec<String>> {
        let _ = org_id;
        Ok(Vec::new())
    }

    /// P3 ③ seam: distinct `targets.type` values of the in-scope assets (org
    /// narrowed), so the harness coverage gate can derive **dynamic** expected
    /// techniques per asset class (e.g. an IP-only scope drops web-only
    /// techniques). Default empty so test doubles + the app layer (until it
    /// overrides via the recon targets port) keep `spec.expected_techniques`
    /// (zero behavior change). See `technique_resolver`.
    async fn in_scope_target_types(&self, org_id: Option<Uuid>) -> anyhow::Result<Vec<String>> {
        let _ = org_id;
        Ok(Vec::new())
    }

    /// Host-aware coverage 2c-1 (设计 2026-06-15-host-aware-coverage-2c §4.1):
    /// in-scope `(value, targets.type)` pairs so `coverage_complete` can classify
    /// each asset by its **authoritative** type (not just value inference).
    /// Default empty ⇒ the gate falls back to value inference (2a/2b behavior).
    async fn in_scope_typed_assets(
        &self,
        org_id: Option<Uuid>,
    ) -> anyhow::Result<Vec<(String, String)>> {
        let _ = org_id;
        Ok(Vec::new())
    }

    /// In-scope recon targets as JSON rows (`target_id` / `value` / `type`) so an
    /// agent tool can enumerate the recon-collected assets, then drill into each
    /// via [`Self::query_target_data`]. Default empty (test doubles); the app
    /// layer overrides it through the recon targets service port.
    async fn in_scope_targets(&self, org_id: Option<Uuid>) -> anyhow::Result<Vec<serde_json::Value>> {
        let _ = org_id;
        Ok(Vec::new())
    }

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
    /// Engagement-org isolation (设计 2026-06-15-engagement-org-isolation): persist
    /// this operation's bound engagement root org id. Default no-op (test doubles);
    /// the app layer writes `operation_state.engagement_org_id`.
    async fn operation_state_set_engagement_org(
        &self,
        operation_id: Uuid,
        org_id: Option<Uuid>,
    ) -> anyhow::Result<()> {
        let _ = (operation_id, org_id);
        Ok(())
    }

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

    // ── Evidence Ledger (P0 · OpenFang-style hash chain) ────────────────

    /// Append a tool-output evidence row to the ledger: writes an
    /// `audit_log(audit_role='evidence')` row carrying an OpenFang-style hash
    /// chain (`prev_hash`/`hash` in the detail JSON) plus a current scope
    /// classification. Returns the new evidence `audit_log.id`.
    ///
    /// PR2 (coverage 投影) · `facts = Some((technique, asset, outcome))` stamps
    /// the three nullable projection columns (NOT part of the hash-chain detail);
    /// `None` keeps the row out of the coverage projection (old behavior).
    ///
    /// Default impl is a no-op returning `0` so test doubles need not wire a
    /// real ledger; the app layer (`GolishDbRepoProvider`) overrides it.
    #[allow(clippy::too_many_arguments)]
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
        let _ = (
            operation_id,
            stage_run_id,
            session_id,
            project_path,
            tool_name,
            kind,
            subject,
            raw_output,
            facts,
        );
        Ok(0)
    }

    /// PR-C step2b（#4 / E3，设计 2026-06-23-technique-outcomes-provenance）：把一条
    /// 覆盖结局 + provenance upsert 进 `technique_outcomes`（命令路径 / enrich 落库点
    /// 调用）。`asset` 由 app 层过 `canonical_asset_key` 归一（E1）；`outcome` ∈
    /// found|empty|error|blocked。非致命：调用方 warn-only、不回滚证据。默认 no-op
    /// （test double 零改动 + gray-switch off 时调用方根本不调）。app 层覆写。
    #[allow(clippy::too_many_arguments)]
    async fn upsert_technique_outcome(
        &self,
        organization_id: Uuid,
        run_id: &str,
        asset: &str,
        technique: &str,
        outcome: &str,
        source: Option<&str>,
        query: Option<&str>,
        evidence_ids: &[i64],
    ) -> anyhow::Result<()> {
        let _ = (
            organization_id,
            run_id,
            asset,
            technique,
            outcome,
            source,
            query,
            evidence_ids,
        );
        Ok(())
    }

    /// #5（设计 2026-06-23-source-query-log）：把一条被动情报「源查询」upsert 进
    /// `source_query_log`（命令路径 / enrich 落库点调用）——比 `upsert_technique_outcome`
    /// 更细：每 `(run × source × query × target)` 一行，多源各一行。`target` 由 app 层过
    /// `canonical_asset_key` 归一（E1，org 级取 `""`）；`status` ∈ found|empty|error|blocked。
    /// 非致命：调用方 warn-only、不回滚证据。消费模型 A：本表仅写 + reviewer 读，**gate 不读**。
    /// 默认 no-op（test double 零改动 + gray-switch off 时调用方根本不调）。app 层覆写。
    #[allow(clippy::too_many_arguments)]
    async fn upsert_source_query(
        &self,
        organization_id: Uuid,
        run_id: &str,
        source: &str,
        query: &str,
        target: &str,
        technique: Option<&str>,
        status: &str,
        evidence_ids: &[i64],
    ) -> anyhow::Result<()> {
        let _ = (
            organization_id,
            run_id,
            source,
            query,
            target,
            technique,
            status,
            evidence_ids,
        );
        Ok(())
    }

    /// PR2 任务 2.5 (coverage 投影) · the session's evidence facts
    /// `(asset, technique, outcome, evidence_id)`, ledger order. Only rows where
    /// all three projection columns are non-NULL (conservative: unmapped rows
    /// never project). Default empty so test doubles need no ledger.
    async fn evidence_facts_for_session(
        &self,
        session_id: &str,
    ) -> anyhow::Result<Vec<(String, String, String, i64)>> {
        let _ = session_id;
        Ok(Vec::new())
    }

    /// PR-D（#4 / E3，设计 2026-06-23-technique-outcomes-provenance）：从
    /// `technique_outcomes` 物化表读某 `(org, run)` 的 `(asset, technique, outcome,
    /// evidence_id)`（`evidence_id` 取该行 `evidence_ids` 首个，无则 0）。gate 灰度
    /// dual-read 投影源。fail-safe 到空（读失败 → 空，gate 退回 coverage_truth/ledger）。
    /// 默认空（test double 零改动 + gray-switch off 时调用方根本不调）。app 层覆写。
    async fn technique_outcome_facts(
        &self,
        organization_id: Uuid,
        run_id: &str,
    ) -> Vec<(String, String, String, i64)> {
        let _ = (organization_id, run_id);
        Vec::new()
    }

    /// 设计 2026-06-12 §5.3 · DB 业务表真值事实 `(asset, technique)`：业务表里
    /// `asset` 上 `technique` 真有结构化数据（`organizations.asns`/`.certificates`
    /// 专列非空、`target_assets(asset_type='subdomain')` 存在、`dns_records` 有记录）。
    /// coverage gate 外层 hook 把这些转成 `Found` EvidenceFact 合并注入，使 coverage
    /// 判定以 DB 真值为准。
    ///
    /// 只产「有数据」(Found 语义)；DB 无数据**绝不**推断 checked_empty (I8)。
    /// `in_scope_assets` 是 gate 的权威资产集（保证维度对齐）；空集 → 空结果。
    /// `org_id` 做 organization 隔离（design 2026-06-09）。
    ///
    /// 默认空（test double 零改动）；app 层 `GolishDbRepoProvider` 覆写。
    async fn db_truth_facts(
        &self,
        org_id: Option<Uuid>,
        in_scope_assets: &[String],
        run_start: Option<chrono::DateTime<chrono::Utc>>,
    ) -> anyhow::Result<Vec<(String, String)>> {
        let _ = (org_id, in_scope_assets, run_start);
        Ok(Vec::new())
    }

    /// Of the given `audit_log.id`s, return the subset that actually exist as
    /// `audit_role='evidence'` rows. The harness gate uses this to reject
    /// deliverables citing fabricated evidence ids.
    ///
    /// Default impl treats every id as existing (no-op = never blocks) so test
    /// doubles keep passing; the app layer overrides it with a real query.
    async fn evidence_existing_ids(
        &self,
        ids: &[i64],
    ) -> anyhow::Result<std::collections::HashSet<i64>> {
        Ok(ids.iter().copied().collect())
    }

    /// Phase 1.5 阶段过门：本 engagement 在-scope 的 organization id 列表（scoping 建的
    /// org 树）。`project_path=None` = 整库口径（chat 会话无 project key，与 `in_scope_assets`
    /// 一致）。fan-out 阶段收尾用它核「全 org 都过」。默认空 ⇒ 调用方 fail-closed（核不到
    /// 全集就不放行）。
    async fn in_scope_org_ids(&self, project_path: Option<&str>) -> anyhow::Result<Vec<Uuid>> {
        let _ = project_path;
        Ok(Vec::new())
    }

    /// Engagement-org isolation (设计 2026-06-15-engagement-org-isolation): every
    /// org id in the subtree rooted at `root_id` — the scoping-confirmed
    /// engagement root plus its descendants (via `parent_id`). The stage_run
    /// fan-out uses it to drop any requested org OUTSIDE the current engagement's
    /// tree (a sibling engagement's org left in the same workspace). Default empty
    /// (test doubles); the app layer overrides it via the recon organizations
    /// repo. Empty ⇒ caller fails OPEN to legacy behavior (no confinement), so
    /// non-DB contexts are unaffected.
    async fn org_subtree_ids(&self, root_id: Uuid) -> anyhow::Result<Vec<Uuid>> {
        let _ = root_id;
        Ok(Vec::new())
    }

    /// Phase 1.5 阶段过门：批量取 `org_stage_completions` 行 `(organization_id, passed_at)`
    /// （收尾 gate 走 repo 通道，取不到 tracking trait 的 `recent_org_stage_completion`）。
    /// 无行的 org 自然缺席（调用方据此判缺口）。默认空。
    async fn org_stage_completions_get(
        &self,
        stage_kind: &str,
        org_ids: &[Uuid],
    ) -> anyhow::Result<Vec<(Uuid, chrono::DateTime<chrono::Utc>)>> {
        let _ = (stage_kind, org_ids);
        Ok(Vec::new())
    }

    /// Recent **real** evidence ids (`audit_role='evidence'`) for a chat session,
    /// newest first. After the gate rejects a deliverable for citing fabricated
    /// refs, it uses this to tell the agent which real ledger ids it can actually
    /// cite (so it stops copying the template placeholders 1/2/3). `session_id`
    /// is the chat-session string both evidence paths stamp on `audit_log`.
    ///
    /// Default empty so test doubles need no ledger; the app layer overrides it.
    async fn recent_evidence_ids(&self, session_id: &str, limit: i64) -> anyhow::Result<Vec<i64>> {
        let _ = (session_id, limit);
        Ok(Vec::new())
    }

    // ── Stage runs + checkpoint (P1 · graph/checkpoint) ─────────────────

    /// Insert a `stage_runs` row (one stage execution instance). Default no-op
    /// so test doubles keep passing; the app layer overrides it.
    async fn stage_run_insert(
        &self,
        id: Uuid,
        operation_id: Uuid,
        stage_kind: &str,
    ) -> anyhow::Result<()> {
        let _ = (id, operation_id, stage_kind);
        Ok(())
    }

    /// Mark a `stage_runs` row terminal (`completed` / `failed` /
    /// `paused_needs_user`). Default no-op.
    async fn stage_run_mark_terminal(&self, id: Uuid, status: &str) -> anyhow::Result<()> {
        let _ = (id, status);
        Ok(())
    }

    /// Overwrite `operation_state.state_blob` (harness resume checkpoint).
    /// Default no-op.
    async fn operation_state_write_state_blob(
        &self,
        operation_id: Uuid,
        state_blob: serde_json::Value,
    ) -> anyhow::Result<()> {
        let _ = (operation_id, state_blob);
        Ok(())
    }

    /// P2 · map each given evidence `audit_log.id` to its `detail->>'kind'`
    /// (omitting ids with no kind). The verification gate uses this to enforce
    /// a stage's `required_evidence_kinds`. Default empty (test doubles).
    async fn evidence_kinds_for(
        &self,
        ids: &[i64],
    ) -> anyhow::Result<std::collections::HashMap<i64, String>> {
        let _ = ids;
        Ok(std::collections::HashMap::new())
    }

    /// P0 Task 6 · map each given evidence `audit_log.id` to its age
    /// (`NOW() - created_at`). The freshness gate compares this against the
    /// `evidence_kinds.json` max_age to block hard-expired evidence. Default
    /// empty (test doubles never block on freshness).
    async fn evidence_ages_for(
        &self,
        ids: &[i64],
    ) -> anyhow::Result<std::collections::HashMap<i64, std::time::Duration>> {
        let _ = ids;
        Ok(std::collections::HashMap::new())
    }

    /// Cross-verify the red_team scoping flow against this session's REAL
    /// `tool_calls` (so the gate can reject a deliverable that asserts human
    /// scope approval without the model having actually run
    /// `ask_human(input_type="unit_review")` + `manage_organizations(action="create")`).
    ///
    /// Returns `None` when verification is impossible (no `tool_calls` recorded
    /// for this session — test doubles or tracking disabled) so the gate FAILS
    /// OPEN and never blocks on infra absence, mirroring [`Self::evidence_existing_ids`].
    /// `Some(seen)` carries the actually-observed actions.
    async fn scoping_actions_for_session(
        &self,
        session_id: Uuid,
    ) -> anyhow::Result<Option<ScopingActionsSeen>> {
        let _ = session_id;
        Ok(None)
    }
}
