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
        .filter_map(|key| v.get(*key).and_then(|value| value.as_str()))
        .map(str::trim)
        .filter(|value| !value.is_empty())
        .collect::<Vec<_>>()
        .join("/")
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

fn port_state_is_open(port_entry: &serde_json::Value) -> bool {
    port_entry
        .get("state")
        .and_then(serde_json::Value::as_str)
        .map(str::trim)
        .filter(|state| !state.is_empty())
        .map(|state| state.eq_ignore_ascii_case("open"))
        .unwrap_or(false)
}

fn enumeration_web_root(
    target: &Target,
    origin: &golish_pentest_domain::WebOriginKey,
    confidence: &str,
    needs_probe: bool,
) -> serde_json::Value {
    json!({
        "web_root_id": format!("derived:{}:{}", target.id, origin.key),
        "target_id": target.id,
        "organization_id": target.organization_id,
        "root_url": origin.root_url,
        "final_url": origin.root_url,
        "scheme": origin.scheme,
        "host": origin.host,
        "port": origin.port,
        "status": target.http_status,
        "title": target.http_title,
        "confidence": confidence,
        "needs_probe": needs_probe,
        "source_stage": "external_attack_surface",
        "exact_web_origin": true,
    })
}

fn derive_enumeration_web_roots(target: &Target) -> Vec<serde_json::Value> {
    let value = target.value.trim();
    if value.is_empty() {
        return Vec::new();
    }

    if let Some(origin) = golish_pentest_domain::canonical_web_origin(value) {
        return vec![enumeration_web_root(
            target,
            &origin,
            if target.http_status.is_some() {
                "high"
            } else {
                "medium"
            },
            target.http_status.is_none(),
        )];
    }

    if matches!(target.target_type, TargetType::Url) {
        return Vec::new();
    }

    if matches!(target.target_type, TargetType::Cidr | TargetType::Wildcard) {
        return Vec::new();
    }

    let mut roots = Vec::new();
    let mut seen = std::collections::BTreeSet::new();
    for port_entry in &target.ports {
        if !port_state_is_open(port_entry) {
            continue;
        }
        let port = json_port(port_entry);
        let service = json_string(port_entry, &["service", "name", "protocol", "proto"]);
        let explicit_origin = port_entry
            .get("url")
            .and_then(serde_json::Value::as_str)
            .and_then(golish_pentest_domain::canonical_web_origin);
        let origin = explicit_origin.or_else(|| {
            golish_pentest_domain::canonical_web_origin_from_service(value, port?, &service)
        });
        let Some(origin) = origin else {
            continue;
        };
        if seen.insert(origin.key.clone()) {
            roots.push(enumeration_web_root(target, &origin, "high", false));
        }
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
                "observed_business_fact": found.contains(*technique),
                "status_hint": if found.contains(*technique) { "observation_only" } else { "no_observation" },
            })
        })
        .collect();
    json!({
        "source": "coverage_truth_legacy_observation",
        "authoritative": false,
        "exact_origin": false,
        "semantics": "advisory business observations only; never closes exact-origin Enumeration coverage",
        "authoritative_source": "stage_worklist_status/check_stage_asset_coverage",
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
                    "source": "coverage_truth_legacy_observation",
                    "authoritative": false,
                    "semantics": "advisory business observations only; never closes exact-origin Enumeration coverage",
                    "authoritative_source": "stage_worklist_status/check_stage_asset_coverage",
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
        // Harness truth is engagement-owned. Binding NULL to the exact predicate
        // deliberately returns no rows: an operation without a confirmed org may
        // not inherit another engagement's targets from the shared database.
        Ok(sqlx::query_scalar(in_scope_assets_sql())
            .bind(org_id)
            .fetch_all(self.pool.as_ref())
            .await?)
    }

    pub(super) async fn in_scope_assets_created_before_impl(
        &self,
        org_id: Option<Uuid>,
        cutoff: chrono::DateTime<chrono::Utc>,
    ) -> anyhow::Result<Vec<String>> {
        Ok(sqlx::query_scalar(in_scope_assets_created_before_sql())
            .bind(org_id)
            .bind(cutoff)
            .fetch_all(self.pool.as_ref())
            .await?)
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
        let Some(org_id) = org_id else {
            return Ok(Vec::new());
        };
        // This is deliberately read-only. DNS/subdomain landing belongs to the
        // successful `recon_map_assets` write path; refreshing here made innocent
        // Scoping/coverage reads contact DNS and silently execute Target Intel work.
        // A missing landing row must stay visible as a coverage gap, not be repaired
        // as a side effect of reading gate truth.
        // 2c-2 (设计 host-aware-coverage-2c §4.3): align each in-scope asset to
        // its targets.type so coverage_truth drops domain-only org facts (CT) on
        // IP assets. Missing type → "" (non-IP, keep all — fail-safe). Reuses the
        // 2c-1 typed in-scope read.
        let type_map: std::collections::HashMap<String, String> = self
            .in_scope_typed_assets_impl(Some(org_id))
            .await?
            .into_iter()
            .collect();
        let types: Vec<String> = in_scope_assets
            .iter()
            .map(|a| type_map.get(a).cloned().unwrap_or_default())
            .collect();
        let rows = golish_db::repo::coverage_truth::coverage_truth_facts(
            &self.pool,
            Some(org_id),
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
        let Some(org_id) = org_id else {
            return Ok(Vec::new());
        };
        let targets = self.recon_targets.in_scope_targets(None).await?;
        // Engagement-org isolation (设计 2026-06-15-engagement-org-isolation):
        // confine the listing to the scoping-confirmed engagement org subtree
        // (root + subsidiaries). A missing org returns no rows above. Once an
        // org is bound, targets with no org binding are excluded (fail-closed:
        // an unowned row is not "this engagement's").
        // Shares the db helper with the `manage_targets` list path so the two
        // org-confinement reads never drift.
        let allowed =
            golish_db::repo::organizations::subtree_id_str_set(&self.pool, Some(org_id)).await;
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
                    // Dead-asset P3 (design 2026-07-02-dead-asset-liveness-state
                    // §5.3): surface liveness so the EAS/enumeration specialist can
                    // see + deprioritise confirmed-dead assets. Read-only context.
                    "liveness_state": t.liveness_state,
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
        let Some(org_id) = org_id else {
            return Ok(Vec::new());
        };
        let targets = self.recon_targets.in_scope_targets(None).await?;
        let allowed =
            golish_db::repo::organizations::subtree_id_str_set(&self.pool, Some(org_id)).await;
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
                    // Dead-asset P3 (design 2026-07-02-dead-asset-liveness-state
                    // §5.3): carry liveness so the EAS specialist can deprioritise
                    // confirmed-dead seeds. Read-only context.
                    "liveness_state": t.liveness_state,
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
        current_wave_target_ids: Option<Vec<Uuid>>,
        current_wave_asset_values: Option<Vec<String>>,
        operation_id: Option<Uuid>,
    ) -> anyhow::Result<serde_json::Value> {
        let stage_kind = golish_agent_kit::harness::StageKind::try_parse(stage)
            .ok_or_else(|| anyhow::anyhow!("unknown stage: {stage}"))?;
        let snapshot = crate::ai::commands::stage_coverage::stage_asset_coverage_snapshot(
            &self.pool,
            organization_id,
            stage_kind,
            session_id,
            stage_started_at,
            current_wave_target_ids,
            current_wave_asset_values,
            false,
            operation_id,
        )
        .await?;
        Ok(serde_json::to_value(snapshot)?)
    }

    /// P3 Phase B (2026-06-11): distinct `targets.type` of the in-scope assets,
    /// so the harness coverage gate derives dynamic expected techniques per asset
    /// class (`technique_resolver`). Organization narrowing is mandatory: a
    /// sibling row with the same value but another type must never rewrite the
    /// current operation's coverage denominator.
    pub(super) async fn in_scope_target_types_impl(
        &self,
        org_id: Option<Uuid>,
    ) -> anyhow::Result<Vec<String>> {
        let targets: Vec<(String,)> = sqlx::query_as(in_scope_target_types_sql())
            .bind(org_id)
            .fetch_all(self.pool.as_ref())
            .await?;
        let mut types: Vec<String> = Vec::new();
        for (ty,) in targets {
            if !types.contains(&ty) {
                types.push(ty);
            }
        }
        Ok(types)
    }

    /// 2c-1 (设计 host-aware-coverage-2c §4.1): in-scope `(value, targets.type)`
    /// pairs for the harness gate's **authoritative** asset classification.
    /// Uses the same exact org predicate as the value axis. A workspace-wide
    /// superset is unsafe because duplicate values across sibling orgs can carry
    /// different authoritative target types.
    pub(super) async fn in_scope_typed_assets_impl(
        &self,
        org_id: Option<Uuid>,
    ) -> anyhow::Result<Vec<(String, String)>> {
        Ok(sqlx::query_as(in_scope_typed_assets_sql())
            .bind(org_id)
            .fetch_all(self.pool.as_ref())
            .await?)
    }

    pub(super) async fn scoping_target_snapshot_impl(
        &self,
        organization_id: Uuid,
    ) -> anyhow::Result<Vec<golish_agent_kit::db_traits::ScopingReviewedTarget>> {
        let rows: Vec<(String, String, String)> = sqlx::query_as(scoping_target_snapshot_sql())
            .bind(organization_id)
            .fetch_all(self.pool.as_ref())
            .await?;
        Ok(rows
            .into_iter()
            .map(
                |(value, target_type, scope)| golish_agent_kit::db_traits::ScopingReviewedTarget {
                    value,
                    target_type,
                    scope,
                },
            )
            .collect())
    }

    pub(super) async fn active_recon_scope_review_candidates_impl(
        &self,
        operation_id: Uuid,
        organization_id: Uuid,
    ) -> anyhow::Result<Vec<golish_agent_kit::db_traits::ScopingReviewedTarget>> {
        let rows: Vec<(String, String, String)> =
            sqlx::query_as(active_recon_scope_review_candidates_sql())
                .bind(operation_id)
                .bind(organization_id)
                .fetch_all(self.pool.as_ref())
                .await?;
        Ok(rows
            .into_iter()
            .map(
                |(value, target_type, scope)| golish_agent_kit::db_traits::ScopingReviewedTarget {
                    value,
                    target_type,
                    scope,
                },
            )
            .collect())
    }

    pub(super) async fn active_recon_scope_review_apply_impl(
        &self,
        operation_id: Uuid,
        organization_id: Uuid,
        approval: golish_agent_kit::db_traits::ActiveReconScopeReviewApproval,
    ) -> anyhow::Result<Vec<golish_agent_kit::db_traits::ScopingReviewedTarget>> {
        use std::collections::BTreeSet;

        let exact = |row: &golish_agent_kit::db_traits::ScopingReviewedTarget| {
            (
                row.value.trim().to_string(),
                row.target_type.trim().to_ascii_lowercase(),
                row.scope.trim().to_ascii_lowercase(),
            )
        };
        let valid = |row: &golish_agent_kit::db_traits::ScopingReviewedTarget| {
            !row.value.trim().is_empty()
                && matches!(
                    row.target_type.trim().to_ascii_lowercase().as_str(),
                    "domain" | "ip" | "cidr" | "url" | "wildcard"
                )
                && row.scope.trim().eq_ignore_ascii_case("in")
        };
        if approval.presented.is_empty()
            || approval.selected.is_empty()
            || !approval.presented.iter().all(valid)
            || !approval.selected.iter().all(valid)
        {
            anyhow::bail!("ACTIVE_RECON_SCOPE_INVALID_REVIEW_ROWS");
        }
        let presented_set = approval
            .presented
            .iter()
            .map(exact)
            .collect::<BTreeSet<_>>();
        let selected_set = approval.selected.iter().map(exact).collect::<BTreeSet<_>>();
        if presented_set.len() != approval.presented.len()
            || selected_set.len() != approval.selected.len()
            || !selected_set.is_subset(&presented_set)
        {
            anyhow::bail!("ACTIVE_RECON_SCOPE_REVIEW_NOT_UNCHANGED_SUBSET");
        }

        let mut tx = self.pool.begin().await?;
        let operation: Option<(String, Option<Uuid>)> =
            sqlx::query_as(active_recon_scope_operation_lock_sql())
                .bind(operation_id)
                .fetch_optional(&mut *tx)
                .await?;
        let Some((current_stage, engagement_org_id)) = operation else {
            anyhow::bail!("ACTIVE_RECON_SCOPE_OPERATION_NOT_FOUND");
        };
        if current_stage != "target_intel" || engagement_org_id != Some(organization_id) {
            anyhow::bail!("ACTIVE_RECON_SCOPE_OPERATION_BINDING_MISMATCH");
        }

        let current_rows: Vec<(String, String, String)> =
            sqlx::query_as(active_recon_scope_review_candidates_sql())
                .bind(operation_id)
                .bind(organization_id)
                .fetch_all(&mut *tx)
                .await?;
        let current = current_rows
            .into_iter()
            .map(
                |(value, target_type, scope)| golish_agent_kit::db_traits::ScopingReviewedTarget {
                    value,
                    target_type,
                    scope,
                },
            )
            .collect::<Vec<_>>();
        let current_set = current.iter().map(exact).collect::<BTreeSet<_>>();
        if current_set.len() != current.len() || current_set != presented_set {
            anyhow::bail!("ACTIVE_RECON_SCOPE_CANDIDATE_SNAPSHOT_CHANGED");
        }

        for row in &approval.presented {
            let selected = selected_set.contains(&exact(row));
            let result = sqlx::query(active_recon_scope_target_update_sql())
                .bind(organization_id)
                .bind(row.target_type.trim().to_ascii_lowercase())
                .bind(row.value.trim())
                .bind(selected)
                .execute(&mut *tx)
                .await?;
            if result.rows_affected() != 1 {
                anyhow::bail!("ACTIVE_RECON_SCOPE_TARGET_IDENTITY_CHANGED");
            }
        }

        let marker = serde_json::json!({
            "schema_version": 1,
            "operation_id": operation_id,
            "organization_id": organization_id,
            "request_id": approval.request_id,
            "presented": approval.presented,
            "selected": approval.selected,
        });
        let updated = sqlx::query(active_recon_scope_state_update_sql())
            .bind(operation_id)
            .bind(organization_id)
            .bind(&marker)
            .execute(&mut *tx)
            .await?;
        if updated.rows_affected() != 1 {
            anyhow::bail!("ACTIVE_RECON_SCOPE_OPERATION_STATE_CHANGED");
        }
        sqlx::query(active_recon_scope_audit_insert_sql())
            .bind(operation_id)
            .bind(organization_id.to_string())
            .bind(&marker)
            .execute(&mut *tx)
            .await?;
        tx.commit().await?;
        Ok(approval.selected)
    }

    pub(super) async fn active_recon_scope_review_authorized_impl(
        &self,
        operation_id: Uuid,
        organization_id: Uuid,
    ) -> anyhow::Result<bool> {
        let marker: Option<(serde_json::Value,)> =
            sqlx::query_as(active_recon_scope_authorization_sql())
                .bind(operation_id)
                .bind(organization_id)
                .fetch_optional(self.pool.as_ref())
                .await?;
        let Some((marker,)) = marker else {
            return Ok(false);
        };
        let operation_id_text = operation_id.to_string();
        let organization_id_text = organization_id.to_string();
        if marker
            .get("schema_version")
            .and_then(serde_json::Value::as_u64)
            != Some(1)
            || marker
                .get("operation_id")
                .and_then(serde_json::Value::as_str)
                != Some(operation_id_text.as_str())
            || marker
                .get("organization_id")
                .and_then(serde_json::Value::as_str)
                != Some(organization_id_text.as_str())
        {
            return Ok(false);
        }
        let selected: Vec<golish_agent_kit::db_traits::ScopingReviewedTarget> = marker
            .get("selected")
            .cloned()
            .and_then(|value| serde_json::from_value(value).ok())
            .unwrap_or_default();
        if selected.is_empty() {
            return Ok(false);
        }
        let trusted = self
            .scoping_target_snapshot_impl(organization_id)
            .await?
            .into_iter()
            .filter(|row| row.scope.trim().eq_ignore_ascii_case("in"))
            .collect::<Vec<_>>();
        let exact = |row: &golish_agent_kit::db_traits::ScopingReviewedTarget| {
            (
                row.value.trim().to_string(),
                row.target_type.trim().to_ascii_lowercase(),
                row.scope.trim().to_ascii_lowercase(),
            )
        };
        let selected = selected
            .iter()
            .map(exact)
            .collect::<std::collections::BTreeSet<_>>();
        let trusted = trusted
            .iter()
            .map(exact)
            .collect::<std::collections::BTreeSet<_>>();
        Ok(selected == trusted)
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
        org_id: Option<Uuid>,
    ) -> anyhow::Result<Vec<String>> {
        let Some(org_id) = org_id else {
            return Ok(Vec::new());
        };
        let targets = self.recon_targets.in_scope_targets(None).await?;
        let allowed =
            golish_db::repo::organizations::subtree_id_str_set(&self.pool, Some(org_id)).await;
        let owned: Vec<_> = targets
            .into_iter()
            .filter(|target| {
                golish_db::repo::organizations::org_id_in_scope(
                    target.organization_id.as_deref(),
                    &allowed,
                )
            })
            .collect();
        Ok(
            golish_app_core::domain::targets::eas_port_delegated_domain_values(&owned)
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
        let Some(org_id) = org_id else {
            return Ok(Vec::new());
        };
        Ok(
            golish_db::repo::coverage_truth::web_capable_ip_assets(&self.pool, Some(org_id))
                .await?
                .into_iter()
                .collect(),
        )
    }

    /// EAS web-stack coverage: all in-scope assets with a confirmed HTTP(S)
    /// surface, not just IP/CIDR enumeration roots.
    pub(super) async fn eas_web_capable_assets_impl(
        &self,
        org_id: Option<Uuid>,
        run_start: Option<chrono::DateTime<chrono::Utc>>,
    ) -> anyhow::Result<Vec<String>> {
        let Some(org_id) = org_id else {
            return Ok(Vec::new());
        };
        Ok(golish_db::repo::coverage_truth::eas_web_capable_assets(
            &self.pool,
            Some(org_id),
            run_start,
        )
        .await?
        .into_iter()
        .collect())
    }

    /// Dead-asset P3 (design 2026-07-02-dead-asset-liveness-state §5.1): in-scope
    /// assets EAS confirmed dead (`liveness_state = 'dead'`), for a downstream
    /// stage gate that opts into `skip_dead_assets` to drop from its denominator.
    pub(super) async fn dead_asset_values_impl(
        &self,
        org_id: Option<Uuid>,
    ) -> anyhow::Result<Vec<String>> {
        let Some(org_id) = org_id else {
            return Ok(Vec::new());
        };
        Ok(
            golish_db::repo::coverage_truth::dead_asset_values(&self.pool, Some(org_id))
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
        let Some(org_id) = org_id else {
            return Ok(Vec::new());
        };
        Ok(
            golish_db::repo::coverage_truth::eas_service_not_applicable_assets(
                &self.pool,
                Some(org_id),
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

    pub(super) async fn org_stage_completions_get_with_run_id_impl(
        &self,
        stage_kind: &str,
        org_ids: &[Uuid],
    ) -> anyhow::Result<Vec<(Uuid, chrono::DateTime<chrono::Utc>, Option<String>)>> {
        let mut out = Vec::with_capacity(org_ids.len());
        for &org in org_ids {
            if let Some(row) =
                golish_db::repo::org_stage_completions::get(&self.pool, org, stage_kind).await?
            {
                out.push((row.organization_id, row.passed_at, row.stage_run_id));
            }
        }
        Ok(out)
    }
}

fn in_scope_assets_sql() -> &'static str {
    r#"SELECT DISTINCT value
       FROM targets
       WHERE scope::text = 'in'
         AND organization_id = $1
       ORDER BY value"#
}

fn in_scope_assets_created_before_sql() -> &'static str {
    r#"SELECT DISTINCT value
       FROM targets
       WHERE scope::text = 'in'
         AND organization_id = $1
         AND created_at <= $2
       ORDER BY value"#
}

fn in_scope_target_types_sql() -> &'static str {
    r#"SELECT DISTINCT target_type::text
       FROM targets
       WHERE scope::text = 'in'
         AND organization_id = $1
       ORDER BY target_type::text"#
}

fn in_scope_typed_assets_sql() -> &'static str {
    r#"SELECT value, target_type::text
       FROM targets
       WHERE scope::text = 'in'
         AND organization_id = $1
       ORDER BY created_at ASC, id ASC"#
}

fn scoping_target_snapshot_sql() -> &'static str {
    r#"SELECT value, target_type::text, scope::text
       FROM targets
       WHERE organization_id = $1
         AND target_type::text IN ('domain', 'ip', 'cidr', 'url', 'wildcard')
         AND lower(COALESCE(source, '')) IN
             ('manual', 'imported', 'customer_provided', 'stage-run-seed', 'seed', 'cli')
       ORDER BY created_at ASC, id ASC"#
}

fn active_recon_scope_review_candidates_sql() -> &'static str {
    r#"SELECT target.value, target.target_type::text, target.scope::text
       FROM operation_state operation
       JOIN targets target
         ON target.organization_id = operation.engagement_org_id
       WHERE operation.operation_id = $1
         AND operation.engagement_org_id = $2
         AND operation.current_stage = 'target_intel'
         AND target.organization_id = $2
         AND target.scope::text = 'in'
         AND target.target_type::text IN ('domain', 'ip', 'cidr', 'url', 'wildcard')
         AND lower(COALESCE(target.source, '')) IN
             ('manual', 'imported', 'customer_provided', 'stage-run-seed', 'seed', 'cli', 'asset_intel')
         AND EXISTS (
             SELECT 1
             FROM targets refreshed
             WHERE refreshed.organization_id = operation.engagement_org_id
               AND refreshed.scope::text = 'in'
               AND lower(COALESCE(refreshed.source, '')) = 'asset_intel'
               AND refreshed.updated_at >= operation.stage_started_at
         )
       ORDER BY target.created_at ASC, target.id ASC"#
}

