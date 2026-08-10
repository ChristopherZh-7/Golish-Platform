//! `ReconScansPort` — recon scan-derived findings as a service port.
//!
//! Covers the `api_endpoints` / `js_analysis` / `fingerprints` /
//! `passive_scans` recon tables. The in-proc adapter mirrors
//! `golish_db::repo::{api_endpoints,js_analysis,fingerprints,passive_scans}`
//! exactly (same SQL, same args) — it is the ONLY place the agent / pentest /
//! platform services are allowed to reach these recon repos. It lives under the
//! recon port domain so the ownership guard treats it as recon-owned.
//!
//! Remote-ready: all params + returns are serializable (`golish_db::models::*`
//! derive Serde); no `PgPool` / closures cross the boundary.

// Port methods mirror the wide `golish_db::repo` insert signatures verbatim;
// the arity is inherited from the SQL columns, not a refactor smell.
#![allow(clippy::too_many_arguments)]

use std::sync::Arc;

use async_trait::async_trait;
use sqlx::PgPool;
use uuid::Uuid;

use golish_db::models::{ApiEndpoint, Fingerprint, JsAnalysisResult, PassiveScanLog};
use golish_db::repo::capability_execution_receipts::{
    CapabilityReceiptInputRef, EvidenceAuthorityRef, ToolTruthExecutionAuthorityRef,
};
use golish_db::repo::enumeration_endpoint_occurrences::{
    CandidateDescriptorWrite, EndpointGroupProjectionSummary, EndpointOccurrenceWrite,
    JsAnalysisDescriptorWrite, ParameterAssessmentWrite, PersistedEndpointOccurrence,
};
use golish_db::repo::scoped::TargetWriteGuard;
use serde::{Deserialize, Serialize};

/// Project-wide passive-scan log projection returned by
/// [`ReconScansPort::passive_scans_list_global_by_project`]. Mirrors the exact
/// column subset the global query selects — the port's own remote-ready DTO, so
/// the consuming platform service no longer owns this projection type.
#[derive(Debug, Clone, Serialize, Deserialize, sqlx::FromRow)]
#[serde(rename_all = "camelCase")]
pub struct ReconPassiveScanGlobal {
    pub id: Uuid,
    pub target_id: Uuid,
    pub test_type: String,
    pub payload: String,
    pub url: String,
    pub result: String,
    pub severity: String,
    pub tool_used: String,
    pub tested_at: chrono::DateTime<chrono::Utc>,
}

/// Outbound port for recon scan-derived findings (read + write).
#[async_trait]
pub trait ReconScansPort: Send + Sync {
    async fn enumeration_persist_js_analysis_descriptor(
        &self,
        authority: &ToolTruthExecutionAuthorityRef,
        input: &CapabilityReceiptInputRef,
        draft: &JsAnalysisDescriptorWrite,
    ) -> anyhow::Result<Uuid>;

    async fn enumeration_bind_js_analysis_terminal_receipt(
        &self,
        authority: &ToolTruthExecutionAuthorityRef,
        descriptor_id: Uuid,
        input: &CapabilityReceiptInputRef,
    ) -> anyhow::Result<()>;

    async fn enumeration_persist_candidate_descriptor(
        &self,
        authority: &ToolTruthExecutionAuthorityRef,
        input: &CapabilityReceiptInputRef,
        draft: &CandidateDescriptorWrite,
    ) -> anyhow::Result<Uuid>;

    /// Persist one immutable V2 occurrence through the closed Tool Truth
    /// authority tuple. The adapter owns the SQL transaction; no pool or
    /// connection crosses this remote-ready boundary.
    async fn enumeration_persist_endpoint_occurrence(
        &self,
        authority: &ToolTruthExecutionAuthorityRef,
        candidate: &CapabilityReceiptInputRef,
        draft: &EndpointOccurrenceWrite,
        evidence_authorities: &[EvidenceAuthorityRef],
    ) -> anyhow::Result<PersistedEndpointOccurrence>;

    async fn enumeration_persist_parameter_assessment(
        &self,
        authority: &ToolTruthExecutionAuthorityRef,
        input: &CapabilityReceiptInputRef,
        draft: &ParameterAssessmentWrite,
    ) -> anyhow::Result<Uuid>;

    async fn enumeration_project_endpoint_groups(
        &self,
        authority: &ToolTruthExecutionAuthorityRef,
        browser_receipt_id: Uuid,
        js_api_receipt_id: Uuid,
    ) -> anyhow::Result<EndpointGroupProjectionSummary>;

