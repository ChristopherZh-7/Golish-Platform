//! Recon / security-analysis domain methods for `GolishDbRepoProvider`
//! (inherent `_impl` layer). Bodies moved verbatim from the original
//! `db_bridge.rs` trait impl; the trait methods in `mod.rs` delegate here.

use serde_json::json;
use uuid::Uuid;

use super::GolishDbRepoProvider;

impl GolishDbRepoProvider {
    pub(super) async fn vuln_intel_search_impl(
        &self,
        cve_id: &str,
        limit: i64,
    ) -> anyhow::Result<serde_json::Value> {
        let entries =
            golish_db::repo::vuln_intel::search_entries(&self.pool, cve_id, limit).await?;
        Ok(serde_json::to_value(entries)?)
    }

    #[allow(clippy::too_many_arguments)]
    pub(super) async fn audit_log_operation_impl(
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
        let entry = golish_db::repo::audit::log_operation(
            &self.pool,
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
        .await?;
        Ok(serde_json::to_value(entry)?)
    }

    #[allow(clippy::too_many_arguments)]
    pub(super) async fn api_endpoints_insert_impl(
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
        let result = golish_db::repo::api_endpoints::insert(
            &self.pool,
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
        .await?;
        Ok(serde_json::to_value(result)?)
    }

    pub(super) async fn js_analysis_insert_impl(
        &self,
        target_id: Uuid,
        project_path: &str,
        url: &str,
        filename: &str,
        _analysis: &serde_json::Value,
    ) -> anyhow::Result<serde_json::Value> {
        let result = golish_db::repo::js_analysis::insert(
            &self.pool,
            target_id,
            Some(project_path),
            url,
            filename,
            None,
            None,
            &json!([]),
            &json!([]),
            &json!([]),
            &json!([]),
            &json!([]),
            false,
            "",
            &json!({}),
        )
        .await?;
        Ok(serde_json::to_value(result)?)
    }

    pub(super) async fn js_analysis_update_file_path_impl(
        &self,
        id: Uuid,
        file_path: &str,
    ) -> anyhow::Result<()> {
        golish_db::repo::js_analysis::update_file_path(&self.pool, id, file_path).await?;
        Ok(())
    }

    #[allow(clippy::too_many_arguments)]
    pub(super) async fn fingerprints_upsert_impl(
        &self,
        target_id: Uuid,
        project_path: &str,
        category: &str,
        name: &str,
        version: Option<&str>,
        confidence: f64,
        raw_data: Option<&serde_json::Value>,
    ) -> anyhow::Result<bool> {
        let _result = golish_db::repo::fingerprints::upsert(
            &self.pool,
            target_id,
            Some(project_path),
            category,
            name,
            version,
            confidence as f32,
            raw_data.unwrap_or(&json!({})),
            None,
            "",
        )
        .await?;
        Ok(true)
    }

    #[allow(clippy::too_many_arguments)]
    pub(super) async fn passive_scans_insert_impl(
        &self,
        target_id: Uuid,
        project_path: &str,
        scan_type: &str,
        tool_name: &str,
        _findings: &serde_json::Value,
        raw_output: Option<&str>,
        severity: &str,
    ) -> anyhow::Result<serde_json::Value> {
        let result = golish_db::repo::passive_scans::insert(
            &self.pool,
            target_id,
            Some(project_path),
            scan_type,
            "",
            "",
            "",
            "",
            raw_output.unwrap_or(""),
            severity,
            tool_name,
            "ai",
            "",
            &json!({}),
        )
        .await?;
        Ok(serde_json::to_value(result)?)
    }

    pub(super) async fn query_target_data_impl(
        &self,
        target_id: Uuid,
        sections: &[String],
    ) -> anyhow::Result<serde_json::Value> {
        let include_all = sections.contains(&"all".to_string());
        let mut data = json!({});

        if include_all || sections.contains(&"assets".to_string()) {
            if let Ok(assets) =
                golish_db::repo::target_assets::list_by_target(&self.pool, target_id).await
            {
                data["assets"] = serde_json::to_value(&assets)?;
                data["assets_count"] = json!(assets.len());
            }
        }
        if include_all || sections.contains(&"endpoints".to_string()) {
            if let Ok(endpoints) =
                golish_db::repo::api_endpoints::list_by_target(&self.pool, target_id).await
            {
                data["endpoints"] = serde_json::to_value(&endpoints)?;
                data["endpoints_count"] = json!(endpoints.len());
            }
        }
        if include_all || sections.contains(&"fingerprints".to_string()) {
            if let Ok(fps) =
                golish_db::repo::fingerprints::list_by_target(&self.pool, target_id).await
            {
                data["fingerprints"] = serde_json::to_value(&fps)?;
            }
        }
        if include_all || sections.contains(&"js_analysis".to_string()) {
            if let Ok(results) =
                golish_db::repo::js_analysis::list_by_target(&self.pool, target_id).await
            {
                data["js_analysis"] = serde_json::to_value(&results)?;
            }
        }
        if include_all || sections.contains(&"scan_logs".to_string()) {
            if let Ok(logs) =
                golish_db::repo::passive_scans::list_by_target(&self.pool, target_id, 100).await
            {
                data["scan_logs"] = serde_json::to_value(&logs)?;
                if let Ok(stats) =
                    golish_db::repo::passive_scans::stats_by_target(&self.pool, target_id).await
                {
                    data["scan_stats"] = stats;
                }
            }
        }
        Ok(data)
    }
}
