//! Stage-run asset coverage read model.
//!
//! This is a UI-facing projection over the same DB-truth inputs the harness gate
//! already uses. It does not decide pass/fail; it explains the current
//! asset-by-technique matrix for a stage/org slice.

use std::collections::{BTreeMap, BTreeSet};

use chrono::{DateTime, Utc};
use serde::{Deserialize, Serialize};
use sqlx::FromRow;
use tauri::State;
use ts_rs::TS;
use uuid::Uuid;

use crate::error::GolishError;
use crate::state::AgentState;
use golish_agent_kit::harness::{
    suggested_capabilities_for_any_technique, tools_from_suggestions, StageCapabilitySuggestion,
    StageKind,
};

#[derive(Debug, Clone, Serialize, Deserialize, TS)]
#[ts(export, export_to = "../../../../frontend/lib/generated/")]
pub struct StageAssetCoverageSnapshot {
    pub stage: String,
    pub organization_id: String,
    pub session_id: Option<String>,
    pub summary: StageAssetCoverageSummary,
    pub assets: Vec<StageAssetCoverageRow>,
}

#[derive(Debug, Clone, Serialize, Deserialize, TS)]
#[ts(export, export_to = "../../../../frontend/lib/generated/")]
pub struct StageAssetCoverageSummary {
    #[ts(type = "number")]
    pub total_assets: usize,
    #[ts(type = "number")]
    pub seed_assets: usize,
    #[ts(type = "number")]
    pub new_assets: usize,
    #[ts(type = "number")]
    pub done_assets: usize,
    #[ts(type = "number")]
    pub pending_assets: usize,
    #[ts(type = "number")]
    pub blocked_assets: usize,
}

#[derive(Debug, Clone, Serialize, Deserialize, TS)]
#[ts(export, export_to = "../../../../frontend/lib/generated/")]
pub struct StageAssetCoverageRow {
    pub target_id: String,
    pub value: String,
    pub target_type: String,
    pub real_ip: String,
    pub source: String,
    pub discovered_phase: String,
    pub created_at: String,
    pub parent_id: Option<String>,
    pub coverage: Vec<StageAssetCoverageCell>,
    // EAS web metadata carried through so the enumeration web-root worklist can
    // build a full `scheme://host:port/` root_url without a per-target
    // query_target_data round-trip (design 2026-07-03-enumeration-throughput-
    // optimization PR-A). `#[ts(skip)]` = worklist-internal JSON only, kept off
    // the frontend binding to avoid churn.
    #[serde(default)]
    #[ts(skip)]
    pub http_status: Option<i32>,
    #[serde(default)]
    #[ts(skip)]
    pub ports: serde_json::Value,
    #[serde(default)]
    #[ts(skip)]
    pub webserver: String,
}

#[derive(Debug, Clone, Serialize, Deserialize, TS)]
#[ts(export, export_to = "../../../../frontend/lib/generated/")]
pub struct StageAssetCoverageCell {
    pub technique: String,
    pub label: String,
    pub state: String,
    pub source: Option<String>,
    #[ts(type = "Array<number>")]
    pub evidence_refs: Vec<i64>,
    pub note: Option<String>,
    #[serde(default)]
    #[ts(skip)]
    pub suggested_capabilities: Vec<StageCapabilitySuggestion>,
    pub suggested_tools: Vec<String>,
}

#[derive(Debug, Clone, FromRow)]
struct TargetCoverageRow {
    id: Uuid,
    value: String,
    target_type: String,
    real_ip: String,
    source: Option<String>,
    parent_id: Option<Uuid>,
    created_at: DateTime<Utc>,
    // EAS web metadata for the enumeration worklist's full-URL derivation (PR-A).
    #[sqlx(default)]
    http_status: Option<i32>,
    #[sqlx(default)]
    ports: serde_json::Value,
    #[sqlx(default)]
    webserver: String,
}

#[derive(Debug, Clone)]
struct OutcomeProjection {
    state: String,
    source: Option<String>,
    evidence_refs: Vec<i64>,
}

#[derive(Debug, FromRow)]
struct TechniqueOutcomeProjectionRow {
    asset: String,
    technique: String,
    outcome: String,
    source: Option<String>,
    evidence_refs: Vec<i64>,
}

#[derive(Debug, FromRow)]
struct SourceQueryProjectionRow {
    source: String,
    target: String,
    technique: Option<String>,
    status: String,
    evidence_refs: Vec<i64>,
}

#[derive(Debug, FromRow)]
struct EvidenceFactProjectionRow {
    asset: String,
    technique: String,
    outcome: String,
    source: Option<String>,
    evidence_refs: Vec<i64>,
    raw_output: Option<String>,
}

const NEXT_WAVE_PENDING: &str = "next_wave_pending";

const LATEST_TECHNIQUE_OUTCOMES_SQL: &str = r#"SELECT asset, technique, outcome, source, evidence_ids AS evidence_refs
   FROM (
     SELECT DISTINCT ON (asset, technique)
            id, asset, technique, outcome, source, evidence_ids, collected_at, updated_at
     FROM technique_outcomes
     WHERE organization_id = $1 AND technique = ANY($2::text[])
     ORDER BY asset, technique, collected_at DESC NULLS LAST, updated_at DESC, id DESC
   ) latest
   ORDER BY asset, technique"#;

const LATEST_SOURCE_QUERY_ROWS_SQL: &str = r#"SELECT source, target, technique, status, evidence_ids AS evidence_refs
   FROM (
     SELECT DISTINCT ON (target, technique, source, query)
            id, source, target, technique, status, evidence_ids, finished_at, created_at
     FROM source_query_log
     WHERE organization_id = $1 AND technique = ANY($2::text[])
     ORDER BY target, technique, source, query, finished_at DESC NULLS LAST, created_at DESC, id DESC
   ) latest
   ORDER BY target, technique, source"#;

const SESSION_EVIDENCE_FACT_ROWS_SQL: &str = r#"SELECT evidence_asset AS asset,
          evidence_technique AS technique,
          evidence_outcome AS outcome,
          tool_name AS source,
          ARRAY[id]::bigint[] AS evidence_refs,
          NULLIF(COALESCE(detail->>'raw_output', details), '') AS raw_output
   FROM audit_log
   WHERE audit_role = 'evidence'
     AND session_id = $1
     AND evidence_technique = ANY($2::text[])
     AND evidence_asset IS NOT NULL
     AND evidence_outcome IS NOT NULL
   ORDER BY id ASC"#;

#[tauri::command]
pub async fn ai_get_stage_asset_coverage(
    state: State<'_, AgentState>,
    organization_id: String,
    stage: String,
    session_id: Option<String>,
    stage_started_at: Option<String>,
) -> Result<StageAssetCoverageSnapshot, GolishError> {
    let org_id = Uuid::parse_str(&organization_id)
        .map_err(|e| GolishError::Validation(format!("invalid organization_id: {e}")))?;
    let stage_kind = StageKind::try_parse(&stage)
        .ok_or_else(|| GolishError::Validation(format!("unknown stage: {stage}")))?;
    let run_start = parse_rfc3339_utc(stage_started_at.as_deref());

    stage_asset_coverage_snapshot(
        &state.db_pool,
        org_id,
        stage_kind,
        session_id.as_deref(),
        run_start,
        true,
    )
    .await
    .map_err(GolishError::from)
}

