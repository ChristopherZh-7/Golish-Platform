//! App-layer implementation of `DbRepoProvider` backed by `golish-db` repo
//! functions and a `PgPool`.
//!
//! The per-domain method bodies live in sibling inherent-impl modules
//! (`wiki` / `recon` / `tasks` / `orchestration`) with a `_impl` suffix; this
//! file holds the struct, the `#[async_trait]` trait impl (thin delegation),
//! and shares the `convert` status/type helpers. Splitting keeps each file
//! within the size budget with zero behavior change.

use std::collections::{BTreeMap, BTreeSet};
use std::sync::Arc;

use anyhow::Context;
use async_trait::async_trait;
use sqlx::PgPool;
use uuid::Uuid;

use golish_agent_kit::db_traits::*;
use golish_agent_kit::runtime_memory::RuntimeMemoryContract;
use golish_app_core::ports::pentest::{PentestPlanPort, PgPentestPlanAdapter};
use golish_app_core::ports::recon::{
    PgReconAssetsAdapter, PgReconDirectoryAdapter, PgReconScansAdapter, PgReconTargetsAdapter,
    ReconAssetsPort, ReconDirectoryPort, ReconScansPort, ReconTargetsPort,
};
use golish_app_core::ports::vuln::{
    PgVulnIntelAdapter, PgWikiKbAdapter, VulnIntelPort, WikiKbPort,
};

mod attack_execution;
mod convert;
// Kept in-tree as an audit reference until the integrated branch is committed;
// the old module combined Browser, JsApi, Parameter and projection authority.
mod enumeration_lanes;
#[cfg(any())]
mod enumeration_producer;
pub(crate) mod evidence;
pub use evidence::TargetIntelSemanticReceiptStore;
pub mod hypothesis_registry;
pub mod investigation_analysis_host;
pub use investigation_analysis_host::PgInvestigationAnalysisHostRepository;
pub mod investigation_nested_dispatch;
mod investigation_verification_advisory;
pub use investigation_nested_dispatch::PgInvestigationNestedDispatchRepository;
pub mod knowledge_context;
pub mod knowledge_memory;
mod orchestration;
mod recon;
pub use recon::TargetIntelReceiptHost;
pub mod reporting;
pub(crate) mod reporting_gate;
mod runtime_memory;
pub(crate) use runtime_memory::operation_state_view_from_db;
mod tasks;
mod tool_truth;
pub mod tool_truth_revalidation;
mod unified_investigation;
pub mod verification_campaign;
mod verification_campaign_scheduler;
pub mod verification_send_authority;
mod wiki;

pub struct GolishDbRepoProvider {
    pool: Arc<PgPool>,
    // Recon cross-service reads/writes route through the recon service ports
    // (servitization S1-2b) instead of calling `golish_db::repo::<recon>` here.
    recon_scans: Arc<dyn ReconScansPort>,
    recon_assets: Arc<dyn ReconAssetsPort>,
    recon_directory: Arc<dyn ReconDirectoryPort>,
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

#[derive(Debug, Clone, sqlx::FromRow)]
struct EnumerationCoverageGateReceiptRow {
    target_id: Uuid,
    exact_origin: String,
    terminal_disposition: String,
    missing: i64,
    unresolved_count: i64,
    closure_graph_sha256: Option<String>,
    recomputed_closure_graph_sha256: String,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
struct EnumerationCoverageGateCensus {
    frozen_subject_count: i64,
    coverage_receipt_count: i64,
    missing_coverage_receipt_count: i64,
    invalid_coverage_receipt_count: i64,
    closure_graph_drift_count: i64,
    residual_occurrence_count: i64,
}

fn enumeration_coverage_gate_census(
    frozen_members: &[EnumerationFrozenRootMemberView],
    coverage_rows: &[EnumerationCoverageGateReceiptRow],
) -> anyhow::Result<EnumerationCoverageGateCensus> {
    // Enumeration execution is keyed by canonical Web Origin. A domain target
    // and an explicit URL target may legitimately project the same origin; the
    // frozen target ids are aliases of one subject, not two scan obligations.
    let mut frozen_aliases = BTreeMap::<String, BTreeSet<Uuid>>::new();
    for member in frozen_members {
        frozen_aliases
            .entry(member.exact_origin.clone())
            .or_default()
            .insert(member.target_id);
    }
    anyhow::ensure!(
        !frozen_aliases.is_empty(),
        "ENUMERATION_GATE_FROZEN_SUBJECTS_EMPTY"
    );

    let mut coverage_by_subject = BTreeMap::<String, Vec<bool>>::new();
    let mut invalid_coverage_receipt_count = 0_i64;
    let mut closure_graph_drift_count = 0_i64;
    let mut residual_occurrence_count = 0_i64;
    for row in coverage_rows {
        let graph_is_valid = row.closure_graph_sha256.as_deref()
            == Some(row.recomputed_closure_graph_sha256.as_str());
        if !graph_is_valid {
            closure_graph_drift_count += 1;
        }
        let receipt_is_valid = frozen_aliases
            .get(&row.exact_origin)
            .is_some_and(|aliases| aliases.contains(&row.target_id))
            && row.missing == 0
            && matches!(
                row.terminal_disposition.as_str(),
                "found" | "checked_empty" | "terminal_with_residual"
            )
            && graph_is_valid;
        if !receipt_is_valid {
            invalid_coverage_receipt_count += 1;
        }
        residual_occurrence_count = residual_occurrence_count
            .checked_add(row.unresolved_count)
            .ok_or_else(|| anyhow::anyhow!("ENUMERATION_GATE_RESIDUAL_COUNT_OVERFLOW"))?;
        coverage_by_subject
            .entry(row.exact_origin.clone())
            .or_default()
            .push(receipt_is_valid);
    }

    let mut missing_coverage_receipt_count = 0_i64;
    for exact_origin in frozen_aliases.keys() {
        match coverage_by_subject.get(exact_origin) {
            Some(rows) if rows.len() == 1 && rows[0] => {}
            Some(rows) => {
                missing_coverage_receipt_count += 1;
                invalid_coverage_receipt_count += i64::try_from(rows.len().saturating_sub(1))?;
            }
            None => missing_coverage_receipt_count += 1,
        }
    }

    Ok(EnumerationCoverageGateCensus {
        frozen_subject_count: i64::try_from(frozen_aliases.len())?,
        coverage_receipt_count: i64::try_from(coverage_rows.len())?,
        missing_coverage_receipt_count,
        invalid_coverage_receipt_count,
        closure_graph_drift_count,
        residual_occurrence_count,
    })
}

#[derive(sqlx::FromRow)]
struct EnumerationUnresolvedOccurrenceRow {
    occurrence_id: Uuid,
    source_target_id: Uuid,
    exact_origin: String,
    producer_receipt_id: Uuid,
    producer_lane: String,
    producer_execution_authority_id: Uuid,
    producer_receipt_set_sha256: String,
}

#[derive(Debug, sqlx::FromRow)]
struct CandidateCampaignAdmissionSubjectRow {
    scope_snapshot_id: Uuid,
    revision_id: Uuid,
    verification_plan_id: Uuid,
    objective_id: Uuid,
    generation_member_count: i64,
}

impl GolishDbRepoProvider {
    pub fn new(pool: Arc<PgPool>) -> Self {
        let recon_scans: Arc<dyn ReconScansPort> = Arc::new(PgReconScansAdapter::new(pool.clone()));
        let recon_assets: Arc<dyn ReconAssetsPort> =
            Arc::new(PgReconAssetsAdapter::new(pool.clone()));
        let recon_directory: Arc<dyn ReconDirectoryPort> =
            Arc::new(PgReconDirectoryAdapter::new(pool.clone()));
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
            recon_directory,
            recon_targets,
            vuln_intel,
            wiki_kb,
            pentest_plan,
        }
    }
}

fn summarize_scope_review_results(
    results: &[Option<String>],
) -> (bool, Vec<ScopingReviewedTarget>) {
    let parsed = results
        .iter()
        .filter_map(|result| result.as_deref().and_then(parse_scope_review_tool_result))
        .collect::<Vec<_>>();
    let approved = results.len() == 1 && parsed.len() == 1;
    (approved, parsed.into_iter().flatten().collect())
}

#[async_trait]
impl DbRepoProvider for GolishDbRepoProvider {
    fn investigation_nested_dispatch_repository(
        &self,
    ) -> InvestigationNestedDispatchResult<Arc<dyn InvestigationNestedDispatchRepository>> {
        Ok(Arc::new(PgInvestigationNestedDispatchRepository::new(
            self.pool.clone(),
        )))
    }