    async fn api_endpoints_insert(
        &self,
        target_id: Uuid,
        project_path: Option<&str>,
        url: &str,
        method: &str,
        path: &str,
        params: &serde_json::Value,
        headers: &serde_json::Value,
        auth_type: Option<&str>,
        source: &str,
        risk_level: &str,
    ) -> anyhow::Result<ApiEndpoint>;

    async fn api_endpoints_list_by_target(
        &self,
        target_id: Uuid,
    ) -> anyhow::Result<Vec<ApiEndpoint>>;

    async fn enumeration_list_endpoints_for_operation_target_origin(
        &self,
        operation_id: Uuid,
        target_id: Uuid,
        exact_origin: &str,
    ) -> anyhow::Result<Vec<ApiEndpoint>>;

    /// Insert an endpoint or, on `(target_id, url, method)` conflict, merge
    /// `params` into the existing row (set union). Backs the AI-assisted JS
    /// param recipe so body/form params can be folded into already-persisted
    /// endpoints without dropping URL-query params.
    async fn api_endpoints_upsert_merge_params(
        &self,
        target_id: Uuid,
        project_path: Option<&str>,
        url: &str,
        method: &str,
        path: &str,
        params: &serde_json::Value,
        headers: &serde_json::Value,
        auth_type: Option<&str>,
        source: &str,
        risk_level: &str,
    ) -> anyhow::Result<ApiEndpoint>;

    /// Atomic target-authorized variant for active Enumeration producers.
    async fn api_endpoints_upsert_merge_params_guarded(
        &self,
        guard: &TargetWriteGuard,
        url: &str,
        method: &str,
        path: &str,
        params: &serde_json::Value,
        headers: &serde_json::Value,
        auth_type: Option<&str>,
        source: &str,
        risk_level: &str,
    ) -> anyhow::Result<ApiEndpoint>;

    /// Idempotent target-authorized insert that reuses an existing endpoint
    /// without mutating its shared legacy payload. Enumeration v2 producers
    /// use this seam because operation-specific observations/parameters are
    /// their authority and an older operation may already have sealed a
    /// projection linked to the same global endpoint row.
    async fn api_endpoints_insert_or_ignore_guarded(
        &self,
        guard: &TargetWriteGuard,
        url: &str,
        method: &str,
        path: &str,
        params: &serde_json::Value,
        headers: &serde_json::Value,
        auth_type: Option<&str>,
        source: &str,
        risk_level: &str,
    ) -> anyhow::Result<ApiEndpoint>;

    async fn api_endpoints_update_response_evidence_guarded(
        &self,
        guard: &TargetWriteGuard,
        id: Uuid,
        headers: &serde_json::Value,
        response_type: Option<&str>,
        status_code: Option<i32>,
        capture_path: Option<&str>,
    ) -> anyhow::Result<()>;

    async fn js_analysis_insert(
        &self,
        target_id: Uuid,
        project_path: Option<&str>,
        url: &str,
        filename: &str,
        size_bytes: Option<i64>,
        hash_sha256: Option<&str>,
        frameworks: &serde_json::Value,
        libraries: &serde_json::Value,
        endpoints_found: &serde_json::Value,
        secrets_found: &serde_json::Value,
        comments: &serde_json::Value,
        source_maps: bool,
        risk_summary: &str,
        raw_analysis: &serde_json::Value,
    ) -> anyhow::Result<JsAnalysisResult>;

    /// Atomic target-authorized variant for active Enumeration producers.
    async fn js_analysis_insert_guarded(
        &self,
        guard: &TargetWriteGuard,
        url: &str,
        filename: &str,
        size_bytes: Option<i64>,
        hash_sha256: Option<&str>,
        frameworks: &serde_json::Value,
        libraries: &serde_json::Value,
        endpoints_found: &serde_json::Value,
        secrets_found: &serde_json::Value,
        comments: &serde_json::Value,
        source_maps: bool,
        risk_summary: &str,
        raw_analysis: &serde_json::Value,
    ) -> anyhow::Result<JsAnalysisResult>;

    async fn js_analysis_update_file_path(&self, id: Uuid, file_path: &str) -> anyhow::Result<()>;

    async fn js_analysis_update_file_path_guarded(
        &self,
        guard: &TargetWriteGuard,
        id: Uuid,
        file_path: &str,
    ) -> anyhow::Result<()>;