pub(crate) async fn stage_asset_coverage_snapshot(
    pool: &sqlx::PgPool,
    org_id: Uuid,
    stage_kind: StageKind,
    session_id: Option<&str>,
    run_start: Option<DateTime<Utc>>,
    allow_latest_outcome_fallback: bool,
) -> anyhow::Result<StageAssetCoverageSnapshot> {
    let mut assets = list_stage_targets(pool, org_id, stage_kind).await?;
    let wave_cutoff = asset_wave_cutoff(stage_kind, run_start);
    let web_capable_assets = if stage_kind == StageKind::Enumeration {
        let (filtered, web_capable) =
            filter_enumeration_assets_by_eas_worklist(pool, org_id, assets).await?;
        assets = filtered;
        web_capable
    } else {
        BTreeSet::new()
    };
    let eas_ip_targets = eas_direct_ip_target_keys(&assets);
    let current_assets: Vec<&TargetCoverageRow> = assets
        .iter()
        .filter(|asset| !is_next_wave_asset(asset, wave_cutoff))
        .collect();
    let truth_assets: Vec<&TargetCoverageRow> = current_assets
        .iter()
        .copied()
        .filter(|asset| eas_alias_parent_ip_key(stage_kind, asset, &eas_ip_targets).is_none())
        .collect();
    let asset_values: Vec<String> = truth_assets.iter().map(|a| a.value.clone()).collect();
    let asset_types: Vec<String> = truth_assets.iter().map(|a| a.target_type.clone()).collect();
    let organization_asset_values: Vec<String> = truth_assets
        .iter()
        .filter(|asset| is_organization_coverage_row(asset))
        .map(|asset| asset.value.clone())
        .collect();

    let found_facts = golish_db::repo::coverage_truth::coverage_truth_facts(
        pool,
        Some(org_id),
        &asset_values,
        &asset_types,
        run_start,
    )
    .await?;
    let found: BTreeSet<(String, String)> = found_facts
        .into_iter()
        .map(|(asset, technique)| {
            (
                coverage_lookup_asset(&asset, technique),
                technique.to_string(),
            )
        })
        .collect();
    let service_not_applicable_assets: BTreeSet<String> =
        if stage_kind == StageKind::ExternalAttackSurface {
            golish_db::repo::coverage_truth::eas_service_not_applicable_assets(
                pool,
                Some(org_id),
                run_start,
            )
            .await?
            .into_iter()
            .map(|asset| {
                coverage_lookup_asset(&asset, golish_db::repo::coverage_truth::TECH_EAS_SERVICE_FP)
            })
            .collect()
        } else {
            BTreeSet::new()
        };
    // Enumeration content not_applicable (design 2026-07-03): DNS/53-only IPs
    // with no web surface are not content-enumeration roots. Keep the UI /
    // worklist read model in lockstep with the gate (org_gate / submit preview)
    // so the worklist does not list these IPs as pending after the gate has
    // terminalised them. Key = raw target value (ENUM coverage_lookup_asset is
    // identity).
    let enum_content_not_applicable_assets: BTreeSet<String> =
        if stage_kind == StageKind::Enumeration {
            golish_db::repo::coverage_truth::eas_service_not_applicable_assets(
                pool,
                Some(org_id),
                run_start,
            )
            .await?
            .into_iter()
            .collect()
        } else {
            BTreeSet::new()
        };

    let outcomes = stage_outcomes(
        pool,
        org_id,
        stage_kind,
        session_id,
        &asset_values,
        &organization_asset_values,
        allow_latest_outcome_fallback,
    )
    .await?;

    let mut rows = Vec::with_capacity(assets.len());
    let mut done_assets = 0usize;
    let mut pending_assets = 0usize;
    let mut blocked_assets = 0usize;
    let mut seed_assets = 0usize;
    let mut new_assets = 0usize;
    let mut current_wave_assets = 0usize;

    for asset in assets {
        let next_wave = is_next_wave_asset(&asset, wave_cutoff);
        let phase = if next_wave {
            "new_in_stage".to_string()
        } else {
            discovered_phase(&asset, run_start)
        };
        let counts_as_asset = counts_as_coverage_asset(stage_kind, &asset, &eas_ip_targets);
        let mut coverage = if next_wave {
            next_wave_coverage_cells_with_eas_parent_ips(
                stage_kind,
                &asset,
                &eas_ip_targets,
                &web_capable_assets,
            )
        } else {
            coverage_cells_with_eas_parent_ips(
                stage_kind,
                &asset,
                &found,
                &outcomes,
                &eas_ip_targets,
                &web_capable_assets,
                &service_not_applicable_assets,
            )
        };
        apply_enum_content_not_applicable(
            stage_kind,
            &asset,
            &mut coverage,
            &enum_content_not_applicable_assets,
        );

        if next_wave && counts_as_asset {
            new_assets += 1;
        } else if counts_as_asset {
            current_wave_assets += 1;
            if phase == "new_in_stage" {
                new_assets += 1;
            } else {
                seed_assets += 1;
            }
            let has_blocked = coverage
                .iter()
                .any(|cell| matches!(cell.state.as_str(), "blocked" | "error"));
            let has_pending = coverage.iter().any(|cell| cell.state == "pending");
            if has_blocked {
                blocked_assets += 1;
            } else if has_pending {
                pending_assets += 1;
            } else {
                done_assets += 1;
            }
        }
        rows.push(StageAssetCoverageRow {
            target_id: asset.id.to_string(),
            value: asset.value,
            target_type: asset.target_type,
            real_ip: asset.real_ip,
            source: asset.source.unwrap_or_default(),
            discovered_phase: phase,
            created_at: asset.created_at.to_rfc3339(),
            parent_id: asset.parent_id.map(|id| id.to_string()),
            coverage,
            http_status: asset.http_status,
            ports: asset.ports,
            webserver: asset.webserver,
        });
    }

    Ok(StageAssetCoverageSnapshot {
        stage: stage_kind.as_str().to_string(),
        organization_id: org_id.to_string(),
        session_id: session_id.map(str::to_string),
        summary: StageAssetCoverageSummary {
            total_assets: current_wave_assets,
            seed_assets,
            new_assets,
            done_assets,
            pending_assets,
            blocked_assets,
        },
        assets: rows,
    })
}

fn asset_wave_cutoff(stage: StageKind, run_start: Option<DateTime<Utc>>) -> Option<DateTime<Utc>> {
    let spec = golish_agent_kit::harness::load_embedded_stage_spec(stage).ok()?;
    spec.asset_wave_barrier.then_some(run_start).flatten()
}

fn is_next_wave_asset(asset: &TargetCoverageRow, wave_cutoff: Option<DateTime<Utc>>) -> bool {
    wave_cutoff.is_some_and(|cutoff| asset.created_at > cutoff)
}

async fn list_stage_targets(
    pool: &sqlx::PgPool,
    organization_id: Uuid,
    stage: StageKind,
) -> anyhow::Result<Vec<TargetCoverageRow>> {
    let mut rows = Vec::new();
    if matches!(stage, StageKind::TargetIntel) {
        if let Some(org_row) = get_organization_row(pool, organization_id).await? {
            rows.push(org_row);
        }
    }
    rows.extend(list_org_targets(pool, organization_id).await?);
    Ok(rows)
}

async fn filter_enumeration_assets_by_eas_worklist(
    pool: &sqlx::PgPool,
    org_id: Uuid,
    assets: Vec<TargetCoverageRow>,
) -> anyhow::Result<(Vec<TargetCoverageRow>, BTreeSet<String>)> {
    if assets.is_empty() {
        return Ok((assets, BTreeSet::new()));
    }
    let asset_values: Vec<String> = assets.iter().map(|a| a.value.clone()).collect();
    let asset_types: Vec<String> = assets.iter().map(|a| a.target_type.clone()).collect();
    let found_facts = golish_db::repo::coverage_truth::coverage_truth_facts(
        pool,
        Some(org_id),
        &asset_values,
        &asset_types,
        None,
    )
    .await?;
    let found: BTreeSet<(String, String)> = found_facts
        .into_iter()
        .map(|(asset, technique)| {
            (
                coverage_lookup_asset(&asset, technique),
                technique.to_string(),
            )
        })
        .collect();
    let web_capable_assets: BTreeSet<String> =
        golish_db::repo::coverage_truth::web_capable_ip_assets(pool, Some(org_id))
            .await?
            .into_iter()
            .collect();
    let filtered = filter_enumeration_assets_by_eas_found(assets, &found, &web_capable_assets);
    Ok((filtered, web_capable_assets))
}

fn filter_enumeration_assets_by_eas_found(
    assets: Vec<TargetCoverageRow>,
    found: &BTreeSet<(String, String)>,
    web_capable_assets: &BTreeSet<String>,
) -> Vec<TargetCoverageRow> {
    let worklist_ids: BTreeSet<Uuid> = assets
        .iter()
        .filter(|asset| {
            let class = coverage_asset_class(asset);
            (matches!(
                class,
                golish_agent_kit::harness::technique_resolver::AssetClass::Domain
                    | golish_agent_kit::harness::technique_resolver::AssetClass::Url
            ) && found.contains(&(
                coverage_lookup_asset(
                    &asset.value,
                    golish_db::repo::coverage_truth::TECH_EAS_LIVENESS,
                ),
                golish_db::repo::coverage_truth::TECH_EAS_LIVENESS.to_string(),
            ))) || (matches!(
                class,
                golish_agent_kit::harness::technique_resolver::AssetClass::Ip
                    | golish_agent_kit::harness::technique_resolver::AssetClass::Cidr
            ) && web_capable_assets.contains(&asset.value))
        })
        .map(|asset| asset.id)
        .collect();

    if worklist_ids.is_empty() {
        return assets;
    }
    assets
        .into_iter()
        .filter(|asset| worklist_ids.contains(&asset.id))
        .collect()
}

async fn get_organization_row(
    pool: &sqlx::PgPool,
    organization_id: Uuid,
) -> anyhow::Result<Option<TargetCoverageRow>> {
    Ok(sqlx::query_as::<_, TargetCoverageRow>(
        r#"SELECT id,
                  name AS value,
                  'organization'::text AS target_type,
                  ''::text AS real_ip,
                  'engagement_org'::text AS source,
                  parent_id,
                  created_at,
                  NULL::int AS http_status,
                  '[]'::jsonb AS ports,
                  ''::text AS webserver
           FROM organizations
           WHERE id = $1"#,
    )
    .bind(organization_id)
    .fetch_optional(pool)
    .await?)
}

async fn list_org_targets(
    pool: &sqlx::PgPool,
    organization_id: Uuid,
) -> anyhow::Result<Vec<TargetCoverageRow>> {
    Ok(sqlx::query_as::<_, TargetCoverageRow>(
        r#"SELECT id, value, target_type::text AS target_type, real_ip, source, parent_id, created_at,
                  http_status, ports, webserver
           FROM targets
           WHERE scope::text = 'in' AND organization_id = $1
           ORDER BY created_at ASC, value ASC"#,
    )
    .bind(organization_id)
    .fetch_all(pool)
    .await?)
}

async fn stage_outcomes(
    pool: &sqlx::PgPool,
    organization_id: Uuid,
    stage: StageKind,
    run_id: Option<&str>,
    asset_values: &[String],
    organization_asset_values: &[String],
    allow_latest_fallback: bool,
) -> anyhow::Result<BTreeMap<(String, String), OutcomeProjection>> {
    let mut out = BTreeMap::new();
    let stage_techniques: BTreeSet<String> = techniques_for_stage(stage)
        .into_iter()
        .map(str::to_string)
        .collect();

    if let Some(run_id) = run_id.filter(|id| !id.trim().is_empty()) {
        for row in
            golish_db::repo::technique_outcomes::list_for_run(pool, organization_id, run_id).await?
        {
            merge_technique_outcome_row(
                &mut out,
                asset_values,
                &stage_techniques,
                TechniqueOutcomeProjectionRow {
                    asset: row.asset,
                    technique: row.technique,
                    outcome: row.outcome,
                    source: row.source,
                    evidence_refs: row.evidence_ids,
                },
            );
        }

        for row in
            golish_db::repo::source_query_log::list_for_run(pool, organization_id, run_id).await?
        {
            merge_source_query_row(
                &mut out,
                asset_values,
                &stage_techniques,
                SourceQueryProjectionRow {
                    source: row.source,
                    target: row.target,
                    technique: row.technique,
                    status: row.status,
                    evidence_refs: row.evidence_ids,
                },
                organization_asset_values,
            );
        }

        for row in evidence_fact_rows_for_session(pool, run_id, &stage_techniques).await? {
            merge_evidence_fact_row(&mut out, asset_values, &stage_techniques, row);
        }
    }

    if allow_latest_fallback && out.is_empty() {
        for row in latest_technique_outcomes(pool, organization_id, &stage_techniques).await? {
            merge_technique_outcome_row(&mut out, asset_values, &stage_techniques, row);
        }
        for row in latest_source_query_rows(pool, organization_id, &stage_techniques).await? {
            merge_source_query_row(
                &mut out,
                asset_values,
                &stage_techniques,
                row,
                organization_asset_values,
            );
        }
    }

    Ok(out)
}

