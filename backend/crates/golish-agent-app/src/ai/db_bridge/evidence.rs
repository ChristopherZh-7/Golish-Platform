//! `GolishDbRepoProvider` evidence-ledger methods (P0 · OpenFang hash chain).
//!
//! `evidence_append_impl` orchestrates the golish-pentest hash-chain append over
//! the shared `PgPool`; `evidence_existing_ids_impl` backs the harness gate's
//! fabricated-ref check via a golish-db query. Both are surfaced on
//! `DbRepoProvider` (see `mod.rs`) so the orchestrator/runtime reach the ledger
//! without holding a raw pool.

use std::collections::{HashMap, HashSet};

use uuid::Uuid;

use golish_agent_kit::harness::wstg_mapping::{
    FORMULAIC_TECHNIQUES, GENERAL_NUCLEI_TECHNIQUES, GOLISH_NDAY, WSTG_ANONYMOUS_ACCESS,
};
use golish_agent_kit::harness::SourceQueryFact;
use golish_pentest::evidence_ledger::append::{append, append_for_organization, EvidenceInput};
use golish_pentest::evidence_ledger::{InMemoryScopeService, ScopeVersion};

use super::GolishDbRepoProvider;

pub(crate) type TargetBoundEvidenceFactSet = HashSet<(String, String, String, i64)>;
pub(crate) type EnumerationEvidenceFactSet = TargetBoundEvidenceFactSet;
const ENUM_PREFLIGHT_BLOCKED_TOOL: &str = "enum_preflight_web_origins";
const ENUM_PREFLIGHT_BLOCKED_KIND: &str = "enumeration_transport_blocked";
const ENUM_ROUTE_RECOVERY_BLOCKED_TOOL: &str = "route_probe_paths";
const ENUM_ROUTE_RECOVERY_BLOCKED_KIND: &str = "dir_probe_recovery_exhausted";
const ENUM_COLLECTION_RECOVERY_BLOCKED_TOOL: &str = "browser_collect_js_api";
const ENUM_COLLECTION_RECOVERY_BLOCKED_KIND: &str = "enumeration_collection_recovery_exhausted";
const EAS_WEB_BLOCKED_TOOL: &str = "whatweb";
const EAS_WEB_BLOCKED_KIND: &str = "eas.fingerprint_web_stack";
const EAS_WEB_BLOCKED_SOURCE: &str = "eas_fingerprint_web_stack";
const VULN_NUCLEI_EVIDENCE_KIND: &str = "vuln.nuclei_observation";
const VULN_NUCLEI_GENERAL_TOOL: &str = "vuln_nuclei_general";
const VULN_NUCLEI_TARGETED_TOOL: &str = "vuln_nuclei_fingerprint_targeted";
const VULN_ANONYMOUS_ACCESS_EVIDENCE_KIND: &str = "vuln.anonymous_access_observation";
const VULN_ANONYMOUS_ACCESS_TOOL: &str = "vuln_probe_anonymous_access";

fn is_vuln_technique(technique: &str) -> bool {
    FORMULAIC_TECHNIQUES.contains(&technique)
}

fn is_vuln_terminal_outcome(technique: &str, outcome: &str) -> bool {
    is_vuln_technique(technique)
        && matches!(outcome, "found" | "empty" | "blocked" | "not_applicable")
}

fn vuln_terminal_source_is_authoritative(
    technique: &str,
    outcome: &str,
    source: Option<&str>,
) -> bool {
    if !is_vuln_terminal_outcome(technique, outcome) {
        return true;
    }
    match technique {
        GOLISH_NDAY => source == Some(VULN_NUCLEI_TARGETED_TOOL),
        WSTG_ANONYMOUS_ACCESS => source == Some(VULN_ANONYMOUS_ACCESS_TOOL),
        technique if GENERAL_NUCLEI_TECHNIQUES.contains(&technique) => {
            source == Some(VULN_NUCLEI_GENERAL_TOOL)
        }
        _ => false,
    }
}

fn vuln_terminal_evidence_kind_is_authoritative(
    technique: &str,
    outcome: &str,
    evidence_kind: Option<&str>,
) -> bool {
    if !is_vuln_terminal_outcome(technique, outcome) {
        return true;
    }
    match technique {
        WSTG_ANONYMOUS_ACCESS => evidence_kind == Some(VULN_ANONYMOUS_ACCESS_EVIDENCE_KIND),
        GOLISH_NDAY => evidence_kind == Some(VULN_NUCLEI_EVIDENCE_KIND),
        technique if GENERAL_NUCLEI_TECHNIQUES.contains(&technique) => {
            evidence_kind == Some(VULN_NUCLEI_EVIDENCE_KIND)
        }
        _ => false,
    }
}

fn is_enumeration_technique(technique: &str) -> bool {
    matches!(
        technique,
        golish_db::repo::coverage_truth::TECH_ENUM_JS
            | golish_db::repo::coverage_truth::TECH_ENUM_DIR
            | golish_db::repo::coverage_truth::TECH_ENUM_PARAM
            | golish_db::repo::coverage_truth::TECH_ENUM_JSAPI
    )
}

fn is_enumeration_terminal_outcome(technique: &str, outcome: &str) -> bool {
    is_enumeration_technique(technique) && matches!(outcome, "found" | "empty" | "blocked")
}

fn enumeration_terminal_source_is_authoritative(
    technique: &str,
    outcome: &str,
    source: Option<&str>,
) -> bool {
    if !is_enumeration_terminal_outcome(technique, outcome) {
        return true;
    }
    if outcome == "blocked" {
        return enumeration_blocked_source_is_authoritative(technique, source);
    }
    match technique {
        golish_db::repo::coverage_truth::TECH_ENUM_JS => source == Some("browser_collect_js_api"),
        golish_db::repo::coverage_truth::TECH_ENUM_JSAPI => source == Some("js_extract_apis"),
        golish_db::repo::coverage_truth::TECH_ENUM_DIR => source == Some("route_probe_paths"),
        golish_db::repo::coverage_truth::TECH_ENUM_PARAM => source == Some("js_extract_apis"),
        _ => false,
    }
}

fn enumeration_blocked_source_is_authoritative(technique: &str, source: Option<&str>) -> bool {
    match source {
        Some(ENUM_PREFLIGHT_BLOCKED_TOOL) => is_enumeration_technique(technique),
        Some(ENUM_ROUTE_RECOVERY_BLOCKED_TOOL) => {
            technique == golish_db::repo::coverage_truth::TECH_ENUM_DIR
        }
        Some(ENUM_COLLECTION_RECOVERY_BLOCKED_TOOL) => matches!(
            technique,
            golish_db::repo::coverage_truth::TECH_ENUM_JS
                | golish_db::repo::coverage_truth::TECH_ENUM_PARAM
                | golish_db::repo::coverage_truth::TECH_ENUM_JSAPI
        ),
        _ => false,
    }
}

fn is_eas_technique(technique: &str) -> bool {
    matches!(
        technique,
        golish_db::repo::coverage_truth::TECH_EAS_LIVENESS
            | golish_db::repo::coverage_truth::TECH_EAS_PORT
            | golish_db::repo::coverage_truth::TECH_EAS_SERVICE_FP
            | golish_db::repo::coverage_truth::TECH_EAS_WEB_FP
    )
}

fn is_eas_terminal_outcome(technique: &str, outcome: &str) -> bool {
    (is_eas_technique(technique) && matches!(outcome, "found" | "empty"))
        || (technique == golish_db::repo::coverage_truth::TECH_EAS_WEB_FP && outcome == "blocked")
}

fn eas_terminal_source_is_authoritative(
    technique: &str,
    outcome: &str,
    source: Option<&str>,
) -> bool {
    technique != golish_db::repo::coverage_truth::TECH_EAS_WEB_FP
        || outcome != "blocked"
        || source == Some(EAS_WEB_BLOCKED_SOURCE)
}

fn strict_evidence_asset_key(asset: &str, technique: &str) -> Option<String> {
    if matches!(
        technique,
        golish_db::repo::coverage_truth::TECH_ENUM_JS
            | golish_db::repo::coverage_truth::TECH_ENUM_DIR
            | golish_db::repo::coverage_truth::TECH_ENUM_PARAM
            | golish_db::repo::coverage_truth::TECH_ENUM_JSAPI
    ) {
        return golish_pentest_domain::canonical_web_origin(asset).map(|origin| origin.key);
    }
    if is_vuln_technique(technique) {
        return golish_pentest_domain::canonical_web_origin(asset).map(|origin| origin.key);
    }
    if technique == golish_db::repo::coverage_truth::TECH_EAS_LIVENESS {
        return golish_agent_kit::harness::evidence_facts::eas_liveness_asset_key(asset);
    }
    if technique == golish_db::repo::coverage_truth::TECH_EAS_WEB_FP {
        return golish_pentest_domain::canonical_web_origin(asset).map(|origin| origin.key);
    }
    if is_eas_technique(technique) {
        return golish_pentest_domain::canonical_asset_key(asset).map(|key| key.key);
    }
    None
}

fn terminal_materialization_asset_key(asset: &str, technique: &str) -> String {
    strict_evidence_asset_key(asset, technique)
        .or_else(|| golish_pentest_domain::canonical_asset_key(asset).map(|key| key.key))
        .unwrap_or_else(|| asset.to_string())
}

fn dns_empty_outcome_hosts(
    summary: &golish_recon_app::organization_recon::PerAssetLandingSummary,
) -> &[String] {
    &summary.dns_empty_hosts
}

fn dns_found_outcome_hosts(
    summary: &golish_recon_app::organization_recon::PerAssetLandingSummary,
) -> &[String] {
    &summary.dns_found_hosts
}

fn dns_partial_outcome_hosts(
    summary: &golish_recon_app::organization_recon::PerAssetLandingSummary,
) -> &[String] {
    &summary.dns_partial_hosts
}

fn dns_error_outcome_hosts(
    summary: &golish_recon_app::organization_recon::PerAssetLandingSummary,
) -> &[String] {
    &summary.dns_error_hosts
}

