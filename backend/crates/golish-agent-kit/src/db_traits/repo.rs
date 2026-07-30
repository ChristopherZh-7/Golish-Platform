//! Structured database repository operations used by `golish-ai`.
//!
//! [`DbRepoProvider`] covers wiki KB, vulnerability intel, security analysis,
//! tasks/subtasks, message chains, and execution plans.

use async_trait::async_trait;
use uuid::Uuid;

use crate::harness::SourceQueryFact;

use super::types::*;

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct AttackV2ReviewBarrierView {
    pub operation_id: Uuid,
    pub wave_run_id: Uuid,
    pub status: String,
    pub resume_version: i64,
    pub wave_unit_count: usize,
    pub review_closed_unit_count: usize,
    pub candidate_count: usize,
    pub proposed_candidate_count: usize,
    pub dispatch_is_stale: bool,
}

/// Server-owned command for closing one exact V2 Verification wave. The
/// operation/snapshot/wave triple comes from validated persisted Verification
/// truth, never from a model deliverable.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct AttackV2ConsolidateWave {
    pub operation_id: Uuid,
    pub scope_snapshot_id: Uuid,
    pub source_wave_run_id: Uuid,
}

/// Durable result returned only after the app bridge commits the Wave
/// consolidation transaction. Counts are safe observability metadata; no
/// candidate hypothesis, exploit material, or evidence body crosses this seam.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct AttackV2WaveConsolidationView {
    pub operation_id: Uuid,
    pub scope_snapshot_id: Uuid,
    pub consolidation_id: Uuid,
    pub source_wave_run_id: Uuid,
    pub target_wave_run_id: Option<Uuid>,
    pub decision_kind: String,
    pub accepted_fact_delta_count: usize,
    pub rejected_fact_delta_count: usize,
    pub residual_risk_count: usize,
    pub pending_enrichment_count: usize,
    pub replayed: bool,
}

/// Real, persisted red_team scoping actions observed for a session (read from
/// `tool_calls`). The scoping gate uses this to verify the model actually
/// performed the unit-candidate review flow instead of merely asserting a
/// `scope_human_approved` claim (which a weak model can fabricate). Creation is
/// no longer mandatory in REUSE mode: an existing org tree confirmed by
/// `unit_review` is already a persisted scope record.
#[derive(Debug, Clone, Default)]
pub struct ScopingActionsSeen {
    /// A completed `manage_organizations(action="propose_candidates")` lifecycle
    /// exists in this operation's Scoping window.
    pub unit_candidates_proposed: bool,
    /// A successful, non-skipped `ask_human(input_type="unit_review")`
    /// completed after a successful candidate proposal for this root org.
    pub unit_review_invoked: bool,
    /// A persisted human subsidiary-scope choice explicitly limited the
    /// engagement to the root/parent organization. This valid branch needs no
    /// candidate proposal or empty unit-review table.
    pub subsidiaries_excluded: bool,
    /// The model invoked `manage_organizations(action="create"/"create_batch")`
    /// and the resulting org row still exists. Informational for audit; REUSE
    /// mode may legitimately leave this false.
    pub organization_created: bool,
    /// A `scope_review` request completed with an affirmative response carrying
    /// a parseable target-row array (not skip/timeout/free text).
    pub scope_review_approved: bool,
    /// Number of completed `scope_review` tool lifecycles in the current
    /// operation's Scoping window. Non-empty trusted scope permits exactly one;
    /// a later confirmation must never erase an earlier edit/rejection.
    pub scope_review_attempts: usize,
    /// Exact rows returned by the human review UI. The scoping gate compares
    /// these against the trusted pre-stage target snapshot before advancing.
    pub scope_review_targets: Vec<ScopingReviewedTarget>,
}

#[derive(Debug, Clone, PartialEq, Eq, serde::Serialize, serde::Deserialize)]
pub struct ScopingReviewedTarget {
    pub value: String,
    #[serde(rename = "type")]
    pub target_type: String,
    pub scope: String,
}

/// Exact human decision applied at the passive-intel -> active-recon boundary.
/// `selected` must be a non-empty unchanged subset of `presented`; the DB
/// implementation revalidates that invariant under the operation row lock.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct ActiveReconScopeReviewApproval {
    pub request_id: String,
    pub presented: Vec<ScopingReviewedTarget>,
    pub selected: Vec<ScopingReviewedTarget>,
}