async fn latest_technique_outcomes(
    pool: &sqlx::PgPool,
    organization_id: Uuid,
    stage_techniques: &BTreeSet<String>,
) -> anyhow::Result<Vec<TechniqueOutcomeProjectionRow>> {
    if stage_techniques.is_empty() {
        return Ok(Vec::new());
    }
    let techniques: Vec<String> = stage_techniques.iter().cloned().collect();
    Ok(
        sqlx::query_as::<_, TechniqueOutcomeProjectionRow>(LATEST_TECHNIQUE_OUTCOMES_SQL)
            .bind(organization_id)
            .bind(&techniques)
            .fetch_all(pool)
            .await?,
    )
}

async fn latest_source_query_rows(
    pool: &sqlx::PgPool,
    organization_id: Uuid,
    stage_techniques: &BTreeSet<String>,
) -> anyhow::Result<Vec<SourceQueryProjectionRow>> {
    if stage_techniques.is_empty() {
        return Ok(Vec::new());
    }
    let techniques: Vec<String> = stage_techniques.iter().cloned().collect();
    Ok(
        sqlx::query_as::<_, SourceQueryProjectionRow>(LATEST_SOURCE_QUERY_ROWS_SQL)
            .bind(organization_id)
            .bind(&techniques)
            .fetch_all(pool)
            .await?,
    )
}

async fn evidence_fact_rows_for_session(
    pool: &sqlx::PgPool,
    run_id: &str,
    stage_techniques: &BTreeSet<String>,
) -> anyhow::Result<Vec<EvidenceFactProjectionRow>> {
    if stage_techniques.is_empty() {
        return Ok(Vec::new());
    }
    let techniques: Vec<String> = stage_techniques.iter().cloned().collect();
    Ok(
        sqlx::query_as::<_, EvidenceFactProjectionRow>(SESSION_EVIDENCE_FACT_ROWS_SQL)
            .bind(run_id)
            .bind(&techniques)
            .fetch_all(pool)
            .await?,
    )
}

fn merge_technique_outcome_row(
    out: &mut BTreeMap<(String, String), OutcomeProjection>,
    asset_values: &[String],
    stage_techniques: &BTreeSet<String>,
    row: TechniqueOutcomeProjectionRow,
) {
    if !stage_techniques.contains(&row.technique) {
        return;
    }
    let Some(asset_key) = matching_stage_asset_key(asset_values, &row.asset, &row.technique) else {
        return;
    };
    merge_outcome(
        out,
        (asset_key, row.technique),
        OutcomeProjection {
            state: outcome_state(&row.outcome),
            source: row.source,
            evidence_refs: row.evidence_refs,
        },
    );
}

fn merge_evidence_fact_row(
    out: &mut BTreeMap<(String, String), OutcomeProjection>,
    asset_values: &[String],
    stage_techniques: &BTreeSet<String>,
    row: EvidenceFactProjectionRow,
) {
    if !stage_techniques.contains(&row.technique) {
        return;
    }
    let Some(asset_key) = matching_stage_asset_key(asset_values, &row.asset, &row.technique) else {
        return;
    };
    merge_outcome(
        out,
        (asset_key, row.technique.clone()),
        OutcomeProjection {
            state: evidence_outcome_state(&row.outcome, &row.technique, row.raw_output.as_deref()),
            source: row.source,
            evidence_refs: row.evidence_refs,
        },
    );
}

fn merge_source_query_row(
    out: &mut BTreeMap<(String, String), OutcomeProjection>,
    asset_values: &[String],
    stage_techniques: &BTreeSet<String>,
    row: SourceQueryProjectionRow,
    organization_asset_values: &[String],
) {
    let Some(technique) = row.technique else {
        return;
    };
    if !stage_techniques.contains(&technique) {
        return;
    }
    let Some(state) = source_query_terminal_state(&row.status) else {
        return;
    };
    let projection = OutcomeProjection {
        state,
        source: Some(row.source),
        evidence_refs: row.evidence_refs,
    };
    if row.target.is_empty() {
        for asset in asset_values {
            merge_outcome(
                out,
                (coverage_lookup_asset(asset, &technique), technique.clone()),
                projection.clone(),
            );
        }
        return;
    }
    if let Some(target_key) = matching_stage_asset_key(asset_values, &row.target, &technique) {
        merge_outcome(out, (target_key, technique), projection);
        return;
    }
    if target_intel_source_technique(&technique) {
        for asset in organization_asset_values {
            merge_outcome(
                out,
                (coverage_lookup_asset(asset, &technique), technique.clone()),
                projection.clone(),
            );
        }
    }
}

fn target_intel_source_technique(technique: &str) -> bool {
    matches!(
        technique,
        golish_db::repo::coverage_truth::TECH_WHOIS
            | golish_db::repo::coverage_truth::TECH_ASN
            | golish_db::repo::coverage_truth::TECH_OSINT
    )
}

fn matching_stage_asset_key(
    asset_values: &[String],
    candidate: &str,
    technique: &str,
) -> Option<String> {
    let candidate_key = coverage_lookup_asset(candidate, technique);
    asset_values
        .iter()
        .map(|asset| coverage_lookup_asset(asset, technique))
        .find(|asset_key| asset_key == &candidate_key)
}

fn coverage_lookup_asset(asset: &str, technique: &str) -> String {
    match technique {
        golish_db::repo::coverage_truth::TECH_EAS_LIVENESS => {
            golish_agent_kit::harness::evidence_facts::eas_liveness_asset_key(asset)
                .unwrap_or_else(|| asset.to_string())
        }
        golish_db::repo::coverage_truth::TECH_EAS_PORT
        | golish_db::repo::coverage_truth::TECH_EAS_SERVICE_FP => {
            golish_pentest_domain::canonical_asset_key(asset)
                .map(|key| key.key)
                .unwrap_or_else(|| asset.to_string())
        }
        _ => asset.to_string(),
    }
}

fn is_organization_coverage_row(asset: &TargetCoverageRow) -> bool {
    asset.target_type == "organization"
}

fn organization_context_technique_applies(technique: &str) -> bool {
    matches!(
        technique,
        golish_db::repo::coverage_truth::TECH_WHOIS
            | golish_db::repo::coverage_truth::TECH_ASN
            | golish_db::repo::coverage_truth::TECH_OSINT
    )
}

fn counts_as_coverage_asset(
    stage: StageKind,
    asset: &TargetCoverageRow,
    eas_ip_targets: &BTreeSet<String>,
) -> bool {
    !is_organization_coverage_row(asset)
        && eas_alias_parent_ip_key(stage, asset, eas_ip_targets).is_none()
}

fn eas_direct_ip_target_keys(assets: &[TargetCoverageRow]) -> BTreeSet<String> {
    assets
        .iter()
        .filter(|asset| is_direct_eas_ip_target(asset))
        .filter_map(|asset| eas_ip_key_from_value(&asset.value))
        .collect()
}

fn is_direct_eas_ip_target(asset: &TargetCoverageRow) -> bool {
    matches!(
        asset.target_type.to_ascii_lowercase().as_str(),
        "ip" | "ipv4" | "ipv6" | "ip_address"
    )
}

fn eas_ip_key_from_value(value: &str) -> Option<String> {
    let key = golish_pentest_domain::canonical_asset_key(value)?;
    matches!(
        key.class,
        golish_agent_kit::harness::technique_resolver::AssetClass::Ip
    )
    .then_some(key.key)
}

fn eas_alias_parent_ip_key(
    stage: StageKind,
    asset: &TargetCoverageRow,
    eas_ip_targets: &BTreeSet<String>,
) -> Option<String> {
    if stage != StageKind::ExternalAttackSurface
        || is_direct_eas_ip_target(asset)
        || is_organization_coverage_row(asset)
    {
        return None;
    }
    let resolved_ip =
        eas_ip_key_from_value(&asset.real_ip).or_else(|| eas_ip_key_from_value(&asset.value))?;
    eas_ip_targets.contains(&resolved_ip).then_some(resolved_ip)
}

fn eas_alias_coverage_cells(parent_ip: &str) -> Vec<StageAssetCoverageCell> {
    techniques_for_stage(StageKind::ExternalAttackSurface)
        .into_iter()
        .map(|technique| {
            cell(
                technique,
                "not_applicable",
                None,
                Vec::new(),
                Some(format!(
                    "covered by resolved IP {parent_ip}; scan the host/IP once instead of each domain or endpoint alias"
                )),
            )
        })
        .collect()
}

#[cfg(test)]
fn coverage_cells(
    stage: StageKind,
    asset: &TargetCoverageRow,
    found: &BTreeSet<(String, String)>,
    outcomes: &BTreeMap<(String, String), OutcomeProjection>,
) -> Vec<StageAssetCoverageCell> {
    coverage_cells_with_eas_parent_ips(
        stage,
        asset,
        found,
        outcomes,
        &BTreeSet::new(),
        &BTreeSet::new(),
        &BTreeSet::new(),
    )
}