/// Build the exact-origin evidence identity set used to validate Enumeration
/// terminal outcome refs. Invalid/non-HTTP(S) assets and non-positive ids are
/// deliberately omitted so malformed audit facts cannot close a coverage cell.
#[cfg(test)]
pub(crate) fn enumeration_evidence_fact_set(
    rows: impl IntoIterator<Item = (String, String, String, i64)>,
) -> EnumerationEvidenceFactSet {
    rows.into_iter()
        .filter_map(|(asset, technique, outcome, evidence_id)| {
            if evidence_id <= 0 {
                return None;
            }
            let asset = golish_pentest_domain::canonical_web_origin(&asset)?.key;
            Some((asset, technique, outcome, evidence_id))
        })
        .collect()
}

fn target_row_still_authorizes_origin(
    row: &golish_db::repo::audit::TargetBoundEvidenceFactRow,
    asset: &str,
) -> bool {
    let Some(asset) = golish_pentest_domain::canonical_web_origin(asset) else {
        return false;
    };
    golish_pentest_domain::confirmed_target_web_origins(
        &row.target_name,
        &row.target_value,
        &row.target_ports,
    )
    .into_iter()
    .any(|origin| origin.key == asset.key)
}

fn row_has_matching_producer_and_current_org(
    row: &golish_db::repo::audit::TargetBoundEvidenceFactRow,
) -> bool {
    row.evidence_organization_id
        .parse::<Uuid>()
        .ok()
        .is_some_and(|producer_org| Some(producer_org) == row.target_organization_id)
}

fn row_has_trusted_enumeration_blocked_producer(
    row: &golish_db::repo::audit::TargetBoundEvidenceFactRow,
) -> bool {
    if row.evidence_outcome != "blocked" {
        return true;
    }
    (is_enumeration_technique(&row.evidence_technique)
        && matches!(
            (row.tool_name.as_deref(), row.evidence_kind.as_deref()),
            (
                Some(ENUM_PREFLIGHT_BLOCKED_TOOL),
                Some(ENUM_PREFLIGHT_BLOCKED_KIND)
            )
        ))
        || (row.evidence_technique == golish_db::repo::coverage_truth::TECH_ENUM_DIR
            && matches!(
                (row.tool_name.as_deref(), row.evidence_kind.as_deref()),
                (
                    Some(ENUM_ROUTE_RECOVERY_BLOCKED_TOOL),
                    Some(ENUM_ROUTE_RECOVERY_BLOCKED_KIND)
                )
            ))
        || (matches!(
            row.evidence_technique.as_str(),
            golish_db::repo::coverage_truth::TECH_ENUM_JS
                | golish_db::repo::coverage_truth::TECH_ENUM_PARAM
                | golish_db::repo::coverage_truth::TECH_ENUM_JSAPI
        ) && matches!(
            (row.tool_name.as_deref(), row.evidence_kind.as_deref()),
            (
                Some(ENUM_COLLECTION_RECOVERY_BLOCKED_TOOL),
                Some(ENUM_COLLECTION_RECOVERY_BLOCKED_KIND)
            )
        ))
}

fn row_has_trusted_eas_blocked_producer(
    row: &golish_db::repo::audit::TargetBoundEvidenceFactRow,
) -> bool {
    row.evidence_outcome != "blocked"
        || (row.evidence_technique == golish_db::repo::coverage_truth::TECH_EAS_WEB_FP
            && matches!(
                (row.tool_name.as_deref(), row.evidence_kind.as_deref()),
                (Some(EAS_WEB_BLOCKED_TOOL), Some(EAS_WEB_BLOCKED_KIND))
            ))
}

fn open_port_urls(
    row: &golish_db::repo::audit::TargetBoundEvidenceFactRow,
) -> impl Iterator<Item = &str> {
    row.target_ports
        .as_array()
        .into_iter()
        .flatten()
        .filter(|entry| {
            entry
                .get("state")
                .and_then(serde_json::Value::as_str)
                .map(str::trim)
                .filter(|state| !state.is_empty())
                .map(|state| state.eq_ignore_ascii_case("open"))
                .unwrap_or(true)
        })
        .filter_map(|entry| entry.get("url").and_then(serde_json::Value::as_str))
}

fn target_row_still_authorizes_eas_fact(
    row: &golish_db::repo::audit::TargetBoundEvidenceFactRow,
) -> bool {
    let technique = row.evidence_technique.as_str();
    let Some(evidence_key) = strict_evidence_asset_key(&row.evidence_asset, technique) else {
        return false;
    };
    match technique {
        golish_db::repo::coverage_truth::TECH_EAS_LIVENESS => [&row.target_name, &row.target_value]
            .into_iter()
            .map(String::as_str)
            .chain(open_port_urls(row))
            .filter_map(golish_agent_kit::harness::evidence_facts::eas_liveness_asset_key)
            .any(|candidate| candidate == evidence_key),
        golish_db::repo::coverage_truth::TECH_EAS_PORT
        | golish_db::repo::coverage_truth::TECH_EAS_SERVICE_FP => {
            let class = golish_agent_kit::harness::technique_resolver::classify_stage_asset(
                golish_agent_kit::harness::StageKind::ExternalAttackSurface,
                Some(&row.target_type),
                &row.target_value,
            );
            matches!(
                class,
                golish_pentest_domain::AssetClass::Ip | golish_pentest_domain::AssetClass::Cidr
            ) && [&row.target_name, &row.target_value]
                .into_iter()
                .filter_map(|candidate| golish_pentest_domain::canonical_asset_key(candidate))
                .any(|candidate| candidate.key == evidence_key)
        }
        golish_db::repo::coverage_truth::TECH_EAS_WEB_FP => [&row.target_name, &row.target_value]
            .into_iter()
            .map(String::as_str)
            .chain(open_port_urls(row))
            .filter_map(golish_pentest_domain::canonical_web_origin)
            .any(|candidate| candidate.key == evidence_key),
        _ => false,
    }
}

/// Convert fresh DB evidence only while its original target still owns the
/// exact origin. This prevents a same-org/project target move from redirecting
/// old evidence to a sibling target that later takes over the origin.
pub(crate) fn enumeration_target_bound_evidence_fact_set(
    rows: impl IntoIterator<Item = golish_db::repo::audit::TargetBoundEvidenceFactRow>,
) -> EnumerationEvidenceFactSet {
    rows.into_iter()
        .filter(row_has_matching_producer_and_current_org)
        .filter(|row| target_row_still_authorizes_origin(row, &row.evidence_asset))
        .filter(row_has_trusted_enumeration_blocked_producer)
        .filter(|row| {
            enumeration_terminal_source_is_authoritative(
                &row.evidence_technique,
                &row.evidence_outcome,
                row.tool_name.as_deref(),
            )
        })
        .filter_map(|row| {
            if row.evidence_id <= 0 {
                return None;
            }
            let asset = golish_pentest_domain::canonical_web_origin(&row.evidence_asset)?.key;
            Some((
                asset,
                row.evidence_technique,
                row.evidence_outcome,
                row.evidence_id,
            ))
        })
        .collect()
}

pub(crate) fn vuln_target_bound_evidence_fact_set(
    organization_id: Uuid,
    rows: impl IntoIterator<Item = golish_db::repo::audit::TargetBoundEvidenceFactRow>,
) -> TargetBoundEvidenceFactSet {
    rows.into_iter()
        .filter(|row| {
            row.target_organization_id == Some(organization_id)
                && row.evidence_organization_id == organization_id.to_string()
        })
        .filter(row_has_matching_producer_and_current_org)
        .filter(|row| target_row_still_authorizes_origin(row, &row.evidence_asset))
        .filter(|row| {
            vuln_terminal_evidence_kind_is_authoritative(
                &row.evidence_technique,
                &row.evidence_outcome,
                row.evidence_kind.as_deref(),
            ) && vuln_terminal_source_is_authoritative(
                &row.evidence_technique,
                &row.evidence_outcome,
                row.tool_name.as_deref(),
            )
        })
        .filter_map(|row| {
            if row.evidence_id <= 0
                || !is_vuln_terminal_outcome(&row.evidence_technique, &row.evidence_outcome)
            {
                return None;
            }
            let asset = strict_evidence_asset_key(&row.evidence_asset, &row.evidence_technique)?;
            Some((
                asset,
                row.evidence_technique,
                row.evidence_outcome,
                row.evidence_id,
            ))
        })
        .collect()
}

pub(crate) fn eas_target_bound_evidence_fact_set(
    organization_id: Uuid,
    rows: impl IntoIterator<Item = golish_db::repo::audit::TargetBoundEvidenceFactRow>,
) -> TargetBoundEvidenceFactSet {
    rows.into_iter()
        .filter(|row| {
            row.target_organization_id == Some(organization_id)
                && row.evidence_organization_id == organization_id.to_string()
        })
        .filter(row_has_matching_producer_and_current_org)
        .filter(target_row_still_authorizes_eas_fact)
        .filter(row_has_trusted_eas_blocked_producer)
        .filter_map(|row| {
            if row.evidence_id <= 0
                || !is_eas_terminal_outcome(&row.evidence_technique, &row.evidence_outcome)
            {
                return None;
            }
            let asset = strict_evidence_asset_key(&row.evidence_asset, &row.evidence_technique)?;
            Some((
                asset,
                row.evidence_technique,
                row.evidence_outcome,
                row.evidence_id,
            ))
        })
        .collect()
}

pub(crate) fn eas_target_bound_evidence_facts(
    organization_id: Uuid,
    rows: impl IntoIterator<Item = golish_db::repo::audit::TargetBoundEvidenceFactRow>,
) -> Vec<(String, String, String, i64)> {
    let mut facts = eas_target_bound_evidence_fact_set(organization_id, rows)
        .into_iter()
        .collect::<Vec<_>>();
    facts.sort();
    facts
}

pub(crate) fn projected_technique_outcome_evidence_id(
    asset: &str,
    technique: &str,
    outcome: &str,
    evidence_ids: &[i64],
    enumeration_evidence_facts: &EnumerationEvidenceFactSet,
    source: Option<&str>,
) -> Option<i64> {
    if !enumeration_terminal_source_is_authoritative(technique, outcome, source) {
        return None;
    }
    if !eas_terminal_source_is_authoritative(technique, outcome, source) {
        return None;
    }
    if !vuln_terminal_source_is_authoritative(technique, outcome, source) {
        return None;
    }
    if is_enumeration_terminal_outcome(technique, outcome)
        || is_eas_terminal_outcome(technique, outcome)
        || is_vuln_terminal_outcome(technique, outcome)
    {
        let asset = strict_evidence_asset_key(asset, technique)?;
        return evidence_ids
            .iter()
            .copied()
            .filter(|id| *id > 0)
            .find(|id| {
                enumeration_evidence_facts.contains(&(
                    asset.clone(),
                    technique.to_string(),
                    outcome.to_string(),
                    *id,
                ))
            });
    }

    let real_evidence_id = evidence_ids.iter().copied().find(|id| *id > 0);
    Some(real_evidence_id.unwrap_or(0))
}