    async fn js_analysis_list_by_target(
        &self,
        target_id: Uuid,
    ) -> anyhow::Result<Vec<JsAnalysisResult>>;

    async fn fingerprints_upsert(
        &self,
        target_id: Uuid,
        project_path: Option<&str>,
        category: &str,
        name: &str,
        version: Option<&str>,
        confidence: f32,
        evidence: &serde_json::Value,
        cpe: Option<&str>,
        source: &str,
    ) -> anyhow::Result<Fingerprint>;

    async fn fingerprints_list_by_target(
        &self,
        target_id: Uuid,
    ) -> anyhow::Result<Vec<Fingerprint>>;

    async fn passive_scans_insert(
        &self,
        target_id: Uuid,
        project_path: Option<&str>,
        test_type: &str,
        payload: &str,
        url: &str,
        parameter: &str,
        result: &str,
        evidence: &str,
        severity: &str,
        tool_used: &str,
        tester: &str,
        notes: &str,
        detail: &serde_json::Value,
    ) -> anyhow::Result<PassiveScanLog>;

    async fn passive_scans_list_by_target(
        &self,
        target_id: Uuid,
        limit: i64,
    ) -> anyhow::Result<Vec<PassiveScanLog>>;

    async fn passive_scans_stats_by_target(
        &self,
        target_id: Uuid,
    ) -> anyhow::Result<serde_json::Value>;

    // ── b2 additions (golish-pentest-app/security_analysis.rs reads) ──
    async fn api_endpoints_list_untested(
        &self,
        target_id: Uuid,
    ) -> anyhow::Result<Vec<ApiEndpoint>>;

    async fn api_endpoints_count_by_target(&self, target_id: Uuid) -> anyhow::Result<(i64, i64)>;

    async fn passive_scans_list_by_url(
        &self,
        url: &str,
        limit: i64,
    ) -> anyhow::Result<Vec<PassiveScanLog>>;

    async fn passive_scans_list_vulnerable(
        &self,
        target_id: Uuid,
    ) -> anyhow::Result<Vec<PassiveScanLog>>;

    // ── b3/b5 additions (pentest_bridge JS analysis + platform audit) ──
    async fn js_analysis_update_file_path_by_url(
        &self,
        target_id: Uuid,
        url: &str,
        file_path: &str,
    ) -> anyhow::Result<u64>;

    async fn passive_scans_list_global_by_project(
        &self,
        project_path: &str,
        limit: i64,
    ) -> anyhow::Result<Vec<ReconPassiveScanGlobal>>;
}

/// In-proc adapter backed by the embedded Postgres pool.
pub struct PgReconScansAdapter {
    pool: Arc<PgPool>,
}

impl PgReconScansAdapter {
    pub fn new(pool: Arc<PgPool>) -> Self {
        Self { pool }
    }
}

#[async_trait]
impl ReconScansPort for PgReconScansAdapter {
    async fn enumeration_persist_js_analysis_descriptor(
        &self,
        authority: &ToolTruthExecutionAuthorityRef,
        input: &CapabilityReceiptInputRef,
        draft: &JsAnalysisDescriptorWrite,
    ) -> anyhow::Result<Uuid> {
        let mut tx = self.pool.begin().await?;
        let id = golish_db::repo::enumeration_endpoint_occurrences::persist_js_analysis_descriptor(
            &mut tx, authority, input, draft,
        )
        .await?;
        tx.commit().await?;
        Ok(id)
    }

    async fn enumeration_bind_js_analysis_terminal_receipt(
        &self,
        authority: &ToolTruthExecutionAuthorityRef,
        descriptor_id: Uuid,
        input: &CapabilityReceiptInputRef,
    ) -> anyhow::Result<()> {
        let mut tx = self.pool.begin().await?;
        golish_db::repo::enumeration_endpoint_occurrences::bind_js_analysis_terminal_receipt(
            &mut tx,
            authority,
            descriptor_id,
            input,
        )
        .await?;
        tx.commit().await?;
        Ok(())
    }

    async fn enumeration_persist_candidate_descriptor(
        &self,
        authority: &ToolTruthExecutionAuthorityRef,
        input: &CapabilityReceiptInputRef,
        draft: &CandidateDescriptorWrite,
    ) -> anyhow::Result<Uuid> {
        let mut tx = self.pool.begin().await?;
        let id = golish_db::repo::enumeration_endpoint_occurrences::persist_candidate_descriptor(
            &mut tx, authority, input, draft,
        )
        .await?;
        tx.commit().await?;
        Ok(id)
    }