fn coverage_cells_with_eas_parent_ips(
    stage: StageKind,
    asset: &TargetCoverageRow,
    found: &BTreeSet<(String, String)>,
    outcomes: &BTreeMap<(String, String), OutcomeProjection>,
    eas_ip_targets: &BTreeSet<String>,
    web_capable_assets: &BTreeSet<String>,
    service_not_applicable_assets: &BTreeSet<String>,
) -> Vec<StageAssetCoverageCell> {
    if let Some(parent_ip) = eas_alias_parent_ip_key(stage, asset, eas_ip_targets) {
        return eas_alias_coverage_cells(&parent_ip);
    }
    let class = coverage_asset_class(asset);
    let mut cells: Vec<StageAssetCoverageCell> = techniques_for_stage(stage)
        .into_iter()
        .map(|technique| {
            if is_organization_coverage_row(asset)
                && !organization_context_technique_applies(technique)
            {
                return cell(
                    technique,
                    "not_applicable",
                    None,
                    Vec::new(),
                    Some("organization context row; this technique applies to concrete domain/IP/URL assets".to_string()),
                );
            }
            if !golish_agent_kit::harness::technique_resolver::technique_applies_web_aware(
                stage,
                class,
                &asset.value,
                technique,
                web_capable_assets.contains(&asset.value),
            ) {
                return cell(
                    technique,
                    "not_applicable",
                    None,
                    Vec::new(),
                    Some("not applicable to this asset type".to_string()),
                );
            }
            let asset_key = coverage_lookup_asset(&asset.value, technique);
            if found.contains(&(asset_key.clone(), technique.to_string())) {
                return cell(technique, "found", None, Vec::new(), None);
            }
            if let Some(outcome) = outcomes.get(&(asset_key, technique.to_string())) {
                return cell(
                    technique,
                    &outcome.state,
                    outcome.source.clone(),
                    outcome.evidence_refs.clone(),
                    None,
                );
            }
            cell(technique, "pending", None, Vec::new(), None)
        })
        .collect();
    apply_eas_service_dependency(stage, asset, &mut cells, service_not_applicable_assets);
    cells
}

/// Enumeration content not_applicable (design 2026-07-03): keep the UI /
/// worklist read model in lockstep with the gate — a DNS/53-only IP with no web
/// surface is not a content-enumeration root, so terminalise its still-pending
/// ENUM axes as not_applicable instead of leaving them as pending work items.
fn apply_enum_content_not_applicable(
    stage: StageKind,
    asset: &TargetCoverageRow,
    cells: &mut [StageAssetCoverageCell],
    enum_content_not_applicable_assets: &BTreeSet<String>,
) {
    if stage != StageKind::Enumeration || !enum_content_not_applicable_assets.contains(&asset.value)
    {
        return;
    }
    for coverage_cell in cells.iter_mut() {
        if coverage_cell.state == "pending" {
            *coverage_cell = cell(
                cell_technique_static(&coverage_cell.technique),
                "not_applicable",
                None,
                Vec::new(),
                Some(
                    "only DNS/53 is open and no web service surface was observed, so content enumeration is not applicable to this IP"
                        .to_string(),
                ),
            );
        }
    }
}

/// Map a runtime technique string back to the interned `&'static str` the `cell`
/// helper needs. Enumeration only has the four ENUM axes; anything else falls
/// through unchanged (defensive — this helper is only called for ENUM rows).
fn cell_technique_static(technique: &str) -> &'static str {
    match technique {
        golish_db::repo::coverage_truth::TECH_ENUM_JS => {
            golish_db::repo::coverage_truth::TECH_ENUM_JS
        }
        golish_db::repo::coverage_truth::TECH_ENUM_DIR => {
            golish_db::repo::coverage_truth::TECH_ENUM_DIR
        }
        golish_db::repo::coverage_truth::TECH_ENUM_PARAM => {
            golish_db::repo::coverage_truth::TECH_ENUM_PARAM
        }
        golish_db::repo::coverage_truth::TECH_ENUM_JSAPI => {
            golish_db::repo::coverage_truth::TECH_ENUM_JSAPI
        }
        _ => golish_db::repo::coverage_truth::TECH_ENUM_JS,
    }
}

fn apply_eas_service_dependency(
    stage: StageKind,
    asset: &TargetCoverageRow,
    cells: &mut [StageAssetCoverageCell],
    service_not_applicable_assets: &BTreeSet<String>,
) {
    if stage != StageKind::ExternalAttackSurface {
        return;
    }
    let service_key = coverage_lookup_asset(
        &asset.value,
        golish_db::repo::coverage_truth::TECH_EAS_SERVICE_FP,
    );
    let dns_only_without_service_surface = service_not_applicable_assets.contains(&service_key);
    let port_has_no_service_surface = cells.iter().any(|coverage_cell| {
        coverage_cell.technique == golish_db::repo::coverage_truth::TECH_EAS_PORT
            && matches!(
                coverage_cell.state.as_str(),
                "checked_empty" | "not_applicable"
            )
    });
    if !port_has_no_service_surface && !dns_only_without_service_surface {
        return;
    }
    let Some(service_cell) = cells.iter_mut().find(|coverage_cell| {
        coverage_cell.technique == golish_db::repo::coverage_truth::TECH_EAS_SERVICE_FP
    }) else {
        return;
    };
    if service_cell.state != "pending" {
        return;
    }
    let note = if dns_only_without_service_surface {
        "only DNS/53 is open and no informative service/version surface was observed, so service fingerprinting is not applicable to this real_ip".to_string()
    } else {
        "no open ports were found, so service fingerprinting is not applicable".to_string()
    };
    *service_cell = cell(
        golish_db::repo::coverage_truth::TECH_EAS_SERVICE_FP,
        "not_applicable",
        None,
        Vec::new(),
        Some(note),
    );
}

#[cfg(test)]
fn next_wave_coverage_cells(
    stage: StageKind,
    asset: &TargetCoverageRow,
) -> Vec<StageAssetCoverageCell> {
    next_wave_coverage_cells_with_eas_parent_ips(stage, asset, &BTreeSet::new(), &BTreeSet::new())
}

fn next_wave_coverage_cells_with_eas_parent_ips(
    stage: StageKind,
    asset: &TargetCoverageRow,
    eas_ip_targets: &BTreeSet<String>,
    web_capable_assets: &BTreeSet<String>,
) -> Vec<StageAssetCoverageCell> {
    if let Some(parent_ip) = eas_alias_parent_ip_key(stage, asset, eas_ip_targets) {
        return eas_alias_coverage_cells(&parent_ip);
    }
    let class = coverage_asset_class(asset);
    techniques_for_stage(stage)
        .into_iter()
        .map(|technique| {
            if is_organization_coverage_row(asset)
                && !organization_context_technique_applies(technique)
            {
                return cell(
                    technique,
                    "not_applicable",
                    None,
                    Vec::new(),
                    Some("organization context row; this technique applies to concrete domain/IP/URL assets".to_string()),
                );
            }
            if !golish_agent_kit::harness::technique_resolver::technique_applies_web_aware(
                stage,
                class,
                &asset.value,
                technique,
                web_capable_assets.contains(&asset.value),
            ) {
                return cell(
                    technique,
                    "not_applicable",
                    None,
                    Vec::new(),
                    Some("not applicable to this asset type".to_string()),
                );
            }
            cell(
                technique,
                NEXT_WAVE_PENDING,
                None,
                Vec::new(),
                Some("newly discovered during this stage; queued for the next wave".to_string()),
            )
        })
        .collect()
}

fn coverage_asset_class(
    asset: &TargetCoverageRow,
) -> golish_agent_kit::harness::technique_resolver::AssetClass {
    golish_agent_kit::harness::technique_resolver::AssetClass::classify(
        Some(&asset.target_type),
        &asset.value,
    )
}

fn techniques_for_stage(stage: StageKind) -> Vec<&'static str> {
    match stage {
        StageKind::TargetIntel => vec![
            golish_db::repo::coverage_truth::TECH_DNS,
            golish_db::repo::coverage_truth::TECH_WHOIS,
            golish_db::repo::coverage_truth::TECH_ASN,
            golish_db::repo::coverage_truth::TECH_CT,
            golish_db::repo::coverage_truth::TECH_SUBDOMAIN,
            golish_db::repo::coverage_truth::TECH_OSINT,
        ],
        StageKind::ExternalAttackSurface => vec![
            golish_db::repo::coverage_truth::TECH_EAS_LIVENESS,
            golish_db::repo::coverage_truth::TECH_EAS_PORT,
            golish_db::repo::coverage_truth::TECH_EAS_SERVICE_FP,
        ],
        StageKind::Enumeration => vec![
            golish_db::repo::coverage_truth::TECH_ENUM_JS,
            golish_db::repo::coverage_truth::TECH_ENUM_DIR,
            golish_db::repo::coverage_truth::TECH_ENUM_PARAM,
            golish_db::repo::coverage_truth::TECH_ENUM_JSAPI,
        ],
        StageKind::VulnTriage => vec![
            "WSTG-INPV-05",
            "WSTG-INPV-01",
            "WSTG-INPV-12",
            "WSTG-ATHZ-04",
            "WSTG-ATHN-02",
            "WSTG-SESS-02",
            "WSTG-CONF-05",
            "WSTG-CRYP-03",
            "WSTG-INFO",
            "GOLISH-NDAY",
        ],
        _ => Vec::new(),
    }
}

fn cell(
    technique: &'static str,
    state: &str,
    source: Option<String>,
    evidence_refs: Vec<i64>,
    note: Option<String>,
) -> StageAssetCoverageCell {
    let suggested_capabilities = if matches!(state, "pending" | "error") {
        suggested_capabilities_for_any_technique(technique)
    } else {
        Vec::new()
    };
    let suggested_tools = tools_from_suggestions(&suggested_capabilities);
    StageAssetCoverageCell {
        technique: technique.to_string(),
        label: technique_label(technique).to_string(),
        state: state.to_string(),
        source,
        evidence_refs,
        note,
        suggested_capabilities,
        suggested_tools,
    }
}

fn outcome_state(outcome: &str) -> String {
    match outcome {
        "found" => "found",
        "empty" => "checked_empty",
        "blocked" => "blocked",
        "error" => "error",
        "not_applicable" => "not_applicable",
        _ => "pending",
    }
    .to_string()
}

fn evidence_outcome_state(outcome: &str, technique: &str, raw_output: Option<&str>) -> String {
    if outcome == "error" && eas_no_target_error_is_checked_empty(technique, raw_output) {
        return "checked_empty".to_string();
    }
    outcome_state(outcome)
}

