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
        let entries = self
            .vuln_intel
            .vuln_intel_search_entries(cve_id, limit)
            .await?;
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
        let result = self
            .recon_scans
            .api_endpoints_insert(
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
        let result = self
            .recon_scans
            .js_analysis_insert(
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
        self.recon_scans
            .js_analysis_update_file_path(id, file_path)
            .await?;
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
        let _result = self
            .recon_scans
            .fingerprints_upsert(
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
        let result = self
            .recon_scans
            .passive_scans_insert(
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
            if let Ok(assets) = self
                .recon_assets
                .target_assets_list_by_target(target_id)
                .await
            {
                data["assets"] = serde_json::to_value(&assets)?;
                data["assets_count"] = json!(assets.len());
            }
        }
        if include_all || sections.contains(&"endpoints".to_string()) {
            if let Ok(endpoints) = self
                .recon_scans
                .api_endpoints_list_by_target(target_id)
                .await
            {
                data["endpoints"] = serde_json::to_value(&endpoints)?;
                data["endpoints_count"] = json!(endpoints.len());
            }
        }
        if include_all || sections.contains(&"fingerprints".to_string()) {
            if let Ok(fps) = self
                .recon_scans
                .fingerprints_list_by_target(target_id)
                .await
            {
                data["fingerprints"] = serde_json::to_value(&fps)?;
            }
        }
        if include_all || sections.contains(&"js_analysis".to_string()) {
            if let Ok(results) = self.recon_scans.js_analysis_list_by_target(target_id).await {
                data["js_analysis"] = serde_json::to_value(&results)?;
            }
        }
        if include_all || sections.contains(&"scan_logs".to_string()) {
            if let Ok(logs) = self
                .recon_scans
                .passive_scans_list_by_target(target_id, 100)
                .await
            {
                data["scan_logs"] = serde_json::to_value(&logs)?;
                if let Ok(stats) = self
                    .recon_scans
                    .passive_scans_stats_by_target(target_id)
                    .await
                {
                    data["scan_stats"] = stats;
                }
            }
        }
        Ok(data)
    }

    pub(super) async fn in_scope_assets_impl(
        &self,
        org_id: Option<Uuid>,
    ) -> anyhow::Result<Vec<String>> {
        // `None` project_path = legacy "all visible" set; the harness has no
        // per-project key today (chat sessions carry project_path=None). `org_id`
        // narrows the asset axis to the current operation's organization
        // (coverage asset-axis isolation, design 2026-06-09); `None` keeps the
        // legacy whole-DB axis.
        self.recon_targets.in_scope_values(None, org_id).await
    }

    /// 设计 2026-06-12 §5.3 · DB 业务表真值事实（转 String technique，与 golish-db
    /// 的 `&'static str` 常量解耦）。`coverage_truth` 是 harness 跨表只读真值投影
    /// （SHARED repo，类比 audit ledger），直接调 golish-db 而非经 recon CRUD port。
    pub(super) async fn db_truth_facts_impl(
        &self,
        org_id: Option<Uuid>,
        in_scope_assets: &[String],
    ) -> anyhow::Result<Vec<(String, String)>> {
        // 2c-2 (设计 host-aware-coverage-2c §4.3): align each in-scope asset to
        // its targets.type so coverage_truth drops domain-only org facts (CT) on
        // IP assets. Missing type → "" (non-IP, keep all — fail-safe). Reuses the
        // 2c-1 typed in-scope read.
        let type_map: std::collections::HashMap<String, String> = self
            .in_scope_typed_assets_impl(org_id)
            .await
            .unwrap_or_default()
            .into_iter()
            .collect();
        let types: Vec<String> = in_scope_assets
            .iter()
            .map(|a| type_map.get(a).cloned().unwrap_or_default())
            .collect();
        let rows = golish_db::repo::coverage_truth::coverage_truth_facts(
            &self.pool,
            org_id,
            in_scope_assets,
            &types,
        )
        .await?;
        Ok(rows.into_iter().map(|(a, t)| (a, t.to_string())).collect())
    }

    pub(super) async fn in_scope_targets_impl(
        &self,
        org_id: Option<Uuid>,
    ) -> anyhow::Result<Vec<serde_json::Value>> {
        let targets = self.recon_targets.in_scope_targets(None).await?;
        // Engagement-org isolation (设计 2026-06-15-engagement-org-isolation):
        // confine the listing to the scoping-confirmed engagement org subtree
        // (root + subsidiaries). `None` org = legacy whole-visible set (chat /
        // pre-scoping). Once an org is bound, targets with no org binding are
        // excluded (fail-closed: an unowned row is not "this engagement's").
        // Shares the db helper with the `manage_targets` list path so the two
        // org-confinement reads never drift.
        let allowed = golish_db::repo::organizations::subtree_id_str_set(&self.pool, org_id).await;
        Ok(targets
            .into_iter()
            .filter(|t| {
                golish_db::repo::organizations::org_id_in_scope(
                    t.organization_id.as_deref(),
                    &allowed,
                )
            })
            .map(|t| {
                json!({
                    "target_id": t.id,
                    "value": t.value,
                    "type": t.target_type.as_str(),
                })
            })
            .collect())
    }

    /// P3 Phase B (2026-06-11): distinct `targets.type` of the in-scope assets,
    /// so the harness coverage gate derives dynamic expected techniques per asset
    /// class (`technique_resolver`). Reuses the recon targets port (no new SQL /
    /// schema); dedupes in first-seen order for deterministic output. `org_id`
    /// narrowing is deferred — chat sessions carry no org binding, so the legacy
    /// whole-visible set matches `in_scope_targets_impl`.
    pub(super) async fn in_scope_target_types_impl(
        &self,
        _org_id: Option<Uuid>,
    ) -> anyhow::Result<Vec<String>> {
        let targets = self.recon_targets.in_scope_targets(None).await?;
        let mut types: Vec<String> = Vec::new();
        for t in targets {
            let ty = t.target_type.as_str().to_string();
            if !types.contains(&ty) {
                types.push(ty);
            }
        }
        Ok(types)
    }

    /// 2c-1 (设计 host-aware-coverage-2c §4.1): in-scope `(value, targets.type)`
    /// pairs for the harness gate's **authoritative** asset classification.
    /// Reuses the recon targets port (mirrors [`Self::in_scope_target_types_impl`];
    /// no new SQL). `org_id` narrowing deferred (chat sessions carry no org
    /// binding) — a superset map is harmless: `coverage_complete` only looks up
    /// its org-narrowed asset axis, and any missing entry falls back to value
    /// inference (2a/2b).
    pub(super) async fn in_scope_typed_assets_impl(
        &self,
        _org_id: Option<Uuid>,
    ) -> anyhow::Result<Vec<(String, String)>> {
        let targets = self.recon_targets.in_scope_targets(None).await?;
        Ok(targets
            .into_iter()
            .map(|t| (t.value, t.target_type.as_str().to_string()))
            .collect())
    }

    /// Phase 1.5 阶段过门：列全库（或指定 project）的 organization id。组织树属 recon 资产
    /// 域；chat 走整库口径（project_path=None），与 `in_scope_assets_impl` 同口径。
    pub(super) async fn in_scope_org_ids_impl(
        &self,
        project_path: Option<&str>,
    ) -> anyhow::Result<Vec<Uuid>> {
        Ok(golish_db::repo::organizations::in_scope_ids(&self.pool, project_path).await?)
    }

    /// Engagement-org isolation (设计 2026-06-15-engagement-org-isolation): the
    /// subtree (root + descendants) of the scoping-confirmed engagement root org,
    /// so the stage_run fan-out confines dispatch to THIS engagement's tree and a
    /// sibling engagement's org (left in the same workspace) is never targeted.
    pub(super) async fn org_subtree_ids_impl(
        &self,
        root_id: Uuid,
    ) -> anyhow::Result<Vec<Uuid>> {
        Ok(golish_db::repo::organizations::subtree_ids(&self.pool, root_id).await?)
    }

    /// Phase 1.5 阶段过门：批量取 per-org 完成账本行 `(organization_id, passed_at)`（收尾
    /// gate 经 repo 通道核「全 org 新鲜 PASS」）。逐 id 取（org 量级小），无行的 org 自然缺席。
    pub(super) async fn org_stage_completions_get_impl(
        &self,
        stage_kind: &str,
        org_ids: &[Uuid],
    ) -> anyhow::Result<Vec<(Uuid, chrono::DateTime<chrono::Utc>)>> {
        let mut out = Vec::with_capacity(org_ids.len());
        for &org in org_ids {
            if let Some(row) =
                golish_db::repo::org_stage_completions::get(&self.pool, org, stage_kind).await?
            {
                out.push((row.organization_id, row.passed_at));
            }
        }
        Ok(out)
    }
}
