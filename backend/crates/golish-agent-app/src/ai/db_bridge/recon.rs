//! Recon / security-analysis domain methods for `GolishDbRepoProvider`
//! (inherent `_impl` layer). Bodies moved verbatim from the original
//! `db_bridge.rs` trait impl; the trait methods in `mod.rs` delegate here.

use serde_json::json;
use uuid::Uuid;

use golish_agent_kit::db_traits::OrgScopeUnit;
use golish_app_core::domain::targets::{Target, TargetType};

use super::GolishDbRepoProvider;

fn section_requested(include_all: bool, sections: &[String], section: &str) -> bool {
    include_all || sections.iter().any(|s| s == section)
}

fn json_string(v: &serde_json::Value, keys: &[&str]) -> String {
    keys.iter()
        .find_map(|key| v.get(*key).and_then(|value| value.as_str()))
        .unwrap_or_default()
        .to_ascii_lowercase()
}

fn json_port(v: &serde_json::Value) -> Option<u16> {
    v.get("port")
        .and_then(|value| {
            value
                .as_u64()
                .or_else(|| value.as_str().and_then(|s| s.parse::<u64>().ok()))
        })
        .and_then(|port| u16::try_from(port).ok())
}

fn is_web_like_port(port: Option<u16>, service: &str) -> bool {
    service.contains("http")
        || service.contains("web")
        || matches!(
            port,
            Some(
                80 | 81
                    | 443
                    | 3000
                    | 5000
                    | 7001
                    | 8000
                    | 8008
                    | 8080
                    | 8081
                    | 8443
                    | 8888
                    | 9000
                    | 9443
            )
        )
}

fn root_url_for(host: &str, port: Option<u16>, service: &str) -> (String, String, Option<u16>) {
    let scheme = if service.contains("https")
        || service.contains("ssl")
        || matches!(port, Some(443 | 8443 | 9443))
    {
        "https"
    } else {
        "http"
    };
    let port_suffix = match (scheme, port) {
        ("http", Some(80)) | ("https", Some(443)) | (_, None) => String::new(),
        (_, Some(port)) => format!(":{port}"),
    };
    (
        format!("{scheme}://{host}{port_suffix}/"),
        scheme.to_string(),
        port,
    )
}

fn derive_enumeration_web_roots(target: &Target) -> Vec<serde_json::Value> {
    let value = target.value.trim().trim_end_matches('/');
    if value.is_empty() {
        return Vec::new();
    }

    if matches!(target.target_type, TargetType::Url)
        || value.starts_with("http://")
        || value.starts_with("https://")
    {
        let scheme = if value.starts_with("https://") {
            "https"
        } else {
            "http"
        };
        return vec![json!({
            "web_root_id": format!("derived:{}:{}", target.id, value),
            "target_id": target.id,
            "organization_id": target.organization_id,
            "root_url": format!("{value}/"),
            "final_url": format!("{value}/"),
            "scheme": scheme,
            "host": value.trim_start_matches("https://").trim_start_matches("http://").split('/').next().unwrap_or(value),
            "port": null,
            "status": target.http_status,
            "title": target.http_title,
            "confidence": if target.http_status.is_some() { "high" } else { "medium" },
            "needs_probe": target.http_status.is_none(),
            "source_stage": "external_attack_surface",
        })];
    }

    if matches!(target.target_type, TargetType::Cidr | TargetType::Wildcard) {
        return Vec::new();
    }

    let has_web_metadata = target.http_status.is_some()
        || !target.webserver.trim().is_empty()
        || !target.content_type.trim().is_empty()
        || !target.http_title.trim().is_empty();

    let mut roots = Vec::new();
    for port_entry in &target.ports {
        let port = json_port(port_entry);
        let service = json_string(port_entry, &["service", "name", "protocol", "proto"]);
        if !is_web_like_port(port, &service) {
            continue;
        }
        let (root_url, scheme, port) = root_url_for(value, port, &service);
        roots.push(json!({
            "web_root_id": format!("derived:{}:{}:{}", target.id, scheme, port.unwrap_or_default()),
            "target_id": target.id,
            "organization_id": target.organization_id,
            "root_url": root_url,
            "final_url": root_url,
            "scheme": scheme,
            "host": value,
            "port": port,
            "status": target.http_status,
            "title": target.http_title,
            "confidence": "high",
            "needs_probe": false,
            "source_stage": "external_attack_surface",
        }));
    }

    if roots.is_empty() && has_web_metadata {
        let (root_url, scheme, port) = root_url_for(value, None, "");
        roots.push(json!({
            "web_root_id": format!("derived:{}:{}:default", target.id, scheme),
            "target_id": target.id,
            "organization_id": target.organization_id,
            "root_url": root_url,
            "final_url": root_url,
            "scheme": scheme,
            "host": value,
            "port": port,
            "status": target.http_status,
            "title": target.http_title,
            "confidence": "medium",
            "needs_probe": target.http_status.is_none(),
            "source_stage": "external_attack_surface",
        }));
    }

    roots
}