fn active_recon_scope_operation_lock_sql() -> &'static str {
    r#"SELECT current_stage, engagement_org_id
       FROM operation_state
       WHERE operation_id = $1
       FOR UPDATE"#
}

fn active_recon_scope_target_update_sql() -> &'static str {
    r#"UPDATE targets
       SET scope = CASE WHEN $4 THEN 'in'::scope_type ELSE 'out'::scope_type END,
           source = CASE
               WHEN $4 AND lower(COALESCE(source, '')) = 'asset_intel'
                   THEN 'customer_provided'
               ELSE source
           END,
           updated_at = NOW()
       WHERE organization_id = $1
         AND target_type::text = $2
         AND value = $3
         AND scope::text = 'in'
         AND lower(COALESCE(source, '')) IN
             ('manual', 'imported', 'customer_provided', 'stage-run-seed', 'seed', 'cli', 'asset_intel')"#
}

fn active_recon_scope_state_update_sql() -> &'static str {
    r#"UPDATE operation_state
       SET state_blob = jsonb_set(
           COALESCE(state_blob, '{}'::jsonb),
           '{active_recon_target_scope}',
           $3::jsonb,
           true
       )
       WHERE operation_id = $1
         AND engagement_org_id = $2
         AND current_stage = 'target_intel'"#
}