    async fn enumeration_persist_endpoint_occurrence(
        &self,
        authority: &ToolTruthExecutionAuthorityRef,
        candidate: &CapabilityReceiptInputRef,
        draft: &EndpointOccurrenceWrite,
        evidence_authorities: &[EvidenceAuthorityRef],
    ) -> anyhow::Result<PersistedEndpointOccurrence> {
        let mut tx = self.pool.begin().await?;
        let persisted =
            golish_db::repo::enumeration_endpoint_occurrences::persist_endpoint_occurrence(
                &mut tx,
                authority,
                candidate,
                draft,
                evidence_authorities,
            )
            .await?;
        tx.commit().await?;
        Ok(persisted)
    }

    async fn enumeration_persist_parameter_assessment(
        &self,
        authority: &ToolTruthExecutionAuthorityRef,
        input: &CapabilityReceiptInputRef,
        draft: &ParameterAssessmentWrite,
    ) -> anyhow::Result<Uuid> {
        let mut tx = self.pool.begin().await?;
        let id = golish_db::repo::enumeration_endpoint_occurrences::persist_parameter_assessment(
            &mut tx, authority, input, draft,
        )
        .await?;
        tx.commit().await?;
        Ok(id)
    }

    async fn enumeration_project_endpoint_groups(
        &self,
        authority: &ToolTruthExecutionAuthorityRef,
        browser_receipt_id: Uuid,
        js_api_receipt_id: Uuid,
    ) -> anyhow::Result<EndpointGroupProjectionSummary> {
        let mut tx = self.pool.begin().await?;
        let summary = golish_db::repo::enumeration_endpoint_occurrences::project_endpoint_groups(
            &mut tx,
            authority,
            browser_receipt_id,
            js_api_receipt_id,
        )
        .await?;
        tx.commit().await?;
        Ok(summary)
    }

    async fn api_endpoints_insert(
        &self,
        target_id: Uuid,
        project_path: Option<&str>,
        url: &str,
        method: &str,
        path: &str,
        params: &serde_json::Value,
        headers: &serde_json::Value,
        auth_type: Option<&str>,
        source: &str,
        risk_level: &str,
    ) -> anyhow::Result<ApiEndpoint> {
        Ok(golish_db::repo::api_endpoints::insert(
            self.pool.as_ref(),
            target_id,
            project_path,
            url,
            method,
            path,
            params,
            headers,
            auth_type,
            source,
            risk_level,
        )
        .await?)
    }

    async fn api_endpoints_upsert_merge_params(
        &self,
        target_id: Uuid,
        project_path: Option<&str>,
        url: &str,
        method: &str,
        path: &str,
        params: &serde_json::Value,
        headers: &serde_json::Value,
        auth_type: Option<&str>,
        source: &str,
        risk_level: &str,
    ) -> anyhow::Result<ApiEndpoint> {
        Ok(golish_db::repo::api_endpoints::upsert_merge_params(
            self.pool.as_ref(),
            target_id,
            project_path,
            url,
            method,
            path,
            params,
            headers,
            auth_type,
            source,
            risk_level,
        )
        .await?)
    }

    async fn api_endpoints_upsert_merge_params_guarded(
        &self,
        guard: &TargetWriteGuard,
        url: &str,
        method: &str,
        path: &str,
        params: &serde_json::Value,
        headers: &serde_json::Value,
        auth_type: Option<&str>,
        source: &str,
        risk_level: &str,
    ) -> anyhow::Result<ApiEndpoint> {
        Ok(golish_db::repo::api_endpoints::upsert_merge_params_guarded(
            self.pool.as_ref(),
            guard,
            url,
            method,
            path,
            params,
            headers,
            auth_type,
            source,
            risk_level,
        )
        .await?)
    }

    async fn api_endpoints_insert_or_ignore_guarded(
        &self,
        guard: &TargetWriteGuard,
        url: &str,
        method: &str,
        path: &str,
        params: &serde_json::Value,
        headers: &serde_json::Value,
        auth_type: Option<&str>,
        source: &str,
        risk_level: &str,
    ) -> anyhow::Result<ApiEndpoint> {
        Ok(golish_db::repo::api_endpoints::insert_or_ignore_guarded(
            self.pool.as_ref(),
            guard,
            url,
            method,
            path,
            params,
            headers,
            auth_type,
            source,
            risk_level,
        )
        .await?)
    }