fn eas_no_target_error_is_checked_empty(technique: &str, raw_output: Option<&str>) -> bool {
    if !matches!(
        technique,
        golish_db::repo::coverage_truth::TECH_EAS_LIVENESS
            | golish_db::repo::coverage_truth::TECH_EAS_PORT
    ) {
        return false;
    }
    let Some(raw_output) = raw_output else {
        return false;
    };
    let output = raw_output.to_ascii_lowercase();
    output.contains("failed to resolve")
        || output.contains("no valid ipv4 or ipv6 targets")
        || output.contains("no valid ipv4/ipv6 targets")
        || output.contains("no targets were specified")
        || output.contains("0 ip addresses (0 hosts up)")
}

fn source_query_terminal_state(status: &str) -> Option<String> {
    match status {
        // Source rows prove the provider/source was tried and ended empty, but
        // only DB truth / evidence facts can turn a cell into `found`.
        "empty" => Some("checked_empty".to_string()),
        "blocked" => Some("blocked".to_string()),
        "error" => Some("error".to_string()),
        _ => None,
    }
}

fn merge_outcome(
    out: &mut BTreeMap<(String, String), OutcomeProjection>,
    key: (String, String),
    next: OutcomeProjection,
) {
    let Some(existing) = out.get_mut(&key) else {
        out.insert(key, next);
        return;
    };
    if outcome_rank(&next.state) > outcome_rank(&existing.state) {
        existing.state = next.state;
        existing.source = next.source;
    } else if existing.source.is_none() {
        existing.source = next.source;
    }
    for evidence_ref in next.evidence_refs {
        if !existing.evidence_refs.contains(&evidence_ref) {
            existing.evidence_refs.push(evidence_ref);
        }
    }
}

fn outcome_rank(state: &str) -> u8 {
    match state {
        "found" => 5,
        "blocked" => 4,
        "error" => 3,
        "checked_empty" => 2,
        "not_applicable" => 1,
        _ => 0,
    }
}

fn technique_label(technique: &str) -> &'static str {
    match technique {
        golish_db::repo::coverage_truth::TECH_DNS => "DNS",
        golish_db::repo::coverage_truth::TECH_WHOIS => "WHOIS",
        golish_db::repo::coverage_truth::TECH_ASN => "ASN",
        golish_db::repo::coverage_truth::TECH_CT => "CT",
        golish_db::repo::coverage_truth::TECH_SUBDOMAIN => "Subdomain",
        golish_db::repo::coverage_truth::TECH_OSINT => "OSINT",
        golish_db::repo::coverage_truth::TECH_EAS_LIVENESS => "Liveness",
        golish_db::repo::coverage_truth::TECH_EAS_PORT => "Port",
        golish_db::repo::coverage_truth::TECH_EAS_SERVICE_FP => "Service",
        golish_db::repo::coverage_truth::TECH_ENUM_JS => "JS",
        golish_db::repo::coverage_truth::TECH_ENUM_DIR => "Directory",
        golish_db::repo::coverage_truth::TECH_ENUM_PARAM => "Parameter",
        golish_db::repo::coverage_truth::TECH_ENUM_JSAPI => "API",
        "WSTG-INPV-05" => "SQL Injection",
        "WSTG-INPV-01" => "XSS",
        "WSTG-INPV-12" => "Command Injection",
        "WSTG-ATHZ-04" => "IDOR",
        "WSTG-ATHN-02" => "Weak Credentials",
        "WSTG-SESS-02" => "Session/CSRF",
        "WSTG-CONF-05" => "Sensitive Config",
        "WSTG-CRYP-03" => "TLS/Crypto",
        "WSTG-INFO" => "Information Leak",
        "GOLISH-NDAY" => "N-day",
        _ => "Coverage",
    }
}

#[cfg(test)]
fn suggested_tools(technique: &str) -> Vec<String> {
    golish_agent_kit::harness::suggested_tools_for_any_technique(technique)
}

fn discovered_phase(asset: &TargetCoverageRow, run_start: Option<DateTime<Utc>>) -> String {
    if is_organization_coverage_row(asset) {
        return "organization_context".to_string();
    }
    if run_start.is_some_and(|started_at| asset.created_at > started_at)
        || asset.source.as_deref() == Some("active_discovered")
    {
        "new_in_stage".to_string()
    } else if matches!(
        asset.source.as_deref(),
        Some("asset_intel" | "intel_provider" | "manual" | "scoping")
    ) {
        "seed".to_string()
    } else {
        "historical".to_string()
    }
}

fn parse_rfc3339_utc(value: Option<&str>) -> Option<DateTime<Utc>> {
    value
        .and_then(|raw| DateTime::parse_from_rfc3339(raw).ok())
        .map(|dt| dt.with_timezone(&Utc))
}

#[cfg(test)]
mod tests {
    use super::*;

    fn target(value: &str, target_type: &str) -> TargetCoverageRow {
        TargetCoverageRow {
            id: Uuid::new_v4(),
            value: value.to_string(),
            target_type: target_type.to_string(),
            real_ip: String::new(),
            source: Some("asset_intel".to_string()),
            parent_id: None,
            created_at: Utc::now(),
            http_status: None,
            ports: serde_json::json!([]),
            webserver: String::new(),
        }
    }

    #[test]
    fn enumeration_exposes_four_axes_js_first() {
        // design 2026-07-01 §4：JS 收集独立成轴，顺序 JS → DIR → PARAM → JSAPI。
        let t = techniques_for_stage(StageKind::Enumeration);
        assert_eq!(
            t,
            vec![
                golish_db::repo::coverage_truth::TECH_ENUM_JS,
                golish_db::repo::coverage_truth::TECH_ENUM_DIR,
                golish_db::repo::coverage_truth::TECH_ENUM_PARAM,
                golish_db::repo::coverage_truth::TECH_ENUM_JSAPI,
            ]
        );
        // label：JS 独立、JSAPI 收窄为 API（避免与 JS 轴混淆）。
        assert_eq!(
            technique_label(golish_db::repo::coverage_truth::TECH_ENUM_JS),
            "JS"
        );
        assert_eq!(
            technique_label(golish_db::repo::coverage_truth::TECH_ENUM_JSAPI),
            "API"
        );
        // suggested_tools：JS 用 browser 收集、JSAPI 用 js_extract 抽取。
        assert_eq!(
            suggested_tools(golish_db::repo::coverage_truth::TECH_ENUM_JS),
            vec!["browser_collect_js_api".to_string()]
        );
        assert_eq!(
            suggested_tools(golish_db::repo::coverage_truth::TECH_ENUM_JSAPI),
            vec!["js_extract_apis".to_string()]
        );
    }

    #[test]
    fn vuln_triage_exposes_formulaic_scan_axes() {
        let t = techniques_for_stage(StageKind::VulnTriage);
        assert_eq!(t.len(), 10);
        assert!(t.contains(&"WSTG-INPV-05"));
        assert!(t.contains(&"WSTG-CONF-05"));
        assert!(t.contains(&"GOLISH-NDAY"));

        let asset = target("https://app.example.com", "url");
        let cells = coverage_cells(
            StageKind::VulnTriage,
            &asset,
            &BTreeSet::new(),
            &BTreeMap::new(),
        );

        assert_eq!(cells.len(), 10);
        assert!(cells.iter().all(|cell| cell.state == "pending"));
        assert!(cells.iter().all(|cell| cell
            .suggested_tools
            .iter()
            .any(|tool| tool == "vuln_run_formulaic_sweep")));
    }

    #[test]
    fn enumeration_worklist_read_model_keeps_eas_live_web_roots() {
        let live_domain = target("app.example.com", "domain");
        let live_url = target("https://portal.example.com/login", "url");
        let dead_domain = target("dead.example.com", "domain");
        let live_ip = target("203.0.113.10", "ip");
        let found = BTreeSet::from([
            (
                coverage_lookup_asset(
                    &live_domain.value,
                    golish_db::repo::coverage_truth::TECH_EAS_LIVENESS,
                ),
                golish_db::repo::coverage_truth::TECH_EAS_LIVENESS.to_string(),
            ),
            (
                coverage_lookup_asset(
                    &live_url.value,
                    golish_db::repo::coverage_truth::TECH_EAS_LIVENESS,
                ),
                golish_db::repo::coverage_truth::TECH_EAS_LIVENESS.to_string(),
            ),
            (
                coverage_lookup_asset(
                    &live_ip.value,
                    golish_db::repo::coverage_truth::TECH_EAS_LIVENESS,
                ),
                golish_db::repo::coverage_truth::TECH_EAS_LIVENESS.to_string(),
            ),
        ]);

        let filtered = filter_enumeration_assets_by_eas_found(
            vec![live_domain.clone(), live_url.clone(), dead_domain, live_ip],
            &found,
            &BTreeSet::new(),
        );

        assert_eq!(
            filtered
                .iter()
                .map(|asset| asset.value.as_str())
                .collect::<Vec<_>>(),
            vec![live_domain.value.as_str(), live_url.value.as_str()]
        );
    }

    #[test]
    fn enumeration_worklist_read_model_does_not_filter_to_empty_without_eas_truth() {
        let assets = vec![
            target("app.example.com", "domain"),
            target("203.0.113.10", "ip"),
        ];

        let filtered = filter_enumeration_assets_by_eas_found(
            assets.clone(),
            &BTreeSet::new(),
            &BTreeSet::new(),
        );

        assert_eq!(filtered.len(), assets.len());
    }

    #[test]
    fn enumeration_worklist_read_model_keeps_http_proven_ip_web_assets() {
        let live_domain = target("app.example.com", "domain");
        let web_ip = target("203.0.113.10", "ip");
        let non_web_ip = target("203.0.113.11", "ip");
        let found = BTreeSet::from([(
            coverage_lookup_asset(
                &live_domain.value,
                golish_db::repo::coverage_truth::TECH_EAS_LIVENESS,
            ),
            golish_db::repo::coverage_truth::TECH_EAS_LIVENESS.to_string(),
        )]);
        let web_capable = BTreeSet::from([web_ip.value.clone()]);

        let filtered = filter_enumeration_assets_by_eas_found(
            vec![live_domain.clone(), web_ip.clone(), non_web_ip],
            &found,
            &web_capable,
        );

        assert_eq!(
            filtered
                .iter()
                .map(|asset| asset.value.as_str())
                .collect::<Vec<_>>(),
            vec![live_domain.value.as_str(), web_ip.value.as_str()]
        );
    }