fn active_recon_scope_audit_insert_sql() -> &'static str {
    r#"INSERT INTO audit_log
          (action, category, details, entity_type, entity_id, source,
           session_id, tool_name, status, detail, run_id, audit_role)
       VALUES
          ('active_recon_target_scope_approved', 'authorization',
           'Human approved exact targets for active reconnaissance',
           'organization', $2, 'active_recon_target_review',
           $1::text, 'scope_review', 'completed', $3::jsonb, $1, 'action')"#
}

fn active_recon_scope_authorization_sql() -> &'static str {
    r#"SELECT state_blob -> 'active_recon_target_scope'
       FROM operation_state
       WHERE operation_id = $1
         AND engagement_org_id = $2
         AND state_blob ? 'active_recon_target_scope'"#
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
            liveness_state: None,
            liveness_reason: None,
            created_at: 0,
            updated_at: 0,
        }
    }

    #[test]
    fn authoritative_asset_queries_require_an_exact_org_scope() {
        for sql in [
            in_scope_assets_sql(),
            in_scope_assets_created_before_sql(),
            in_scope_target_types_sql(),
            in_scope_typed_assets_sql(),
        ] {
            assert!(sql.contains("scope::text = 'in'"));
            assert!(sql.contains("organization_id = $1"));
            assert!(!sql.contains("$1 IS NULL"));
        }
    }

    #[test]
    fn scoping_snapshot_trusts_customer_intake_but_not_discovery_sources() {
        let sql = scoping_target_snapshot_sql();
        assert!(sql.contains("'customer_provided'"));
        assert!(!sql.contains("'active_discovered'"));
        assert!(!sql.contains("'asset_intel'"));
    }

    #[test]
    fn active_recon_scope_candidates_are_operation_org_stage_and_window_bound() {
        let sql = active_recon_scope_review_candidates_sql();
        assert!(sql.contains("operation.operation_id = $1"));
        assert!(sql.contains("operation.engagement_org_id = $2"));
        assert!(sql.contains("operation.current_stage = 'target_intel'"));
        assert!(sql.contains("refreshed.updated_at >= operation.stage_started_at"));
        assert!(sql.contains("'asset_intel'"));
        assert!(!sql.contains("active_discovered"));
    }

    #[test]
    fn active_recon_scope_apply_is_exact_and_operation_bound() {
        let update = active_recon_scope_target_update_sql();
        assert!(update.contains("organization_id = $1"));
        assert!(update.contains("target_type::text = $2"));
        assert!(update.contains("value = $3"));
        assert!(update.contains("customer_provided"));
        let state = active_recon_scope_state_update_sql();
        assert!(state.contains("active_recon_target_scope"));
        assert!(state.contains("current_stage = 'target_intel'"));
    }

    #[test]
    fn derives_web_root_from_url_target() {
        let mut target = target("https://app.example.com/app", TargetType::Url);
        target.http_status = Some(200);
        let roots = derive_enumeration_web_roots(&target);
        assert_eq!(roots.len(), 1);
        assert_eq!(roots[0]["root_url"], "https://app.example.com:443/");
        assert_eq!(roots[0]["scheme"], "https");
        assert_eq!(roots[0]["port"], 443);
        assert_eq!(roots[0]["needs_probe"], false);
    }

    #[test]
    fn url_shaped_value_overrides_stale_target_type_and_scheme_case() {
        let target = target("HTTPS://APP.Example.com/app", TargetType::Domain);

        let roots = derive_enumeration_web_roots(&target);
        assert_eq!(roots.len(), 1);
        assert_eq!(roots[0]["root_url"], "https://app.example.com:443/");
    }

    #[test]
    fn web_root_url_target_preserves_nondefault_scheme_and_port() {
        let target = target("http://app.example.com:8443/login", TargetType::Url);
        let roots = derive_enumeration_web_roots(&target);
        assert_eq!(roots.len(), 1);
        assert_eq!(roots[0]["root_url"], "http://app.example.com:8443/");
        assert_eq!(roots[0]["scheme"], "http");
        assert_eq!(roots[0]["port"], 8443);
    }

    #[test]
    fn derives_web_root_from_web_like_port() {
        let mut target = target("app.example.com", TargetType::Domain);
        target.ports = vec![json!({"port": 8443, "state": "open", "service": "https"})];
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
    fn host_metadata_without_exact_url_or_port_does_not_guess_an_origin() {
        let mut target = target("app.example.com", TargetType::Domain);
        target.http_status = Some(200);
        assert!(derive_enumeration_web_roots(&target).is_empty());
    }

    #[test]
    fn common_web_port_with_unknown_tcp_service_does_not_guess_an_origin() {
        let mut target = target("app.example.com", TargetType::Domain);
        target.ports = vec![json!({
            "port": 80,
            "service": "unknown",
            "protocol": "tcp",
            "state": "open"
        })];
        assert!(derive_enumeration_web_roots(&target).is_empty());
    }

    #[test]
    fn closed_port_url_does_not_become_enumeration_root() {
        let mut target = target("app.example.com", TargetType::Domain);
        target.ports = vec![json!({
            "port": 8443,
            "service": "https",
            "state": "closed",
            "url": "https://app.example.com:8443/"
        })];
        assert!(derive_enumeration_web_roots(&target).is_empty());
    }

    #[test]
    fn port_without_confirmed_open_state_does_not_become_enumeration_root() {
        let mut target = target("app.example.com", TargetType::Domain);
        target.ports = vec![json!({
            "port": 8443,
            "service": "https",
            "url": "https://app.example.com:8443/"
        })];

        assert!(derive_enumeration_web_roots(&target).is_empty());
    }

    #[test]
    fn service_metadata_uses_all_hints_without_guessing_tls_from_port_number() {
        let mut target = target("app.example.com", TargetType::Domain);
        target.ports = vec![
            json!({
                "port": 8080,
                "state": "open",
                "service": "tcp",
                "name": "https-alt"
            }),
            json!({
                "port": 8443,
                "state": "open",
                "service": "http"
            }),
        ];

        let roots = derive_enumeration_web_roots(&target);
        let urls = roots
            .iter()
            .filter_map(|root| root["root_url"].as_str())
            .collect::<Vec<_>>();
        assert_eq!(
            urls,
            vec![
                "https://app.example.com:8080/",
                "http://app.example.com:8443/"
            ]
        );
    }

    #[test]
    fn legacy_coverage_summary_is_explicitly_advisory_only() {
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
        assert_eq!(jsapi["observed_business_fact"], true);
        assert_eq!(jsapi["status_hint"], "observation_only");
        assert_eq!(dir["observed_business_fact"], false);
        assert_eq!(dir["status_hint"], "no_observation");
        assert_eq!(summary["authoritative"], false);
        assert_eq!(
            summary["semantics"],
            "advisory business observations only; never closes exact-origin Enumeration coverage"
        );
    }
}