    async fn api_endpoints_update_response_evidence_guarded(
        &self,
        guard: &TargetWriteGuard,
        id: Uuid,
        headers: &serde_json::Value,
        response_type: Option<&str>,
        status_code: Option<i32>,
        capture_path: Option<&str>,
    ) -> anyhow::Result<()> {
        golish_db::repo::api_endpoints::update_response_evidence_guarded(
            self.pool.as_ref(),
            guard,
            id,
            headers,
            response_type,
            status_code,
            capture_path,
        )
        .await?;
        Ok(())
    }

    async fn api_endpoints_list_by_target(
        &self,
        target_id: Uuid,
    ) -> anyhow::Result<Vec<ApiEndpoint>> {
        Ok(
            golish_db::repo::api_endpoints::list_by_current_target_owner(
                self.pool.as_ref(),
                target_id,
            )
            .await?,
        )
    }

    async fn enumeration_list_endpoints_for_operation_target_origin(
        &self,
        operation_id: Uuid,
        target_id: Uuid,
        exact_origin: &str,
    ) -> anyhow::Result<Vec<ApiEndpoint>> {
        Ok(
            golish_db::repo::enumeration_surface_manifest::list_endpoints_for_operation_target_origin(
                self.pool.as_ref(),
                operation_id,
                target_id,
                exact_origin,
            )
            .await?,
        )
    }

    async fn js_analysis_insert(
        &self,
        target_id: Uuid,
        project_path: Option<&str>,
        url: &str,
        filename: &str,
        size_bytes: Option<i64>,
        hash_sha256: Option<&str>,
        frameworks: &serde_json::Value,
        libraries: &serde_json::Value,
        endpoints_found: &serde_json::Value,
        secrets_found: &serde_json::Value,
        comments: &serde_json::Value,
        source_maps: bool,
        risk_summary: &str,
        raw_analysis: &serde_json::Value,
    ) -> anyhow::Result<JsAnalysisResult> {
        Ok(golish_db::repo::js_analysis::insert(
            self.pool.as_ref(),
            target_id,
            project_path,
            url,
            filename,
            size_bytes,
            hash_sha256,
            frameworks,
            libraries,
            endpoints_found,
            secrets_found,
            comments,
            source_maps,
            risk_summary,
            raw_analysis,
        )
        .await?)
    }

    async fn js_analysis_insert_guarded(
        &self,
        guard: &TargetWriteGuard,
        url: &str,
        filename: &str,
        size_bytes: Option<i64>,
        hash_sha256: Option<&str>,
        frameworks: &serde_json::Value,
        libraries: &serde_json::Value,
        endpoints_found: &serde_json::Value,
        secrets_found: &serde_json::Value,
        comments: &serde_json::Value,
        source_maps: bool,
        risk_summary: &str,
        raw_analysis: &serde_json::Value,
    ) -> anyhow::Result<JsAnalysisResult> {
        Ok(golish_db::repo::js_analysis::insert_guarded(
            self.pool.as_ref(),
            guard,
            url,
            filename,
            size_bytes,
            hash_sha256,
            frameworks,
            libraries,
            endpoints_found,
            secrets_found,
            comments,
            source_maps,
            risk_summary,
            raw_analysis,
        )
        .await?)
    }

    async fn js_analysis_update_file_path(&self, id: Uuid, file_path: &str) -> anyhow::Result<()> {
        golish_db::repo::js_analysis::update_file_path(self.pool.as_ref(), id, file_path).await?;
        Ok(())
    }

    async fn js_analysis_update_file_path_guarded(
        &self,
        guard: &TargetWriteGuard,
        id: Uuid,
        file_path: &str,
    ) -> anyhow::Result<()> {
        golish_db::repo::js_analysis::update_file_path_guarded(
            self.pool.as_ref(),
            guard,
            id,
            file_path,
        )
        .await?;
        Ok(())
    }

    async fn js_analysis_list_by_target(
        &self,
        target_id: Uuid,
    ) -> anyhow::Result<Vec<JsAnalysisResult>> {
        Ok(golish_db::repo::js_analysis::list_by_current_target_owner(
            self.pool.as_ref(),
            target_id,
        )
        .await?)
    }