    #[test]
    fn enumeration_web_capable_ip_gets_four_pending_cells() {
        let asset = target("203.0.113.10", "ip");

        let non_web_cells = coverage_cells(
            StageKind::Enumeration,
            &asset,
            &BTreeSet::new(),
            &BTreeMap::new(),
        );
        assert!(non_web_cells
            .iter()
            .all(|cell| cell.state == "not_applicable"));

        let web_cells = coverage_cells_with_eas_parent_ips(
            StageKind::Enumeration,
            &asset,
            &BTreeSet::new(),
            &BTreeMap::new(),
            &BTreeSet::new(),
            &BTreeSet::from([asset.value.clone()]),
            &BTreeSet::new(),
        );
        assert_eq!(
            web_cells
                .iter()
                .map(|cell| (cell.label.as_str(), cell.state.as_str()))
                .collect::<Vec<_>>(),
            vec![
                ("JS", "pending"),
                ("Directory", "pending"),
                ("Parameter", "pending"),
                ("API", "pending")
            ]
        );
    }

    #[test]
    fn enum_content_not_applicable_terminalises_pending_web_ip_axes() {
        // design 2026-07-03: a web-capable IP that port truth proves is
        // DNS/53-only gets its four pending ENUM axes terminalised as
        // not_applicable so it does not wedge the gate / clutter the worklist.
        let asset = target("203.0.113.10", "ip");
        let mut cells = coverage_cells_with_eas_parent_ips(
            StageKind::Enumeration,
            &asset,
            &BTreeSet::new(),
            &BTreeMap::new(),
            &BTreeSet::new(),
            &BTreeSet::from([asset.value.clone()]),
            &BTreeSet::new(),
        );
        assert!(cells.iter().all(|c| c.state == "pending"));

        apply_enum_content_not_applicable(
            StageKind::Enumeration,
            &asset,
            &mut cells,
            &BTreeSet::from([asset.value.clone()]),
        );
        assert!(cells.iter().all(|c| c.state == "not_applicable"));
        assert!(cells[0]
            .note
            .as_deref()
            .is_some_and(|n| n.contains("only DNS/53")));

        // An IP not in the not_applicable set keeps its pending axes.
        let mut kept = coverage_cells_with_eas_parent_ips(
            StageKind::Enumeration,
            &asset,
            &BTreeSet::new(),
            &BTreeMap::new(),
            &BTreeSet::new(),
            &BTreeSet::from([asset.value.clone()]),
            &BTreeSet::new(),
        );
        apply_enum_content_not_applicable(
            StageKind::Enumeration,
            &asset,
            &mut kept,
            &BTreeSet::new(),
        );
        assert!(kept.iter().all(|c| c.state == "pending"));
    }

    #[test]
    fn enumeration_pending_cells_only_suggest_current_first_party_tools() {
        let asset = target("https://app.example.com", "url");

        let cells = coverage_cells(
            StageKind::Enumeration,
            &asset,
            &BTreeSet::new(),
            &BTreeMap::new(),
        );

        assert_eq!(
            cells[0].suggested_tools,
            vec!["browser_collect_js_api".to_string()]
        );
        assert_eq!(
            cells[1].suggested_tools,
            vec!["route_probe_paths".to_string()]
        );
        assert_eq!(
            cells[2].suggested_tools,
            vec![
                "browser_collect_js_api".to_string(),
                "js_extract_apis".to_string()
            ]
        );
        assert_eq!(
            cells[3].suggested_tools,
            vec!["js_extract_apis".to_string()]
        );
        assert!(cells
            .iter()
            .flat_map(|cell| cell.suggested_tools.iter())
            .all(|tool| tool != "ffuf" && tool != "arjun"));
    }

    #[test]
    fn latest_fallback_sql_aliases_evidence_ids_for_projection_rows() {
        assert!(LATEST_TECHNIQUE_OUTCOMES_SQL.contains("evidence_ids AS evidence_refs"));
        assert!(LATEST_SOURCE_QUERY_ROWS_SQL.contains("evidence_ids AS evidence_refs"));
        assert!(SESSION_EVIDENCE_FACT_ROWS_SQL.contains("ARRAY[id]::bigint[] AS evidence_refs"));
    }

    #[test]
    fn eas_url_asset_only_requires_liveness() {
        let asset = target("https://app.example.com/login", "url");
        let host_only_found = BTreeSet::from([(
            "app.example.com".to_string(),
            golish_db::repo::coverage_truth::TECH_EAS_LIVENESS.to_string(),
        )]);

        let host_only_cells = coverage_cells(
            golish_agent_kit::harness::StageKind::ExternalAttackSurface,
            &asset,
            &host_only_found,
            &BTreeMap::new(),
        );

        assert_eq!(host_only_cells[0].state, "pending");

        let endpoint_found = BTreeSet::from([(
            "app.example.com/login".to_string(),
            golish_db::repo::coverage_truth::TECH_EAS_LIVENESS.to_string(),
        )]);

        let cells = coverage_cells(
            golish_agent_kit::harness::StageKind::ExternalAttackSurface,
            &asset,
            &endpoint_found,
            &BTreeMap::new(),
        );

        assert_eq!(cells[0].state, "found");
        assert_eq!(cells[1].state, "not_applicable");
        assert_eq!(cells[2].state, "not_applicable");
    }

    #[test]
    fn eas_url_shaped_hostname_domain_asset_uses_value_aware_applicability() {
        let asset = target("https://app.example.com/login", "domain");
        let endpoint_found = BTreeSet::from([(
            "app.example.com/login".to_string(),
            golish_db::repo::coverage_truth::TECH_EAS_LIVENESS.to_string(),
        )]);

        let cells = coverage_cells(
            golish_agent_kit::harness::StageKind::ExternalAttackSurface,
            &asset,
            &endpoint_found,
            &BTreeMap::new(),
        );

        assert_eq!(cells[0].state, "found");
        assert_eq!(cells[1].state, "not_applicable");
        assert_eq!(cells[2].state, "not_applicable");
    }

    #[test]
    fn eas_domain_alias_to_existing_ip_does_not_count_as_direct_coverage_asset() {
        let ip = target("115.28.135.55", "ip");
        let mut domain = target("moresec.cn", "domain");
        domain.real_ip = "115.28.135.55".to_string();
        let assets = vec![ip, domain.clone()];
        let parent_ips = eas_direct_ip_target_keys(&assets);

        assert!(!counts_as_coverage_asset(
            StageKind::ExternalAttackSurface,
            &domain,
            &parent_ips
        ));

        let cells = coverage_cells_with_eas_parent_ips(
            StageKind::ExternalAttackSurface,
            &domain,
            &BTreeSet::new(),
            &BTreeMap::new(),
            &parent_ips,
            &BTreeSet::new(),
            &BTreeSet::new(),
        );

        assert!(cells.iter().all(|cell| cell.state == "not_applicable"));
        assert!(cells.iter().all(|cell| cell.suggested_tools.is_empty()));
        assert!(cells[0]
            .note
            .as_deref()
            .is_some_and(|note| note.contains("115.28.135.55")));
    }

    #[test]
    fn eas_ip_endpoint_alias_to_existing_ip_does_not_count_as_direct_coverage_asset() {
        let ip = target("115.28.135.55", "ip");
        let endpoint = target("http://115.28.135.55:8080/login", "url");
        let assets = vec![ip, endpoint.clone()];
        let parent_ips = eas_direct_ip_target_keys(&assets);

        assert!(!counts_as_coverage_asset(
            StageKind::ExternalAttackSurface,
            &endpoint,
            &parent_ips
        ));

        let cells = coverage_cells_with_eas_parent_ips(
            StageKind::ExternalAttackSurface,
            &endpoint,
            &BTreeSet::new(),
            &BTreeMap::new(),
            &parent_ips,
            &BTreeSet::new(),
            &BTreeSet::new(),
        );

        assert_eq!(
            cells
                .iter()
                .map(|cell| cell.state.as_str())
                .collect::<Vec<_>>(),
            vec!["not_applicable", "not_applicable", "not_applicable"]
        );
    }

    #[test]
    fn eas_domain_without_existing_ip_remains_a_direct_coverage_asset() {
        let mut domain = target("moresec.cn", "domain");
        domain.real_ip = "115.28.135.55".to_string();

        assert!(counts_as_coverage_asset(
            StageKind::ExternalAttackSurface,
            &domain,
            &BTreeSet::new()
        ));

        let cells = coverage_cells(
            StageKind::ExternalAttackSurface,
            &domain,
            &BTreeSet::new(),
            &BTreeMap::new(),
        );

        assert_eq!(
            cells
                .iter()
                .map(|cell| (cell.technique.as_str(), cell.state.as_str()))
                .collect::<Vec<_>>(),
            vec![
                (
                    golish_db::repo::coverage_truth::TECH_EAS_LIVENESS,
                    "pending"
                ),
                (
                    golish_db::repo::coverage_truth::TECH_EAS_PORT,
                    "not_applicable"
                ),
                (
                    golish_db::repo::coverage_truth::TECH_EAS_SERVICE_FP,
                    "not_applicable"
                )
            ]
        );
    }

    #[test]
    fn empty_outcome_is_checked_empty() {
        let asset = target("203.0.113.10", "ip");
        let outcomes = BTreeMap::from([(
            (
                asset.value.clone(),
                golish_db::repo::coverage_truth::TECH_EAS_PORT.to_string(),
            ),
            OutcomeProjection {
                state: outcome_state("empty"),
                source: Some("naabu".to_string()),
                evidence_refs: vec![42],
            },
        )]);

        let cells = coverage_cells(
            golish_agent_kit::harness::StageKind::ExternalAttackSurface,
            &asset,
            &BTreeSet::new(),
            &outcomes,
        );

        assert_eq!(cells[1].state, "checked_empty");
        assert_eq!(cells[1].source.as_deref(), Some("naabu"));
        assert_eq!(cells[1].evidence_refs, vec![42]);
    }