/// Parse the persisted `ask_human` tool result for a scope-review response.
/// The outer result contains `response` as a JSON string; that string is the
/// editable table's array. Free text, skip, timeout and malformed rows are not
/// approvals.
pub fn parse_scope_review_tool_result(result: &str) -> Option<Vec<ScopingReviewedTarget>> {
    let outer: serde_json::Value = serde_json::from_str(result).ok()?;
    if outer.get("error").is_some()
        || outer
            .get("skipped")
            .and_then(serde_json::Value::as_bool)
            .unwrap_or(false)
    {
        return None;
    }
    let response = outer.get("response")?.as_str()?;
    let rows: serde_json::Value = serde_json::from_str(response).ok()?;
    let rows = rows.as_array()?;
    let mut parsed = Vec::new();
    for row in rows {
        let value = row.get("value")?.as_str()?.trim();
        let target_type = row.get("type")?.as_str()?.trim();
        let scope = row.get("scope")?.as_str()?.trim();
        if value.is_empty()
            || !matches!(target_type, "domain" | "ip" | "cidr" | "url" | "wildcard")
            || !matches!(scope, "in" | "out")
        {
            return None;
        }
        parsed.push(ScopingReviewedTarget {
            value: value.to_string(),
            target_type: target_type.to_string(),
            scope: scope.to_string(),
        });
    }
    Some(parsed)
}

#[cfg(test)]
mod scoping_review_result_tests {
    use super::parse_scope_review_tool_result;

    #[test]
    fn parses_approved_table_and_rejects_skip_or_free_text() {
        let approved = serde_json::json!({
            "response": serde_json::json!([{
                "value": "moresec.cn",
                "type": "domain",
                "scope": "in"
            }]).to_string(),
            "skipped": false
        })
        .to_string();
        let rows = parse_scope_review_tool_result(&approved).expect("approved rows parse");
        assert_eq!(rows.len(), 1);
        assert_eq!(rows[0].value, "moresec.cn");

        assert!(parse_scope_review_tool_result(r#"{"skipped":true}"#).is_none());
        assert!(
            parse_scope_review_tool_result(r#"{"response":"auto-approved","skipped":false}"#)
                .is_none()
        );
    }
}

/// One organization in the scoping-confirmed engagement tree.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct OrgScopeUnit {
    pub id: Uuid,
    pub name: String,
    pub parent_id: Option<Uuid>,
}

/// DB-authoritative Cleanup closeout counts. A deliverable never supplies
/// these values; every non-zero count blocks Cleanup/Reporting progression.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct CleanupCloseoutGateSnapshot {
    pub missing_obligation_count: i64,
    pub nonterminal_obligation_count: i64,
    pub undisclosed_residual_count: i64,
    pub invalid_terminal_truth_count: i64,
}

impl CleanupCloseoutGateSnapshot {
    pub const fn allows_closeout(self) -> bool {
        self.missing_obligation_count == 0
            && self.nonterminal_obligation_count == 0
            && self.undisclosed_residual_count == 0
            && self.invalid_terminal_truth_count == 0
    }
}

/// Provides all database repository operations that golish-ai needs.
///
/// The application layer implements this trait. golish-ai callers access
/// it through `DbTracker::repo()`.
#[async_trait]
pub trait DbRepoProvider: Send + Sync {
    /// Read the immutable Tool Truth contract for a known operation. Test and
    /// standalone adapters retain legacy behavior unless they model rollout.
    async fn tool_truth_contract(
        &self,
        _operation_id: Uuid,
    ) -> anyhow::Result<golish_pentest_domain::tool_truth::ToolTruthContract> {
        Ok(golish_pentest_domain::tool_truth::ToolTruthContract::LegacyV1)
    }

    /// Seal a server-derived denominator from an immutable source identity.
    /// Implementations must reject legacy operations and must not accept
    /// caller-authored members through another seam.
    async fn tool_truth_seal_denominator(
        &self,
        _request: SealToolTruthDenominatorRequest,
    ) -> anyhow::Result<ToolTruthDenominatorView> {
        Err(anyhow::anyhow!(
            "tool truth denominator sealer is unavailable"
        ))
    }

    /// Persist a receipt-derived shadow assessment after the legacy Gate has
    /// already decided. Failures are diagnostic and must never rewrite the
    /// caller's frozen GateResult.
    async fn tool_truth_record_shadow_assessment(
        &self,
        _request: RecordToolTruthShadowAssessment,
    ) -> anyhow::Result<()> {
        Ok(())
    }

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

    /// Same axis as [`DbRepoProvider::in_scope_assets`], but frozen to target rows
    /// that existed at or before `cutoff`. Wave-aware stages use this as a
    /// no-schema current-wave denominator: newly discovered targets remain
    /// persisted, but do not block the current wave's gate.
    ///
    /// Default delegates to the live axis so test doubles and non-app impls keep
    /// prior behavior until they opt into cutoff support.
    async fn in_scope_assets_created_before(
        &self,
        org_id: Option<Uuid>,
        cutoff: chrono::DateTime<chrono::Utc>,
    ) -> anyhow::Result<Vec<String>> {
        let _ = cutoff;
        self.in_scope_assets(org_id).await
    }