    fn investigation_analysis_host_repository(
        &self,
    ) -> InvestigationAnalysisHostResult<Arc<dyn InvestigationAnalysisHostRepository>> {
        Ok(Arc::new(PgInvestigationAnalysisHostRepository::new(
            self.pool.clone(),
        )))
    }

    fn unified_investigation_repository(
        &self,
    ) -> UnifiedInvestigationRepoResult<Arc<dyn UnifiedInvestigationRepository>> {
        Ok(Arc::new(
            unified_investigation::PgUnifiedInvestigationRepository::new(self.pool.clone()),
        ))
    }

    fn verification_campaign_repository(
        &self,
    ) -> RepoResult<Arc<dyn VerificationCampaignRepository>> {
        Ok(Arc::new(
            verification_campaign::PgVerificationCampaignRepository::new(self.pool.clone()),
        ))
    }

    fn verification_campaign_shadow_repository(
        &self,
    ) -> RepoResult<Arc<dyn VerificationCampaignShadowRepository>> {
        Ok(Arc::new(
            verification_campaign::PgVerificationCampaignRepository::new(self.pool.clone()),
        ))
    }

    async fn tool_truth_contract(
        &self,
        operation_id: Uuid,
    ) -> anyhow::Result<golish_pentest_domain::tool_truth::ToolTruthContract> {
        golish_db::repo::operation_state::get_tool_truth_contract(&self.pool, operation_id)
            .await?
            .ok_or_else(|| anyhow::anyhow!("TOOL_TRUTH_OPERATION_CONTRACT_MISSING"))
    }

    async fn tool_truth_seal_denominator(
        &self,
        request: SealToolTruthDenominatorRequest,
    ) -> anyhow::Result<ToolTruthDenominatorView> {
        self.tool_truth_seal_denominator_impl(request).await
    }

    async fn tool_truth_finalize_stage_root(
        &self,
        request: FinalizeStageToolTruthRequest,
    ) -> anyhow::Result<StageToolTruthCloseoutView> {
        self.tool_truth_finalize_stage_root_impl(request).await
    }

    async fn enumeration_frozen_root_members(
        &self,
        operation_id: Uuid,
        organization_id: Uuid,
        stage_execution_id: Uuid,
        stage_run_unit_id: Uuid,
    ) -> anyhow::Result<Vec<EnumerationFrozenRootMemberView>> {
        self.enumeration_frozen_root_members_impl(
            operation_id,
            organization_id,
            stage_execution_id,
            stage_run_unit_id,
        )
        .await
    }

    async fn tool_truth_record_shadow_assessment(
        &self,
        request: RecordToolTruthShadowAssessment,
    ) -> anyhow::Result<()> {
        self.tool_truth_record_shadow_assessment_impl(request).await
    }

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
        operation_id: Option<Uuid>,
        target_id: Uuid,
        project_path: &str,
        scan_type: &str,
        tool_name: &str,
        _findings: &serde_json::Value,
        raw_output: Option<&str>,
        severity: &str,
    ) -> anyhow::Result<serde_json::Value> {
        let _ = operation_id;
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
        self.query_target_data_impl(None, target_id, sections).await
    }

    async fn query_target_data_for_operation(
        &self,
        operation_id: Option<Uuid>,
        target_id: Uuid,
        sections: &[String],
    ) -> anyhow::Result<serde_json::Value> {
        self.query_target_data_impl(operation_id, target_id, sections)
            .await
    }

    async fn in_scope_assets(&self, org_id: Option<Uuid>) -> anyhow::Result<Vec<String>> {
        self.in_scope_assets_impl(org_id).await
    }

    async fn in_scope_assets_created_before(
        &self,
        org_id: Option<Uuid>,
        cutoff: chrono::DateTime<chrono::Utc>,
    ) -> anyhow::Result<Vec<String>> {
        self.in_scope_assets_created_before_impl(org_id, cutoff)
            .await
    }

    async fn cleanup_closeout_gate(
        &self,
        operation_id: Uuid,
        organization_id: Uuid,
    ) -> anyhow::Result<golish_agent_kit::db_traits::CleanupCloseoutGateSnapshot> {
        use golish_cleanup_app::CleanupCloseoutPort;
        let gate = golish_cleanup_app::PgCleanupRepository::new(self.pool.as_ref().clone())
            .closeout_counts(operation_id, organization_id)
            .await
            .map_err(anyhow::Error::new)?;
        Ok(golish_agent_kit::db_traits::CleanupCloseoutGateSnapshot {
            missing_obligation_count: gate.missing_obligation_count,
            nonterminal_obligation_count: gate.nonterminal_obligation_count,
            undisclosed_residual_count: gate.undisclosed_residual_count,
            invalid_terminal_truth_count: gate.invalid_terminal_truth_count,
        })
    }

    async fn in_scope_targets(
        &self,
        org_id: Option<Uuid>,
    ) -> anyhow::Result<Vec<serde_json::Value>> {
        self.in_scope_targets_impl(org_id).await
    }

    async fn attack_surface_seeds(
        &self,
        org_id: Option<Uuid>,
        cap: Option<usize>,
    ) -> anyhow::Result<Vec<serde_json::Value>> {
        self.attack_surface_seeds_impl(org_id, cap).await
    }

    async fn stage_asset_coverage(
        &self,
        organization_id: Uuid,
        stage: &str,
        session_id: Option<&str>,
        stage_started_at: Option<chrono::DateTime<chrono::Utc>>,
        current_wave_target_ids: Option<Vec<Uuid>>,
        current_wave_asset_values: Option<Vec<String>>,
    ) -> anyhow::Result<serde_json::Value> {
        self.stage_asset_coverage_impl(
            organization_id,
            stage,
            session_id,
            stage_started_at,
            current_wave_target_ids,
            current_wave_asset_values,
            None,
        )
        .await
    }

    async fn stage_asset_coverage_for_operation(
        &self,
        operation_id: Option<uuid::Uuid>,
        organization_id: uuid::Uuid,
        stage: &str,
        session_id: Option<&str>,
        stage_started_at: Option<chrono::DateTime<chrono::Utc>>,
        current_wave_target_ids: Option<Vec<uuid::Uuid>>,
        current_wave_asset_values: Option<Vec<String>>,
    ) -> anyhow::Result<serde_json::Value> {
        self.stage_asset_coverage_impl(
            organization_id,
            stage,
            session_id,
            stage_started_at,
            current_wave_target_ids,
            current_wave_asset_values,
            operation_id,
        )
        .await
    }

