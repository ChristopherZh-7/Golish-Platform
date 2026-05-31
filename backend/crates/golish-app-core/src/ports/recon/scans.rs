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

    async fn js_analysis_update_file_path(&self, id: Uuid, file_path: &str) -> anyhow::Result<()>;

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

    // ── b3/b5 additions (pentest_bridge js_collect + platform audit) ──
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

    async fn api_endpoints_list_by_target(
        &self,
        target_id: Uuid,
    ) -> anyhow::Result<Vec<ApiEndpoint>> {
        Ok(golish_db::repo::api_endpoints::list_by_target(self.pool.as_ref(), target_id).await?)
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

    async fn js_analysis_update_file_path(&self, id: Uuid, file_path: &str) -> anyhow::Result<()> {
        golish_db::repo::js_analysis::update_file_path(self.pool.as_ref(), id, file_path).await?;
        Ok(())
    }

    async fn js_analysis_list_by_target(
        &self,
        target_id: Uuid,
    ) -> anyhow::Result<Vec<JsAnalysisResult>> {
        Ok(golish_db::repo::js_analysis::list_by_target(self.pool.as_ref(), target_id).await?)
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
        Ok(golish_db::repo::fingerprints::list_by_target(self.pool.as_ref(), target_id).await?)
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
            golish_db::repo::passive_scans::list_by_target(self.pool.as_ref(), target_id, limit)
                .await?,
        )
    }

    async fn passive_scans_stats_by_target(
        &self,
        target_id: Uuid,
    ) -> anyhow::Result<serde_json::Value> {
        Ok(golish_db::repo::passive_scans::stats_by_target(self.pool.as_ref(), target_id).await?)
    }

    async fn api_endpoints_list_untested(
        &self,
        target_id: Uuid,
    ) -> anyhow::Result<Vec<ApiEndpoint>> {
        Ok(golish_db::repo::api_endpoints::list_untested(self.pool.as_ref(), target_id).await?)
    }

    async fn api_endpoints_count_by_target(&self, target_id: Uuid) -> anyhow::Result<(i64, i64)> {
        Ok(golish_db::repo::api_endpoints::count_by_target(self.pool.as_ref(), target_id).await?)
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
        Ok(golish_db::repo::passive_scans::list_vulnerable(self.pool.as_ref(), target_id).await?)
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