    /// Cleanup Gate truth. Default fails closed so a test double or alternate
    /// backend cannot accidentally pass Cleanup without implementing the
    /// canonical obligation/residual read.
    async fn cleanup_closeout_gate(
        &self,
        operation_id: Uuid,
        organization_id: Uuid,
    ) -> anyhow::Result<CleanupCloseoutGateSnapshot> {
        let _ = (operation_id, organization_id);
        anyhow::bail!("CLEANUP_CLOSEOUT_REPO_UNAVAILABLE")
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

    /// EAS host-aware alias exclusion (design 2026-06-30-eas-domain-port-
    /// delegation): in-scope asset values whose resolved IP is already an
    /// in-scope IP target, so the EAS gate can treat them as explanatory aliases
    /// of the direct IP row. Domains without such an IP remain liveness-only;
    /// PORT/SERVICE applies only to IP/CIDR via `technique_resolver`.
    /// Default empty ⇒ no alias exclusion.
    async fn eas_port_delegated_assets(&self, org_id: Option<Uuid>) -> anyhow::Result<Vec<String>> {
        let _ = org_id;
        Ok(Vec::new())
    }

    /// Enumeration IP-web coverage (design 2026-07-01): in-scope IP/CIDR assets
    /// whose `targets.http_status` proves an HTTP service. Default empty keeps
    /// bare IPs out of enumeration for test doubles and non-app implementations.
    async fn enumeration_web_capable_assets(
        &self,
        org_id: Option<Uuid>,
    ) -> anyhow::Result<Vec<String>> {
        let _ = org_id;
        Ok(Vec::new())
    }

    /// EAS web-stack denominator: in-scope assets that this EAS run has proven
    /// have an HTTP(S) surface and therefore need WEB-FINGERPRINT coverage.
    /// Unlike [`Self::enumeration_web_capable_assets`], this includes domains,
    /// URLs, and IP/CIDR assets and honors the stage freshness window when
    /// `run_start` is provided.
    async fn eas_web_capable_assets(
        &self,
        org_id: Option<Uuid>,
        run_start: Option<chrono::DateTime<chrono::Utc>>,
    ) -> anyhow::Result<Vec<String>> {
        let _ = (org_id, run_start);
        Ok(Vec::new())
    }

    /// Exact HTTP(S) origins actively confirmed for current in-scope targets in
    /// this EAS freshness window. `current_wave_target_ids=None` reads the live
    /// org axis; `Some` freezes the denominator to the active wave. Relation
    /// rows must remain target/org bound and never infer authorization from a
    /// DNS-only IP observation.
    async fn eas_required_web_origins(
        &self,
        organization_id: Uuid,
        since: chrono::DateTime<chrono::Utc>,
        current_wave_target_ids: Option<Vec<Uuid>>,
    ) -> anyhow::Result<Vec<String>> {
        let _ = (organization_id, since, current_wave_target_ids);
        Ok(Vec::new())
    }

    /// DNS/53-only assets with no web surface. EAS SERVICE no longer consumes
    /// this as automatic not_applicable; Enumeration uses it to terminalise
    /// content axes for IPs that are not web roots. Default empty ⇒ no derived
    /// not_applicable cells.
    async fn eas_service_not_applicable_assets(
        &self,
        org_id: Option<Uuid>,
        run_start: Option<chrono::DateTime<chrono::Utc>>,
    ) -> anyhow::Result<Vec<String>> {
        let _ = (org_id, run_start);
        Ok(Vec::new())
    }

    /// Dead-asset P3 (design 2026-07-02-dead-asset-liveness-state §5): in-scope
    /// asset values EAS has confirmed dead (`targets.liveness_state = 'dead'`),
    /// for a downstream stage gate to drop from its coverage denominator when its
    /// spec opts in via `skip_dead_assets`. Only `'dead'` (never `'unreachable'`,
    /// which may be transient). Default empty keeps test doubles / non-app impls
    /// and every stage without the flag on their prior denominator.
    async fn dead_asset_values(&self, org_id: Option<Uuid>) -> anyhow::Result<Vec<String>> {
        let _ = org_id;
        Ok(Vec::new())
    }

    /// In-scope recon targets as JSON rows (`target_id` / `value` / `type`) so an
    /// agent tool can enumerate the recon-collected assets, then drill into each
    /// via [`Self::query_target_data`]. Default empty (test doubles); the app
    /// layer overrides it through the recon targets service port.
    async fn in_scope_targets(
        &self,
        org_id: Option<Uuid>,
    ) -> anyhow::Result<Vec<serde_json::Value>> {
        let _ = org_id;
        Ok(Vec::new())
    }

    /// L1b (design 2026-06-24-intel-to-eas-handoff): in-scope recon targets as
    /// rich attack-surface seeds (value/type/source/status/real_ip/ports/
    /// http_status/cdn_waf + a computed `priority`), ranked so the EAS specialist
    /// can prioritise instead of flat-scanning. `cap` truncates the ranked set
    /// (D3 per-org cap; `None` = no cap). Default empty (test doubles); the app
    /// layer overrides it through the recon targets service port.
    async fn attack_surface_seeds(
        &self,
        org_id: Option<Uuid>,
        cap: Option<usize>,
    ) -> anyhow::Result<Vec<serde_json::Value>> {
        let _ = (org_id, cap);
        Ok(Vec::new())
    }

    /// Read-only stage asset coverage snapshot. This mirrors the UI coverage
    /// panel's DB-truth projection so the agent can preflight pending/error
    /// cells before deciding whether to submit a stage deliverable.
    async fn stage_asset_coverage(
        &self,
        organization_id: Uuid,
        stage: &str,
        session_id: Option<&str>,
        stage_started_at: Option<chrono::DateTime<chrono::Utc>>,
        current_wave_target_ids: Option<Vec<Uuid>>,
        current_wave_asset_values: Option<Vec<String>>,
    ) -> anyhow::Result<serde_json::Value> {
        let _ = (
            stage_started_at,
            current_wave_target_ids,
            current_wave_asset_values,
        );
        Ok(serde_json::json!({
            "stage": stage,
            "organization_id": organization_id,
            "session_id": session_id,
            "summary": {
                "total_assets": 0,
                "seed_assets": 0,
                "new_assets": 0,
                "done_assets": 0,
                "pending_assets": 0,
                "blocked_assets": 0
            },
            "assets": []
        }))
    }

    /// Operation-aware variant used by Enumeration routing. The default keeps
    /// existing test doubles/UI projections unchanged; the DB-backed app
    /// implementation uses the trusted operation id to apply exact transport
    /// handoffs from the immediately preceding EAS epoch.
    async fn stage_asset_coverage_for_operation(
        &self,
        operation_id: Option<Uuid>,
        organization_id: Uuid,
        stage: &str,
        session_id: Option<&str>,
        stage_started_at: Option<chrono::DateTime<chrono::Utc>>,
        current_wave_target_ids: Option<Vec<Uuid>>,
        current_wave_asset_values: Option<Vec<String>>,
    ) -> anyhow::Result<serde_json::Value> {
        let _ = operation_id;
        self.stage_asset_coverage(
            organization_id,
            stage,
            session_id,
            stage_started_at,
            current_wave_target_ids,
            current_wave_asset_values,
        )
        .await
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
        runtime_memory_contract: crate::runtime_memory::RuntimeMemoryContract,
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

    /// Authorize Scoping's pre-freeze passive subsidiary discovery against
    /// exact durable lifecycle truth. The model-supplied organization id is
    /// never sufficient by itself; production verifies operation, Scoping
    /// execution, project root and latest same-root human choice. Test doubles
    /// deny by default.
    async fn scoping_passive_recon_organization_authorized(
        &self,
        operation_id: Uuid,
        stage_execution_id: Uuid,
        organization_id: Uuid,
    ) -> anyhow::Result<bool> {
        let _ = (operation_id, stage_execution_id, organization_id);
        Ok(false)
    }

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

    /// Append organization-bound evidence for a stage worker. The trusted
    /// runtime supplies `organization_id`; it is not model input. Production
    /// implementations persist that ownership witness inside the hash-bound
    /// evidence detail so a same-operation sibling org cannot cite the row.
    /// Test doubles may retain the legacy behavior by inheriting this default.
    #[allow(clippy::too_many_arguments)]
    async fn evidence_append_for_organization(
        &self,
        operation_id: Uuid,
        organization_id: Uuid,
        stage_run_id: Option<Uuid>,
        session_id: Option<&str>,
        project_path: Option<&str>,
        tool_name: &str,
        kind: &str,
        subject: &str,
        raw_output: &str,
        facts: Option<(&str, &str, &str)>,
    ) -> anyhow::Result<i64> {
        let _ = organization_id;
        self.evidence_append(
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

    /// Materialize a final-gate-approved `blocked` / `not_applicable` coverage
    /// cell without overwriting producer-owned terminal truth. Returns true only
    /// when the row was inserted or replaced an unfinished partial/error row.
    /// Default no-op keeps pure/test repositories backward compatible.
    #[allow(clippy::too_many_arguments)]
    async fn upsert_terminal_technique_outcome_if_unfinished(
        &self,
        organization_id: Uuid,
        run_id: &str,
        asset: &str,
        technique: &str,
        outcome: &str,
        source: Option<&str>,
        query: Option<&str>,
        evidence_ids: &[i64],
    ) -> anyhow::Result<bool> {
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
        Ok(false)
    }

    /// Target-intel DNS empty-marker hook: after a passive provider run has real
    /// evidence, the app layer may refresh unresolved in-scope domain targets and
    /// persist `(asset, GOLISH-INTEL-DNS, empty)` rows for domains that were actually
    /// resolved and returned no DNS answers. Default no-op keeps tests/doubles small.
    async fn mark_target_intel_dns_empty_outcomes(
        &self,
        organization_id: Uuid,
        run_id: &str,
        evidence_ids: &[i64],
    ) -> anyhow::Result<usize> {
        let _ = (organization_id, run_id, evidence_ids);
        Ok(0)
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

    /// #6（设计 2026-06-23-expansion-queue）：把一条「待扩展线索」enqueue 进
    /// `expansion_queue`（发现点调用，如 recon_discover_subsidiaries 抽出的子公司候选）。
    /// `lead_type` ∈ new_domain|brand|app|github_org|subsidiary|email_domain；`lead_value`
    /// 是线索主体（公司名/域名，不过 canonical_asset_key）。入队恒 `pending`（status 由 impl
    /// 设）。非致命：调用方 warn-only。消费模型 A：本表仅写 + reviewer 读，**gate 不读/不 block**。
    /// 默认 no-op（test double 零改动 + gray-switch off 时调用方根本不调）。app 层覆写。
    #[allow(clippy::too_many_arguments)]
    async fn enqueue_expansion_lead(
        &self,
        organization_id: Uuid,
        run_id: &str,
        lead_type: &str,
        lead_value: &str,
        source: Option<&str>,
        confidence: Option<f32>,
        evidence_ids: &[i64],
    ) -> anyhow::Result<()> {
        let _ = (
            organization_id,
            run_id,
            lead_type,
            lead_value,
            source,
            confidence,
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

    /// Strict active-EAS ledger projection. Implementations must bind every row
    /// to the producer-time organization, the target's current in-scope
    /// organization/project ownership, and `created_at >= since`, then verify
    /// that the evidence asset/technique remains authorized by that target.
    /// Legacy unbound rows fail closed. Default empty prevents test doubles or
    /// older providers from falling back to session-wide evidence for EAS.
    async fn eas_evidence_facts_for_session_org_fresh(
        &self,
        session_id: &str,
        organization_id: Uuid,
        since: chrono::DateTime<chrono::Utc>,
    ) -> anyhow::Result<Vec<(String, String, String, i64)>> {
        let _ = (session_id, organization_id, since);
        Ok(Vec::new())
    }

    /// PR-D（#4 / E3，设计 2026-06-23-technique-outcomes-provenance）：从
    /// `technique_outcomes` 物化表读某 `(org, run)` 的 provenance-preserving
    /// [`TechniqueOutcomeFact`]（`evidence_id` 取该行 `evidence_ids` 首个，无则 0）。gate 灰度
    /// dual-read 投影源。fail-safe 到空（读失败 → 空，gate 退回 coverage_truth/ledger）。
    /// 默认空（test double 零改动 + gray-switch off 时调用方根本不调）。app 层覆写。
    async fn technique_outcome_facts(
        &self,
        organization_id: Uuid,
        run_id: &str,
    ) -> Vec<TechniqueOutcomeFact> {
        let _ = (organization_id, run_id);
        Vec::new()
    }

    /// 护栏 4（设计 2026-07-02-gate-capability-ledger Phase 1）：与
    /// [`Self::technique_outcome_facts`] 相同的投影，但套 stage-run freshness cutoff。
    /// `since = None` → presence-only（等价旧方法）；`since = Some(cutoff)` → 只投影
    /// `collected_at >= cutoff` 的行，避免同 session 旧 stage-run 的 technique_outcomes
    /// 泄漏进本 stage-run 的 coverage gate。
    ///
    /// 默认忽略 `since`、委托回 presence-only 方法，让既有 impl / test double 零改动；
    /// app 层覆写以走 `list_for_run_fresh`。
    async fn technique_outcome_facts_fresh(
        &self,
        organization_id: Uuid,
        run_id: &str,
        since: Option<chrono::DateTime<chrono::Utc>>,
    ) -> Vec<TechniqueOutcomeFact> {
        let _ = since;
        self.technique_outcome_facts(organization_id, run_id).await
    }

    /// Strict projection variant for producers whose durable outcome `run_id`
    /// is operation-scoped while the evidence ledger remains chat-session
    /// scoped. The default preserves existing providers; the app repository
    /// overrides it to validate the exact target-bound evidence tuple from the
    /// separate evidence session.
    async fn technique_outcome_facts_fresh_with_evidence_session(
        &self,
        organization_id: Uuid,
        outcome_run_id: &str,
        evidence_session_id: &str,
        since: Option<chrono::DateTime<chrono::Utc>>,
    ) -> Vec<TechniqueOutcomeFact> {
        let _ = evidence_session_id;
        self.technique_outcome_facts_fresh(organization_id, outcome_run_id, since)
            .await
    }

    /// Fail-closed existence projection used only while building a V2 final
    /// seal. Unlike the gate-facing gray-read methods above, this must preserve
    /// every exact `(organization, run)` row and propagate repository failures:
    /// coverage truth remains fully hash-bound, while canonical references may
    /// name only rows that the final handoff transaction can actually resolve.
    /// Freshness and ownership are re-checked under lock by the canonical-fact
    /// resolver, so this seam intentionally has no per-wave cutoff.
    async fn final_seal_technique_outcome_facts(
        &self,
        organization_id: Uuid,
        run_id: &str,
    ) -> anyhow::Result<Vec<TechniqueOutcomeFact>> {
        let _ = (organization_id, run_id);
        anyhow::bail!("final-seal technique outcome projection is not implemented")
    }

    /// #5（source_query_log gate-read）：从 `source_query_log` 读某 `(org, run)` 的
    /// source/provider terminal rows。gate 只用它证明 source 已尝试，绝不投影 found。
    /// 查询错误与权威空结果保持区分，供 org-bound Intel/EAS gate fail closed。
    async fn source_query_facts(
        &self,
        organization_id: Uuid,
        run_id: &str,
    ) -> anyhow::Result<Vec<SourceQueryFact>> {
        let _ = (organization_id, run_id);
        Ok(Vec::new())
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

    /// Same scope as [`DbRepoProvider::org_subtree_ids`], but with display names
    /// for deterministic `stage_run` fan-out. Default keeps older test doubles
    /// working by falling back to id-as-name rows.
    async fn org_subtree_units(&self, root_id: Uuid) -> anyhow::Result<Vec<OrgScopeUnit>> {
        Ok(self
            .org_subtree_ids(root_id)
            .await?
            .into_iter()
            .map(|id| OrgScopeUnit {
                id,
                name: id.to_string(),
                parent_id: None,
            })
            .collect())
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

    /// Operation-bound completion projection. The table is historically unique
    /// by `(organization_id, stage_kind)`, so concurrent operations must inspect
    /// `stage_run_id` and only consume rows produced by themselves. The default
    /// preserves compatibility for repositories without this projection while
    /// failing closed for operation-bound callers (`stage_run_id = None`).
    async fn org_stage_completions_get_with_run_id(
        &self,
        stage_kind: &str,
        org_ids: &[Uuid],
    ) -> anyhow::Result<Vec<(Uuid, chrono::DateTime<chrono::Utc>, Option<String>)>> {
        Ok(self
            .org_stage_completions_get(stage_kind, org_ids)
            .await?
            .into_iter()
            .map(|(organization_id, passed_at)| (organization_id, passed_at, None))
            .collect())
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

    /// Recent **real** evidence rows for a chat session, newest first, each carrying
    /// debug context: `evidence_id`, `tool`, `subject`, `technique`, `asset`,
    /// `outcome`, `kind`, `age_seconds`. Backs the read-only `list_recent_evidence`
    /// tool. Model-authored evidence ids are optional now, but when a worker chooses
    /// to cite a ledger id this endpoint lets it cite a real one instead of a
    /// placeholder. Returns compact JSON objects (mirrors the `Vec<Value>` shape of
    /// [`Self::in_scope_targets`]). Default empty so test doubles need no ledger; the
    /// app layer overrides it with a real query.
    async fn recent_evidence_detailed(
        &self,
        session_id: &str,
        limit: i64,
    ) -> anyhow::Result<Vec<serde_json::Value>> {
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

    /// Current or initial durable asset wave for a wave-aware stage. App-backed
    /// impls freeze the current denominator to the returned `asset_values`; the
    /// default `None` keeps non-DB test/eval contexts on the legacy live axis.
    async fn stage_asset_wave_current_or_create_initial(
        &self,
        stage_execution_id: Uuid,
        operation_id: Uuid,
        organization_id: Uuid,
        stage_kind: &str,
        started_at: chrono::DateTime<chrono::Utc>,
        limit: i64,
    ) -> anyhow::Result<Option<StageAssetWaveView>> {
        let _ = (
            stage_execution_id,
            operation_id,
            organization_id,
            stage_kind,
            started_at,
            limit,
        );
        Ok(None)
    }

    /// Current running durable asset wave without creating a new one. Used when
    /// an org-level completion already exists and the runtime only needs to
    /// decide whether a pending wave is truly new work or legacy bookkeeping.
    async fn stage_asset_wave_current_running(
        &self,
        operation_id: Uuid,
        organization_id: Uuid,
        stage_kind: &str,
    ) -> anyhow::Result<Option<StageAssetWaveView>> {
        let _ = (operation_id, organization_id, stage_kind);
        Ok(None)
    }

    /// Dispatch-only current-wave read. Receipt-writing implementations seal
    /// the exact wave before returning it; diagnostic/Gate reads keep using
    /// `stage_asset_wave_current_running` and cannot cause writes.
    async fn stage_asset_wave_current_running_for_dispatch(
        &self,
        _stage_execution_id: Uuid,
        operation_id: Uuid,
        organization_id: Uuid,
        stage_kind: &str,
    ) -> anyhow::Result<Option<StageAssetWaveView>> {
        self.stage_asset_wave_current_running(operation_id, organization_id, stage_kind)
            .await
    }

    /// Promote unassigned in-scope targets into the next wave for the same
    /// `(operation, organization, stage)`. `None` means no new assets are waiting.
    async fn stage_asset_wave_create_next(
        &self,
        operation_id: Uuid,
        organization_id: Uuid,
        stage_kind: &str,
        parent_wave_id: Option<Uuid>,
        limit: i64,
    ) -> anyhow::Result<Option<StageAssetWaveView>> {
        let _ = (
            operation_id,
            organization_id,
            stage_kind,
            parent_wave_id,
            limit,
        );
        Ok(None)
    }

    /// Atomically either queue the next unassigned asset wave or seal the
    /// per-org stage completion behind a target-writer barrier. App-backed
    /// implementations must not expose a completion watermark from a separate
    /// transaction than the final candidate read.
    async fn stage_asset_wave_create_next_or_seal_completion(
        &self,
        operation_id: Uuid,
        organization_id: Uuid,
        stage_kind: &str,
        parent_wave_id: Option<Uuid>,
        limit: i64,
        stage_run_id: Option<&str>,
    ) -> anyhow::Result<Option<StageAssetWaveView>> {
        let _ = (
            operation_id,
            organization_id,
            stage_kind,
            parent_wave_id,
            limit,
            stage_run_id,
        );
        Ok(None)
    }

    /// Mark a durable wave as completed after its per-org gate passes. Default
    /// no-op so legacy/non-DB contexts remain unchanged.
    async fn stage_asset_wave_complete(&self, wave_id: Uuid) -> anyhow::Result<()> {
        let _ = wave_id;
        Ok(())
    }

    /// Whether every item in a wave points to a target created no later than
    /// `cutoff`. This lets resume skip backfill completed waves for historical
    /// assets covered by an older deterministic org gate pass.
    async fn stage_asset_wave_all_items_created_at_or_before(
        &self,
        wave_id: Uuid,
        cutoff: chrono::DateTime<chrono::Utc>,
    ) -> anyhow::Result<bool> {
        let _ = (wave_id, cutoff);
        Ok(false)
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

    /// Cross-verify the red_team scoping flow against REAL `tool_calls` from the
    /// current operation's Scoping window (so a later operation in the same chat
    /// session cannot reuse an older human approval).
    ///
    /// Returns `None` when verification is impossible (no `tool_calls` recorded
    /// for this session — test doubles or tracking disabled). The orchestrator
    /// fails closed when policy requires a unit review or a non-empty target
    /// review, while an organization-only empty target snapshot needs no target
    /// lifecycle. `Some(seen)` carries every actually-observed review attempt.
    async fn scoping_actions_for_session(
        &self,
        session_id: Uuid,
        organization_id: Uuid,
        not_before: chrono::DateTime<chrono::Utc>,
    ) -> anyhow::Result<Option<ScopingActionsSeen>> {
        let _ = (session_id, organization_id, not_before);
        Ok(None)
    }

    /// Trusted target rows currently bound to the scoping-confirmed org. The
    /// application implementation excludes provider/active-discovered rows so
    /// a previous Intel/EAS observation cannot masquerade as a user seed.
    async fn scoping_target_snapshot(
        &self,
        organization_id: Uuid,
    ) -> anyhow::Result<Vec<ScopingReviewedTarget>> {
        let _ = organization_id;
        Ok(Vec::new())
    }

    /// Provider-derived exact targets refreshed in this operation's current
    /// Target Intel window. These are review candidates, never authority.
    async fn active_recon_scope_review_candidates(
        &self,
        operation_id: Uuid,
        organization_id: Uuid,
    ) -> anyhow::Result<Vec<ScopingReviewedTarget>> {
        let _ = (operation_id, organization_id);
        Ok(Vec::new())
    }

    /// Atomically promote the selected unchanged subset to trusted intake,
    /// exclude the unselected rows, and persist an operation-bound approval.
    async fn active_recon_scope_review_apply(
        &self,
        operation_id: Uuid,
        organization_id: Uuid,
        approval: ActiveReconScopeReviewApproval,
    ) -> anyhow::Result<Vec<ScopingReviewedTarget>> {
        let _ = (operation_id, organization_id, approval);
        anyhow::bail!("ACTIVE_RECON_SCOPE_REPO_UNAVAILABLE")
    }

    /// Verify that a company-only resume owns an exact authorization written
    /// by this same operation and that it still matches trusted target truth.
    async fn active_recon_scope_review_authorized(
        &self,
        operation_id: Uuid,
        organization_id: Uuid,
    ) -> anyhow::Result<bool> {
        let _ = (operation_id, organization_id);
        anyhow::bail!("ACTIVE_RECON_SCOPE_REPO_UNAVAILABLE")
    }

    // ── Candidate V2 manifest authority ────────────────────────────────

    /// Atomically create/replay the exact WaveUnit entry manifest from the
    /// upstream vuln_triage final-sealed handoff. Formulaic observations can
    /// become only seeds/work-items; this contract has no Candidate/Finding
    /// write surface. Missing implementations fail closed, never as empty work.
    async fn attack_v2_seed_candidate_manifest(
        &self,
        input: crate::harness::attack_execution::SeedCandidateManifest,
    ) -> anyhow::Result<crate::harness::attack_execution::CandidateManifestSnapshot> {
        let _ = input;
        anyhow::bail!("ATTACK_V2_REPO_UNAVAILABLE")
    }

    /// Load the immutable complete manifest for one trusted runtime Unit.
    /// Returning an empty vector for an unavailable repository would let a
    /// pending Candidate stage pass vacuously, so the default is an explicit
    /// stable error code.
    async fn attack_v2_candidate_manifest_for_unit(
        &self,
        operation_id: Uuid,
        stage_run_unit_id: Uuid,
        organization_id: Uuid,
    ) -> anyhow::Result<crate::harness::attack_execution::CandidateManifestSnapshot> {
        let _ = (operation_id, stage_run_unit_id, organization_id);
        anyhow::bail!("ATTACK_V2_REPO_UNAVAILABLE")
    }

    /// Server-only attack_candidate stage-entry materialization. The concrete
    /// repository reloads the current Unit, exact upstream vuln_triage final
    /// handoff, and its authoritative observation evidence before freezing the
    /// complete manifest. No model DTO participates.
    async fn attack_v2_seed_candidate_manifest_for_unit(
        &self,
        operation_id: Uuid,
        stage_run_unit_id: Uuid,
        organization_id: Uuid,
    ) -> anyhow::Result<crate::harness::attack_execution::CandidateManifestSnapshot> {
        let _ = (operation_id, stage_run_unit_id, organization_id);
        anyhow::bail!("ATTACK_V2_REPO_UNAVAILABLE")
    }

    /// Durable review barrier for the current exact Candidate wave. The app
    /// implementation derives project/snapshot/wave/org ownership entirely
    /// from DB state and also performs deterministic expiry/stale-dispatch
    /// reconciliation. Missing implementations must stop stage routing.
    async fn attack_v2_review_barrier_for_operation(
        &self,
        operation_id: Uuid,
    ) -> anyhow::Result<AttackV2ReviewBarrierView> {
        let _ = operation_id;
        anyhow::bail!("ATTACK_V2_REVIEW_REPO_UNAVAILABLE")
    }

    /// Exact persisted Verification truth for the current V2 wave. `None` org
    /// loads every frozen WaveUnit; callers must treat an empty result or any
    /// error as unavailable truth, never as a no-candidate pass.
    async fn attack_v2_verification_truth_for_operation(
        &self,
        operation_id: Uuid,
        organization_id: Option<Uuid>,
    ) -> anyhow::Result<Option<crate::harness::attack_execution::VerificationTruthSet>> {
        let _ = (operation_id, organization_id);
        anyhow::bail!("ATTACK_V2_VERIFICATION_TRUTH_UNAVAILABLE")
    }

    /// Close the exact current Verification Wave and, when policy/fuel allows,
    /// atomically open its follow-on Wave. The implementation must commit the
    /// short DB transaction before returning this view. Missing implementations
    /// fail closed so V2 can never fall back to the process-local wave cursor.
    async fn attack_v2_consolidate_wave(
        &self,
        input: AttackV2ConsolidateWave,
    ) -> anyhow::Result<AttackV2WaveConsolidationView> {
        let _ = input;
        anyhow::bail!("ATTACK_V2_CONSOLIDATION_UNAVAILABLE")
    }

    /// Reporting stage-entry seam. The concrete repository builds or reuses a
    /// current validated revision from the complete canonical DB source set.
    /// It never renders/finalizes an artifact. Missing implementations fail
    /// closed so the terminal stage cannot pass from model prose.
    async fn reporting_build_validated_revision(
        &self,
        operation_id: Uuid,
    ) -> anyhow::Result<crate::harness::ReportingGateTruth> {
        let _ = operation_id;
        anyhow::bail!("REPORTING_TRUTH_REPO_UNAVAILABLE")
    }

    /// Re-read current Reporting truth immediately before Gate evaluation.
    /// `None` means no current report/revision exists and must block.
    async fn reporting_gate_truth(
        &self,
        operation_id: Uuid,
    ) -> anyhow::Result<Option<crate::harness::ReportingGateTruth>> {
        let _ = operation_id;
        anyhow::bail!("REPORTING_TRUTH_REPO_UNAVAILABLE")
    }
}