impl GolishDbRepoProvider {
    #[allow(clippy::too_many_arguments)]
    pub(crate) async fn evidence_append_impl(
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
        self.evidence_append_scoped_impl(
            operation_id,
            None,
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

    #[allow(clippy::too_many_arguments)]
    pub(crate) async fn evidence_append_for_organization_impl(
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
        self.evidence_append_scoped_impl(
            operation_id,
            Some(organization_id),
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

    #[allow(clippy::too_many_arguments)]
    async fn evidence_append_scoped_impl(
        &self,
        operation_id: Uuid,
        organization_id: Option<Uuid>,
        stage_run_id: Option<Uuid>,
        session_id: Option<&str>,
        project_path: Option<&str>,
        tool_name: &str,
        kind: &str,
        subject: &str,
        raw_output: &str,
        facts: Option<(&str, &str, &str)>,
    ) -> anyhow::Result<i64> {
        // MVP scope service: InMemory default-InScope. The production
        // `organizations.scope_rules` lookup is the deferred Task 7 of the P0
        // plan; swapping it in later does not change this call site.
        let scope = InMemoryScopeService::new(ScopeVersion::new(1));
        let (technique, asset, outcome) = match facts {
            Some((t, a, o)) => (Some(t), Some(a), Some(o)),
            None => (None, None, None),
        };
        let input = EvidenceInput {
            kind,
            subject,
            raw_output,
            tool_name,
            operation_id,
            stage_run_id,
            project_path,
            session_id,
            target_id: None,
            technique,
            asset,
            outcome,
        };
        let eid = match organization_id {
            Some(organization_id) => {
                append_for_organization(&self.pool, &scope, input, organization_id).await
            }
            None => append(&self.pool, &scope, input).await,
        }
        .map_err(|e| anyhow::anyhow!("evidence append failed: {e}"))?;
        Ok(eid.as_i64())
    }

    /// PR2 任务 2.5 · 只读投影源: 本会话三列齐全的证据事实.
    pub(crate) async fn evidence_facts_for_session_impl(
        &self,
        session_id: &str,
    ) -> anyhow::Result<Vec<(String, String, String, i64)>> {
        let rows =
            golish_db::repo::audit::evidence_facts_for_session(&self.pool, session_id).await?;
        Ok(rows)
    }

    pub(crate) async fn eas_evidence_facts_for_session_org_fresh_impl(
        &self,
        session_id: &str,
        organization_id: Uuid,
        since: chrono::DateTime<chrono::Utc>,
    ) -> anyhow::Result<Vec<(String, String, String, i64)>> {
        let rows = golish_db::repo::audit::evidence_facts_for_session_org_fresh(
            &self.pool,
            session_id,
            organization_id,
            since,
        )
        .await?;
        Ok(eas_target_bound_evidence_facts(organization_id, rows))
    }

    /// PR-C step2b（#4 / E3，设计 2026-06-23-technique-outcomes-provenance）：upsert 一条
    /// 覆盖结局 + provenance 进 `technique_outcomes`。EAS LIVENESS 使用 gate-compatible
    /// endpoint key（保留 URL port/path），其它 host-level technique 仍过
    /// `canonical_asset_key` 归一。`collected_at` 取当前时刻；`result_count`/`confidence`
    /// 暂留 None（落点暂无该信号）。
    #[allow(clippy::too_many_arguments)]
    pub(crate) async fn upsert_technique_outcome_impl(
        &self,
        organization_id: uuid::Uuid,
        run_id: &str,
        asset: &str,
        technique: &str,
        outcome: &str,
        source: Option<&str>,
        query: Option<&str>,
        evidence_ids: &[i64],
    ) -> anyhow::Result<()> {
        let canonical = if technique == golish_agent_kit::harness::evidence_facts::TECH_EAS_LIVENESS
        {
            golish_agent_kit::harness::evidence_facts::eas_liveness_asset_key(asset)
                .unwrap_or_else(|| asset.to_string())
        } else {
            golish_pentest_domain::canonical_asset_key(asset)
                .map(|k| k.key)
                .unwrap_or_else(|| asset.to_string())
        };
        let write = golish_db::repo::technique_outcomes::TechniqueOutcomeWrite {
            organization_id,
            run_id: run_id.to_string(),
            asset: canonical,
            technique: technique.to_string(),
            outcome: outcome.to_string(),
            source: source.map(str::to_string),
            query: query.map(str::to_string),
            result_count: None,
            confidence: None,
            evidence_ids: evidence_ids.to_vec(),
            collected_at: Some(chrono::Utc::now()),
        };
        golish_db::repo::technique_outcomes::upsert(&self.pool, &write).await
    }

    #[allow(clippy::too_many_arguments)]
    pub(crate) async fn upsert_terminal_technique_outcome_if_unfinished_impl(
        &self,
        organization_id: uuid::Uuid,
        run_id: &str,
        asset: &str,
        technique: &str,
        outcome: &str,
        source: Option<&str>,
        query: Option<&str>,
        evidence_ids: &[i64],
    ) -> anyhow::Result<bool> {
        let canonical = terminal_materialization_asset_key(asset, technique);
        let write = golish_db::repo::technique_outcomes::TechniqueOutcomeWrite {
            organization_id,
            run_id: run_id.to_string(),
            asset: canonical,
            technique: technique.to_string(),
            outcome: outcome.to_string(),
            source: source.map(str::to_string),
            query: query.map(str::to_string),
            result_count: None,
            confidence: None,
            evidence_ids: evidence_ids.to_vec(),
            collected_at: Some(chrono::Utc::now()),
        };
        golish_db::repo::technique_outcomes::upsert_terminal_if_unfinished(&self.pool, &write).await
    }

    /// Target-intel DNS attempt marker: refresh per-asset DNS landing, persist
    /// domains that were really resolved and returned no DNS answers as `empty`,
    /// and persist resolver/transport failures as non-terminal `error`. This
    /// keeps checked-empty, failed, and never-attempted distinct (I8).
    pub(crate) async fn mark_target_intel_dns_empty_outcomes_impl(
        &self,
        organization_id: uuid::Uuid,
        run_id: &str,
        evidence_ids: &[i64],
    ) -> anyhow::Result<usize> {
        if evidence_ids.is_empty() {
            return Ok(0);
        }

        let summary = golish_recon_app::organization_recon::refresh_per_asset_landing_summary(
            &self.pool,
            organization_id,
        )
        .await;
        let mut stored = 0usize;
        for (hosts, outcome) in [
            (dns_found_outcome_hosts(&summary), "found"),
            (dns_empty_outcome_hosts(&summary), "empty"),
            (dns_partial_outcome_hosts(&summary), "partial"),
            (dns_error_outcome_hosts(&summary), "error"),
        ] {
            for host in hosts {
                match self
                    .upsert_technique_outcome_impl(
                        organization_id,
                        run_id,
                        host,
                        golish_db::repo::coverage_truth::TECH_DNS,
                        outcome,
                        Some("resolver"),
                        Some(host),
                        evidence_ids,
                    )
                    .await
                {
                    Ok(()) => stored += 1,
                    Err(e) => tracing::warn!(
                        target: "harness::evidence",
                        organization_id = %organization_id,
                        asset = %host,
                        %outcome,
                        error = %e,
                        "target_intel DNS outcome upsert failed (continuing)"
                    ),
                }
            }
        }
        if summary.dns_refresh_failed {
            self.upsert_source_query_impl(
                organization_id,
                run_id,
                "resolver",
                "map_assets:GOLISH-INTEL-DNS",
                "",
                Some(golish_db::repo::coverage_truth::TECH_DNS),
                "error",
                evidence_ids,
            )
            .await?;
            stored += 1;
        }
        Ok(stored)
    }

    /// #5（设计 2026-06-23-source-query-log）：upsert 一条被动情报「源查询」进
    /// `source_query_log`（逐源查询日志，比 `technique_outcomes` 更细）。非空 `target` 在此
    /// 过 `canonical_asset_key` 归一（E1）；org 级查询（空串 target）原样保留。`finished_at`
    /// 取当前时刻；`result_count` / `started_at` / `detail` 暂留 None（命令路径暂无该信号）。
    #[allow(clippy::too_many_arguments)]
    pub(crate) async fn upsert_source_query_impl(
        &self,
        organization_id: uuid::Uuid,
        run_id: &str,
        source: &str,
        query: &str,
        target: &str,
        technique: Option<&str>,
        status: &str,
        evidence_ids: &[i64],
    ) -> anyhow::Result<()> {
        let canonical_target = if target.is_empty() {
            String::new()
        } else {
            golish_pentest_domain::canonical_asset_key(target)
                .map(|k| k.key)
                .unwrap_or_else(|| target.to_string())
        };
        let write = golish_db::repo::source_query_log::SourceQueryLogWrite {
            organization_id,
            run_id: run_id.to_string(),
            source: source.to_string(),
            query: query.to_string(),
            target: canonical_target,
            technique: technique.map(str::to_string),
            status: status.to_string(),
            result_count: None,
            evidence_ids: evidence_ids.to_vec(),
            detail: None,
            started_at: None,
            finished_at: Some(chrono::Utc::now()),
        };
        golish_db::repo::source_query_log::upsert(&self.pool, &write).await
    }

    /// #6（设计 2026-06-23-expansion-queue）：enqueue 一条「待扩展线索」进
    /// `expansion_queue`。入队恒 `status="pending"`（冲突时 SQL 不重置 status）；
    /// `discovered_at` 取当前时刻；`detail` 暂留 None。`lead_value` 不过
    /// canonical_asset_key（子公司线索是公司名，非 in-scope 主机）。
    #[allow(clippy::too_many_arguments)]
    pub(crate) async fn enqueue_expansion_lead_impl(
        &self,
        organization_id: uuid::Uuid,
        run_id: &str,
        lead_type: &str,
        lead_value: &str,
        source: Option<&str>,
        confidence: Option<f32>,
        evidence_ids: &[i64],
    ) -> anyhow::Result<()> {
        let write = golish_db::repo::expansion_queue::ExpansionLeadWrite {
            organization_id,
            run_id: run_id.to_string(),
            lead_type: lead_type.to_string(),
            lead_value: lead_value.to_string(),
            source: source.map(str::to_string),
            confidence,
            status: "pending".to_string(),
            evidence_ids: evidence_ids.to_vec(),
            detail: None,
            discovered_at: Some(chrono::Utc::now()),
        };
        golish_db::repo::expansion_queue::enqueue(&self.pool, &write).await
    }

    /// PR-D（#4 / E3）：读某 `(org, run)` 的 technique_outcomes 行 → coverage 投影元组
    /// `(asset, technique, outcome, evidence_id)`。Enumeration terminal 行只取能与
    /// same-run audit evidence 四元组完整匹配的 id；其它 technique 保留首个正数 id / 0
    /// 哨兵兼容。fail-safe：读失败 → 空 + warn（gate 退回 coverage_truth/ledger）。
    pub(crate) async fn technique_outcome_facts_impl(
        &self,
        organization_id: uuid::Uuid,
        run_id: &str,
    ) -> Vec<golish_agent_kit::db_traits::TechniqueOutcomeFact> {
        self.technique_outcome_facts_fresh_impl(organization_id, run_id, None)
            .await
    }

    /// Raw, fail-closed row-existence projection for the V2 final-seal catalog.
    /// Gate completion keeps using the guarded projection below; this method is
    /// deliberately narrower and only proves which exact canonical keys exist.
    pub(crate) async fn final_seal_technique_outcome_facts_impl(
        &self,
        organization_id: uuid::Uuid,
        run_id: &str,
    ) -> anyhow::Result<Vec<golish_agent_kit::db_traits::TechniqueOutcomeFact>> {
        let rows =
            golish_db::repo::technique_outcomes::list_for_run(&self.pool, organization_id, run_id)
                .await?;
        Ok(rows
            .into_iter()
            .map(|row| {
                let evidence_id = row
                    .evidence_ids
                    .iter()
                    .copied()
                    .find(|id| *id > 0)
                    .unwrap_or(0);
                golish_agent_kit::db_traits::TechniqueOutcomeFact::new(
                    row.asset,
                    row.technique,
                    row.outcome,
                    evidence_id,
                    row.source,
                )
            })
            .collect())
    }

    /// 护栏 4（设计 2026-07-02-gate-capability-ledger Phase 1）：同
    /// [`Self::technique_outcome_facts_impl`]，但套 stage-run freshness cutoff——
    /// `since = None` → presence-only；`since = Some(cutoff)` → 只投影
    /// `collected_at >= cutoff` 的行，避免同 session 旧 stage-run 的行泄漏进 gate。
    /// fail-safe：读失败 → 空 + warn（gate 退回 coverage_truth/ledger）。
    pub(crate) async fn technique_outcome_facts_fresh_impl(
        &self,
        organization_id: uuid::Uuid,
        run_id: &str,
        since: Option<chrono::DateTime<chrono::Utc>>,
    ) -> Vec<golish_agent_kit::db_traits::TechniqueOutcomeFact> {
        self.technique_outcome_facts_fresh_with_evidence_session_impl(
            organization_id,
            run_id,
            run_id,
            since,
        )
        .await
    }

    pub(crate) async fn technique_outcome_facts_fresh_with_evidence_session_impl(
        &self,
        organization_id: uuid::Uuid,
        outcome_run_id: &str,
        evidence_session_id: &str,
        since: Option<chrono::DateTime<chrono::Utc>>,
    ) -> Vec<golish_agent_kit::db_traits::TechniqueOutcomeFact> {
        let rows = match golish_db::repo::technique_outcomes::list_for_run_fresh(
            &self.pool,
            organization_id,
            outcome_run_id,
            since,
        )
        .await
        {
            Ok(rows) => rows,
            Err(e) => {
                tracing::warn!(
                    target: "harness::submit_tool",
                    error = %e,
                    "technique_outcome_facts read failed; gate runs without technique_outcomes projection"
                );
                return Vec::new();
            }
        };

        let target_bound_evidence_facts = if rows.iter().any(|row| {
            is_enumeration_terminal_outcome(&row.technique, &row.outcome)
                || is_eas_terminal_outcome(&row.technique, &row.outcome)
                || is_vuln_terminal_outcome(&row.technique, &row.outcome)
        }) {
            match since {
                Some(cutoff) => match golish_db::repo::audit::evidence_facts_for_session_org_fresh(
                    &self.pool,
                    evidence_session_id,
                    organization_id,
                    cutoff,
                )
                .await
                {
                    Ok(rows) => {
                        let mut facts = enumeration_target_bound_evidence_fact_set(rows.clone());
                        facts.extend(eas_target_bound_evidence_fact_set(
                            organization_id,
                            rows.clone(),
                        ));
                        facts.extend(vuln_target_bound_evidence_fact_set(organization_id, rows));
                        facts
                    }
                    Err(e) => {
                        tracing::warn!(
                            target: "harness::submit_tool",
                            error = %e,
                            "org-bound fresh evidence facts read failed; strict EAS/Enumeration/Vuln terminal outcomes remain unprojected"
                        );
                        TargetBoundEvidenceFactSet::new()
                    }
                },
                None => {
                    tracing::warn!(
                        target: "harness::submit_tool",
                        "strict EAS/Enumeration/Vuln terminal outcomes require a stage freshness cutoff"
                    );
                    TargetBoundEvidenceFactSet::new()
                }
            }
        } else {
            TargetBoundEvidenceFactSet::new()
        };

        rows.into_iter()
            .filter_map(|r| {
                let eid = projected_technique_outcome_evidence_id(
                    &r.asset,
                    &r.technique,
                    &r.outcome,
                    &r.evidence_ids,
                    &target_bound_evidence_facts,
                    r.source.as_deref(),
                )?;
                Some(golish_agent_kit::db_traits::TechniqueOutcomeFact::new(
                    r.asset,
                    r.technique,
                    r.outcome,
                    eid,
                    r.source,
                ))
            })
            .collect()
    }

    /// #5 Phase 3（provider-source closure）：读 `source_query_log` 的 terminal source
    /// rows → gate/source-coverage 只读 facts。查询错误保持为错误，避免最终 gate
    /// 把不可用读取误当作权威空结果。
    pub(crate) async fn source_query_facts_impl(
        &self,
        organization_id: uuid::Uuid,
        run_id: &str,
    ) -> anyhow::Result<Vec<SourceQueryFact>> {
        Ok(
            golish_db::repo::source_query_log::list_for_run(&self.pool, organization_id, run_id)
                .await?
                .into_iter()
                .map(|r| SourceQueryFact {
                    source: r.source,
                    query: r.query,
                    target: r.target,
                    technique: r.technique,
                    status: r.status,
                    evidence_ids: r.evidence_ids,
                })
                .collect(),
        )
    }

    pub(crate) async fn evidence_existing_ids_impl(
        &self,
        ids: &[i64],
    ) -> anyhow::Result<HashSet<i64>> {
        let found = golish_db::repo::audit::existing_evidence_ids(&self.pool, ids).await?;
        Ok(found.into_iter().collect())
    }

    pub(crate) async fn recent_evidence_ids_impl(
        &self,
        session_id: &str,
        limit: i64,
    ) -> anyhow::Result<Vec<i64>> {
        let rows =
            golish_db::repo::audit::recent_evidence_ids_for_session(&self.pool, session_id, limit)
                .await?;
        Ok(rows)
    }

    /// `list_recent_evidence` backing read (设计 2026-07-02-eas-worker-evidence): the
    /// session's recent real evidence rows as compact JSON, each with the context a
    /// worker needs to cite the right id (tool / subject / technique / asset /
    /// outcome / kind / age_seconds). Null fields are dropped so the agent sees only
    /// present context.
    pub(crate) async fn recent_evidence_detailed_impl(
        &self,
        session_id: &str,
        limit: i64,
    ) -> anyhow::Result<Vec<serde_json::Value>> {
        let rows = golish_db::repo::audit::recent_evidence_detailed_for_session(
            &self.pool, session_id, limit,
        )
        .await?;
        Ok(rows
            .into_iter()
            .map(|r| {
                let mut obj = serde_json::Map::new();
                obj.insert("evidence_id".to_string(), serde_json::json!(r.id));
                if let Some(tool) = r.tool_name.filter(|s| !s.is_empty()) {
                    obj.insert("tool".to_string(), serde_json::json!(tool));
                }
                if let Some(subject) = r.subject.filter(|s| !s.is_empty()) {
                    obj.insert("subject".to_string(), serde_json::json!(subject));
                }
                if let Some(technique) = r.technique.filter(|s| !s.is_empty()) {
                    obj.insert("technique".to_string(), serde_json::json!(technique));
                }
                if let Some(asset) = r.asset.filter(|s| !s.is_empty()) {
                    obj.insert("asset".to_string(), serde_json::json!(asset));
                }
                if let Some(outcome) = r.outcome.filter(|s| !s.is_empty()) {
                    obj.insert("outcome".to_string(), serde_json::json!(outcome));
                }
                if let Some(kind) = r.kind.filter(|s| !s.is_empty()) {
                    obj.insert("kind".to_string(), serde_json::json!(kind));
                }
                if let Some(age) = r.age_seconds.filter(|s| *s >= 0.0) {
                    obj.insert(
                        "age_seconds".to_string(),
                        serde_json::json!(age.round() as i64),
                    );
                }
                serde_json::Value::Object(obj)
            })
            .collect())
    }

    pub(crate) async fn evidence_kinds_for_impl(
        &self,
        ids: &[i64],
    ) -> anyhow::Result<HashMap<i64, String>> {
        let rows = golish_db::repo::audit::evidence_kinds_for(&self.pool, ids).await?;
        Ok(rows
            .into_iter()
            .filter_map(|(id, kind)| kind.map(|k| (id, k)))
            .collect())
    }

    pub(crate) async fn evidence_ages_for_impl(
        &self,
        ids: &[i64],
    ) -> anyhow::Result<HashMap<i64, std::time::Duration>> {
        let rows = golish_db::repo::audit::evidence_ages_for(&self.pool, ids).await?;
        Ok(rows
            .into_iter()
            .filter_map(|(id, secs)| {
                // Negative age (clock skew) or NULL → drop; the gate treats a
                // missing age as "unknown" and does not block on it.
                secs.filter(|s| *s >= 0.0)
                    .map(|s| (id, std::time::Duration::from_secs_f64(s)))
            })
            .collect())
    }
}

/// P2 · expose the ledger existence check to the `submit_stage_deliverable`
/// tool via its narrow [`EvidenceLedgerQuery`] seam (no full `DbRepoProvider`
/// dependency from the tool).
#[async_trait::async_trait]
impl crate::ai::harness_submit_tool::EvidenceLedgerQuery for GolishDbRepoProvider {
    async fn existing_evidence_ids(&self, ids: &[i64]) -> anyhow::Result<HashSet<i64>> {
        self.evidence_existing_ids_impl(ids).await
    }

    async fn recent_evidence_ids(&self, session_id: &str, limit: i64) -> anyhow::Result<Vec<i64>> {
        self.recent_evidence_ids_impl(session_id, limit).await
    }

    async fn cleanup_closeout_gate(
        &self,
        operation_id: uuid::Uuid,
        organization_id: uuid::Uuid,
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

    async fn evidence_kinds_for(
        &self,
        ids: &[i64],
    ) -> anyhow::Result<std::collections::HashMap<i64, String>> {
        self.evidence_kinds_for_impl(ids).await
    }

    async fn evidence_facts(
        &self,
        session_id: &str,
    ) -> Vec<golish_agent_kit::harness::EvidenceFact> {
        use golish_agent_kit::harness::{EvidenceFact, EvidenceOutcome};
        match self.evidence_facts_for_session_impl(session_id).await {
            Ok(rows) => rows
                .into_iter()
                .filter_map(|(asset, technique, outcome, evidence_id)| {
                    let outcome = match outcome.as_str() {
                        "found" => EvidenceOutcome::Found,
                        "empty" => EvidenceOutcome::Empty,
                        "blocked" => EvidenceOutcome::Blocked,
                        // T2：失败检查（gray-switch GOLISH_FAILURE_OUTCOME_ERROR）记 error。
                        "error" => EvidenceOutcome::Error,
                        _ => return None,
                    };
                    Some(EvidenceFact {
                        asset,
                        technique,
                        outcome,
                        evidence_id,
                    })
                })
                .collect(),
            Err(e) => {
                tracing::warn!(
                    target: "harness::submit_tool",
                    error = %e,
                    "evidence_facts_for_session failed; submit gate preview runs without projection"
                );
                Vec::new()
            }
        }
    }

    async fn eas_evidence_facts_for_session_org_fresh(
        &self,
        session_id: &str,
        organization_id: uuid::Uuid,
        since: chrono::DateTime<chrono::Utc>,
    ) -> Vec<golish_agent_kit::harness::EvidenceFact> {
        use golish_agent_kit::harness::{EvidenceFact, EvidenceOutcome};
        match self
            .eas_evidence_facts_for_session_org_fresh_impl(session_id, organization_id, since)
            .await
        {
            Ok(rows) => rows
                .into_iter()
                .filter_map(|(asset, technique, outcome, evidence_id)| {
                    let outcome = match outcome.as_str() {
                        "found" => EvidenceOutcome::Found,
                        "empty" => EvidenceOutcome::Empty,
                        _ => return None,
                    };
                    Some(EvidenceFact {
                        asset,
                        technique,
                        outcome,
                        evidence_id,
                    })
                })
                .collect(),
            Err(error) => {
                tracing::warn!(
                    target: "harness::submit_tool",
                    %error,
                    "strict EAS evidence fact lookup failed; submit preview keeps EAS cells pending"
                );
                Vec::new()
            }
        }
    }

    async fn db_truth_facts(
        &self,
        org_id: Option<uuid::Uuid>,
        assets: &[String],
    ) -> Vec<golish_agent_kit::harness::EvidenceFact> {
        self.db_truth_facts_with_run_start(org_id, assets, None)
            .await
    }

    async fn db_truth_facts_with_run_start(
        &self,
        org_id: Option<uuid::Uuid>,
        assets: &[String],
        run_start: Option<chrono::DateTime<chrono::Utc>>,
    ) -> Vec<golish_agent_kit::harness::EvidenceFact> {
        use golish_agent_kit::harness::{EvidenceFact, EvidenceOutcome};
        match self.db_truth_facts_impl(org_id, assets, run_start).await {
            // coverage_truth is Found-only (it never infers checked_empty), and
            // the projection is evidence-id-agnostic, so the business-table truth
            // maps to Found facts with the sentinel id 0.
            Ok(rows) => rows
                .into_iter()
                .map(|(asset, technique)| EvidenceFact {
                    asset,
                    technique,
                    outcome: EvidenceOutcome::Found,
                    evidence_id: 0,
                })
                .collect(),
            Err(e) => {
                tracing::warn!(
                    target: "harness::submit_tool",
                    error = %e,
                    "db_truth_facts failed; submit gate preview runs without the org-truth half"
                );
                Vec::new()
            }
        }
    }

    async fn in_scope_assets(&self, org_id: Option<uuid::Uuid>) -> Vec<String> {
        // org-isolated (`in_scope_values(None, org_id)`), unlike the whole-DB
        // `in_scope_targets`; keeps the submit preview's asset axis to THIS org.
        self.in_scope_assets_impl(org_id).await.unwrap_or_default()
    }

    async fn in_scope_assets_created_before(
        &self,
        org_id: Option<uuid::Uuid>,
        cutoff: chrono::DateTime<chrono::Utc>,
    ) -> Vec<String> {
        self.in_scope_assets_created_before_impl(org_id, cutoff)
            .await
            .unwrap_or_default()
    }

    async fn stage_asset_coverage(
        &self,
        organization_id: uuid::Uuid,
        stage: golish_agent_kit::harness::StageKind,
        session_id: Option<&str>,
        stage_started_at: Option<chrono::DateTime<chrono::Utc>>,
        current_wave_target_ids: Option<Vec<uuid::Uuid>>,
        current_wave_asset_values: Option<Vec<String>>,
    ) -> anyhow::Result<Option<serde_json::Value>> {
        self.stage_asset_coverage_impl(
            organization_id,
            stage.as_str(),
            session_id,
            stage_started_at,
            current_wave_target_ids,
            current_wave_asset_values,
            None,
        )
        .await
        .map(Some)
    }

    async fn stage_asset_coverage_for_operation(
        &self,
        operation_id: Option<uuid::Uuid>,
        organization_id: uuid::Uuid,
        stage: golish_agent_kit::harness::StageKind,
        session_id: Option<&str>,
        stage_started_at: Option<chrono::DateTime<chrono::Utc>>,
        current_wave_target_ids: Option<Vec<uuid::Uuid>>,
        current_wave_asset_values: Option<Vec<String>>,
    ) -> anyhow::Result<Option<serde_json::Value>> {
        self.stage_asset_coverage_impl(
            organization_id,
            stage.as_str(),
            session_id,
            stage_started_at,
            current_wave_target_ids,
            current_wave_asset_values,
            operation_id,
        )
        .await
        .map(Some)
    }

    async fn in_scope_typed_assets(&self, org_id: Option<uuid::Uuid>) -> Vec<(String, String)> {
        // T3 (设计 2026-06-23-submit-preview-authoritative-context): host-aware
        // asset_types for the submit preview (same source as the stage-close gate).
        self.in_scope_typed_assets_impl(org_id)
            .await
            .unwrap_or_default()
    }

    async fn eas_port_delegated_assets(&self, org_id: Option<uuid::Uuid>) -> Vec<String> {
        // 方案 A (设计 2026-06-30-eas-domain-port-delegation): EAS alias exclusion
        // for the submit preview (same source as the stage-close gate).
        self.eas_port_delegated_assets_impl(org_id)
            .await
            .unwrap_or_default()
    }

    async fn enumeration_web_capable_assets(&self, org_id: Option<uuid::Uuid>) -> Vec<String> {
        // 设计 2026-07-01 §5.3: EAS/httpx-proven IP web roots for the submit
        // preview (same source as the stage-close gate / org_gate).
        self.enumeration_web_capable_assets_impl(org_id)
            .await
            .unwrap_or_default()
    }

    async fn eas_web_capable_assets(
        &self,
        org_id: Option<uuid::Uuid>,
        run_start: Option<chrono::DateTime<chrono::Utc>>,
    ) -> Vec<String> {
        self.eas_web_capable_assets_impl(org_id, run_start)
            .await
            .unwrap_or_default()
    }

    async fn eas_required_web_origins(
        &self,
        organization_id: uuid::Uuid,
        since: chrono::DateTime<chrono::Utc>,
        current_wave_target_ids: Option<Vec<uuid::Uuid>>,
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
        org_id: Option<uuid::Uuid>,
        run_start: Option<chrono::DateTime<chrono::Utc>>,
    ) -> Vec<String> {
        self.eas_service_not_applicable_assets_impl(org_id, run_start)
            .await
            .unwrap_or_default()
    }

    async fn in_scope_target_types(&self, org_id: Option<uuid::Uuid>) -> Vec<String> {
        // T3: distinct targets.type for the preview's dynamic expected_techniques.
        self.in_scope_target_types_impl(org_id)
            .await
            .unwrap_or_default()
    }

    async fn technique_outcome_facts(
        &self,
        org_id: uuid::Uuid,
        run_id: &str,
    ) -> Vec<golish_agent_kit::db_traits::TechniqueOutcomeFact> {
        // PR-D (#4/E3): submit 预检 dual-read 投影源（与 DbRepoProvider 同 impl）。
        self.technique_outcome_facts_impl(org_id, run_id).await
    }

    async fn technique_outcome_facts_fresh(
        &self,
        org_id: uuid::Uuid,
        run_id: &str,
        since: Option<chrono::DateTime<chrono::Utc>>,
    ) -> Vec<golish_agent_kit::db_traits::TechniqueOutcomeFact> {
        self.technique_outcome_facts_fresh_impl(org_id, run_id, since)
            .await
    }

    async fn technique_outcome_facts_fresh_with_evidence_session(
        &self,
        org_id: uuid::Uuid,
        outcome_run_id: &str,
        evidence_session_id: &str,
        since: Option<chrono::DateTime<chrono::Utc>>,
    ) -> Vec<golish_agent_kit::db_traits::TechniqueOutcomeFact> {
        self.technique_outcome_facts_fresh_with_evidence_session_impl(
            org_id,
            outcome_run_id,
            evidence_session_id,
            since,
        )
        .await
    }

    async fn source_query_facts(&self, org_id: uuid::Uuid, run_id: &str) -> Vec<SourceQueryFact> {
        match self.source_query_facts_impl(org_id, run_id).await {
            Ok(rows) => rows,
            Err(error) => {
                tracing::warn!(
                    target: "harness::submit_tool",
                    %error,
                    "source_query_facts preview read failed"
                );
                Vec::new()
            }
        }
    }

    async fn operation_stage_started_at(
        &self,
        operation_id: uuid::Uuid,
    ) -> Option<(
        golish_agent_kit::harness::StageKind,
        chrono::DateTime<chrono::Utc>,
    )> {
        let state = self.operation_state_get_impl(operation_id).await.ok()??;
        let stage = golish_agent_kit::harness::StageKind::try_parse(&state.current_stage)?;
        Some((stage, state.stage_started_at))
    }

    async fn stage_asset_wave_current_running(
        &self,
        operation_id: uuid::Uuid,
        organization_id: uuid::Uuid,
        stage: golish_agent_kit::harness::StageKind,
    ) -> anyhow::Result<Option<golish_agent_kit::db_traits::StageAssetWaveView>> {
        self.stage_asset_wave_current_running_impl(operation_id, organization_id, stage.as_str())
            .await
    }

    async fn candidate_manifest_work_item_keys(
        &self,
        operation_id: uuid::Uuid,
        stage_run_unit_id: uuid::Uuid,
        organization_id: uuid::Uuid,
    ) -> anyhow::Result<Vec<String>> {
        Ok(self
            .attack_v2_candidate_manifest_for_unit_impl(
                operation_id,
                stage_run_unit_id,
                organization_id,
            )
            .await?
            .work_items
            .into_iter()
            .map(|item| item.work_item_key)
            .collect())
    }

    async fn candidate_manifest_for_unit(
        &self,
        operation_id: uuid::Uuid,
        stage_run_unit_id: uuid::Uuid,
        organization_id: uuid::Uuid,
    ) -> anyhow::Result<golish_agent_kit::harness::attack_execution::CandidateManifestSnapshot>
    {
        self.attack_v2_candidate_manifest_for_unit_impl(
            operation_id,
            stage_run_unit_id,
            organization_id,
        )
        .await
    }

    async fn verification_truth_for_operation(
        &self,
        operation_id: uuid::Uuid,
    ) -> anyhow::Result<Option<golish_agent_kit::harness::attack_execution::VerificationTruthSet>>
    {
        self.attack_v2_verification_truth_for_operation_impl(operation_id, None)
            .await
    }

    async fn reporting_truth_for_operation(
        &self,
        operation_id: uuid::Uuid,
    ) -> anyhow::Result<Option<golish_agent_kit::harness::ReportingGateTruth>> {
        super::reporting_gate::load_reporting_gate_truth(&self.pool, operation_id).await
    }
}

#[cfg(test)]
mod tests {
    use super::{
        dns_empty_outcome_hosts, dns_error_outcome_hosts, dns_found_outcome_hosts,
        dns_partial_outcome_hosts, eas_target_bound_evidence_facts, enumeration_evidence_fact_set,
        enumeration_target_bound_evidence_fact_set, projected_technique_outcome_evidence_id,
        terminal_materialization_asset_key, vuln_target_bound_evidence_fact_set,
    };
    use uuid::Uuid;

    #[test]
    fn dns_error_hosts_are_not_selected_for_checked_empty_outcomes() {
        let summary = golish_recon_app::organization_recon::PerAssetLandingSummary {
            subdomains: 0,
            dns_records: 0,
            dns_found_hosts: vec!["found.example.com".to_string()],
            dns_empty_hosts: vec!["empty.example.com".to_string()],
            dns_partial_hosts: vec!["partial.example.com".to_string()],
            dns_error_hosts: vec!["timeout.example.com".to_string()],
            dns_refresh_failed: false,
        };

        assert_eq!(
            dns_empty_outcome_hosts(&summary),
            &["empty.example.com".to_string()]
        );
        assert!(!dns_empty_outcome_hosts(&summary)
            .iter()
            .any(|host| host == "timeout.example.com"));
        assert_eq!(
            dns_error_outcome_hosts(&summary),
            &["timeout.example.com".to_string()]
        );
        assert_eq!(
            dns_found_outcome_hosts(&summary),
            &["found.example.com".to_string()]
        );
        assert_eq!(
            dns_partial_outcome_hosts(&summary),
            &["partial.example.com".to_string()]
        );
    }

    fn target_bound_row(
        organization_id: Uuid,
        asset: &str,
        technique: &str,
        target_value: &str,
        target_type: &str,
    ) -> golish_db::repo::audit::TargetBoundEvidenceFactRow {
        golish_db::repo::audit::TargetBoundEvidenceFactRow {
            evidence_asset: asset.to_string(),
            evidence_technique: technique.to_string(),
            evidence_outcome: "found".to_string(),
            evidence_id: 91,
            evidence_organization_id: organization_id.to_string(),
            tool_name: Some("route_probe_paths".to_string()),
            evidence_kind: Some("enumeration_route_probe".to_string()),
            target_id: Uuid::new_v4(),
            target_organization_id: Some(organization_id),
            target_type: target_type.to_string(),
            target_name: target_value.to_string(),
            target_value: target_value.to_string(),
            target_ports: serde_json::json!([]),
        }
    }

    #[test]
    fn vuln_terminal_evidence_requires_current_owner_and_exact_wrapper_tuple() {
        let org = Uuid::new_v4();
        let origin = "https://app.example.com:443";
        let mut row = target_bound_row(org, origin, "WSTG-CONF-05", origin, "url");
        row.evidence_outcome = "empty".to_string();
        row.tool_name = Some("vuln_nuclei_general".to_string());
        row.evidence_kind = Some("vuln.nuclei_observation".to_string());

        let accepted = vuln_target_bound_evidence_fact_set(org, vec![row.clone()]);
        assert!(accepted.contains(&(
            origin.to_string(),
            "WSTG-CONF-05".to_string(),
            "empty".to_string(),
            91,
        )));

        for forged in [
            {
                let mut forged = row.clone();
                forged.target_organization_id = Some(Uuid::new_v4());
                forged
            },
            {
                let mut forged = row.clone();
                forged.evidence_asset = "https://other.example.com:443".to_string();
                forged
            },
            {
                let mut forged = row.clone();
                forged.evidence_kind = Some("legacy_shell_scan".to_string());
                forged
            },
            {
                let mut forged = row.clone();
                forged.tool_name = Some("vuln_nuclei_fingerprint_targeted".to_string());
                forged
            },
        ] {
            assert!(
                vuln_target_bound_evidence_fact_set(org, vec![forged.clone()]).is_empty(),
                "mismatched Vuln evidence tuple must fail closed: {forged:?}"
            );
        }
    }

    #[test]
    fn vuln_anonymous_access_terminal_evidence_requires_its_exact_wrapper_tuple() {
        let org = Uuid::new_v4();
        let origin = "https://app.example.com:443";
        let mut row = target_bound_row(org, origin, "WSTG-ATHN-04", origin, "url");
        row.evidence_outcome = "not_applicable".to_string();
        row.tool_name = Some("vuln_probe_anonymous_access".to_string());
        row.evidence_kind = Some("vuln.anonymous_access_observation".to_string());

        let accepted = vuln_target_bound_evidence_fact_set(org, vec![row.clone()]);
        assert!(accepted.contains(&(
            origin.to_string(),
            "WSTG-ATHN-04".to_string(),
            "not_applicable".to_string(),
            91,
        )));

        for forged in [
            {
                let mut forged = row.clone();
                forged.tool_name = Some("vuln_nuclei_general".to_string());
                forged
            },
            {
                let mut forged = row.clone();
                forged.evidence_kind = Some("vuln.nuclei_observation".to_string());
                forged
            },
            {
                let mut forged = row.clone();
                forged.evidence_technique = "WSTG-UNKNOWN".to_string();
                forged
            },
        ] {
            assert!(
                vuln_target_bound_evidence_fact_set(org, vec![forged.clone()]).is_empty(),
                "mismatched anonymous-access tuple must fail closed: {forged:?}"
            );
        }
    }

    #[test]
    fn vuln_outcome_projection_rejects_mismatched_id_and_source() {
        let org = Uuid::new_v4();
        let origin = "https://app.example.com:443";
        let mut row = target_bound_row(org, origin, "GOLISH-NDAY", origin, "url");
        row.evidence_outcome = "found".to_string();
        row.tool_name = Some("vuln_nuclei_fingerprint_targeted".to_string());
        row.evidence_kind = Some("vuln.nuclei_observation".to_string());
        let guarded = vuln_target_bound_evidence_fact_set(org, vec![row]);

        assert_eq!(
            projected_technique_outcome_evidence_id(
                origin,
                "GOLISH-NDAY",
                "found",
                &[91],
                &guarded,
                Some("vuln_nuclei_fingerprint_targeted"),
            ),
            Some(91)
        );
        assert_eq!(
            projected_technique_outcome_evidence_id(
                origin,
                "GOLISH-NDAY",
                "found",
                &[92],
                &guarded,
                Some("vuln_nuclei_fingerprint_targeted"),
            ),
            None
        );
        assert_eq!(
            projected_technique_outcome_evidence_id(
                origin,
                "GOLISH-NDAY",
                "found",
                &[91],
                &guarded,
                Some("vuln_nuclei_general"),
            ),
            None
        );
    }

    #[test]
    fn enumeration_terminal_outcome_projection_requires_real_evidence() {
        let technique = golish_db::repo::coverage_truth::TECH_ENUM_DIR;
        let asset = "https://app.example.com:443";
        let matching = enumeration_evidence_fact_set(vec![(
            "https://app.example.com/path".to_string(),
            technique.to_string(),
            "found".to_string(),
            91,
        )]);
        assert_eq!(
            projected_technique_outcome_evidence_id(
                asset,
                technique,
                "found",
                &[],
                &matching,
                Some("route_probe_paths"),
            ),
            None
        );
        assert_eq!(
            projected_technique_outcome_evidence_id(
                asset,
                technique,
                "empty",
                &[0],
                &matching,
                Some("route_probe_paths"),
            ),
            None
        );
        assert_eq!(
            projected_technique_outcome_evidence_id(
                asset,
                technique,
                "found",
                &[0, 91],
                &matching,
                Some("route_probe_paths"),
            ),
            Some(91)
        );
        let blocked = enumeration_evidence_fact_set(vec![(
            asset.to_string(),
            technique.to_string(),
            "blocked".to_string(),
            92,
        )]);
        for source in [
            None,
            Some("browser_collect_js_api"),
            Some("Enum_Preflight_Web_Origins"),
        ] {
            assert_eq!(
                projected_technique_outcome_evidence_id(
                    asset,
                    technique,
                    "blocked",
                    &[92],
                    &blocked,
                    source,
                ),
                None,
                "blocked materialization must reject untrusted source {source:?}"
            );
        }
        assert_eq!(
            projected_technique_outcome_evidence_id(
                asset,
                technique,
                "blocked",
                &[92],
                &blocked,
                Some("route_probe_paths"),
            ),
            Some(92),
            "DIR recovery exhaustion may be closed only by route_probe_paths"
        );
        assert_eq!(
            projected_technique_outcome_evidence_id(
                asset,
                technique,
                "blocked",
                &[92],
                &blocked,
                Some("enum_preflight_web_origins"),
            ),
            Some(92)
        );
    }

    #[test]
    fn enumeration_terminal_outcome_projection_enforces_technique_source_owner() {
        let asset = "https://app.example.com:443";
        let cases = [
            (
                golish_db::repo::coverage_truth::TECH_ENUM_JS,
                &["browser_collect_js_api"][..],
                "js_extract_apis",
            ),
            (
                golish_db::repo::coverage_truth::TECH_ENUM_JSAPI,
                &["js_extract_apis"][..],
                "browser_collect_js_api",
            ),
            (
                golish_db::repo::coverage_truth::TECH_ENUM_DIR,
                &["route_probe_paths"][..],
                "browser_collect_js_api",
            ),
            (
                golish_db::repo::coverage_truth::TECH_ENUM_PARAM,
                &["js_extract_apis"][..],
                "browser_collect_js_api",
            ),
        ];

        for (technique, allowed_sources, forged_source) in cases {
            let facts = enumeration_evidence_fact_set(vec![(
                asset.to_string(),
                technique.to_string(),
                "found".to_string(),
                91,
            )]);
            for source in allowed_sources {
                assert_eq!(
                    projected_technique_outcome_evidence_id(
                        asset,
                        technique,
                        "found",
                        &[91],
                        &facts,
                        Some(source),
                    ),
                    Some(91),
                    "{technique} should accept its owner {source}"
                );
            }
            assert_eq!(
                projected_technique_outcome_evidence_id(
                    asset,
                    technique,
                    "found",
                    &[91],
                    &facts,
                    Some(forged_source),
                ),
                None,
                "{technique} must reject forged source {forged_source}"
            );
        }
    }

    #[test]
    fn enumeration_evidence_fact_rejects_non_owner_producer() {
        let org = Uuid::new_v4();
        let asset = "https://app.example.com:443";
        for technique in [
            golish_db::repo::coverage_truth::TECH_ENUM_JSAPI,
            golish_db::repo::coverage_truth::TECH_ENUM_PARAM,
        ] {
            let mut row = target_bound_row(org, asset, technique, asset, "url");
            row.tool_name = Some("browser_collect_js_api".to_string());

            assert!(
                enumeration_target_bound_evidence_fact_set(vec![row]).is_empty(),
                "browser evidence must not terminally own {technique}"
            );
        }
    }

    #[test]
    fn enumeration_blocked_audit_fact_requires_trusted_preflight_tool_and_kind() {
        let org = Uuid::new_v4();
        let asset = "https://app.example.com:443";
        let mut row = target_bound_row(
            org,
            asset,
            golish_db::repo::coverage_truth::TECH_ENUM_DIR,
            asset,
            "url",
        );
        row.evidence_outcome = "blocked".to_string();

        for (tool, kind) in [
            (
                Some("route_probe_paths"),
                Some("enumeration_transport_blocked"),
            ),
            (Some("enum_preflight_web_origins"), Some("generic_error")),
            (None, Some("enumeration_transport_blocked")),
        ] {
            let mut forged = row.clone();
            forged.tool_name = tool.map(str::to_string);
            forged.evidence_kind = kind.map(str::to_string);
            assert!(
                enumeration_target_bound_evidence_fact_set(vec![forged]).is_empty(),
                "forged blocked fact {tool:?}/{kind:?} must fail closed"
            );
        }

        row.tool_name = Some("enum_preflight_web_origins".to_string());
        row.evidence_kind = Some("enumeration_transport_blocked".to_string());
        assert_eq!(
            enumeration_target_bound_evidence_fact_set(vec![row.clone()]).len(),
            1
        );

        for technique in [
            golish_db::repo::coverage_truth::TECH_ENUM_JS,
            golish_db::repo::coverage_truth::TECH_ENUM_DIR,
            golish_db::repo::coverage_truth::TECH_ENUM_PARAM,
            golish_db::repo::coverage_truth::TECH_ENUM_JSAPI,
        ] {
            let mut preflight = row.clone();
            preflight.evidence_technique = technique.to_string();
            preflight.tool_name = Some("enum_preflight_web_origins".to_string());
            preflight.evidence_kind = Some("enumeration_transport_blocked".to_string());
            assert_eq!(
                enumeration_target_bound_evidence_fact_set(vec![preflight]).len(),
                1,
                "preflight should own blocked {technique}"
            );
        }

        row.tool_name = Some("route_probe_paths".to_string());
        row.evidence_kind = Some("dir_probe_recovery_exhausted".to_string());
        assert_eq!(
            enumeration_target_bound_evidence_fact_set(vec![row.clone()]).len(),
            1,
            "DIR producer may publish evidence-backed recovery exhaustion"
        );

        row.evidence_technique = golish_db::repo::coverage_truth::TECH_ENUM_JS.to_string();
        assert!(
            enumeration_target_bound_evidence_fact_set(vec![row]).is_empty(),
            "route producer must not publish blocked for another axis"
        );
    }

    #[test]
    fn enumeration_browser_recovery_blocked_is_limited_to_browser_axes_and_kind() {
        let org = Uuid::new_v4();
        let asset = "https://app.example.com:443";
        for technique in [
            golish_db::repo::coverage_truth::TECH_ENUM_JS,
            golish_db::repo::coverage_truth::TECH_ENUM_JSAPI,
            golish_db::repo::coverage_truth::TECH_ENUM_PARAM,
        ] {
            let mut row = target_bound_row(org, asset, technique, asset, "url");
            row.evidence_outcome = "blocked".to_string();
            row.tool_name = Some("browser_collect_js_api".to_string());
            row.evidence_kind = Some("enumeration_collection_recovery_exhausted".to_string());
            assert_eq!(
                enumeration_target_bound_evidence_fact_set(vec![row]).len(),
                1,
                "browser recovery exhaustion should own blocked {technique}"
            );

            let mut wrong_kind = target_bound_row(org, asset, technique, asset, "url");
            wrong_kind.evidence_outcome = "blocked".to_string();
            wrong_kind.tool_name = Some("browser_collect_js_api".to_string());
            wrong_kind.evidence_kind = Some("enumeration_transport_blocked".to_string());
            assert!(
                enumeration_target_bound_evidence_fact_set(vec![wrong_kind]).is_empty(),
                "browser blocked {technique} must carry its recovery-exhausted kind"
            );
        }

        let mut dir = target_bound_row(
            org,
            asset,
            golish_db::repo::coverage_truth::TECH_ENUM_DIR,
            asset,
            "url",
        );
        dir.evidence_outcome = "blocked".to_string();
        dir.tool_name = Some("browser_collect_js_api".to_string());
        dir.evidence_kind = Some("enumeration_collection_recovery_exhausted".to_string());
        assert!(enumeration_target_bound_evidence_fact_set(vec![dir]).is_empty());
    }

    #[test]
    fn enumeration_evidence_owner_matches_no_url_http_service_denominator() {
        let org = Uuid::new_v4();
        let mut row = target_bound_row(
            org,
            "https://203.0.113.10:443",
            golish_db::repo::coverage_truth::TECH_ENUM_DIR,
            "203.0.113.10",
            "ip_address",
        );
        row.evidence_outcome = "blocked".to_string();
        row.tool_name = Some("enum_preflight_web_origins".to_string());
        row.evidence_kind = Some("enumeration_transport_blocked".to_string());
        row.target_ports = serde_json::json!([
            {"port": 443, "state": "open", "service": "ssl/http"}
        ]);

        assert_eq!(
            enumeration_target_bound_evidence_fact_set(vec![row.clone()]).len(),
            1,
            "worklist-origin synthesis and evidence owner validation must agree"
        );

        for ports in [
            serde_json::json!([{"port": 443, "state": "closed", "service": "ssl/http"}]),
            serde_json::json!([{"port": 443, "state": "open", "service": "ssh"}]),
        ] {
            let mut denied = row.clone();
            denied.target_ports = ports;
            assert!(enumeration_target_bound_evidence_fact_set(vec![denied]).is_empty());
        }
    }

    #[test]
    fn enumeration_terminal_outcome_rejects_mismatched_evidence_fact() {
        let technique = golish_db::repo::coverage_truth::TECH_ENUM_DIR;
        let asset = "https://app.example.com:443";
        for fact in [
            (
                "https://other.example.com:443".to_string(),
                technique.to_string(),
                "found".to_string(),
                91,
            ),
            (
                asset.to_string(),
                golish_db::repo::coverage_truth::TECH_ENUM_JS.to_string(),
                "found".to_string(),
                91,
            ),
            (
                asset.to_string(),
                technique.to_string(),
                "empty".to_string(),
                91,
            ),
            (
                asset.to_string(),
                technique.to_string(),
                "found".to_string(),
                92,
            ),
        ] {
            let facts = enumeration_evidence_fact_set(vec![fact]);
            assert_eq!(
                projected_technique_outcome_evidence_id(
                    asset,
                    technique,
                    "found",
                    &[91],
                    &facts,
                    Some("route_probe_paths"),
                ),
                None
            );
        }
    }

    #[test]
    fn nonterminal_and_non_enumeration_projection_keep_legacy_sentinel() {
        assert_eq!(
            projected_technique_outcome_evidence_id(
                "https://app.example.com:443",
                golish_db::repo::coverage_truth::TECH_ENUM_DIR,
                "partial",
                &[],
                &Default::default(),
                Some("route_probe_paths"),
            ),
            Some(0)
        );
        assert_eq!(
            projected_technique_outcome_evidence_id(
                "example.com",
                golish_db::repo::coverage_truth::TECH_DNS,
                "found",
                &[],
                &Default::default(),
                None,
            ),
            Some(0)
        );
    }

    #[test]
    fn moved_origin_evidence_rejects_stale_port_url_until_target_value_owns_it() {
        let row = target_bound_row(
            Uuid::new_v4(),
            "https://app.example.com:443",
            golish_db::repo::coverage_truth::TECH_ENUM_DIR,
            "other.example.com",
            "domain",
        );
        assert!(enumeration_target_bound_evidence_fact_set(vec![row.clone()]).is_empty());

        let mut still_owned = row;
        still_owned.target_ports = serde_json::json!([{
            "state": "open",
            "url": "https://app.example.com/"
        }]);
        assert!(
            enumeration_target_bound_evidence_fact_set(vec![still_owned.clone()]).is_empty(),
            "a foreign historical ports[].url cannot restore moved origin ownership"
        );

        // `target_name` deliberately remains the foreign display label: only
        // the current executable target value may authorize this exact origin.
        still_owned.target_value = "app.example.com".to_string();
        let facts = enumeration_target_bound_evidence_fact_set(vec![still_owned.clone()]);
        assert!(facts.contains(&(
            "https://app.example.com:443".to_string(),
            golish_db::repo::coverage_truth::TECH_ENUM_DIR.to_string(),
            "found".to_string(),
            91,
        )));

        still_owned.target_ports[0]["state"] = serde_json::json!("closed");
        assert!(enumeration_target_bound_evidence_fact_set(vec![still_owned]).is_empty());
    }

    #[test]
    fn eas_target_bound_evidence_requires_producer_and_current_org() {
        let org_a = Uuid::new_v4();
        let org_b = Uuid::new_v4();
        let row = target_bound_row(
            org_a,
            "192.0.2.10",
            golish_db::repo::coverage_truth::TECH_EAS_PORT,
            "192.0.2.10",
            "ip_address",
        );

        assert_eq!(
            eas_target_bound_evidence_facts(org_a, vec![row.clone()]).len(),
            1
        );
        assert!(eas_target_bound_evidence_facts(org_b, vec![row.clone()]).is_empty());

        let mut moved_to_b = row;
        moved_to_b.target_organization_id = Some(org_b);
        assert!(eas_target_bound_evidence_facts(org_a, vec![moved_to_b]).is_empty());
    }

    #[test]
    fn eas_target_move_same_project_cannot_reuse_old_asset_fact() {
        let org = Uuid::new_v4();
        let mut moved = target_bound_row(
            org,
            "192.0.2.10",
            golish_db::repo::coverage_truth::TECH_EAS_PORT,
            "192.0.2.10",
            "ip_address",
        );
        moved.target_name = "192.0.2.20".to_string();
        moved.target_value = "192.0.2.20".to_string();

        assert!(eas_target_bound_evidence_facts(org, vec![moved]).is_empty());
    }

    #[test]
    fn eas_ip_origin_url_target_cannot_authorize_bare_ip_port_fact() {
        let org = Uuid::new_v4();
        let mut row = target_bound_row(
            org,
            "127.0.0.1",
            golish_db::repo::coverage_truth::TECH_EAS_PORT,
            "http://127.0.0.1:54537",
            "url",
        );
        row.tool_name = Some("eas_discover_ports".to_string());

        assert!(
            eas_target_bound_evidence_facts(org, vec![row]).is_empty(),
            "a URL target must not be promoted into authorization for its bare IP host"
        );
    }

    #[test]
    fn eas_legacy_evidence_without_producer_org_fails_closed() {
        let org = Uuid::new_v4();
        let mut legacy = target_bound_row(
            org,
            "192.0.2.10",
            golish_db::repo::coverage_truth::TECH_EAS_LIVENESS,
            "192.0.2.10",
            "ip_address",
        );
        legacy.evidence_organization_id.clear();

        assert!(eas_target_bound_evidence_facts(org, vec![legacy]).is_empty());
    }

    #[test]
    fn eas_terminal_outcome_ref_must_match_guarded_audit_quadruple() {
        let org = Uuid::new_v4();
        let row = target_bound_row(
            org,
            "192.0.2.10",
            golish_db::repo::coverage_truth::TECH_EAS_PORT,
            "192.0.2.10",
            "ip_address",
        );
        let facts = super::eas_target_bound_evidence_fact_set(org, vec![row]);

        assert_eq!(
            projected_technique_outcome_evidence_id(
                "192.0.2.10",
                golish_db::repo::coverage_truth::TECH_EAS_PORT,
                "found",
                &[91],
                &facts,
                Some("eas_discover_ports"),
            ),
            Some(91)
        );
        for (asset, outcome, ids) in [
            ("192.0.2.11", "found", vec![91]),
            ("192.0.2.10", "empty", vec![91]),
            ("192.0.2.10", "found", vec![92]),
            ("192.0.2.10", "found", vec![0]),
        ] {
            assert_eq!(
                projected_technique_outcome_evidence_id(
                    asset,
                    golish_db::repo::coverage_truth::TECH_EAS_PORT,
                    outcome,
                    &ids,
                    &facts,
                    Some("eas_discover_ports"),
                ),
                None
            );
        }
    }

    #[test]
    fn eas_web_evidence_and_outcome_match_exact_scheme_host_and_port() {
        let org = Uuid::new_v4();
        let mut row = target_bound_row(
            org,
            "https://app.example.com:443",
            golish_db::repo::coverage_truth::TECH_EAS_WEB_FP,
            "app.example.com",
            "domain",
        );
        row.tool_name = Some("whatweb".to_string());
        row.target_ports = serde_json::json!([{
            "state": "open",
            "url": "https://app.example.com:443/"
        }, {
            "state": "open",
            "url": "https://app.example.com:8443/"
        }]);
        let facts = super::eas_target_bound_evidence_fact_set(org, vec![row]);

        assert!(facts.contains(&(
            "https://app.example.com:443".to_string(),
            golish_db::repo::coverage_truth::TECH_EAS_WEB_FP.to_string(),
            "found".to_string(),
            91,
        )));
        assert_eq!(
            projected_technique_outcome_evidence_id(
                "https://app.example.com:443/path",
                golish_db::repo::coverage_truth::TECH_EAS_WEB_FP,
                "found",
                &[91],
                &facts,
                Some("whatweb"),
            ),
            Some(91)
        );
        for sibling in [
            "http://app.example.com:80",
            "https://app.example.com:8443",
            "https://other.example.com:443",
        ] {
            assert_eq!(
                projected_technique_outcome_evidence_id(
                    sibling,
                    golish_db::repo::coverage_truth::TECH_EAS_WEB_FP,
                    "found",
                    &[91],
                    &facts,
                    Some("whatweb"),
                ),
                None,
                "a sibling origin must not reuse exact-origin evidence"
            );
        }
    }

    #[test]
    fn vuln_terminal_materialization_keeps_sibling_origins_distinct() {
        let technique = "WSTG-ATHN-04";

        assert_eq!(
            terminal_materialization_asset_key("http://app.example.com/", technique),
            "http://app.example.com:80"
        );
        assert_eq!(
            terminal_materialization_asset_key("https://app.example.com/", technique),
            "https://app.example.com:443"
        );
        assert_eq!(
            terminal_materialization_asset_key("https://app.example.com:8443/path", technique),
            "https://app.example.com:8443"
        );
    }

    #[test]
    fn eas_web_blocked_projection_requires_exact_wrapper_owned_evidence() {
        let org = Uuid::new_v4();
        let mut row = target_bound_row(
            org,
            "https://app.example.com:443",
            golish_db::repo::coverage_truth::TECH_EAS_WEB_FP,
            "app.example.com",
            "domain",
        );
        row.evidence_outcome = "blocked".to_string();
        row.tool_name = Some("whatweb".to_string());
        row.evidence_kind = Some("eas.fingerprint_web_stack".to_string());
        row.target_ports = serde_json::json!([{
            "state": "open",
            "url": "https://app.example.com:443/"
        }, {
            "state": "open",
            "url": "https://app.example.com:8443/"
        }]);

        let facts = super::eas_target_bound_evidence_fact_set(org, vec![row.clone()]);
        assert!(facts.contains(&(
            "https://app.example.com:443".to_string(),
            golish_db::repo::coverage_truth::TECH_EAS_WEB_FP.to_string(),
            "blocked".to_string(),
            91,
        )));
        assert_eq!(
            projected_technique_outcome_evidence_id(
                "https://app.example.com:443/path",
                golish_db::repo::coverage_truth::TECH_EAS_WEB_FP,
                "blocked",
                &[91],
                &facts,
                Some("eas_fingerprint_web_stack"),
            ),
            Some(91)
        );

        for (asset, outcome, ids, source) in [
            (
                "http://app.example.com:80",
                "blocked",
                vec![91],
                Some("eas_fingerprint_web_stack"),
            ),
            (
                "https://app.example.com:8443",
                "blocked",
                vec![91],
                Some("eas_fingerprint_web_stack"),
            ),
            (
                "https://app.example.com:443",
                "empty",
                vec![91],
                Some("eas_fingerprint_web_stack"),
            ),
            (
                "https://app.example.com:443",
                "blocked",
                vec![92],
                Some("eas_fingerprint_web_stack"),
            ),
            (
                "https://app.example.com:443",
                "blocked",
                vec![91],
                Some("whatweb"),
            ),
        ] {
            assert_eq!(
                projected_technique_outcome_evidence_id(
                    asset,
                    golish_db::repo::coverage_truth::TECH_EAS_WEB_FP,
                    outcome,
                    &ids,
                    &facts,
                    source,
                ),
                None,
                "blocked projection must match source, outcome, evidence id, and exact origin"
            );
        }

        let mut wrong_tool = row.clone();
        wrong_tool.tool_name = Some("httpx".to_string());
        assert!(super::eas_target_bound_evidence_fact_set(org, vec![wrong_tool]).is_empty());

        let mut wrong_kind = row;
        wrong_kind.evidence_kind = Some("eas.http_liveness".to_string());
        assert!(super::eas_target_bound_evidence_fact_set(org, vec![wrong_kind]).is_empty());
    }
}