fn build_enumeration_coverage_summary(facts: &[(String, String)]) -> serde_json::Value {
    let mut found = std::collections::BTreeSet::new();
    for (_, technique) in facts {
        if matches!(
            technique.as_str(),
            golish_db::repo::coverage_truth::TECH_ENUM_DIR
                | golish_db::repo::coverage_truth::TECH_ENUM_PARAM
                | golish_db::repo::coverage_truth::TECH_ENUM_JSAPI
        ) {
            found.insert(technique.clone());
        }
    }
    let techniques = [
        golish_db::repo::coverage_truth::TECH_ENUM_DIR,
        golish_db::repo::coverage_truth::TECH_ENUM_PARAM,
        golish_db::repo::coverage_truth::TECH_ENUM_JSAPI,
    ];
    let rows: Vec<_> = techniques
        .iter()
        .map(|technique| {
            json!({
                "technique": technique,
                "db_found": found.contains(*technique),
                "status_hint": if found.contains(*technique) { "found" } else { "no_found_fact" },
            })
        })
        .collect();
    json!({
        "source": "coverage_truth",
        "semantics": "found-only; absence is not checked_empty",
        "techniques": rows,
    })
}

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
        let needs_target_row = section_requested(include_all, sections, "web_roots")
            || section_requested(include_all, sections, "coverage");
        let target_row = if needs_target_row {
            let target_id_str = target_id.to_string();
            self.recon_targets
                .in_scope_targets(None)
                .await
                .ok()
                .and_then(|targets| {
                    targets
                        .into_iter()
                        .find(|target| target.id == target_id_str)
                })
        } else {
            None
        };

        if section_requested(include_all, sections, "assets") {
            if let Ok(assets) = self
                .recon_assets
                .target_assets_list_by_target(target_id)
                .await
            {
                data["assets"] = serde_json::to_value(&assets)?;
                data["assets_count"] = json!(assets.len());
            }
        }
        if section_requested(include_all, sections, "endpoints") {
            if let Ok(endpoints) = self
                .recon_scans
                .api_endpoints_list_by_target(target_id)
                .await
            {
                data["endpoints"] = serde_json::to_value(&endpoints)?;
                data["endpoints_count"] = json!(endpoints.len());
            }
        }
        if section_requested(include_all, sections, "directories") {
            if let Ok(directories) = self
                .recon_directory
                .directory_entries_list_by_target(target_id)
                .await
            {
                data["directories"] = serde_json::to_value(&directories)?;
                data["directories_count"] = json!(directories.len());
            }
        }
        if section_requested(include_all, sections, "fingerprints") {
            if let Ok(fps) = self
                .recon_scans
                .fingerprints_list_by_target(target_id)
                .await
            {
                data["fingerprints"] = serde_json::to_value(&fps)?;
            }
        }
        if section_requested(include_all, sections, "js_analysis") {
            if let Ok(results) = self.recon_scans.js_analysis_list_by_target(target_id).await {
                data["js_analysis"] = serde_json::to_value(&results)?;
            }
        }
        if section_requested(include_all, sections, "web_roots") {
            let web_roots = target_row
                .as_ref()
                .map(derive_enumeration_web_roots)
                .unwrap_or_default();
            data["web_roots"] = json!(web_roots);
            data["web_roots_count"] = json!(web_roots.len());
        }
        if section_requested(include_all, sections, "coverage") {
            if let Some(target) = &target_row {
                let org_id = target
                    .organization_id
                    .as_deref()
                    .and_then(|id| Uuid::parse_str(id).ok());
                let facts = self
                    .db_truth_facts_impl(org_id, std::slice::from_ref(&target.value), None)
                    .await
                    .unwrap_or_default();
                data["coverage"] = build_enumeration_coverage_summary(&facts);
            } else {
                data["coverage"] = json!({
                    "source": "coverage_truth",
                    "semantics": "found-only; absence is not checked_empty",
                    "error": "target row not found in in-scope targets",
                });
            }
        }
        if section_requested(include_all, sections, "scan_logs") {
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

    pub(super) async fn in_scope_assets_created_before_impl(
        &self,
        org_id: Option<Uuid>,
        cutoff: chrono::DateTime<chrono::Utc>,
    ) -> anyhow::Result<Vec<String>> {
        self.recon_targets
            .in_scope_values_created_before(None, org_id, cutoff)
            .await
    }

    /// 设计 2026-06-12 §5.3 · DB 业务表真值事实（转 String technique，与 golish-db
    /// 的 `&'static str` 常量解耦）。`coverage_truth` 是 harness 跨表只读真值投影
    /// （SHARED repo，类比 audit ledger），直接调 golish-db 而非经 recon CRUD port。
    pub(super) async fn db_truth_facts_impl(
        &self,
        org_id: Option<Uuid>,
        in_scope_assets: &[String],
        run_start: Option<chrono::DateTime<chrono::Utc>>,
    ) -> anyhow::Result<Vec<(String, String)>> {
        // Coverage-landing refresh (fix 2026-06-17 enrich-timing): the recon
        // sub-agent calls `recon_map_assets` (which lands subdomains) BEFORE
        // `manage_targets add` registers the in-scope targets, so at enrich time the
        // gate-read tables (`target_assets`/`dns_records`) stayed empty → this
        // authoritative db-truth read saw no per-asset DNS/SUBDOMAIN facts → the
        // target_intel coverage gate dead-looped. Re-run the per-asset landing now
        // that targets exist; idempotent (NOT EXISTS/upsert skip) and non-fatal.
        if let Some(org_id) = org_id {
            let (subs, dns) =
                golish_recon_app::organization_recon::refresh_per_asset_landing(&self.pool, org_id)
                    .await;
            if subs > 0 || dns > 0 {
                tracing::info!(
                    target: "harness::submit_tool",
                    org_id = %org_id,
                    subdomains = subs,
                    dns_records = dns,
                    "per-asset coverage landing refreshed before db-truth read"
                );
            }
        }
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
            run_start,
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
                // L1a (design 2026-06-24-intel-to-eas-handoff): widen the EAS
                // handoff with the intel context already carried on the row
                // (source / status / real_ip / ports / org / http_status / cdn_waf)
                // so the next stage can prioritise instead of flat-scanning a bare
                // id/value/type list.
                json!({
                    "target_id": t.id,
                    "value": t.value,
                    "type": t.target_type.as_str(),
                    "source": t.source,
                    "status": t.status.as_str(),
                    "real_ip": t.real_ip,
                    "ports": t.ports,
                    "organization_id": t.organization_id,
                    "http_status": t.http_status,
                    "cdn_waf": t.cdn_waf,
                })
            })
            .collect())
    }

    /// L1b (design 2026-06-24-intel-to-eas-handoff): rich, ranked attack-surface
    /// seeds for the EAS specialist. Same org-subtree isolation as
    /// [`Self::in_scope_targets_impl`], but each row carries the intel context +
    /// a computed `priority`, and the set is ranked (resolved/alive web hosts
    /// first, whole netblocks last) and optionally capped (D3).
    pub(super) async fn attack_surface_seeds_impl(
        &self,
        org_id: Option<Uuid>,
        cap: Option<usize>,
    ) -> anyhow::Result<Vec<serde_json::Value>> {
        let targets = self.recon_targets.in_scope_targets(None).await?;
        let allowed = golish_db::repo::organizations::subtree_id_str_set(&self.pool, org_id).await;
        let owned: Vec<golish_app_core::domain::targets::Target> = targets
            .into_iter()
            .filter(|t| {
                golish_db::repo::organizations::org_id_in_scope(
                    t.organization_id.as_deref(),
                    &allowed,
                )
            })
            .collect();
        let ranked = golish_app_core::domain::targets::rank_attack_surface_seeds(owned, cap);
        Ok(ranked
            .into_iter()
            .map(|t| {
                let priority = golish_app_core::domain::targets::attack_surface_priority(&t);
                json!({
                    "target_id": t.id,
                    "value": t.value,
                    "type": t.target_type.as_str(),
                    "source": t.source,
                    "status": t.status.as_str(),
                    "real_ip": t.real_ip,
                    "ports": t.ports,
                    "organization_id": t.organization_id,
                    "http_status": t.http_status,
                    "cdn_waf": t.cdn_waf,
                    "priority": priority,
                })
            })
            .collect())
    }

    pub(super) async fn stage_asset_coverage_impl(
        &self,
        organization_id: Uuid,
        stage: &str,
        session_id: Option<&str>,
        stage_started_at: Option<chrono::DateTime<chrono::Utc>>,
    ) -> anyhow::Result<serde_json::Value> {
        let stage_kind = golish_agent_kit::harness::StageKind::try_parse(stage)
            .ok_or_else(|| anyhow::anyhow!("unknown stage: {stage}"))?;
        let snapshot = crate::ai::commands::stage_coverage::stage_asset_coverage_snapshot(
            &self.pool,
            organization_id,
            stage_kind,
            session_id,
            stage_started_at,
            false,
        )
        .await?;
        Ok(serde_json::to_value(snapshot)?)
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

    /// EAS host-aware alias exclusion (设计 2026-06-30-eas-domain-port-
    /// delegation): in-scope asset values whose resolved IP is already an
    /// in-scope IP target. Reuses the recon targets port (mirrors
    /// [`Self::in_scope_typed_assets_impl`]; no new SQL). org_id narrowing is
    /// deferred (chat sessions carry no org binding) — a superset is harmless:
    /// the gate only excludes aliases that are ALSO on its org-narrowed asset
    /// axis. Domains without such an IP remain liveness-only; PORT/SERVICE
    /// applies only to concrete IP/CIDR hosts.
    pub(super) async fn eas_port_delegated_assets_impl(
        &self,
        _org_id: Option<Uuid>,
    ) -> anyhow::Result<Vec<String>> {
        let targets = self.recon_targets.in_scope_targets(None).await?;
        Ok(
            golish_app_core::domain::targets::eas_port_delegated_domain_values(&targets)
                .into_iter()
                .collect(),
        )
    }

    /// Enumeration IP-web coverage (design 2026-07-01): IP/CIDR targets that
    /// EAS/httpx has proven are HTTP services via `targets.http_status`.
    pub(super) async fn enumeration_web_capable_assets_impl(
        &self,
        org_id: Option<Uuid>,
    ) -> anyhow::Result<Vec<String>> {
        Ok(
            golish_db::repo::coverage_truth::web_capable_ip_assets(&self.pool, org_id)
                .await?
                .into_iter()
                .collect(),
        )
    }

    pub(super) async fn eas_service_not_applicable_assets_impl(
        &self,
        org_id: Option<Uuid>,
        run_start: Option<chrono::DateTime<chrono::Utc>>,
    ) -> anyhow::Result<Vec<String>> {
        Ok(
            golish_db::repo::coverage_truth::eas_service_not_applicable_assets(
                &self.pool,
                org_id,
                run_start,
            )
            .await?
            .into_iter()
            .collect(),
        )
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
    pub(super) async fn org_subtree_ids_impl(&self, root_id: Uuid) -> anyhow::Result<Vec<Uuid>> {
        Ok(golish_db::repo::organizations::subtree_ids(&self.pool, root_id).await?)
    }

    pub(super) async fn org_subtree_units_impl(
        &self,
        root_id: Uuid,
    ) -> anyhow::Result<Vec<OrgScopeUnit>> {
        Ok(golish_db::repo::organizations::subtree(&self.pool, root_id)
            .await?
            .into_iter()
            .map(|org| OrgScopeUnit {
                id: org.id,
                name: org.name,
                parent_id: org.parent_id,
            })
            .collect())
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

#[cfg(test)]
mod tests {
    use super::*;
    use golish_app_core::domain::targets::{Scope, TargetStatus};

    fn target(value: &str, target_type: TargetType) -> Target {
        Target {
            id: "00000000-0000-0000-0000-000000000001".to_string(),
            name: value.to_string(),
            target_type,
            value: value.to_string(),
            tags: Vec::new(),
            notes: String::new(),
            scope: Scope::InScope,
            status: TargetStatus::Active,
            grp: "default".to_string(),
            owner: String::new(),
            time_window_start: None,
            time_window_end: None,
            organization_id: Some("00000000-0000-0000-0000-000000000002".to_string()),
            source: "external_attack_surface".to_string(),
            parent_id: None,
            ports: Vec::new(),
            real_ip: String::new(),
            cdn_waf: String::new(),
            http_title: String::new(),
            http_status: None,
            webserver: String::new(),
            os_info: String::new(),
            content_type: String::new(),
            created_at: 0,
            updated_at: 0,
        }
    }

    #[test]
    fn derives_web_root_from_url_target() {
        let mut target = target("https://app.example.com/app", TargetType::Url);
        target.http_status = Some(200);
        let roots = derive_enumeration_web_roots(&target);
        assert_eq!(roots.len(), 1);
        assert_eq!(roots[0]["root_url"], "https://app.example.com/app/");
        assert_eq!(roots[0]["scheme"], "https");
        assert_eq!(roots[0]["needs_probe"], false);
    }

    #[test]
    fn derives_web_root_from_web_like_port() {
        let mut target = target("app.example.com", TargetType::Domain);
        target.ports = vec![json!({"port": 8443, "service": "https"})];
        let roots = derive_enumeration_web_roots(&target);
        assert_eq!(roots.len(), 1);
        assert_eq!(roots[0]["root_url"], "https://app.example.com:8443/");
        assert_eq!(roots[0]["port"], 8443);
    }

    #[test]
    fn cidr_does_not_become_web_root_without_materialized_host() {
        let target = target("10.0.0.0/24", TargetType::Cidr);
        assert!(derive_enumeration_web_roots(&target).is_empty());
    }

    #[test]
    fn coverage_summary_is_found_only() {
        let facts = vec![(
            "app.example.com".to_string(),
            golish_db::repo::coverage_truth::TECH_ENUM_JSAPI.to_string(),
        )];
        let summary = build_enumeration_coverage_summary(&facts);
        let techniques = summary["techniques"].as_array().unwrap();
        let jsapi = techniques
            .iter()
            .find(|row| row["technique"] == golish_db::repo::coverage_truth::TECH_ENUM_JSAPI)
            .unwrap();
        let dir = techniques
            .iter()
            .find(|row| row["technique"] == golish_db::repo::coverage_truth::TECH_ENUM_DIR)
            .unwrap();
        assert_eq!(jsapi["db_found"], true);
        assert_eq!(jsapi["status_hint"], "found");
        assert_eq!(dir["db_found"], false);
        assert_eq!(dir["status_hint"], "no_found_fact");
        assert_eq!(
            summary["semantics"],
            "found-only; absence is not checked_empty"
        );
    }
}