    async fn fingerprints_upsert(
        &self,
        target_id: Uuid,
        project_path: Option<&str>,
        category: &str,
        name: &str,
        version: Option<&str>,
        confidence: f32,
        evidence: &serde_json::Value,
        cpe: Option<&str>,
        source: &str,
    ) -> anyhow::Result<Fingerprint> {
        Ok(golish_db::repo::fingerprints::upsert(
            self.pool.as_ref(),
            target_id,
            project_path,
            category,
            name,
            version,
            confidence,
            evidence,
            cpe,
            source,
        )
        .await?)
    }

    async fn fingerprints_list_by_target(
        &self,
        target_id: Uuid,
    ) -> anyhow::Result<Vec<Fingerprint>> {
        Ok(golish_db::repo::fingerprints::list_by_current_target_owner(
            self.pool.as_ref(),
            target_id,
        )
        .await?)
    }

    async fn passive_scans_insert(
        &self,
        target_id: Uuid,
        project_path: Option<&str>,
        test_type: &str,
        payload: &str,
        url: &str,
        parameter: &str,
        result: &str,
        evidence: &str,
        severity: &str,
        tool_used: &str,
        tester: &str,
        notes: &str,
        detail: &serde_json::Value,
    ) -> anyhow::Result<PassiveScanLog> {
        Ok(golish_db::repo::passive_scans::insert(
            self.pool.as_ref(),
            target_id,
            project_path,
            test_type,
            payload,
            url,
            parameter,
            result,
            evidence,
            severity,
            tool_used,
            tester,
            notes,
            detail,
        )
        .await?)
    }

    async fn passive_scans_list_by_target(
        &self,
        target_id: Uuid,
        limit: i64,
    ) -> anyhow::Result<Vec<PassiveScanLog>> {
        Ok(
            golish_db::repo::passive_scans::list_by_current_target_owner(
                self.pool.as_ref(),
                target_id,
                limit,
            )
            .await?,
        )
    }

    async fn passive_scans_stats_by_target(
        &self,
        target_id: Uuid,
    ) -> anyhow::Result<serde_json::Value> {
        Ok(
            golish_db::repo::passive_scans::stats_by_current_target_owner(
                self.pool.as_ref(),
                target_id,
            )
            .await?,
        )
    }

    async fn api_endpoints_list_untested(
        &self,
        target_id: Uuid,
    ) -> anyhow::Result<Vec<ApiEndpoint>> {
        Ok(
            golish_db::repo::api_endpoints::list_untested_by_current_target_owner(
                self.pool.as_ref(),
                target_id,
            )
            .await?,
        )
    }

    async fn api_endpoints_count_by_target(&self, target_id: Uuid) -> anyhow::Result<(i64, i64)> {
        Ok(
            golish_db::repo::api_endpoints::count_by_current_target_owner(
                self.pool.as_ref(),
                target_id,
            )
            .await?,
        )
    }

    async fn passive_scans_list_by_url(
        &self,
        url: &str,
        limit: i64,
    ) -> anyhow::Result<Vec<PassiveScanLog>> {
        Ok(golish_db::repo::passive_scans::list_by_url(self.pool.as_ref(), url, limit).await?)
    }

    async fn passive_scans_list_vulnerable(
        &self,
        target_id: Uuid,
    ) -> anyhow::Result<Vec<PassiveScanLog>> {
        Ok(
            golish_db::repo::passive_scans::list_vulnerable_by_current_target_owner(
                self.pool.as_ref(),
                target_id,
            )
            .await?,
        )
    }

    async fn js_analysis_update_file_path_by_url(
        &self,
        target_id: Uuid,
        url: &str,
        file_path: &str,
    ) -> anyhow::Result<u64> {
        Ok(golish_db::repo::js_analysis::update_file_path_by_url(
            self.pool.as_ref(),
            target_id,
            url,
            file_path,
        )
        .await?)
    }

    async fn passive_scans_list_global_by_project(
        &self,
        project_path: &str,
        limit: i64,
    ) -> anyhow::Result<Vec<ReconPassiveScanGlobal>> {
        Ok(
            golish_db::repo::passive_scans::list_global_by_project::<ReconPassiveScanGlobal>(
                self.pool.as_ref(),
                project_path,
                limit,
            )
            .await?,
        )
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    // Compile-time guarantee the port stays object-safe (consumers store
    // `Arc<dyn ReconScansPort>`).
    #[test]
    fn recon_scans_port_is_object_safe() {
        fn _assert(_: &dyn ReconScansPort) {}
    }
}