    async fn enumeration_occurrence_gate_snapshot(
        &self,
        operation_id: Uuid,
        organization_id: Uuid,
    ) -> anyhow::Result<Option<EnumerationOccurrenceGateSnapshot>> {
        let contract: Option<String> = sqlx::query_scalar(
            "SELECT enumeration_analysis_contract FROM operation_state WHERE operation_id=$1",
        )
        .bind(operation_id)
        .fetch_optional(self.pool.as_ref())
        .await?;
        let contract = contract
            .ok_or_else(|| anyhow::anyhow!("Enumeration occurrence Gate operation is missing"))?;
        if contract == "legacy_v1" {
            return Ok(None);
        }
        anyhow::ensure!(
            matches!(contract.as_str(), "agent_team_v2_shadow" | "agent_team_v2"),
            "Enumeration occurrence Gate contract is invalid"
        );
        let unit: Option<(Uuid, Uuid)> = sqlx::query_as(
            r#"SELECT id,stage_execution_id
                 FROM stage_run_units
                WHERE operation_id=$1 AND organization_id=$2
                  AND stage_kind='enumeration'
                  AND status IN ('running','gate_blocked')
                ORDER BY generation DESC,updated_at DESC,id DESC
                LIMIT 1"#,
        )
        .bind(operation_id)
        .bind(organization_id)
        .fetch_optional(self.pool.as_ref())
        .await?;
        let (stage_run_unit_id, stage_execution_id) = unit.ok_or_else(|| {
            anyhow::anyhow!("Enumeration occurrence Gate has no active exact Stage Team Unit")
        })?;
        let frozen_members = self
            .enumeration_frozen_root_members_impl(
                operation_id,
                organization_id,
                stage_execution_id,
                stage_run_unit_id,
            )
            .await?;
        let coverage_rows = sqlx::query_as::<_, EnumerationCoverageGateReceiptRow>(
            r#"SELECT receipt.target_id,receipt.exact_origin,
                      receipt.terminal_disposition,receipt.missing,
                      receipt.unresolved_count,seal.closure_graph_sha256,
                      enumeration_compute_lane_closure_graph_sha256(receipt.id)
                          AS recomputed_closure_graph_sha256
                 FROM enumeration_lane_commit_receipts receipt
                 LEFT JOIN enumeration_lane_closure_graph_seals seal
                   ON seal.lane_receipt_id=receipt.id
                WHERE receipt.operation_id=$1 AND receipt.organization_id=$2
                  AND receipt.stage_execution_id=$3
                  AND receipt.stage_run_unit_id=$4
                  AND receipt.lane='coverage'
                ORDER BY receipt.target_id,receipt.exact_origin,receipt.id"#,
        )
        .bind(operation_id)
        .bind(organization_id)
        .bind(stage_execution_id)
        .bind(stage_run_unit_id)
        .fetch_all(self.pool.as_ref())
        .await?;
        let census = enumeration_coverage_gate_census(&frozen_members, &coverage_rows)?;
        Ok(Some(EnumerationOccurrenceGateSnapshot {
            enforces_closeout: contract == "agent_team_v2",
            stage_execution_id,
            stage_run_unit_id,
            frozen_subject_count: census.frozen_subject_count,
            coverage_receipt_count: census.coverage_receipt_count,
            missing_coverage_receipt_count: census.missing_coverage_receipt_count,
            invalid_coverage_receipt_count: census.invalid_coverage_receipt_count,
            closure_graph_drift_count: census.closure_graph_drift_count,
            residual_occurrence_count: census.residual_occurrence_count,
        }))
    }

    async fn enumeration_commit_browser_producer_v2(
        &self,
        request: CommitEnumerationBrowserProducerV2,
    ) -> anyhow::Result<EnumerationLaneClosureReceiptV2> {
        self.enumeration_commit_browser_producer_v2_impl(request)
            .await
            .context("ENUMERATION_BROWSER_REPOSITORY_BOUNDARY")
    }