    #[test]
    fn eas_port_checked_empty_makes_service_not_applicable() {
        let asset = target("122.114.60.205", "ip");
        let outcomes = BTreeMap::from([(
            (
                asset.value.clone(),
                golish_db::repo::coverage_truth::TECH_EAS_PORT.to_string(),
            ),
            OutcomeProjection {
                state: outcome_state("empty"),
                source: Some("naabu".to_string()),
                evidence_refs: vec![42],
            },
        )]);

        let cells = coverage_cells(
            golish_agent_kit::harness::StageKind::ExternalAttackSurface,
            &asset,
            &BTreeSet::new(),
            &outcomes,
        );

        assert_eq!(cells[1].state, "checked_empty");
        assert_eq!(cells[2].state, "not_applicable");
        assert_eq!(
            cells[2].note.as_deref(),
            Some("no open ports were found, so service fingerprinting is not applicable")
        );
        assert!(cells[2].suggested_tools.is_empty());
    }

    #[test]
    fn eas_port_found_keeps_service_pending_without_service_outcome() {
        let asset = target("122.114.60.205", "ip");
        let found = BTreeSet::from([(
            asset.value.clone(),
            golish_db::repo::coverage_truth::TECH_EAS_PORT.to_string(),
        )]);

        let cells = coverage_cells(
            golish_agent_kit::harness::StageKind::ExternalAttackSurface,
            &asset,
            &found,
            &BTreeMap::new(),
        );

        assert_eq!(cells[1].state, "found");
        assert_eq!(cells[2].state, "pending");
        assert_eq!(cells[2].suggested_tools, vec!["nmap".to_string()]);
    }

    #[test]
    fn eas_dns_only_ip_makes_service_not_applicable() {
        let asset = target("122.114.60.53", "ip");
        let found = BTreeSet::from([(
            asset.value.clone(),
            golish_db::repo::coverage_truth::TECH_EAS_PORT.to_string(),
        )]);
        let service_not_applicable = BTreeSet::from([asset.value.clone()]);

        let cells = coverage_cells_with_eas_parent_ips(
            golish_agent_kit::harness::StageKind::ExternalAttackSurface,
            &asset,
            &found,
            &BTreeMap::new(),
            &BTreeSet::new(),
            &BTreeSet::new(),
            &service_not_applicable,
        );

        assert_eq!(cells[1].state, "found");
        assert_eq!(cells[2].state, "not_applicable");
        assert!(cells[2]
            .note
            .as_deref()
            .is_some_and(|note| note.contains("only DNS/53 is open")));
        assert!(cells[2].suggested_tools.is_empty());
    }

    #[test]
    fn eas_explicit_service_outcome_wins_over_port_empty_derivation() {
        let asset = target("122.114.60.205", "ip");
        let outcomes = BTreeMap::from([
            (
                (
                    asset.value.clone(),
                    golish_db::repo::coverage_truth::TECH_EAS_PORT.to_string(),
                ),
                OutcomeProjection {
                    state: outcome_state("empty"),
                    source: Some("naabu".to_string()),
                    evidence_refs: vec![42],
                },
            ),
            (
                (
                    asset.value.clone(),
                    golish_db::repo::coverage_truth::TECH_EAS_SERVICE_FP.to_string(),
                ),
                OutcomeProjection {
                    state: outcome_state("error"),
                    source: Some("nmap".to_string()),
                    evidence_refs: vec![43],
                },
            ),
        ]);

        let cells = coverage_cells(
            golish_agent_kit::harness::StageKind::ExternalAttackSurface,
            &asset,
            &BTreeSet::new(),
            &outcomes,
        );

        assert_eq!(cells[1].state, "checked_empty");
        assert_eq!(cells[2].state, "error");
        assert_eq!(cells[2].source.as_deref(), Some("nmap"));
        assert_eq!(cells[2].evidence_refs, vec![43]);
    }

    #[test]
    fn not_applicable_outcome_stays_terminal_in_read_model() {
        let asset = target("203.0.113.10", "ip");
        let outcomes = BTreeMap::from([(
            (
                asset.value.clone(),
                golish_db::repo::coverage_truth::TECH_EAS_PORT.to_string(),
            ),
            OutcomeProjection {
                state: outcome_state("not_applicable"),
                source: Some("submit_stage_deliverable".to_string()),
                evidence_refs: Vec::new(),
            },
        )]);

        let cells = coverage_cells(
            golish_agent_kit::harness::StageKind::ExternalAttackSurface,
            &asset,
            &BTreeSet::new(),
            &outcomes,
        );

        assert_eq!(cells[1].state, "not_applicable");
        assert_eq!(cells[1].source.as_deref(), Some("submit_stage_deliverable"));
    }

    #[test]
    fn error_outcome_is_distinct_from_blocked() {
        let asset = target("203.0.113.10", "ip");
        let outcomes = BTreeMap::from([(
            (
                asset.value.clone(),
                golish_db::repo::coverage_truth::TECH_EAS_PORT.to_string(),
            ),
            OutcomeProjection {
                state: outcome_state("error"),
                source: Some("naabu".to_string()),
                evidence_refs: vec![43],
            },
        )]);

        let cells = coverage_cells(
            golish_agent_kit::harness::StageKind::ExternalAttackSurface,
            &asset,
            &BTreeSet::new(),
            &outcomes,
        );

        assert_eq!(cells[1].state, "error");
        assert_eq!(cells[1].source.as_deref(), Some("naabu"));
        assert_eq!(cells[1].evidence_refs, vec![43]);
    }

    #[test]
    fn active_discovered_source_marks_new_asset() {
        let mut asset = target("203.0.113.10", "ip");
        asset.source = Some("active_discovered".to_string());
        assert_eq!(discovered_phase(&asset, None), "new_in_stage");
    }

    #[test]
    fn next_wave_cells_are_marked_without_suggested_tools() {
        let asset = target("203.0.113.10", "ip");

        let cells = next_wave_coverage_cells(StageKind::ExternalAttackSurface, &asset);

        assert_eq!(cells[0].state, NEXT_WAVE_PENDING);
        assert_eq!(
            cells[0].note.as_deref(),
            Some("newly discovered during this stage; queued for the next wave")
        );
        assert!(cells[0].suggested_tools.is_empty());
    }

    #[test]
    fn wave_cutoff_treats_equal_timestamp_as_current_wave() {
        let mut asset = target("203.0.113.10", "ip");
        let cutoff = Utc::now();
        asset.created_at = cutoff;

        assert!(!is_next_wave_asset(&asset, Some(cutoff)));
    }

    #[test]
    fn target_intel_org_row_only_requires_org_context_dimensions() {
        let asset = target("Acme Root", "organization");

        let cells = coverage_cells(
            StageKind::TargetIntel,
            &asset,
            &BTreeSet::new(),
            &BTreeMap::new(),
        );

        assert_eq!(
            cells
                .iter()
                .map(|cell| cell.label.as_str())
                .collect::<Vec<_>>(),
            vec!["DNS", "WHOIS", "ASN", "CT", "Subdomain", "OSINT"]
        );
        assert_eq!(cells[0].state, "not_applicable");
        assert_eq!(cells[1].state, "pending");
        assert_eq!(cells[2].state, "pending");
        assert_eq!(cells[3].state, "not_applicable");
        assert_eq!(cells[4].state, "not_applicable");
        assert_eq!(cells[5].state, "pending");
        assert!(cells[0].suggested_tools.is_empty());
        assert!(cells[3].suggested_tools.is_empty());
        assert!(cells[4].suggested_tools.is_empty());
        assert_eq!(
            cells[1].suggested_tools,
            vec!["recon_lookup_whois".to_string()]
        );
        assert_eq!(
            cells[5].suggested_tools,
            vec!["recon_map_assets".to_string()]
        );
    }

    #[test]
    fn source_query_found_does_not_create_found_cell() {
        assert_eq!(source_query_terminal_state("found"), None);
        assert_eq!(
            source_query_terminal_state("empty").as_deref(),
            Some("checked_empty")
        );
        assert_eq!(
            source_query_terminal_state("error").as_deref(),
            Some("error")
        );
    }

    #[test]
    fn evidence_fact_row_fills_missing_liveness_cell() {
        let asset_values = vec!["157.240.9.36".to_string()];
        let stage_techniques =
            BTreeSet::from([golish_db::repo::coverage_truth::TECH_EAS_LIVENESS.to_string()]);
        let mut outcomes = BTreeMap::new();

        merge_evidence_fact_row(
            &mut outcomes,
            &asset_values,
            &stage_techniques,
            EvidenceFactProjectionRow {
                asset: "157.240.9.36".to_string(),
                technique: golish_db::repo::coverage_truth::TECH_EAS_LIVENESS.to_string(),
                outcome: "empty".to_string(),
                source: Some("httpx".to_string()),
                evidence_refs: vec![12699],
                raw_output: None,
            },
        );

        let key = (
            coverage_lookup_asset(
                &asset_values[0],
                golish_db::repo::coverage_truth::TECH_EAS_LIVENESS,
            ),
            golish_db::repo::coverage_truth::TECH_EAS_LIVENESS.to_string(),
        );
        let outcome = outcomes.get(&key).expect("evidence fact is merged");
        assert_eq!(outcome.state, "checked_empty");
        assert_eq!(outcome.source.as_deref(), Some("httpx"));
        assert_eq!(outcome.evidence_refs, vec![12699]);
    }