    async fn enumeration_recover_lane_receipt_v2(
        &self,
        request: RecoverEnumerationLaneReceiptV2,
    ) -> anyhow::Result<Option<EnumerationLaneClosureReceiptV2>> {
        let owner_chain_exists: bool = sqlx::query_scalar(
            r#"SELECT EXISTS(
                   SELECT 1
                     FROM operation_state operation
                     JOIN stage_run_units unit
                       ON unit.operation_id=operation.operation_id
                      AND unit.id=$4
                      AND unit.stage_execution_id=$3
                      AND unit.organization_id=$2
                      AND unit.stage_kind='enumeration'
                     JOIN operation_org_scope_snapshots snapshot
                       ON snapshot.id=unit.scope_snapshot_id
                      AND snapshot.operation_id=unit.operation_id
                      AND snapshot.project_scope_id=operation.project_scope_id
                    WHERE operation.operation_id=$1
                      AND operation.current_stage='enumeration'
                      AND operation.enumeration_analysis_contract IN (
                          'agent_team_v2_shadow','agent_team_v2'
                      )
                      AND unit.status IN ('running','gate_blocked')
                      AND snapshot.sealed_at IS NOT NULL
               )"#,
        )
        .bind(request.operation_id)
        .bind(request.organization_id)
        .bind(request.stage_execution_id)
        .bind(request.stage_run_unit_id)
        .fetch_one(self.pool.as_ref())
        .await?;
        anyhow::ensure!(
            owner_chain_exists,
            "ENUMERATION_RECEIPT_RECOVERY_OWNER_CHAIN_MISMATCH"
        );
        let lane = match request.lane {
            EnumerationLaneKindV2::Browser => "browser",
            EnumerationLaneKindV2::JsApi => "js_api",
            EnumerationLaneKindV2::Parameter => "parameter",
            EnumerationLaneKindV2::Resolution => "resolution",
            EnumerationLaneKindV2::Coverage => "coverage",
        };
        let mut expected_dependencies = request.dependency_receipt_ids.clone();
        expected_dependencies.sort_unstable();
        expected_dependencies.dedup();
        anyhow::ensure!(
            expected_dependencies == request.dependency_receipt_ids,
            "ENUMERATION_RECEIPT_RECOVERY_DEPENDENCY_MANIFEST_INVALID"
        );
        let mut tx = self.pool.begin().await?;
        let recovered = golish_db::repo::enumeration_endpoint_occurrences::recover_enumeration_lane_commit_receipt(
            &mut tx,
            request.operation_id,
            request.organization_id,
            request.stage_execution_id,
            request.stage_run_unit_id,
            request.target_id,
            &request.exact_origin,
            lane,
            request.resolution_occurrence_id,
        )
        .await?;
        let recovered = recovered
            .map(|row| {
                anyhow::ensure!(
                    row.dependency_receipt_ids == expected_dependencies,
                    "ENUMERATION_RECEIPT_RECOVERY_DEPENDENCY_DRIFT"
                );
                enumeration_lanes::lane_receipt_view(row, true)
            })
            .transpose()?;
        tx.commit().await?;
        Ok(recovered)
    }

    async fn enumeration_commit_js_api_producer_v2(
        &self,
        request: CommitEnumerationJsApiProducerV2,
    ) -> anyhow::Result<EnumerationLaneClosureReceiptV2> {
        self.enumeration_commit_js_api_producer_v2_impl(request)
            .await
    }

    async fn enumeration_reduce_parameter_v2(
        &self,
        request: ReduceEnumerationParameterV2,
    ) -> anyhow::Result<EnumerationLaneClosureReceiptV2> {
        self.enumeration_reduce_parameter_v2_impl(request).await
    }

    async fn enumeration_close_resolution_v2(
        &self,
        request: CloseEnumerationResolutionV2,
    ) -> anyhow::Result<EnumerationLaneClosureReceiptV2> {
        self.enumeration_close_resolution_v2_impl(request).await
    }

    async fn enumeration_review_coverage_v2(
        &self,
        request: ReviewEnumerationCoverageV2,
    ) -> anyhow::Result<EnumerationLaneClosureReceiptV2> {
        self.enumeration_review_coverage_v2_impl(request).await
    }

    async fn enumeration_unresolved_occurrences(
        &self,
        operation_id: Uuid,
        organization_id: Uuid,
        stage_execution_id: Uuid,
        stage_run_unit_id: Uuid,
    ) -> anyhow::Result<Vec<EnumerationUnresolvedOccurrenceView>> {
        let owner_chain_exists: bool = sqlx::query_scalar(
            r#"SELECT EXISTS(
                   SELECT 1
                     FROM operation_state operation
                     JOIN stage_run_units unit
                       ON unit.operation_id=operation.operation_id
                     JOIN operation_org_scope_snapshots snapshot
                       ON snapshot.id=unit.scope_snapshot_id
                      AND snapshot.operation_id=unit.operation_id
                      AND snapshot.project_scope_id=operation.project_scope_id
                    WHERE operation.operation_id=$1
                      AND operation.current_stage='enumeration'
                      AND operation.enumeration_analysis_contract IN (
                          'agent_team_v2_shadow','agent_team_v2'
                      )
                      AND unit.id=$4
                      AND unit.stage_execution_id=$3
                      AND unit.organization_id=$2
                      AND unit.stage_kind='enumeration'
                      AND unit.status IN ('running','gate_blocked')
                      AND snapshot.sealed_at IS NOT NULL
                      AND EXISTS (
                          SELECT 1
                            FROM operation_org_scope_units scope_unit
                           WHERE scope_unit.snapshot_id=unit.scope_snapshot_id
                             AND scope_unit.organization_id=$2
                      )
               )"#,
        )
        .bind(operation_id)
        .bind(organization_id)
        .bind(stage_execution_id)
        .bind(stage_run_unit_id)
        .fetch_one(self.pool.as_ref())
        .await?;
        anyhow::ensure!(
            owner_chain_exists,
            "Enumeration unresolved occurrence owner chain is unavailable or mismatched"
        );

        let mut tx = self.pool.begin().await?;
        let rows = sqlx::query_as::<_, EnumerationUnresolvedOccurrenceRow>(
            r#"SELECT occurrence.id AS occurrence_id,
                      occurrence.source_target_id,
                      origin.origin AS exact_origin,
                      producer.id AS producer_receipt_id,
                      producer.lane AS producer_lane,
                      producer.execution_authority_id AS producer_execution_authority_id,
                      producer.receipt_set_sha256 AS producer_receipt_set_sha256
                 FROM operation_state operation
                 JOIN stage_run_units unit
                   ON unit.operation_id=operation.operation_id
                  AND unit.id=$4
                  AND unit.stage_execution_id=$3
                  AND unit.organization_id=$2
                  AND unit.stage_kind='enumeration'
                 JOIN operation_org_scope_snapshots snapshot
                   ON snapshot.id=unit.scope_snapshot_id
                  AND snapshot.operation_id=unit.operation_id
                  AND snapshot.project_scope_id=operation.project_scope_id
                 JOIN enumeration_endpoint_occurrences occurrence
                   ON occurrence.operation_id=operation.operation_id
                  AND occurrence.project_scope_id=operation.project_scope_id
                  AND occurrence.project_path_at_freeze=snapshot.project_path_at_freeze
                  AND occurrence.scope_snapshot_id=unit.scope_snapshot_id
                  AND occurrence.organization_id=unit.organization_id
                  AND occurrence.stage_execution_id=unit.stage_execution_id
                 JOIN enumeration_lane_commit_receipts producer
                   ON producer.execution_authority_id=occurrence.execution_authority_id
                  AND producer.operation_id=occurrence.operation_id
                  AND producer.organization_id=occurrence.organization_id
                  AND producer.stage_execution_id=occurrence.stage_execution_id
                  AND producer.stage_run_unit_id=unit.id
                  AND producer.target_id=occurrence.source_target_id
                  AND producer.lane IN ('browser','js_api')
                 JOIN targets target
                   ON target.id=occurrence.source_target_id
                  AND target.organization_id=occurrence.organization_id
                  AND target.project_path=occurrence.project_path_at_freeze
                 JOIN web_origins origin
                   ON origin.id=occurrence.source_web_origin_id
                  AND origin.organization_id=occurrence.organization_id
                  AND origin.project_path=occurrence.project_path_at_freeze
                WHERE operation.operation_id=$1
                  AND occurrence.candidate_classification='endpoint'
                  AND occurrence.scope_decision='in_scope'
                  AND occurrence.resolution_status IN ('ambiguous','unresolved')
                  AND producer.exact_origin=origin.origin
                ORDER BY origin.origin,occurrence.source_target_id,occurrence.id"#,
        )
        .bind(operation_id)
        .bind(organization_id)
        .bind(stage_execution_id)
        .bind(stage_run_unit_id)
        .fetch_all(&mut *tx)
        .await?;
        let mut unresolved = Vec::with_capacity(rows.len());
        for row in rows {
            let producer_lane = match row.producer_lane.as_str() {
                "browser" => EnumerationLaneKindV2::Browser,
                "js_api" => EnumerationLaneKindV2::JsApi,
                _ => anyhow::bail!("ENUMERATION_UNRESOLVED_PRODUCER_LANE_INVALID"),
            };
            let producer = golish_db::repo::enumeration_endpoint_occurrences::load_enumeration_lane_commit_receipt(
                &mut tx,
                row.producer_receipt_id,
                operation_id,
                organization_id,
                stage_execution_id,
                stage_run_unit_id,
                row.source_target_id,
                &row.exact_origin,
                &row.producer_lane,
                row.producer_execution_authority_id,
                &row.producer_receipt_set_sha256,
            )
            .await?;
            unresolved.push(EnumerationUnresolvedOccurrenceView {
                occurrence_id: row.occurrence_id,
                source_target_id: row.source_target_id,
                exact_origin: row.exact_origin,
                producer_receipt: enumeration_lanes::lane_receipt_view(producer, false)?,
            });
            anyhow::ensure!(
                unresolved
                    .last()
                    .is_some_and(|entry| entry.producer_receipt.lane == producer_lane),
                "ENUMERATION_UNRESOLVED_PRODUCER_RECEIPT_DRIFT"
            );
        }
        tx.commit().await?;
        Ok(unresolved)
    }

    async fn admit_candidate_generation_campaigns(
        &self,
        stable_request_id: Uuid,
        operation_id: Uuid,
        organization_id: Uuid,
        generation_seal_id: Uuid,
    ) -> anyhow::Result<CandidateCampaignAdmissionBatchView> {
        let subjects = sqlx::query_as::<_, CandidateCampaignAdmissionSubjectRow>(
            r#"SELECT scope.id AS scope_snapshot_id,
                      revision.revision_id,
                      plan.plan_id AS verification_plan_id,
                      objective.objective_id,
                      seal.member_count AS generation_member_count
                 FROM operation_state operation
                 JOIN hypothesis_generation_seals seal
                   ON seal.seal_id=$3
                 JOIN hypothesis_generations generation
                   ON generation.generation_id=seal.generation_id
                  AND generation.operation_id=operation.operation_id
                  AND generation.organization_id=$2
                 JOIN candidate_analysis_snapshots candidate_snapshot
                   ON candidate_snapshot.snapshot_id=generation.candidate_snapshot_id
                  AND candidate_snapshot.operation_id=generation.operation_id
                  AND candidate_snapshot.organization_id=generation.organization_id
                  AND candidate_snapshot.snapshot_status IN (
                      'sealed_ready','sealed_analysis_ready_with_residuals'
                  )
                 JOIN operation_org_scope_snapshots scope
                   ON scope.id=candidate_snapshot.scope_snapshot_id
                  AND scope.operation_id=operation.operation_id
                  AND scope.project_scope_id=operation.project_scope_id
                  AND scope.sealed_at IS NOT NULL
                 JOIN operation_org_scope_units scope_unit
                   ON scope_unit.snapshot_id=scope.id
                  AND scope_unit.organization_id=generation.organization_id
                 JOIN hypothesis_generation_members generation_member
                   ON generation_member.generation_id=generation.generation_id
                  AND generation_member.operation_id=generation.operation_id
                  AND generation_member.organization_id=generation.organization_id
                 JOIN attack_hypothesis_revisions revision
                   ON revision.revision_id=generation_member.revision_id
                  AND revision.operation_id=generation.operation_id
                  AND revision.organization_id=generation.organization_id
                 JOIN attack_hypothesis_verification_plans plan
                   ON plan.revision_id=revision.revision_id
                  AND plan.sealed_at IS NOT NULL
                 JOIN attack_hypothesis_verification_plan_objectives objective
                   ON objective.plan_id=plan.plan_id
                  AND objective.revision_id=revision.revision_id
                WHERE operation.operation_id=$1
                  AND operation.tool_truth_contract='receipt_v1'
                  AND operation.investigation_rollout_mode IN (
                      'registry_authoritative_legacy_projection','new_only'
                  )
                  AND (
                      (
                          operation.stage_topology_contract='legacy_candidate_verification_v1'
                          AND operation.current_stage='attack_candidate'
                      ) OR (
                          operation.stage_topology_contract='unified_investigation_v1'
                          AND operation.current_stage='investigation'
                      )
                  )
                ORDER BY generation_member.ordinal,objective.ordinal,
                         revision.revision_id,objective.objective_id"#,
        )
        .bind(stable_request_id)
        .bind(operation_id)
        .bind(organization_id)
        .bind(generation_seal_id)
        .fetch_all(self.pool.as_ref())
        .await?;
        anyhow::ensure!(
            !subjects.is_empty(),
            "Candidate generation has no complete authoritative Plan C admission subjects"
        );
        let expected_member_count = subjects[0].generation_member_count;
        anyhow::ensure!(
            expected_member_count > 0
                && subjects
                    .iter()
                    .all(
                        |subject| subject.generation_member_count == expected_member_count
                            && subject.scope_snapshot_id == subjects[0].scope_snapshot_id
                    ),
            "Candidate generation Campaign admission owner census drifted"
        );
        let revision_ids = subjects
            .iter()
            .map(|subject| subject.revision_id)
            .collect::<std::collections::BTreeSet<_>>();
        anyhow::ensure!(
            i64::try_from(revision_ids.len()).ok() == Some(expected_member_count),
            "Candidate generation Campaign admission omits a sealed revision or plan"
        );
        let objective_ids = subjects
            .iter()
            .map(|subject| subject.objective_id)
            .collect::<std::collections::BTreeSet<_>>();
        anyhow::ensure!(
            objective_ids.len() == subjects.len(),
            "Candidate generation Campaign admission contains duplicate objectives"
        );

        let campaign_repository =
            verification_campaign::PgVerificationCampaignRepository::new(self.pool.clone());
        let capability_registry =
            golish_pentest_app::pentest_bridge::VerificationCapabilityRegistry::authoritative_v1();
        let capability_ids = capability_registry
            .capability_ids()
            .map(str::to_owned)
            .collect::<Vec<_>>();
        anyhow::ensure!(
            capability_ids.len() == 4,
            "Plan C authoritative capability registry is not the frozen four-member census"
        );
        let mut assessment_seals = std::collections::BTreeMap::new();
        for subject in &subjects {
            for capability_id in &capability_ids {
                let assessment = capability_registry
                    .assessment(capability_id)
                    .ok_or_else(|| anyhow::anyhow!("Plan C capability registry drifted"))?;
                campaign_repository
                    .record_capability_assessment(RecordCapabilityAssessment {
                        stable_request_id: Uuid::new_v5(
                            &stable_request_id,
                            format!(
                                "candidate-campaign-capability.v1:{}:{capability_id}",
                                subject.objective_id
                            )
                            .as_bytes(),
                        ),
                        operation_id,
                        scope_snapshot_id: subject.scope_snapshot_id,
                        organization_id,
                        // The assessment writer derives generation authority
                        // directly. The Wave seal is necessarily created only
                        // after this exact four-member assessment set exists.
                        wave_coverage_seal_id: Uuid::nil(),
                        objective_id: subject.objective_id,
                        capability_id: capability_id.clone(),
                        disposition: verification_campaign::compiler_disposition_to_repository(
                            assessment.disposition,
                        ),
                        adapter_contract_version: assessment.adapter_contract_version.clone(),
                        adapter_contract_digest: assessment.adapter_contract_digest.clone(),
                        residual_reason_code:
                            (verification_campaign::compiler_disposition_to_repository(
                                assessment.disposition,
                            ) != CapabilityAssessmentDispositionV1::Available)
                                .then(|| assessment.reason_code.clone()),
                    })
                    .await
                    .map_err(anyhow::Error::new)?;
            }
            let sealed = campaign_repository
                .seal_capability_assessment_set(SealCapabilityAssessmentSet {
                    stable_request_id: Uuid::new_v5(
                        &stable_request_id,
                        format!(
                            "candidate-campaign-capability-set.v1:{}",
                            subject.objective_id
                        )
                        .as_bytes(),
                    ),
                    operation_id,
                    scope_snapshot_id: subject.scope_snapshot_id,
                    organization_id,
                    wave_coverage_seal_id: Uuid::nil(),
                    objective_id: subject.objective_id,
                })
                .await
                .map_err(anyhow::Error::new)?;
            assessment_seals.insert(subject.objective_id, sealed.seal_id);
        }

        let wave = campaign_repository
            .seal_wave_coverage_denominator(SealWaveCoverage {
                stable_request_id: Uuid::new_v5(
                    &stable_request_id,
                    b"candidate-campaign-wave-coverage.v1",
                ),
                operation_id,
                scope_snapshot_id: subjects[0].scope_snapshot_id,
                organization_id,
                generation_seal_id,
                verification_plan_id: subjects[0].verification_plan_id,
            })
            .await
            .map_err(anyhow::Error::new)?;
        anyhow::ensure!(
            wave.member_count > 0,
            "Candidate generation Wave denominator is unexpectedly empty"
        );

        let mut campaign_ids = Vec::with_capacity(subjects.len());
        let mut replayed_campaign_count = 0_u32;
        for subject in &subjects {
            let campaign = campaign_repository
                .admit_campaign_with_fresh_tool_truth(AdmitCampaignRequest {
                    stable_consumer_request_id: Uuid::new_v5(
                        &stable_request_id,
                        format!("candidate-campaign-admission.v1:{}", subject.objective_id)
                            .as_bytes(),
                    ),
                    operation_id,
                    scope_snapshot_id: subject.scope_snapshot_id,
                    organization_id,
                    generation_seal_id,
                    verification_plan_id: subject.verification_plan_id,
                    objective_id: subject.objective_id,
                    wave_coverage_seal_id: wave.seal_id,
                    capability_assessment_set_seal_id: assessment_seals[&subject.objective_id],
                    expected_campaign_id: None,
                })
                .await
                .map_err(anyhow::Error::new)?;
            anyhow::ensure!(
                campaign.operation_id == operation_id
                    && campaign.objective_id == subject.objective_id,
                "Campaign admission returned mismatched authority"
            );
            replayed_campaign_count =
                replayed_campaign_count.saturating_add(u32::from(campaign.replayed));
            campaign_ids.push(campaign.campaign_id);
        }
        Ok(CandidateCampaignAdmissionBatchView {
            generation_seal_id,
            objective_count: u32::try_from(subjects.len())?,
            campaign_ids,
            replayed_campaign_count,
        })
    }

    async fn drive_authoritative_verification_campaigns(
        &self,
        operation_id: Uuid,
    ) -> anyhow::Result<VerificationCampaignSchedulerView> {
        verification_campaign_scheduler::drive_authoritative_verification_campaigns(
            self.pool.clone(),
            operation_id,
        )
        .await
    }

    async fn prepare_authoritative_verification_consults(
        &self,
        operation_id: Uuid,
    ) -> anyhow::Result<Vec<VerificationConsultWorkItemView>> {
        verification_campaign_scheduler::prepare_authoritative_verification_consults(
            self.pool.clone(),
            operation_id,
        )
        .await
    }

    async fn record_authoritative_verification_consult_terminal(
        &self,
        command: RecordVerificationConsultTerminal,
    ) -> anyhow::Result<()> {
        verification_campaign_scheduler::record_authoritative_verification_consult_terminal(
            self.pool.clone(),
            command,
        )
        .await
    }

    async fn in_scope_target_types(&self, org_id: Option<Uuid>) -> anyhow::Result<Vec<String>> {
        self.in_scope_target_types_impl(org_id).await
    }

    async fn in_scope_typed_assets(
        &self,
        org_id: Option<Uuid>,
    ) -> anyhow::Result<Vec<(String, String)>> {
        self.in_scope_typed_assets_impl(org_id).await
    }

    async fn scoping_target_snapshot(
        &self,
        organization_id: Uuid,
    ) -> anyhow::Result<Vec<ScopingReviewedTarget>> {
        self.scoping_target_snapshot_impl(organization_id).await
    }

    async fn active_recon_scope_review_candidates(
        &self,
        operation_id: Uuid,
        organization_id: Uuid,
    ) -> anyhow::Result<Vec<ScopingReviewedTarget>> {
        self.active_recon_scope_review_candidates_impl(operation_id, organization_id)
            .await
    }

    async fn active_recon_scope_review_apply(
        &self,
        operation_id: Uuid,
        organization_id: Uuid,
        approval: ActiveReconScopeReviewApproval,
    ) -> anyhow::Result<Vec<ScopingReviewedTarget>> {
        self.active_recon_scope_review_apply_impl(operation_id, organization_id, approval)
            .await
    }

    async fn active_recon_scope_review_authorized(
        &self,
        operation_id: Uuid,
        organization_id: Uuid,
    ) -> anyhow::Result<bool> {
        self.active_recon_scope_review_authorized_impl(operation_id, organization_id)
            .await
    }

    async fn attack_v2_seed_candidate_manifest(
        &self,
        input: golish_agent_kit::harness::attack_execution::SeedCandidateManifest,
    ) -> anyhow::Result<golish_agent_kit::harness::attack_execution::CandidateManifestSnapshot>
    {
        self.attack_v2_seed_candidate_manifest_impl(input).await
    }

    async fn attack_v2_candidate_manifest_for_unit(
        &self,
        operation_id: Uuid,
        stage_run_unit_id: Uuid,
        organization_id: Uuid,
    ) -> anyhow::Result<golish_agent_kit::harness::attack_execution::CandidateManifestSnapshot>
    {
        self.attack_v2_candidate_manifest_for_unit_impl(
            operation_id,
            stage_run_unit_id,
            organization_id,
        )
        .await
    }

    async fn attack_v2_seed_candidate_manifest_for_unit(
        &self,
        operation_id: Uuid,
        stage_run_unit_id: Uuid,
        organization_id: Uuid,
    ) -> anyhow::Result<golish_agent_kit::harness::attack_execution::CandidateManifestSnapshot>
    {
        self.attack_v2_seed_candidate_manifest_for_unit_impl(
            operation_id,
            stage_run_unit_id,
            organization_id,
        )
        .await
    }

    async fn attack_v2_review_barrier_for_operation(
        &self,
        operation_id: Uuid,
    ) -> anyhow::Result<golish_agent_kit::db_traits::AttackV2ReviewBarrierView> {
        self.attack_v2_review_barrier_for_operation_impl(operation_id)
            .await
    }

    async fn attack_v2_verification_truth_for_operation(
        &self,
        operation_id: Uuid,
        organization_id: Option<Uuid>,
    ) -> anyhow::Result<Option<golish_agent_kit::harness::attack_execution::VerificationTruthSet>>
    {
        self.attack_v2_verification_truth_for_operation_impl(operation_id, organization_id)
            .await
    }

    async fn attack_v2_consolidate_wave(
        &self,
        input: golish_agent_kit::db_traits::AttackV2ConsolidateWave,
    ) -> anyhow::Result<golish_agent_kit::db_traits::AttackV2WaveConsolidationView> {
        self.attack_v2_consolidate_wave_impl(input).await
    }

    async fn reporting_build_validated_revision(
        &self,
        operation_id: Uuid,
    ) -> anyhow::Result<golish_agent_kit::harness::ReportingGateTruth> {
        reporting_gate::build_or_reuse_validated_report(self.pool.clone(), operation_id).await
    }

    async fn reporting_gate_truth(
        &self,
        operation_id: Uuid,
    ) -> anyhow::Result<Option<golish_agent_kit::harness::ReportingGateTruth>> {
        reporting_gate::load_reporting_gate_truth(&self.pool, operation_id).await
    }

    async fn eas_port_delegated_assets(&self, org_id: Option<Uuid>) -> anyhow::Result<Vec<String>> {
        self.eas_port_delegated_assets_impl(org_id).await
    }

    async fn enumeration_web_capable_assets(
        &self,
        org_id: Option<Uuid>,
    ) -> anyhow::Result<Vec<String>> {
        self.enumeration_web_capable_assets_impl(org_id).await
    }

    async fn eas_web_capable_assets(
        &self,
        org_id: Option<Uuid>,
        run_start: Option<chrono::DateTime<chrono::Utc>>,
    ) -> anyhow::Result<Vec<String>> {
        self.eas_web_capable_assets_impl(org_id, run_start).await
    }

    async fn eas_required_web_origins(
        &self,
        organization_id: Uuid,
        since: chrono::DateTime<chrono::Utc>,
        current_wave_target_ids: Option<Vec<Uuid>>,
    ) -> anyhow::Result<Vec<String>> {
        golish_db::repo::surface_identity_queries::list_eas_required_web_origins(
            &self.pool,
            organization_id,
            since,
            current_wave_target_ids.as_deref(),
        )
        .await
        .map_err(Into::into)
    }

    async fn eas_service_not_applicable_assets(
        &self,
        org_id: Option<Uuid>,
        run_start: Option<chrono::DateTime<chrono::Utc>>,
    ) -> anyhow::Result<Vec<String>> {
        self.eas_service_not_applicable_assets_impl(org_id, run_start)
            .await
    }

    async fn dead_asset_values(&self, org_id: Option<Uuid>) -> anyhow::Result<Vec<String>> {
        self.dead_asset_values_impl(org_id).await
    }

    async fn db_truth_facts(
        &self,
        org_id: Option<Uuid>,
        in_scope_assets: &[String],
        run_start: Option<chrono::DateTime<chrono::Utc>>,
    ) -> anyhow::Result<Vec<(String, String)>> {
        self.db_truth_facts_impl(org_id, in_scope_assets, run_start)
            .await
    }

    async fn in_scope_org_ids(&self, project_path: Option<&str>) -> anyhow::Result<Vec<Uuid>> {
        self.in_scope_org_ids_impl(project_path).await
    }

    async fn org_subtree_ids(&self, root_id: Uuid) -> anyhow::Result<Vec<Uuid>> {
        self.org_subtree_ids_impl(root_id).await
    }

    async fn org_subtree_units(&self, root_id: Uuid) -> anyhow::Result<Vec<OrgScopeUnit>> {
        self.org_subtree_units_impl(root_id).await
    }

    async fn org_stage_completions_get(
        &self,
        stage_kind: &str,
        org_ids: &[Uuid],
    ) -> anyhow::Result<Vec<(Uuid, chrono::DateTime<chrono::Utc>)>> {
        self.org_stage_completions_get_impl(stage_kind, org_ids)
            .await
    }

    async fn org_stage_completions_get_with_run_id(
        &self,
        stage_kind: &str,
        org_ids: &[Uuid],
    ) -> anyhow::Result<Vec<(Uuid, chrono::DateTime<chrono::Utc>, Option<String>)>> {
        self.org_stage_completions_get_with_run_id_impl(stage_kind, org_ids)
            .await
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
        runtime_memory_contract: RuntimeMemoryContract,
    ) -> anyhow::Result<()> {
        self.operation_state_insert_impl(
            operation_id,
            profile,
            current_stage,
            runtime_memory_contract,
        )
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

    async fn operation_state_set_engagement_org(
        &self,
        operation_id: Uuid,
        org_id: Option<Uuid>,
    ) -> anyhow::Result<()> {
        self.operation_state_set_engagement_org_impl(operation_id, org_id)
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

    async fn stage_asset_wave_current_or_create_initial(
        &self,
        stage_execution_id: Uuid,
        operation_id: Uuid,
        organization_id: Uuid,
        stage_kind: &str,
        started_at: chrono::DateTime<chrono::Utc>,
        limit: i64,
    ) -> anyhow::Result<Option<StageAssetWaveView>> {
        self.stage_asset_wave_current_or_create_initial_impl(
            stage_execution_id,
            operation_id,
            organization_id,
            stage_kind,
            started_at,
            limit,
        )
        .await
    }

    async fn stage_asset_wave_create_next(
        &self,
        operation_id: Uuid,
        organization_id: Uuid,
        stage_kind: &str,
        parent_wave_id: Option<Uuid>,
        limit: i64,
    ) -> anyhow::Result<Option<StageAssetWaveView>> {
        self.stage_asset_wave_create_next_impl(
            operation_id,
            organization_id,
            stage_kind,
            parent_wave_id,
            limit,
        )
        .await
    }

    async fn stage_asset_wave_create_next_or_seal_completion(
        &self,
        operation_id: Uuid,
        organization_id: Uuid,
        stage_kind: &str,
        parent_wave_id: Option<Uuid>,
        limit: i64,
        stage_run_id: Option<&str>,
    ) -> anyhow::Result<Option<StageAssetWaveView>> {
        self.stage_asset_wave_create_next_or_seal_completion_impl(
            operation_id,
            organization_id,
            stage_kind,
            parent_wave_id,
            limit,
            stage_run_id,
        )
        .await
    }

    async fn stage_asset_wave_current_running(
        &self,
        operation_id: Uuid,
        organization_id: Uuid,
        stage_kind: &str,
    ) -> anyhow::Result<Option<StageAssetWaveView>> {
        self.stage_asset_wave_current_running_impl(operation_id, organization_id, stage_kind)
            .await
    }

    async fn stage_asset_wave_current_running_for_dispatch(
        &self,
        stage_execution_id: Uuid,
        operation_id: Uuid,
        organization_id: Uuid,
        stage_kind: &str,
    ) -> anyhow::Result<Option<StageAssetWaveView>> {
        self.stage_asset_wave_current_running_for_dispatch_impl(
            stage_execution_id,
            operation_id,
            organization_id,
            stage_kind,
        )
        .await
    }

    async fn stage_asset_wave_complete(&self, wave_id: Uuid) -> anyhow::Result<()> {
        self.stage_asset_wave_complete_impl(wave_id).await
    }

    async fn stage_asset_wave_all_items_created_at_or_before(
        &self,
        wave_id: Uuid,
        cutoff: chrono::DateTime<chrono::Utc>,
    ) -> anyhow::Result<bool> {
        self.stage_asset_wave_all_items_created_at_or_before_impl(wave_id, cutoff)
            .await
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

    async fn scoping_passive_recon_organization_authorized(
        &self,
        operation_id: Uuid,
        stage_execution_id: Uuid,
        organization_id: Uuid,
    ) -> anyhow::Result<bool> {
        let authorized =
            golish_db::repo::operation_scope_decisions::scoping_passive_recon_organization_authorized(
                &self.pool,
                operation_id,
                stage_execution_id,
                organization_id,
            )
            .await?;
        Ok(authorized)
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
        self.evidence_append_for_organization_impl(
            operation_id,
            organization_id,
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

    async fn semantic_intel_receipt_append(
        &self,
        operation_id: Uuid,
        organization_id: Uuid,
        session_id: Uuid,
        project_path: Option<&str>,
        receipt: &serde_json::Value,
    ) -> anyhow::Result<i64> {
        self.semantic_intel_receipt_append_impl(
            operation_id,
            organization_id,
            session_id,
            project_path,
            receipt,
        )
        .await
    }

    async fn semantic_intel_terminal_receipt(
        &self,
        operation_id: Uuid,
        organization_id: Uuid,
        session_id: Uuid,
        stable_query_key: &str,
    ) -> anyhow::Result<Option<serde_json::Value>> {
        self.semantic_intel_terminal_receipt_impl(
            operation_id,
            organization_id,
            session_id,
            stable_query_key,
        )
        .await
    }

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
        self.upsert_technique_outcome_impl(
            organization_id,
            run_id,
            asset,
            technique,
            outcome,
            source,
            query,
            evidence_ids,
        )
        .await
    }

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
        self.upsert_terminal_technique_outcome_if_unfinished_impl(
            organization_id,
            run_id,
            asset,
            technique,
            outcome,
            source,
            query,
            evidence_ids,
        )
        .await
    }

    async fn mark_target_intel_dns_empty_outcomes(
        &self,
        organization_id: Uuid,
        run_id: &str,
        evidence_ids: &[i64],
    ) -> anyhow::Result<usize> {
        self.mark_target_intel_dns_empty_outcomes_impl(organization_id, run_id, evidence_ids)
            .await
    }

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
        self.upsert_source_query_impl(
            organization_id,
            run_id,
            source,
            query,
            target,
            technique,
            status,
            evidence_ids,
        )
        .await
    }

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
        self.enqueue_expansion_lead_impl(
            organization_id,
            run_id,
            lead_type,
            lead_value,
            source,
            confidence,
            evidence_ids,
        )
        .await
    }

    async fn evidence_facts_for_session(
        &self,
        session_id: &str,
    ) -> anyhow::Result<Vec<(String, String, String, i64)>> {
        self.evidence_facts_for_session_impl(session_id).await
    }

    async fn eas_evidence_facts_for_session_org_fresh(
        &self,
        session_id: &str,
        organization_id: Uuid,
        since: chrono::DateTime<chrono::Utc>,
    ) -> anyhow::Result<Vec<(String, String, String, i64)>> {
        self.eas_evidence_facts_for_session_org_fresh_impl(session_id, organization_id, since)
            .await
    }

    async fn technique_outcome_facts(
        &self,
        organization_id: Uuid,
        run_id: &str,
    ) -> Vec<golish_agent_kit::db_traits::TechniqueOutcomeFact> {
        self.technique_outcome_facts_impl(organization_id, run_id)
            .await
    }

    async fn technique_outcome_facts_fresh(
        &self,
        organization_id: Uuid,
        run_id: &str,
        since: Option<chrono::DateTime<chrono::Utc>>,
    ) -> Vec<golish_agent_kit::db_traits::TechniqueOutcomeFact> {
        // 护栏 4 (2026-07-02-gate-capability-ledger Phase 1)：stage 关闭 gate 投影套
        // freshness cutoff（execute.rs 传入 run_start），避免同 session 旧 stage-run
        // 的 technique_outcomes 泄漏进本 stage-run 的 coverage 判定。
        self.technique_outcome_facts_fresh_impl(organization_id, run_id, since)
            .await
    }

    async fn technique_outcome_facts_fresh_with_evidence_session(
        &self,
        organization_id: Uuid,
        outcome_run_id: &str,
        evidence_session_id: &str,
        since: Option<chrono::DateTime<chrono::Utc>>,
    ) -> Vec<golish_agent_kit::db_traits::TechniqueOutcomeFact> {
        self.technique_outcome_facts_fresh_with_evidence_session_impl(
            organization_id,
            outcome_run_id,
            evidence_session_id,
            since,
        )
        .await
    }

    async fn final_seal_technique_outcome_facts(
        &self,
        organization_id: Uuid,
        run_id: &str,
    ) -> anyhow::Result<Vec<golish_agent_kit::db_traits::TechniqueOutcomeFact>> {
        self.final_seal_technique_outcome_facts_impl(organization_id, run_id)
            .await
    }

    async fn source_query_facts(
        &self,
        organization_id: Uuid,
        run_id: &str,
    ) -> anyhow::Result<Vec<golish_agent_kit::harness::SourceQueryFact>> {
        self.source_query_facts_impl(organization_id, run_id).await
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

    async fn recent_evidence_ids_for_stage_attempt(
        &self,
        session_id: &str,
        stage_execution_id: Uuid,
        limit: i64,
    ) -> anyhow::Result<Vec<i64>> {
        self.recent_evidence_ids_for_stage_attempt_impl(session_id, stage_execution_id, limit)
            .await
    }

    async fn recent_evidence_detailed(
        &self,
        session_id: &str,
        limit: i64,
    ) -> anyhow::Result<Vec<serde_json::Value>> {
        self.recent_evidence_detailed_impl(session_id, limit).await
    }

    async fn recent_evidence_detailed_for_worker(
        &self,
        operation_id: uuid::Uuid,
        worker_run_id: uuid::Uuid,
        limit: i64,
    ) -> anyhow::Result<Vec<serde_json::Value>> {
        self.recent_evidence_detailed_for_worker_impl(operation_id, worker_run_id, limit)
            .await
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
        organization_id: Uuid,
        not_before: chrono::DateTime<chrono::Utc>,
    ) -> anyhow::Result<Option<ScopingActionsSeen>> {
        let (
            total,
            unit_candidates_proposed,
            unit_review_invoked,
            subsidiaries_excluded,
            organization_created,
            scope_review_results,
        ) = golish_db::repo::tool_calls::scoping_actions_for_session(
            &self.pool,
            session_id,
            organization_id,
            not_before,
        )
        .await?;
        // No recorded tool calls in this operation's Scoping window means the
        // persisted review lifecycle is unavailable. The interactive gate treats
        // `None` as incomplete rather than borrowing an older session approval.
        if total == 0 {
            return Ok(None);
        }
        let scope_review_attempts = scope_review_results.len();
        let (scope_review_approved, scope_review_targets) =
            summarize_scope_review_results(&scope_review_results);
        Ok(Some(ScopingActionsSeen {
            unit_candidates_proposed,
            unit_review_invoked,
            subsidiaries_excluded,
            organization_created,
            scope_review_approved,
            scope_review_attempts,
            scope_review_targets,
        }))
    }
}

#[cfg(test)]
mod scoping_review_summary_tests {
    use super::summarize_scope_review_results;

    fn review(value: &str) -> Option<String> {
        Some(
            serde_json::json!({
                "response": serde_json::json!([{
                    "value": value,
                    "type": "domain",
                    "scope": "in"
                }])
                .to_string(),
                "skipped": false
            })
            .to_string(),
        )
    }

    #[test]
    fn repeated_review_keeps_all_rows_but_is_never_approved() {
        let (approved, rows) =
            summarize_scope_review_results(&[review("edited.example"), review("trusted.example")]);
        assert!(!approved);
        assert_eq!(
            rows.iter()
                .map(|row| row.value.as_str())
                .collect::<Vec<_>>(),
            vec!["edited.example", "trusted.example"]
        );
    }
}

#[cfg(test)]
mod enumeration_coverage_gate_census_tests {
    use super::*;

    fn frozen_member(target_id: Uuid, technique: &str) -> EnumerationFrozenRootMemberView {
        EnumerationFrozenRootMemberView {
            target_id,
            exact_origin: "https://moresec.cn:443".to_string(),
            technique: technique.to_string(),
            expected_capability: "enum.test".to_string(),
        }
    }

    fn coverage_row(target_id: Uuid) -> EnumerationCoverageGateReceiptRow {
        EnumerationCoverageGateReceiptRow {
            target_id,
            exact_origin: "https://moresec.cn:443".to_string(),
            terminal_disposition: "found".to_string(),
            missing: 0,
            unresolved_count: 0,
            closure_graph_sha256: Some("sha256:graph".to_string()),
            recomputed_closure_graph_sha256: "sha256:graph".to_string(),
        }
    }

    #[test]
    fn domain_and_url_target_ids_are_one_exact_origin_subject() {
        let domain_target_id = Uuid::new_v4();
        let url_target_id = Uuid::new_v4();
        let techniques = [
            "GOLISH-ENUM-DIR",
            "GOLISH-ENUM-JS",
            "GOLISH-ENUM-JSAPI",
            "GOLISH-ENUM-PARAM",
        ];
        let frozen = [domain_target_id, url_target_id]
            .into_iter()
            .flat_map(|target_id| {
                techniques
                    .into_iter()
                    .map(move |technique| frozen_member(target_id, technique))
            })
            .collect::<Vec<_>>();

        let census = enumeration_coverage_gate_census(&frozen, &[coverage_row(url_target_id)])
            .expect("target aliases should share one exact-origin closure");

        assert_eq!(census.frozen_subject_count, 1);
        assert_eq!(census.coverage_receipt_count, 1);
        assert_eq!(census.missing_coverage_receipt_count, 0);
        assert_eq!(census.invalid_coverage_receipt_count, 0);
    }

    #[test]
    fn coverage_receipt_from_a_foreign_target_alias_fails_closed() {
        let frozen_target_id = Uuid::new_v4();
        let frozen = [frozen_member(frozen_target_id, "GOLISH-ENUM-DIR")];

        let census = enumeration_coverage_gate_census(&frozen, &[coverage_row(Uuid::new_v4())])
            .expect("foreign identity is represented as an invalid census row");

        assert_eq!(census.frozen_subject_count, 1);
        assert_eq!(census.missing_coverage_receipt_count, 1);
        assert_eq!(census.invalid_coverage_receipt_count, 1);
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
    use golish_agent_kit::runtime_memory::RuntimeMemoryContract;
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

        repo.operation_state_insert(
            op,
            "assessment",
            StageKind::ExternalAttackSurface.as_str(),
            RuntimeMemoryContract::V2Only,
        )
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

        repo.operation_state_insert(
            op,
            "assessment",
            StageKind::ExternalAttackSurface.as_str(),
            RuntimeMemoryContract::V2Only,
        )
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