    #[test]
    fn unresolvable_eas_port_error_is_displayed_as_checked_empty() {
        let asset = target("203.0.113.10", "ip");
        let asset_values = vec![asset.value.clone()];
        let stage_techniques =
            BTreeSet::from([golish_db::repo::coverage_truth::TECH_EAS_PORT.to_string()]);
        let mut outcomes = BTreeMap::new();

        merge_evidence_fact_row(
            &mut outcomes,
            &asset_values,
            &stage_techniques,
            EvidenceFactProjectionRow {
                asset: asset.value.clone(),
                technique: golish_db::repo::coverage_truth::TECH_EAS_PORT.to_string(),
                outcome: "error".to_string(),
                source: Some("nmap".to_string()),
                evidence_refs: vec![12710],
                raw_output: Some(
                    "Failed to resolve \"www.google...\".\nWARNING: No targets were specified, so 0 hosts scanned."
                        .to_string(),
                ),
            },
        );

        let cells = coverage_cells(
            StageKind::ExternalAttackSurface,
            &asset,
            &BTreeSet::new(),
            &outcomes,
        );

        assert_eq!(cells[1].state, "checked_empty");
        assert_eq!(cells[1].source.as_deref(), Some("nmap"));
        assert_eq!(cells[1].evidence_refs, vec![12710]);
        assert_eq!(cells[2].state, "not_applicable");
    }

    #[test]
    fn generic_evidence_error_stays_error() {
        assert_eq!(
            evidence_outcome_state(
                "error",
                golish_db::repo::coverage_truth::TECH_EAS_PORT,
                Some("nmap crashed while parsing arguments"),
            ),
            "error"
        );
    }

    #[test]
    fn outcome_row_matches_liveness_endpoint_alias() {
        let asset_values = vec!["http://app.example.com:90".to_string()];
        let stage_techniques =
            BTreeSet::from([golish_db::repo::coverage_truth::TECH_EAS_LIVENESS.to_string()]);
        let mut outcomes = BTreeMap::new();

        merge_technique_outcome_row(
            &mut outcomes,
            &asset_values,
            &stage_techniques,
            TechniqueOutcomeProjectionRow {
                asset: "app.example.com:90".to_string(),
                technique: golish_db::repo::coverage_truth::TECH_EAS_LIVENESS.to_string(),
                outcome: "empty".to_string(),
                source: Some("httpx".to_string()),
                evidence_refs: vec![7],
            },
        );

        let key = (
            coverage_lookup_asset(
                &asset_values[0],
                golish_db::repo::coverage_truth::TECH_EAS_LIVENESS,
            ),
            golish_db::repo::coverage_truth::TECH_EAS_LIVENESS.to_string(),
        );
        let outcome = outcomes.get(&key).expect("endpoint outcome is merged");
        assert_eq!(outcome.state, "checked_empty");
        assert_eq!(outcome.source.as_deref(), Some("httpx"));
        assert_eq!(outcome.evidence_refs, vec![7]);
    }

    #[test]
    fn host_level_eas_outcome_matches_url_wrapped_ip_asset() {
        let asset_values = vec!["http://115.159.235.124:8080".to_string()];
        let stage_techniques =
            BTreeSet::from([golish_db::repo::coverage_truth::TECH_EAS_PORT.to_string()]);
        let mut outcomes = BTreeMap::new();

        merge_technique_outcome_row(
            &mut outcomes,
            &asset_values,
            &stage_techniques,
            TechniqueOutcomeProjectionRow {
                asset: "115.159.235.124".to_string(),
                technique: golish_db::repo::coverage_truth::TECH_EAS_PORT.to_string(),
                outcome: "empty".to_string(),
                source: Some("naabu".to_string()),
                evidence_refs: vec![42],
            },
        );

        let key = (
            coverage_lookup_asset(
                &asset_values[0],
                golish_db::repo::coverage_truth::TECH_EAS_PORT,
            ),
            golish_db::repo::coverage_truth::TECH_EAS_PORT.to_string(),
        );
        let outcome = outcomes
            .get(&key)
            .expect("host-level outcome is merged onto URL-wrapped IP asset");
        assert_eq!(outcome.state, "checked_empty");
        assert_eq!(outcome.source.as_deref(), Some("naabu"));
        assert_eq!(outcome.evidence_refs, vec![42]);
    }

    #[test]
    fn outcome_row_ignores_assets_outside_current_snapshot() {
        let asset_values = vec!["app.example.com".to_string()];
        let stage_techniques =
            BTreeSet::from([golish_db::repo::coverage_truth::TECH_EAS_PORT.to_string()]);
        let mut outcomes = BTreeMap::new();

        merge_technique_outcome_row(
            &mut outcomes,
            &asset_values,
            &stage_techniques,
            TechniqueOutcomeProjectionRow {
                asset: "other.example.com".to_string(),
                technique: golish_db::repo::coverage_truth::TECH_EAS_PORT.to_string(),
                outcome: "empty".to_string(),
                source: Some("naabu".to_string()),
                evidence_refs: vec![9],
            },
        );

        assert!(outcomes.is_empty());
    }

    #[test]
    fn org_level_source_query_terminal_state_fans_out_to_current_assets() {
        let asset_values = vec!["a.example.com".to_string(), "b.example.com".to_string()];
        let stage_techniques =
            BTreeSet::from([golish_db::repo::coverage_truth::TECH_WHOIS.to_string()]);
        let mut outcomes = BTreeMap::new();

        merge_source_query_row(
            &mut outcomes,
            &asset_values,
            &stage_techniques,
            SourceQueryProjectionRow {
                source: "rdap".to_string(),
                target: String::new(),
                technique: Some(golish_db::repo::coverage_truth::TECH_WHOIS.to_string()),
                status: "empty".to_string(),
                evidence_refs: vec![11],
            },
            &[],
        );

        for asset in asset_values {
            let key = (
                asset,
                golish_db::repo::coverage_truth::TECH_WHOIS.to_string(),
            );
            let outcome = outcomes.get(&key).expect("org-level source row fans out");
            assert_eq!(outcome.state, "checked_empty");
            assert_eq!(outcome.source.as_deref(), Some("rdap"));
            assert_eq!(outcome.evidence_refs, vec![11]);
        }
    }

    #[test]
    fn target_intel_source_query_with_unmatched_target_rolls_up_to_org_row() {
        let organization_asset_values = vec!["大连平安大厦开发有限公司".to_string()];
        let asset_values = organization_asset_values.clone();
        let stage_techniques =
            BTreeSet::from([golish_db::repo::coverage_truth::TECH_WHOIS.to_string()]);
        let mut outcomes = BTreeMap::new();

        merge_source_query_row(
            &mut outcomes,
            &asset_values,
            &stage_techniques,
            SourceQueryProjectionRow {
                source: "crt.sh".to_string(),
                target: "pingan.com".to_string(),
                technique: Some(golish_db::repo::coverage_truth::TECH_WHOIS.to_string()),
                status: "empty".to_string(),
                evidence_refs: vec![21],
            },
            &organization_asset_values,
        );

        let key = (
            organization_asset_values[0].clone(),
            golish_db::repo::coverage_truth::TECH_WHOIS.to_string(),
        );
        let outcome = outcomes
            .get(&key)
            .expect("unmatched target_intel source query rolls up to organization row");
        assert_eq!(outcome.state, "checked_empty");
        assert_eq!(outcome.source.as_deref(), Some("crt.sh"));
        assert_eq!(outcome.evidence_refs, vec![21]);
    }

    #[test]
    fn domain_only_source_query_does_not_roll_up_to_org_row() {
        let organization_asset_values = vec!["大连平安大厦开发有限公司".to_string()];
        let asset_values = organization_asset_values.clone();
        let stage_techniques =
            BTreeSet::from([golish_db::repo::coverage_truth::TECH_DNS.to_string()]);
        let mut outcomes = BTreeMap::new();

        merge_source_query_row(
            &mut outcomes,
            &asset_values,
            &stage_techniques,
            SourceQueryProjectionRow {
                source: "resolver".to_string(),
                target: "pingan.com".to_string(),
                technique: Some(golish_db::repo::coverage_truth::TECH_DNS.to_string()),
                status: "empty".to_string(),
                evidence_refs: vec![21],
            },
            &organization_asset_values,
        );

        assert!(
            outcomes.is_empty(),
            "domain-only DNS source rows should not create company-name coverage"
        );
    }

    #[test]
    fn unmatched_source_query_without_org_row_is_ignored() {
        let asset_values = vec!["registered.example.com".to_string()];
        let stage_techniques =
            BTreeSet::from([golish_db::repo::coverage_truth::TECH_DNS.to_string()]);
        let mut outcomes = BTreeMap::new();

        merge_source_query_row(
            &mut outcomes,
            &asset_values,
            &stage_techniques,
            SourceQueryProjectionRow {
                source: "crt.sh".to_string(),
                target: "other.example.com".to_string(),
                technique: Some(golish_db::repo::coverage_truth::TECH_DNS.to_string()),
                status: "empty".to_string(),
                evidence_refs: vec![22],
            },
            &[],
        );

        assert!(outcomes.is_empty());
    }

    #[test]
    fn outcome_merge_keeps_stronger_terminal_state_and_evidence() {
        let key = (
            "acme.com".to_string(),
            golish_db::repo::coverage_truth::TECH_WHOIS.to_string(),
        );
        let mut outcomes = BTreeMap::new();
        merge_outcome(
            &mut outcomes,
            key.clone(),
            OutcomeProjection {
                state: "checked_empty".to_string(),
                source: Some("rdap".to_string()),
                evidence_refs: vec![1],
            },
        );
        merge_outcome(
            &mut outcomes,
            key.clone(),
            OutcomeProjection {
                state: "blocked".to_string(),
                source: Some("whois".to_string()),
                evidence_refs: vec![2],
            },
        );
        merge_outcome(
            &mut outcomes,
            key.clone(),
            OutcomeProjection {
                state: "checked_empty".to_string(),
                source: Some("cached-rdap".to_string()),
                evidence_refs: vec![2, 3],
            },
        );

        let merged = outcomes.get(&key).expect("merged outcome exists");
        assert_eq!(merged.state, "blocked");
        assert_eq!(merged.source.as_deref(), Some("whois"));
        assert_eq!(merged.evidence_refs, vec![1, 2, 3]);
    }
}
