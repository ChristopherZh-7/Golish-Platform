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
use golish_agent_kit::harness::org_gate::stage_accepts_outcome_projection;
use golish_agent_kit::harness::{
    suggested_capabilities_for_any_technique, tools_from_suggestions, StageCapabilitySuggestion,
    StageKind,
};

use crate::ai::db_bridge::evidence::{
    eas_target_bound_evidence_fact_set, enumeration_target_bound_evidence_fact_set,
    projected_technique_outcome_evidence_id, vuln_target_bound_evidence_fact_set,
    TargetBoundEvidenceFactSet,
};

#[derive(Debug, Clone, Serialize, Deserialize, TS)]
#[ts(export, export_to = "../../../../frontend/lib/generated/")]
pub struct StageAssetCoverageSnapshot {
    pub stage: String,
    pub organization_id: String,
    pub session_id: Option<String>,
    pub summary: StageAssetCoverageSummary,
    pub assets: Vec<StageAssetCoverageRow>,
    #[serde(default, skip_serializing_if = "Vec::is_empty")]
    #[ts(skip)]
    pub eas_transport_excluded_origins: Vec<EasTransportExcludedOrigin>,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct EasTransportExcludedOrigin {
    pub target_id: String,
    pub origin: String,
    pub reason: String,
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
    /// True only when `value` is an exact normalized `scheme://host:port`
    /// identity. Enumeration and Vuln Triage snapshots exclude false rows
    /// entirely.
    #[serde(default)]
    #[ts(skip)]
    pub exact_web_origin: bool,
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
    #[serde(default, skip_serializing_if = "serde_json::Value::is_null")]
    #[ts(skip)]
    pub details: serde_json::Value,
}

#[derive(Debug, Clone, FromRow)]
struct TargetCoverageRow {
    id: Uuid,
    name: String,
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
    #[sqlx(default)]
    liveness_state: String,
    #[sqlx(default)]
    exact_web_origin: bool,
}

#[derive(Debug, Clone)]
struct OutcomeProjection {
    state: String,
    source: Option<String>,
    evidence_refs: Vec<i64>,
}

#[derive(Debug, Clone, Default)]
struct EasWebOriginCoverage {
    required_by_target: BTreeMap<Uuid, Vec<String>>,
    completed: BTreeMap<String, OutcomeProjection>,
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

fn ui_allows_latest_outcome_fallback(stage: StageKind, _session_id: Option<&str>) -> bool {
    !matches!(
        stage,
        StageKind::ExternalAttackSurface | StageKind::Enumeration | StageKind::VulnTriage
    )
}

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
        None,
        None,
        ui_allows_latest_outcome_fallback(stage_kind, session_id.as_deref()),
        None,
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
    current_wave_target_ids: Option<Vec<Uuid>>,
    current_wave_asset_values: Option<Vec<String>>,
    allow_latest_outcome_fallback: bool,
    operation_id: Option<Uuid>,
) -> anyhow::Result<StageAssetCoverageSnapshot> {
    anyhow::ensure!(
        stage_kind != StageKind::ExternalAttackSurface || run_start.is_some(),
        "external_attack_surface coverage requires current stage_started_at for exact Web Origins"
    );
    anyhow::ensure!(
        stage_kind != StageKind::VulnTriage || operation_id.is_some(),
        "vuln_triage coverage requires a trusted operation id for the final-sealed Enumeration surface and operation-scoped outcomes"
    );
    let mut assets = list_stage_targets(pool, org_id, stage_kind).await?;
    if stage_kind == StageKind::TargetIntel {
        assets = target_intel_stage_input_assets(assets, run_start);
    }
    let current_wave =
        explicit_current_wave_membership(current_wave_target_ids, current_wave_asset_values)?;
    ensure_current_wave_targets_present(&assets, current_wave.as_ref())?;
    assets = exclude_dead_targets_if_opted_in(stage_kind, assets);
    let wave_cutoff = asset_wave_cutoff(stage_kind, run_start);
    let web_capable_assets = match stage_kind {
        StageKind::Enumeration => {
            let (filtered, web_capable) =
                filter_enumeration_assets_by_eas_worklist(pool, org_id, assets).await?;
            assets = filtered;
            web_capable
        }
        StageKind::ExternalAttackSurface => {
            golish_db::repo::coverage_truth::eas_web_capable_assets(pool, Some(org_id), run_start)
                .await?
                .into_iter()
                .collect()
        }
        _ => BTreeSet::new(),
    };
    let mut eas_transport_excluded_origins = Vec::new();
    if stage_kind == StageKind::Enumeration {
        let transport_exclusions = match operation_id {
            Some(operation_id) => {
                golish_db::repo::operation_state::list_eas_web_transport_blocked_origins(
                    pool,
                    operation_id,
                    org_id,
                )
                .await?
                .into_iter()
                .map(|(target_id, origin)| {
                    let origin = golish_pentest_domain::canonical_web_origin(&origin)
                        .map(|origin| origin.key)
                        .unwrap_or(origin);
                    (target_id, origin)
                })
                .collect()
            }
            None => BTreeSet::new(),
        };
        eas_transport_excluded_origins = transport_exclusions
            .iter()
            .filter(|(target_id, origin)| {
                assets.iter().any(|asset| {
                    asset.id == *target_id
                        && golish_pentest_domain::confirmed_target_web_origins(
                            &asset.name,
                            &asset.value,
                            &asset.ports,
                        )
                        .iter()
                        .any(|candidate| &candidate.key == origin)
                })
            })
            .map(|(target_id, origin)| EasTransportExcludedOrigin {
                target_id: target_id.to_string(),
                origin: origin.clone(),
                reason: "three_same_class_whatweb_failures_and_independent_transport_block"
                    .to_string(),
            })
            .collect();
        assets = expand_enumeration_web_origin_rows_for_wave_excluding(
            assets,
            current_wave.as_ref().map(|wave| &wave.target_ids),
            &transport_exclusions,
        );
    } else if stage_kind == StageKind::VulnTriage {
        let inherited_origins = match operation_id {
            Some(operation_id) => {
                golish_db::repo::stage_handoffs::list_final_sealed_enumeration_origins(
                    pool,
                    operation_id,
                    org_id,
                )
                .await?
            }
            None => BTreeSet::new(),
        };
        assets = filter_vuln_assets_by_enumeration_surface(
            expand_exact_web_origin_rows_for_wave_excluding(
                assets,
                current_wave.as_ref().map(|wave| &wave.target_ids),
                &BTreeSet::new(),
            ),
            &inherited_origins,
        )?;
    }
    let current_assets: Vec<&TargetCoverageRow> = assets
        .iter()
        .filter(|asset| {
            !is_deferred_wave_asset(
                asset,
                wave_cutoff,
                current_wave.as_ref().map(|wave| &wave.target_ids),
            )
        })
        .collect();
    let asset_values: Vec<String> = current_assets.iter().map(|a| a.value.clone()).collect();
    let asset_types: Vec<String> = current_assets
        .iter()
        .map(|a| a.target_type.clone())
        .collect();
    let organization_asset_values: Vec<String> = current_assets
        .iter()
        .filter(|asset| is_organization_coverage_row(asset))
        .map(|asset| asset.value.clone())
        .collect();

    let business_found: BTreeSet<(String, String)> =
        golish_db::repo::coverage_truth::coverage_truth_facts(
            pool,
            Some(org_id),
            &asset_values,
            &asset_types,
            run_start,
        )
        .await?
        .into_iter()
        .map(|(asset, technique)| {
            (
                coverage_lookup_asset(&asset, technique),
                technique.to_string(),
            )
        })
        .collect();
    // EAS business truth corroborates a strict guarded found outcome but cannot
    // terminalise a cell by itself. Enumeration and Vuln Triage likewise use
    // exact-origin outcomes only. Other stages retain direct DB-found rendering.
    let found = if business_truth_closes_stage_cells(stage_kind) {
        business_found.clone()
    } else {
        BTreeSet::new()
    };
    // EAS SERVICE is strict per confirmed-open port: once a port exists, it must
    // have a port-level fingerprint attempt/result or an explicit terminal
    // outcome. The DNS/53-only helper remains for Enumeration content axes, but
    // EAS SERVICE no longer auto-terminalises open ports as not_applicable.
    let service_not_applicable_assets: BTreeSet<String> = BTreeSet::new();
    let outcome_run_id = if stage_kind == StageKind::VulnTriage {
        operation_id.map(|operation_id| operation_id.to_string())
    } else {
        session_id.map(str::to_string)
    };
    let outcomes = stage_outcomes(
        pool,
        org_id,
        stage_kind,
        outcome_run_id.as_deref(),
        session_id,
        matches!(
            stage_kind,
            StageKind::ExternalAttackSurface | StageKind::Enumeration | StageKind::VulnTriage
        )
        .then_some(run_start)
        .flatten(),
        &asset_values,
        &organization_asset_values,
        &business_found,
        allow_latest_outcome_fallback,
    )
    .await?;
    let eas_web_origins = if stage_kind == StageKind::ExternalAttackSurface {
        load_eas_web_origin_coverage(
            pool,
            org_id,
            session_id,
            run_start,
            current_wave.as_ref().map(|wave| &wave.target_ids),
        )
        .await?
    } else {
        EasWebOriginCoverage::default()
    };

    let mut rows = Vec::with_capacity(assets.len());
    let mut done_assets = 0usize;
    let mut pending_assets = 0usize;
    let mut blocked_assets = 0usize;
    let mut seed_assets = 0usize;
    let mut new_assets = 0usize;
    let mut current_wave_assets = 0usize;

    for asset in assets {
        let next_wave = is_deferred_wave_asset(
            &asset,
            wave_cutoff,
            current_wave.as_ref().map(|wave| &wave.target_ids),
        );
        let phase = if next_wave {
            "new_in_stage".to_string()
        } else {
            discovered_phase(&asset, run_start)
        };
        let counts_as_asset = counts_as_coverage_asset(&asset);
        let mut coverage = if next_wave {
            next_wave_coverage_cells_with_eas_parent_ips(stage_kind, &asset, &web_capable_assets)
        } else {
            coverage_cells_with_eas_parent_ips(
                stage_kind,
                &asset,
                &found,
                &outcomes,
                &web_capable_assets,
                &service_not_applicable_assets,
            )
        };
        if !next_wave {
            apply_eas_web_origin_details(stage_kind, &asset, &mut coverage, &eas_web_origins);
        }
        if next_wave && counts_as_asset {
            if current_wave.is_none() {
                new_assets += 1;
            }
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
            let has_pending = coverage
                .iter()
                .any(|cell| matches!(cell.state.as_str(), "pending" | "partial"));
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
            exact_web_origin: asset.exact_web_origin,
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
        eas_transport_excluded_origins,
    })
}

async fn load_eas_web_origin_coverage(
    pool: &sqlx::PgPool,
    organization_id: Uuid,
    run_id: Option<&str>,
    run_start: Option<DateTime<Utc>>,
    current_wave_target_ids: Option<&BTreeSet<Uuid>>,
) -> anyhow::Result<EasWebOriginCoverage> {
    let Some(run_start) = run_start else {
        return Ok(EasWebOriginCoverage::default());
    };
    let wave_ids = current_wave_target_ids.map(|ids| ids.iter().copied().collect::<Vec<_>>());
    let required_rows =
        golish_db::repo::surface_identity_queries::list_eas_required_web_origin_rows(
            pool,
            organization_id,
            run_start,
            wave_ids.as_deref(),
        )
        .await?;
    let mut required_by_target = BTreeMap::<Uuid, Vec<String>>::new();
    for row in required_rows {
        let Some(origin) = golish_pentest_domain::canonical_web_origin(&row.origin) else {
            anyhow::bail!(
                "malformed authoritative EAS Web Origin '{}' for target {}",
                row.origin,
                row.target_id
            );
        };
        required_by_target
            .entry(row.target_id)
            .or_default()
            .push(origin.key);
    }
    for origins in required_by_target.values_mut() {
        origins.sort();
        origins.dedup();
    }

    let Some(run_id) = run_id.map(str::trim).filter(|run_id| !run_id.is_empty()) else {
        return Ok(EasWebOriginCoverage {
            required_by_target,
            completed: BTreeMap::new(),
        });
    };
    let outcome_rows = golish_db::repo::technique_outcomes::list_for_run_fresh(
        pool,
        organization_id,
        run_id,
        Some(run_start),
    )
    .await?;
    let evidence_rows = golish_db::repo::audit::evidence_facts_for_session_org_fresh(
        pool,
        run_id,
        organization_id,
        run_start,
    )
    .await?;
    let guarded = eas_target_bound_evidence_fact_set(organization_id, evidence_rows);
    let mut completed = BTreeMap::new();
    for row in outcome_rows {
        if row.technique != golish_db::repo::coverage_truth::TECH_EAS_WEB_FP
            || !matches!(row.outcome.as_str(), "found" | "empty" | "blocked")
        {
            continue;
        }
        let Some(evidence_id) = projected_technique_outcome_evidence_id(
            &row.asset,
            &row.technique,
            &row.outcome,
            &row.evidence_ids,
            &guarded,
            row.source.as_deref(),
        ) else {
            continue;
        };
        let Some(origin) = golish_pentest_domain::canonical_web_origin(&row.asset) else {
            continue;
        };
        completed.insert(
            origin.key,
            OutcomeProjection {
                state: outcome_state(&row.outcome),
                source: row.source,
                evidence_refs: vec![evidence_id],
            },
        );
    }
    Ok(EasWebOriginCoverage {
        required_by_target,
        completed,
    })
}

fn asset_wave_cutoff(stage: StageKind, run_start: Option<DateTime<Utc>>) -> Option<DateTime<Utc>> {
    let spec = golish_agent_kit::harness::load_embedded_stage_spec(stage).ok()?;
    spec.asset_wave_barrier.then_some(run_start).flatten()
}

#[derive(Debug, Clone)]
struct CurrentWaveMembership {
    target_ids: BTreeSet<Uuid>,
}

fn explicit_current_wave_membership(
    current_wave_target_ids: Option<Vec<Uuid>>,
    current_wave_asset_values: Option<Vec<String>>,
) -> anyhow::Result<Option<CurrentWaveMembership>> {
    let (target_ids, asset_values) = match (current_wave_target_ids, current_wave_asset_values) {
        (None, None) => return Ok(None),
        (Some(target_ids), Some(asset_values)) => (target_ids, asset_values),
        _ => anyhow::bail!("explicit current asset wave requires both target_ids and asset_values"),
    };
    anyhow::ensure!(
        !target_ids.is_empty() && !asset_values.is_empty(),
        "explicit current asset wave has no items"
    );
    anyhow::ensure!(
        target_ids.len() == asset_values.len(),
        "explicit current asset wave has mismatched target_ids and asset_values"
    );
    anyhow::ensure!(
        target_ids.iter().all(|target_id| !target_id.is_nil()),
        "explicit current asset wave contains a nil target_id"
    );
    anyhow::ensure!(
        asset_values.iter().all(|value| !value.trim().is_empty()),
        "explicit current asset wave contains a blank asset_value"
    );
    let target_ids = target_ids.into_iter().collect::<BTreeSet<_>>();
    anyhow::ensure!(
        target_ids.len() == asset_values.len(),
        "explicit current asset wave contains duplicate target_ids"
    );
    Ok(Some(CurrentWaveMembership { target_ids }))
}

fn ensure_current_wave_targets_present(
    assets: &[TargetCoverageRow],
    current_wave: Option<&CurrentWaveMembership>,
) -> anyhow::Result<()> {
    let Some(current_wave) = current_wave else {
        return Ok(());
    };
    let present = assets.iter().map(|asset| asset.id).collect::<BTreeSet<_>>();
    let missing = current_wave
        .target_ids
        .difference(&present)
        .copied()
        .collect::<Vec<_>>();
    anyhow::ensure!(
        missing.is_empty(),
        "current asset wave references missing or out-of-scope target ids: {missing:?}"
    );
    Ok(())
}

fn is_next_wave_asset(asset: &TargetCoverageRow, wave_cutoff: Option<DateTime<Utc>>) -> bool {
    wave_cutoff.is_some_and(|cutoff| asset.created_at > cutoff)
}

fn is_deferred_wave_asset(
    asset: &TargetCoverageRow,
    wave_cutoff: Option<DateTime<Utc>>,
    current_wave_target_ids: Option<&BTreeSet<Uuid>>,
) -> bool {
    if let Some(current_target_ids) = current_wave_target_ids {
        return !current_target_ids.contains(&asset.id);
    }
    is_next_wave_asset(asset, wave_cutoff)
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

fn target_intel_anchor_only_assets(assets: Vec<TargetCoverageRow>) -> Vec<TargetCoverageRow> {
    let anchors: Vec<(Uuid, String)> = assets
        .iter()
        .filter(|asset| matches!(asset.target_type.as_str(), "domain" | "wildcard"))
        .map(|asset| (asset.id, asset.value.clone()))
        .collect();
    assets
        .into_iter()
        .filter(|asset| {
            asset.target_type != "domain"
                || !anchors.iter().any(|(parent_id, parent)| {
                    *parent_id != asset.id
                        && golish_agent_kit::harness::technique_resolver::target_intel_anchor_covers_child(
                            parent,
                            &asset.value,
                        )
                })
        })
        .collect()
}

fn target_intel_stage_input_assets(
    mut assets: Vec<TargetCoverageRow>,
    stage_started_at: Option<DateTime<Utc>>,
) -> Vec<TargetCoverageRow> {
    if let Some(stage_started_at) = stage_started_at {
        assets.retain(|asset| asset.created_at <= stage_started_at);
    }
    target_intel_anchor_only_assets(assets)
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
            let class = coverage_asset_class(StageKind::Enumeration, asset);
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
                || (matches!(
                    class,
                    golish_agent_kit::harness::technique_resolver::AssetClass::Ip
                        | golish_agent_kit::harness::technique_resolver::AssetClass::Cidr
                ) && !golish_pentest_domain::confirmed_target_web_origins(
                    &asset.name,
                    &asset.value,
                    &asset.ports,
                )
                .is_empty())
        })
        .map(|asset| asset.id)
        .collect();

    assets
        .into_iter()
        .filter(|asset| worklist_ids.contains(&asset.id))
        .collect()
}

fn exclude_dead_targets_if_opted_in(
    stage: StageKind,
    assets: Vec<TargetCoverageRow>,
) -> Vec<TargetCoverageRow> {
    let opted_in = golish_agent_kit::harness::load_embedded_stage_spec(stage)
        .map(|spec| spec.skip_dead_assets)
        .unwrap_or(false);
    if !opted_in {
        return assets;
    }
    assets
        .into_iter()
        .filter(|asset| !asset.liveness_state.eq_ignore_ascii_case("dead"))
        .collect()
}

#[cfg(test)]
fn expand_enumeration_web_origin_rows(assets: Vec<TargetCoverageRow>) -> Vec<TargetCoverageRow> {
    expand_enumeration_web_origin_rows_for_wave(assets, None)
}

#[cfg(test)]
fn expand_enumeration_web_origin_rows_for_wave(
    assets: Vec<TargetCoverageRow>,
    current_wave_target_ids: Option<&BTreeSet<Uuid>>,
) -> Vec<TargetCoverageRow> {
    expand_enumeration_web_origin_rows_for_wave_excluding(
        assets,
        current_wave_target_ids,
        &BTreeSet::new(),
    )
}

fn expand_enumeration_web_origin_rows_for_wave_excluding(
    assets: Vec<TargetCoverageRow>,
    current_wave_target_ids: Option<&BTreeSet<Uuid>>,
    transport_exclusions: &BTreeSet<(Uuid, String)>,
) -> Vec<TargetCoverageRow> {
    expand_exact_web_origin_rows_for_wave_excluding(
        assets,
        current_wave_target_ids,
        transport_exclusions,
    )
}

#[cfg(test)]
fn expand_vuln_triage_web_origin_rows(assets: Vec<TargetCoverageRow>) -> Vec<TargetCoverageRow> {
    expand_exact_web_origin_rows_for_wave_excluding(assets, None, &BTreeSet::new())
}

fn filter_vuln_assets_by_enumeration_surface(
    assets: Vec<TargetCoverageRow>,
    inherited_origins: &BTreeSet<String>,
) -> anyhow::Result<Vec<TargetCoverageRow>> {
    let filtered: Vec<_> = assets
        .into_iter()
        .filter(|asset| inherited_origins.contains(&asset.value))
        .collect();
    let materialized_origins: BTreeSet<_> =
        filtered.iter().map(|asset| asset.value.clone()).collect();
    anyhow::ensure!(
        materialized_origins == *inherited_origins,
        "vuln_triage current target inventory cannot materialize the complete final-sealed Enumeration surface"
    );
    Ok(filtered)
}

/// Materialize one denominator row per exact HTTP(S) origin while retaining
/// the owning target id. Enumeration and Vuln Triage share this identity
/// contract so a self-landed tool outcome closes the same cell that generated
/// its work item.
fn expand_exact_web_origin_rows_for_wave_excluding(
    assets: Vec<TargetCoverageRow>,
    current_wave_target_ids: Option<&BTreeSet<Uuid>>,
    transport_exclusions: &BTreeSet<(Uuid, String)>,
) -> Vec<TargetCoverageRow> {
    let mut rows = Vec::new();
    let mut origin_indexes = BTreeMap::new();
    for asset in assets {
        for origin in golish_pentest_domain::confirmed_target_web_origins(
            &asset.name,
            &asset.value,
            &asset.ports,
        ) {
            if transport_exclusions.contains(&(asset.id, origin.key.clone())) {
                continue;
            }
            if let Some(index) = origin_indexes.get(&origin.key).copied() {
                let existing: &TargetCoverageRow = &rows[index];
                let candidate_is_current =
                    current_wave_target_ids.is_some_and(|ids| ids.contains(&asset.id));
                let existing_is_current =
                    current_wave_target_ids.is_some_and(|ids| ids.contains(&existing.id));
                if candidate_is_current && !existing_is_current {
                    let mut row = asset.clone();
                    row.value = origin.key;
                    row.target_type = "url".to_string();
                    row.exact_web_origin = true;
                    rows[index] = row;
                }
                continue;
            }
            let mut row = asset.clone();
            row.value = origin.key;
            row.target_type = "url".to_string();
            row.exact_web_origin = true;
            origin_indexes.insert(row.value.clone(), rows.len());
            rows.push(row);
        }
    }
    rows
}

async fn get_organization_row(
    pool: &sqlx::PgPool,
    organization_id: Uuid,
) -> anyhow::Result<Option<TargetCoverageRow>> {
    Ok(sqlx::query_as::<_, TargetCoverageRow>(
        r#"SELECT id,
                  name,
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
        r#"SELECT id, name, value, target_type::text AS target_type, real_ip, source, parent_id, created_at,
                  http_status, ports, webserver, COALESCE(liveness_state, 'unknown') AS liveness_state
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
    outcome_run_id: Option<&str>,
    evidence_session_id: Option<&str>,
    run_start: Option<DateTime<Utc>>,
    asset_values: &[String],
    organization_asset_values: &[String],
    business_found: &BTreeSet<(String, String)>,
    allow_latest_fallback: bool,
) -> anyhow::Result<BTreeMap<(String, String), OutcomeProjection>> {
    let mut out = BTreeMap::new();
    if !stage_accepts_outcome_projection(stage, run_start.is_some()) {
        return Ok(out);
    }
    let stage_techniques: BTreeSet<String> = techniques_for_stage(stage)
        .into_iter()
        .map(str::to_string)
        .collect();

    if let Some(run_id) = outcome_run_id.filter(|id| !id.trim().is_empty()) {
        let technique_outcome_rows = golish_db::repo::technique_outcomes::list_for_run_fresh(
            pool,
            organization_id,
            run_id,
            run_start,
        )
        .await?;
        let target_bound_evidence_facts = if matches!(
            stage,
            StageKind::ExternalAttackSurface | StageKind::Enumeration | StageKind::VulnTriage
        ) && technique_outcome_rows.iter().any(|row| {
            matches!(
                row.outcome.as_str(),
                "found" | "empty" | "blocked" | "not_applicable"
            )
        }) {
            match run_start {
                Some(cutoff) => match golish_db::repo::audit::evidence_facts_for_session_org_fresh(
                    pool,
                    evidence_session_id.unwrap_or(run_id),
                    organization_id,
                    cutoff,
                )
                .await
                {
                    Ok(rows) => match stage {
                        StageKind::ExternalAttackSurface => {
                            eas_target_bound_evidence_fact_set(organization_id, rows)
                        }
                        StageKind::Enumeration => enumeration_target_bound_evidence_fact_set(rows),
                        StageKind::VulnTriage => {
                            vuln_target_bound_evidence_fact_set(organization_id, rows)
                        }
                        _ => TargetBoundEvidenceFactSet::new(),
                    },
                    Err(error) => {
                        tracing::warn!(
                            target: "stage_coverage",
                            %error,
                            "org-bound fresh evidence facts read failed; strict EAS/Enumeration/Vuln terminal outcomes remain pending"
                        );
                        TargetBoundEvidenceFactSet::new()
                    }
                },
                None => TargetBoundEvidenceFactSet::new(),
            }
        } else {
            TargetBoundEvidenceFactSet::new()
        };

        for row in technique_outcome_rows {
            merge_stage_technique_outcome_row(
                stage,
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
                &target_bound_evidence_facts,
                business_found,
            );
        }

        if !matches!(
            stage,
            StageKind::ExternalAttackSurface | StageKind::Enumeration | StageKind::VulnTriage
        ) {
            for row in evidence_fact_rows_for_session(pool, run_id, &stage_techniques).await? {
                merge_evidence_fact_row(&mut out, asset_values, &stage_techniques, row);
            }

            // Merge exact source status last. Target Intel deliberately keeps a
            // current-run source error retryable even when the same attempt
            // persisted partial business data/evidence; the final gate follows
            // the same rule. `merge_source_query_row` only force-overrides on
            // `error`, while empty/blocked still defer to stronger found truth.
            let source_rows =
                golish_db::repo::source_query_log::list_for_run(pool, organization_id, run_id)
                    .await?
                    .into_iter()
                    .map(|row| SourceQueryProjectionRow {
                        source: row.source,
                        target: row.target,
                        technique: row.technique,
                        status: row.status,
                        evidence_refs: row.evidence_ids,
                    })
                    .collect::<Vec<_>>();
            let found_sources = source_query_found_sources(
                &source_rows,
                asset_values,
                &stage_techniques,
                organization_asset_values,
            );
            for row in source_rows {
                merge_source_query_row_with_authoritative_sources(
                    &mut out,
                    asset_values,
                    &stage_techniques,
                    row,
                    organization_asset_values,
                    &found_sources,
                    business_found,
                );
            }
        }
    }

    if allow_latest_fallback && out.is_empty() {
        let no_target_bound_evidence_facts = TargetBoundEvidenceFactSet::new();
        for row in latest_technique_outcomes(pool, organization_id, &stage_techniques).await? {
            merge_stage_technique_outcome_row(
                stage,
                &mut out,
                asset_values,
                &stage_techniques,
                row,
                &no_target_bound_evidence_facts,
                business_found,
            );
        }
        if stage != StageKind::Enumeration {
            let source_rows =
                latest_source_query_rows(pool, organization_id, &stage_techniques).await?;
            let found_sources = source_query_found_sources(
                &source_rows,
                asset_values,
                &stage_techniques,
                organization_asset_values,
            );
            for row in source_rows {
                merge_source_query_row_with_authoritative_sources(
                    &mut out,
                    asset_values,
                    &stage_techniques,
                    row,
                    organization_asset_values,
                    &found_sources,
                    business_found,
                );
            }
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
    if row.technique == golish_db::repo::coverage_truth::TECH_EAS_SERVICE_FP
        && row.outcome == "found"
    {
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

fn merge_stage_technique_outcome_row(
    stage: StageKind,
    out: &mut BTreeMap<(String, String), OutcomeProjection>,
    asset_values: &[String],
    stage_techniques: &BTreeSet<String>,
    row: TechniqueOutcomeProjectionRow,
    target_bound_evidence_facts: &TargetBoundEvidenceFactSet,
    business_found: &BTreeSet<(String, String)>,
) {
    if !matches!(
        stage,
        StageKind::ExternalAttackSurface | StageKind::Enumeration | StageKind::VulnTriage
    ) {
        merge_technique_outcome_row(out, asset_values, stage_techniques, row);
        return;
    }
    if !stage_techniques.contains(&row.technique) {
        return;
    }
    let requires_target_bound_evidence = matches!(row.outcome.as_str(), "found" | "empty")
        || (stage == StageKind::VulnTriage
            && matches!(row.outcome.as_str(), "blocked" | "not_applicable"))
        || (stage == StageKind::Enumeration && row.outcome == "blocked")
        || (stage == StageKind::ExternalAttackSurface
            && row.technique == golish_db::repo::coverage_truth::TECH_EAS_WEB_FP
            && row.outcome == "blocked");
    let terminal_evidence_id = if requires_target_bound_evidence {
        let Some(evidence_id) = projected_technique_outcome_evidence_id(
            &row.asset,
            &row.technique,
            &row.outcome,
            &row.evidence_refs,
            target_bound_evidence_facts,
            row.source.as_deref(),
        ) else {
            return;
        };
        Some(evidence_id)
    } else {
        None
    };
    let Some(asset_key) = matching_stage_asset_key(asset_values, &row.asset, &row.technique) else {
        return;
    };
    if stage == StageKind::ExternalAttackSurface
        && row.outcome == "found"
        && !business_found.contains(&(asset_key.clone(), row.technique.clone()))
        && !golish_agent_kit::harness::org_gate::eas_cidr_range_outcome_is_self_corroborating(
            &row.asset,
            &row.technique,
            row.source.as_deref(),
        )
    {
        return;
    }
    out.insert(
        (asset_key, row.technique),
        OutcomeProjection {
            state: outcome_state(&row.outcome),
            source: row.source,
            evidence_refs: terminal_evidence_id
                .map(|evidence_id| vec![evidence_id])
                .unwrap_or(row.evidence_refs),
        },
    );
}

fn business_truth_closes_stage_cells(stage: StageKind) -> bool {
    !matches!(
        stage,
        StageKind::ExternalAttackSurface | StageKind::Enumeration | StageKind::VulnTriage
    )
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

#[cfg(test)]
fn merge_source_query_row(
    out: &mut BTreeMap<(String, String), OutcomeProjection>,
    asset_values: &[String],
    stage_techniques: &BTreeSet<String>,
    row: SourceQueryProjectionRow,
    organization_asset_values: &[String],
) {
    merge_source_query_row_with_authoritative_sources(
        out,
        asset_values,
        stage_techniques,
        row,
        organization_asset_values,
        &BTreeMap::new(),
        &BTreeSet::new(),
    );
}

type SourceQueryFoundSources = BTreeMap<(String, String), BTreeSet<String>>;

fn source_query_projection_keys(
    asset_values: &[String],
    stage_techniques: &BTreeSet<String>,
    row: &SourceQueryProjectionRow,
    organization_asset_values: &[String],
) -> Vec<(String, String)> {
    let Some(technique) = row.technique.as_deref() else {
        return Vec::new();
    };
    if !stage_techniques.contains(technique) {
        return Vec::new();
    }
    if row.target.is_empty() {
        return asset_values
            .iter()
            .map(|asset| {
                (
                    coverage_lookup_asset(asset, technique),
                    technique.to_string(),
                )
            })
            .collect();
    }
    let target_keys = source_query_matching_stage_asset_keys(asset_values, &row.target, technique);
    if !target_keys.is_empty() {
        return target_keys
            .into_iter()
            .map(|target_key| (target_key, technique.to_string()))
            .collect();
    }
    if target_intel_source_technique(technique) {
        return organization_asset_values
            .iter()
            .map(|asset| {
                (
                    coverage_lookup_asset(asset, technique),
                    technique.to_string(),
                )
            })
            .collect();
    }
    Vec::new()
}

fn source_query_found_sources(
    rows: &[SourceQueryProjectionRow],
    asset_values: &[String],
    stage_techniques: &BTreeSet<String>,
    organization_asset_values: &[String],
) -> SourceQueryFoundSources {
    let mut found_sources = BTreeMap::new();
    for row in rows.iter().filter(|row| row.status == "found") {
        let source = row.source.trim().to_ascii_lowercase();
        for key in source_query_projection_keys(
            asset_values,
            stage_techniques,
            row,
            organization_asset_values,
        ) {
            found_sources
                .entry(key)
                .or_insert_with(BTreeSet::new)
                .insert(source.clone());
        }
    }
    found_sources
}

fn merge_source_query_row_with_authoritative_sources(
    out: &mut BTreeMap<(String, String), OutcomeProjection>,
    asset_values: &[String],
    stage_techniques: &BTreeSet<String>,
    row: SourceQueryProjectionRow,
    organization_asset_values: &[String],
    found_sources: &SourceQueryFoundSources,
    business_found: &BTreeSet<(String, String)>,
) {
    let Some(state) = source_query_terminal_state(&row.status) else {
        return;
    };
    let keys = source_query_projection_keys(
        asset_values,
        stage_techniques,
        &row,
        organization_asset_values,
    );
    if keys.is_empty() {
        return;
    }
    let source_key = row.source.trim().to_ascii_lowercase();
    let projection = OutcomeProjection {
        state,
        source: Some(row.source),
        evidence_refs: row.evidence_refs,
    };
    for key in keys {
        let sibling_found = row.status == "error"
            && business_found.contains(&key)
            && found_sources
                .get(&key)
                .is_some_and(|sources| sources.iter().any(|source| source != &source_key));
        if !sibling_found {
            merge_source_outcome(out, key, projection.clone());
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

fn source_query_matching_stage_asset_keys(
    asset_values: &[String],
    candidate: &str,
    technique: &str,
) -> Vec<String> {
    let candidate_key = coverage_lookup_asset(candidate, technique);
    let mut matched = BTreeSet::new();
    for asset in asset_values {
        let asset_key = coverage_lookup_asset(asset, technique);
        if asset_key == candidate_key
            || (technique == golish_db::repo::coverage_truth::TECH_SUBDOMAIN
                && asset_key.strip_prefix("*.") == Some(candidate_key.as_str()))
        {
            matched.insert(asset_key);
        }
    }
    matched.into_iter().collect()
}

fn coverage_lookup_asset(asset: &str, technique: &str) -> String {
    match technique {
        golish_db::repo::coverage_truth::TECH_EAS_LIVENESS => {
            golish_agent_kit::harness::evidence_facts::eas_liveness_asset_key(asset)
                .unwrap_or_else(|| asset.to_string())
        }
        golish_db::repo::coverage_truth::TECH_EAS_PORT
        | golish_db::repo::coverage_truth::TECH_EAS_SERVICE_FP
        | golish_db::repo::coverage_truth::TECH_EAS_WEB_FP => {
            golish_pentest_domain::canonical_asset_key(asset)
                .map(|key| key.key)
                .unwrap_or_else(|| asset.to_string())
        }
        golish_db::repo::coverage_truth::TECH_ENUM_JS
        | golish_db::repo::coverage_truth::TECH_ENUM_DIR
        | golish_db::repo::coverage_truth::TECH_ENUM_PARAM
        | golish_db::repo::coverage_truth::TECH_ENUM_JSAPI => {
            golish_pentest_domain::canonical_web_origin(asset)
                .map(|origin| origin.key)
                .unwrap_or_else(|| asset.to_string())
        }
        technique if technique.starts_with("WSTG-") || technique == "GOLISH-NDAY" => {
            golish_pentest_domain::canonical_web_origin(asset)
                .map(|origin| origin.key)
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

fn counts_as_coverage_asset(asset: &TargetCoverageRow) -> bool {
    !is_organization_coverage_row(asset)
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
    )
}

fn coverage_cells_with_eas_parent_ips(
    stage: StageKind,
    asset: &TargetCoverageRow,
    found: &BTreeSet<(String, String)>,
    outcomes: &BTreeMap<(String, String), OutcomeProjection>,
    web_capable_assets: &BTreeSet<String>,
    service_not_applicable_assets: &BTreeSet<String>,
) -> Vec<StageAssetCoverageCell> {
    let class = coverage_asset_class(stage, asset);
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
            if matches!(stage, StageKind::Enumeration | StageKind::VulnTriage)
                && !asset.exact_web_origin
            {
                return cell(
                    technique,
                    "pending",
                    None,
                    Vec::new(),
                    Some(
                        "the prior stage has not materialized an exact scheme://host:port origin for this web asset"
                            .to_string(),
                    ),
                );
            }
            let asset_key = coverage_lookup_asset(&asset.value, technique);
            let outcome = outcomes.get(&(asset_key.clone(), technique.to_string()));
            // Target Intel may persist real partial records (for example A
            // succeeded while CNAME/MX/TXT transport failed). The current-run
            // retry marker must remain visible even though the business table
            // now contains a positive row; otherwise preflight says ready while
            // the final deterministic gate correctly blocks.
            if stage == StageKind::TargetIntel
                && outcome.is_some_and(|outcome| matches!(outcome.state.as_str(), "partial" | "error"))
            {
                let outcome = outcome.expect("checked above");
                return cell(
                    technique,
                    &outcome.state,
                    outcome.source.clone(),
                    outcome.evidence_refs.clone(),
                    None,
                );
            }
            if found.contains(&(asset_key.clone(), technique.to_string())) {
                return cell(technique, "found", None, Vec::new(), None);
            }
            if let Some(outcome) = outcome {
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
    apply_eas_ip_liveness_port_dependency(stage, asset, &mut cells);
    apply_eas_service_dependency(stage, asset, &mut cells, service_not_applicable_assets);
    apply_eas_service_missing_port_details(stage, asset, &mut cells);
    cells
}

fn apply_eas_web_origin_details(
    stage: StageKind,
    asset: &TargetCoverageRow,
    cells: &mut [StageAssetCoverageCell],
    origins: &EasWebOriginCoverage,
) {
    if stage != StageKind::ExternalAttackSurface {
        return;
    }
    let Some(required) = origins.required_by_target.get(&asset.id) else {
        return;
    };
    if required.is_empty() {
        return;
    }
    let Some(web_cell) = cells
        .iter_mut()
        .find(|cell| cell.technique == golish_db::repo::coverage_truth::TECH_EAS_WEB_FP)
    else {
        return;
    };

    let mut completed = Vec::new();
    let mut missing = Vec::new();
    let mut evidence_refs = BTreeSet::new();
    let mut sources = BTreeSet::new();
    let mut any_found = false;
    let mut blocked = Vec::new();
    for origin in required {
        match origins.completed.get(origin) {
            Some(outcome) => {
                completed.push(origin.clone());
                any_found |= outcome.state == "found";
                if outcome.state == "blocked" {
                    blocked.push(origin.clone());
                }
                evidence_refs.extend(outcome.evidence_refs.iter().copied());
                if let Some(source) = outcome.source.as_ref() {
                    sources.insert(source.clone());
                }
            }
            None => missing.push(origin.clone()),
        }
    }

    let state = if missing.is_empty() {
        if any_found {
            "found".to_string()
        } else if !blocked.is_empty() {
            "blocked".to_string()
        } else {
            "checked_empty".to_string()
        }
    } else if completed.is_empty() {
        "pending".to_string()
    } else {
        "partial".to_string()
    };
    let source = if sources.len() == 1 {
        sources.iter().next().cloned()
    } else {
        None
    };
    let evidence_refs = evidence_refs.into_iter().collect();
    let note = (!missing.is_empty()).then(|| {
        format!(
            "{} of {} confirmed exact Web Origins still need fingerprinting",
            missing.len(),
            required.len()
        )
    });
    let recommended_target_urls = missing
        .iter()
        .map(|target_url| {
            serde_json::json!({
                "target_id": asset.id.to_string(),
                "target_url": target_url,
            })
        })
        .collect::<Vec<_>>();
    let details = serde_json::json!({
        "required_origins": required,
        "completed_origins": completed,
        "blocked_origins": blocked,
        "missing_origins": missing,
        "recommended_tool": "eas_fingerprint_web_stack",
        "recommended_args": {
            "target_urls": recommended_target_urls,
        },
    });
    let mut replacement = cell(
        golish_db::repo::coverage_truth::TECH_EAS_WEB_FP,
        &state,
        source,
        evidence_refs,
        note,
    );
    replacement.details = details;
    *web_cell = replacement;
}

fn apply_eas_service_dependency(
    stage: StageKind,
    _asset: &TargetCoverageRow,
    cells: &mut [StageAssetCoverageCell],
    _service_not_applicable_assets: &BTreeSet<String>,
) {
    if stage != StageKind::ExternalAttackSurface {
        return;
    }
    let port_has_no_service_surface = cells.iter().any(|coverage_cell| {
        coverage_cell.technique == golish_db::repo::coverage_truth::TECH_EAS_PORT
            && matches!(
                coverage_cell.state.as_str(),
                "checked_empty" | "not_applicable"
            )
    });
    if !port_has_no_service_surface {
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
    *service_cell = cell(
        golish_db::repo::coverage_truth::TECH_EAS_SERVICE_FP,
        "not_applicable",
        None,
        Vec::new(),
        Some("no open ports were found, so service fingerprinting is not applicable".to_string()),
    );
}

fn apply_eas_service_missing_port_details(
    stage: StageKind,
    asset: &TargetCoverageRow,
    cells: &mut [StageAssetCoverageCell],
) {
    if stage != StageKind::ExternalAttackSurface {
        return;
    }
    let missing_ports =
        golish_db::repo::coverage_truth::missing_service_fingerprint_ports_from_ports_json(
            &asset.ports,
        );
    if missing_ports.is_empty() {
        return;
    }
    let Some(service_cell) = cells.iter_mut().find(|coverage_cell| {
        coverage_cell.technique == golish_db::repo::coverage_truth::TECH_EAS_SERVICE_FP
    }) else {
        return;
    };
    if !matches!(service_cell.state.as_str(), "pending" | "error") {
        return;
    }
    let ports_arg = format_u16_ports(&missing_ports);
    service_cell.note = Some(format!(
        "confirmed open port(s) still need service fingerprinting: {ports_arg}"
    ));
    service_cell.details = serde_json::json!({
        "missing_open_ports": missing_ports,
        "recommended_tool": "eas_fingerprint_services",
        "recommended_args": {
            "targets": [asset.value.clone()],
            "ports": ports_arg
        }
    });
}

fn format_u16_ports(ports: &[u16]) -> String {
    ports
        .iter()
        .map(u16::to_string)
        .collect::<Vec<_>>()
        .join(",")
}

fn apply_eas_ip_liveness_port_dependency(
    stage: StageKind,
    asset: &TargetCoverageRow,
    cells: &mut [StageAssetCoverageCell],
) {
    if stage != StageKind::ExternalAttackSurface {
        return;
    }
    let class = coverage_asset_class(stage, asset);
    if !matches!(
        class,
        golish_agent_kit::harness::technique_resolver::AssetClass::Ip
            | golish_agent_kit::harness::technique_resolver::AssetClass::Cidr
    ) {
        return;
    }
    let port_projection = cells
        .iter()
        .find(|coverage_cell| {
            coverage_cell.technique == golish_db::repo::coverage_truth::TECH_EAS_PORT
        })
        .map(|coverage_cell| {
            (
                coverage_cell.state.clone(),
                coverage_cell.source.clone(),
                coverage_cell.evidence_refs.clone(),
            )
        });
    let Some(liveness_cell) = cells.iter_mut().find(|coverage_cell| {
        coverage_cell.technique == golish_db::repo::coverage_truth::TECH_EAS_LIVENESS
    }) else {
        return;
    };
    if liveness_cell.state == "pending" {
        match port_projection.as_ref().map(|(state, _, _)| state.as_str()) {
            Some("found") => {
                *liveness_cell = cell(
                    golish_db::repo::coverage_truth::TECH_EAS_LIVENESS,
                    "found",
                    port_projection
                        .as_ref()
                        .and_then(|(_, source, _)| source.clone()),
                    port_projection
                        .as_ref()
                        .map(|(_, _, evidence_refs)| evidence_refs.clone())
                        .unwrap_or_default(),
                    Some("open ports prove this concrete host is live".to_string()),
                );
            }
            Some("checked_empty" | "not_applicable") => {
                *liveness_cell = cell(
                    golish_db::repo::coverage_truth::TECH_EAS_LIVENESS,
                    "checked_empty",
                    port_projection
                        .as_ref()
                        .and_then(|(_, source, _)| source.clone()),
                    port_projection
                        .as_ref()
                        .map(|(_, _, evidence_refs)| evidence_refs.clone())
                        .unwrap_or_default(),
                    Some("port discovery found no open ports for this concrete host".to_string()),
                );
            }
            _ => {
                let suggestions = suggested_capabilities_for_any_technique(
                    golish_db::repo::coverage_truth::TECH_EAS_PORT,
                );
                liveness_cell.suggested_capabilities = suggestions;
                liveness_cell.suggested_tools =
                    tools_from_suggestions(&liveness_cell.suggested_capabilities);
                liveness_cell.note = Some(
                    "for concrete IP/CIDR assets, run port discovery first; open ports prove liveness"
                        .to_string(),
                );
            }
        }
    }
}

#[cfg(test)]
fn next_wave_coverage_cells(
    stage: StageKind,
    asset: &TargetCoverageRow,
) -> Vec<StageAssetCoverageCell> {
    next_wave_coverage_cells_with_eas_parent_ips(stage, asset, &BTreeSet::new())
}

fn next_wave_coverage_cells_with_eas_parent_ips(
    stage: StageKind,
    asset: &TargetCoverageRow,
    web_capable_assets: &BTreeSet<String>,
) -> Vec<StageAssetCoverageCell> {
    let class = coverage_asset_class(stage, asset);
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
                Some("not in the current asset wave; queued for a supplemental wave".to_string()),
            )
        })
        .collect()
}

fn coverage_asset_class(
    stage: StageKind,
    asset: &TargetCoverageRow,
) -> golish_agent_kit::harness::technique_resolver::AssetClass {
    golish_agent_kit::harness::technique_resolver::classify_stage_asset(
        stage,
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
            golish_db::repo::coverage_truth::TECH_EAS_WEB_FP,
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
            "WSTG-ATHN-04",
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
    let suggested_capabilities = if matches!(state, "pending" | "error" | "partial") {
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
        details: serde_json::Value::Null,
    }
}

fn outcome_state(outcome: &str) -> String {
    match outcome {
        "found" => "found",
        "empty" => "checked_empty",
        "blocked" => "blocked",
        "error" => "error",
        "partial" => "partial",
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

fn merge_source_outcome(
    out: &mut BTreeMap<(String, String), OutcomeProjection>,
    key: (String, String),
    next: OutcomeProjection,
) {
    if next.state != "error" {
        merge_outcome(out, key, next);
        return;
    }
    let Some(existing) = out.get_mut(&key) else {
        out.insert(key, next);
        return;
    };
    let gate_materialized_terminal =
        matches!(existing.state.as_str(), "blocked" | "not_applicable")
            && existing.source.as_deref() == Some("submit_stage_deliverable");
    if !gate_materialized_terminal {
        existing.state = next.state;
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
        "found" => 6,
        "blocked" => 5,
        "error" => 4,
        "partial" => 3,
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
        golish_db::repo::coverage_truth::TECH_EAS_WEB_FP => "Web FP",
        golish_db::repo::coverage_truth::TECH_ENUM_JS => "JS",
        golish_db::repo::coverage_truth::TECH_ENUM_DIR => "Directory",
        golish_db::repo::coverage_truth::TECH_ENUM_PARAM => "Parameter",
        golish_db::repo::coverage_truth::TECH_ENUM_JSAPI => "API",
        "WSTG-INPV-05" => "SQL Injection",
        "WSTG-INPV-01" => "XSS",
        "WSTG-INPV-12" => "Command Injection",
        "WSTG-ATHN-04" => "Anonymous Access",
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

    #[tokio::test]
    async fn vuln_triage_snapshot_without_operation_id_fails_before_any_db_read() {
        let pool = sqlx::postgres::PgPoolOptions::new()
            .connect_lazy("postgres://golish:golish@127.0.0.1:1/unavailable")
            .expect("lazy pool");
        let error = stage_asset_coverage_snapshot(
            &pool,
            Uuid::new_v4(),
            StageKind::VulnTriage,
            Some("chat-session"),
            Some(Utc::now()),
            None,
            None,
            false,
            None,
        )
        .await
        .expect_err("Vuln coverage without trusted operation identity must fail closed");

        assert!(error
            .to_string()
            .contains("requires a trusted operation id"));
    }

    fn target(value: &str, target_type: &str) -> TargetCoverageRow {
        TargetCoverageRow {
            id: Uuid::new_v4(),
            name: value.to_string(),
            value: value.to_string(),
            target_type: target_type.to_string(),
            real_ip: String::new(),
            source: Some("asset_intel".to_string()),
            parent_id: None,
            created_at: Utc::now(),
            http_status: None,
            ports: serde_json::json!([]),
            webserver: String::new(),
            liveness_state: "unknown".to_string(),
            exact_web_origin: false,
        }
    }

    #[test]
    fn target_intel_coverage_excludes_targets_created_by_the_current_stage() {
        let stage_started_at = Utc::now();
        let mut seed = target("seed.moresec.cn", "domain");
        seed.created_at = stage_started_at - chrono::Duration::seconds(1);
        let mut discovered = target("api.moresec.cn", "domain");
        discovered.created_at = stage_started_at + chrono::Duration::seconds(1);
        let mut organization = target("organization:test", "organization");
        organization.created_at = stage_started_at - chrono::Duration::seconds(1);

        let filtered = target_intel_stage_input_assets(
            vec![organization, seed, discovered],
            Some(stage_started_at),
        );

        assert_eq!(
            filtered
                .iter()
                .map(|asset| asset.value.as_str())
                .collect::<Vec<_>>(),
            vec!["organization:test", "seed.moresec.cn"]
        );
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
        // suggested_tools：producer + trusted preflight；preflight 可在 producer
        // 失败前先给 transport-blocked origin 一个确定性终态路径。
        assert_eq!(
            suggested_tools(golish_db::repo::coverage_truth::TECH_ENUM_JS),
            vec![
                "browser_collect_js_api".to_string(),
                "enum_preflight_web_origins".to_string(),
            ]
        );
        assert_eq!(
            suggested_tools(golish_db::repo::coverage_truth::TECH_ENUM_JSAPI),
            vec![
                "js_extract_apis".to_string(),
                "enum_preflight_web_origins".to_string(),
            ]
        );
    }

    #[test]
    fn enumeration_target_expands_all_exact_web_origins() {
        let mut asset = target("app.example.com", "domain");
        asset.http_status = Some(200);
        asset.ports = serde_json::json!([
            {"port": 80, "state": "open", "service": "http", "url": "http://app.example.com/login"},
            {"port": 443, "state": "open", "service": "https", "url": "https://app.example.com/"},
            {"port": 8443, "state": "open", "service": "https-alt", "url": "https://app.example.com:8443/admin"},
            {"port": 8443, "state": "open", "service": "https-alt", "url": "https://app.example.com:8443/other"},
            {"port": 9444, "state": "", "service": "unknown", "url": "https://app.example.com:9444/"},
            {"port": 9443, "state": "closed", "service": "https", "url": "https://app.example.com:9443/"},
            {"port": 22, "state": "open", "service": "ssh"}
        ]);

        let origins = expand_enumeration_web_origin_rows(vec![asset]);

        assert_eq!(
            origins
                .iter()
                .map(|row| row.value.as_str())
                .collect::<Vec<_>>(),
            vec![
                "http://app.example.com:80",
                "https://app.example.com:443",
                "https://app.example.com:8443",
            ]
        );
        assert!(origins.iter().all(|row| row.target_type == "url"));
        assert!(origins.iter().all(|row| row.exact_web_origin));
        assert_eq!(
            origins
                .iter()
                .map(|row| row.id)
                .collect::<BTreeSet<_>>()
                .len(),
            1,
            "all origins retain the owning target id"
        );
    }

    #[test]
    fn enumeration_wave_membership_follows_original_domain_ip_and_url_path_target() {
        let mut domain = target("app.example.com", "domain");
        domain.ports = serde_json::json!([
            {"port": 80, "state": "open", "url": "http://app.example.com/login"},
            {"port": 443, "state": "open", "url": "https://app.example.com/"}
        ]);
        let mut ip = target("203.0.113.10", "ip");
        ip.ports = serde_json::json!([
            {"port": 8080, "state": "open", "url": "http://203.0.113.10:8080/"},
            {"port": 8443, "state": "open", "url": "https://203.0.113.10:8443/admin"}
        ]);
        let mut url_path = target("https://portal.example.com/login?next=/", "url");
        url_path.ports = serde_json::json!([
            {"port": 443, "state": "open", "url": "https://portal.example.com/dashboard"},
            {"port": 9443, "state": "open", "url": "https://portal.example.com:9443/admin"}
        ]);

        for asset in [domain, ip, url_path] {
            let original_value = asset.value.clone();
            let current_wave = BTreeSet::from([asset.id]);
            let origins = expand_enumeration_web_origin_rows(vec![asset]);
            assert!(
                origins.len() >= 2,
                "fixture {original_value} must exercise multi-origin expansion"
            );
            assert!(
                origins.iter().all(|origin| !is_deferred_wave_asset(
                    origin,
                    None,
                    Some(&current_wave),
                )),
                "all origins derived from wave target {original_value} must stay in its wave: {origins:?}"
            );
        }
    }

    #[test]
    fn enumeration_wave_membership_defers_every_origin_of_foreign_target() {
        let mut asset = target("outside.example.com", "domain");
        asset.ports = serde_json::json!([
            {"port": 80, "state": "open", "url": "http://outside.example.com/"},
            {"port": 443, "state": "open", "url": "https://outside.example.com/"}
        ]);
        let current_wave = BTreeSet::from([Uuid::new_v4()]);

        let origins = expand_enumeration_web_origin_rows(vec![asset]);

        assert_eq!(origins.len(), 2);
        assert!(origins.iter().all(|origin| is_deferred_wave_asset(
            origin,
            None,
            Some(&current_wave),
        )));
    }

    #[test]
    fn enumeration_origin_dedupe_prefers_current_wave_owner_over_old_owner() {
        // Legacy duplicate rows may own the same exact host/origin. Foreign-host
        // ports[].url is deliberately ignored by the authorization helper, so
        // this dedupe fixture keeps both owners on the real shared identity.
        let mut old_owner = target("shared.example.com", "domain");
        old_owner.ports = serde_json::json!([
            {"port": 443, "state": "open", "url": "https://shared.example.com/app"}
        ]);
        let mut current_owner = target("shared.example.com", "domain");
        current_owner.ports = serde_json::json!([
            {"port": 443, "state": "open", "url": "https://shared.example.com/admin"}
        ]);
        let current_owner_id = current_owner.id;

        let origins = expand_enumeration_web_origin_rows_for_wave(
            vec![old_owner, current_owner],
            Some(&BTreeSet::from([current_owner_id])),
        );

        assert_eq!(origins.len(), 1);
        assert_eq!(origins[0].id, current_owner_id);
    }

    #[test]
    fn enumeration_excludes_confirmed_dead_target_before_origin_expansion() {
        let mut live = target("live.example.com", "domain");
        live.liveness_state = "alive".to_string();
        live.ports = serde_json::json!([
            {"port": 443, "state": "open", "url": "https://live.example.com/"}
        ]);
        let mut dead = target("dead.example.com", "domain");
        dead.liveness_state = "dead".to_string();
        dead.ports = serde_json::json!([
            {"port": 443, "state": "open", "url": "https://dead.example.com/"}
        ]);

        let filtered =
            exclude_dead_targets_if_opted_in(StageKind::Enumeration, vec![live.clone(), dead]);
        let origins = expand_enumeration_web_origin_rows(filtered);

        assert_eq!(origins.len(), 1);
        assert_eq!(origins[0].id, live.id);
        assert_eq!(origins[0].value, "https://live.example.com:443");
    }

    #[test]
    fn enumeration_all_dead_targets_produce_an_authoritative_empty_axis() {
        let mut first = target("https://dead-a.example.com/", "url");
        first.liveness_state = "dead".to_string();
        let mut second = target("https://dead-b.example.com/", "url");
        second.liveness_state = "dead".to_string();

        let filtered =
            exclude_dead_targets_if_opted_in(StageKind::Enumeration, vec![first, second]);
        assert!(filtered.is_empty());
    }

    #[test]
    fn enumeration_globally_dedupes_same_origin_with_stable_owner() {
        let mut first = target("shared.example", "domain");
        first.ports = serde_json::json!([
            {"port": 443, "state": "open", "url": "https://shared.example/app"}
        ]);
        let first_id = first.id;
        let mut second = target("shared.example", "domain");
        second.ports = serde_json::json!([
            {"port": 443, "state": "open", "url": "https://shared.example/other"}
        ]);

        let origins = expand_enumeration_web_origin_rows(vec![first, second]);

        assert_eq!(origins.len(), 1);
        assert_eq!(origins[0].value, "https://shared.example:443");
        assert_eq!(origins[0].id, first_id, "first DB-ordered owner is stable");
    }

    #[test]
    fn enumeration_target_without_exact_origin_is_not_a_content_denominator_row() {
        let asset = target("unresolved.example.com", "domain");

        let rows = expand_enumeration_web_origin_rows(vec![asset]);

        assert!(rows.is_empty());
    }

    #[test]
    fn enumeration_transport_handoff_excludes_only_exact_owner_origin() {
        let blocked_target = Uuid::from_u128(41);
        let sibling_target = Uuid::from_u128(42);
        let mut blocked = target("blocked.example", "domain");
        blocked.id = blocked_target;
        blocked.ports = serde_json::json!([{"port": 443, "state": "open", "service": "https"}]);
        let mut sibling = target("sibling.example", "domain");
        sibling.id = sibling_target;
        sibling.ports = serde_json::json!([{"port": 443, "state": "open", "service": "https"}]);
        let assets = vec![blocked, sibling];
        let exclusions =
            BTreeSet::from([(blocked_target, "https://blocked.example:443".to_string())]);

        let rows = expand_enumeration_web_origin_rows_for_wave_excluding(assets, None, &exclusions);

        assert!(!rows
            .iter()
            .any(|row| { row.id == blocked_target && row.value == "https://blocked.example:443" }));
        assert!(rows
            .iter()
            .any(|row| { row.id == sibling_target && row.value == "https://sibling.example:443" }));
    }

    #[test]
    fn unresolved_enumeration_origin_ignores_legacy_host_completion() {
        let asset = target("unresolved.example.com", "domain");
        let outcomes = BTreeMap::from([(
            (
                asset.value.clone(),
                golish_db::repo::coverage_truth::TECH_ENUM_DIR.to_string(),
            ),
            OutcomeProjection {
                state: "found".to_string(),
                source: Some("legacy-host-outcome".to_string()),
                evidence_refs: vec![9],
            },
        )]);

        let cells = coverage_cells_with_eas_parent_ips(
            StageKind::Enumeration,
            &asset,
            &BTreeSet::new(),
            &outcomes,
            &BTreeSet::from([asset.value.clone()]),
            &BTreeSet::new(),
        );

        assert_eq!(
            cells
                .iter()
                .find(|cell| { cell.technique == golish_db::repo::coverage_truth::TECH_ENUM_DIR })
                .unwrap()
                .state,
            "pending"
        );
    }

    #[test]
    fn partial_technique_outcome_remains_visibly_unfinished() {
        assert_eq!(outcome_state("partial"), "partial");
        assert!(outcome_rank("partial") > outcome_rank("pending"));
        assert!(outcome_rank("partial") < outcome_rank("found"));
    }

    #[test]
    fn enumeration_current_outcome_replaces_historical_found_projection() {
        let technique = golish_db::repo::coverage_truth::TECH_ENUM_JS.to_string();
        let asset = "https://app.example.com:443".to_string();
        let mut outcomes = BTreeMap::from([(
            (asset.clone(), technique.clone()),
            OutcomeProjection {
                state: "found".to_string(),
                source: Some("historical-evidence".to_string()),
                evidence_refs: vec![12],
            },
        )]);

        merge_stage_technique_outcome_row(
            StageKind::Enumeration,
            &mut outcomes,
            std::slice::from_ref(&asset),
            &BTreeSet::from([technique.clone()]),
            TechniqueOutcomeProjectionRow {
                asset: asset.clone(),
                technique: technique.clone(),
                outcome: "partial".to_string(),
                source: Some("browser_collect_js_api".to_string()),
                evidence_refs: Vec::new(),
            },
            &Default::default(),
            &BTreeSet::new(),
        );

        assert_eq!(outcomes[&(asset, technique)].state, "partial");
    }

    #[test]
    fn enumeration_terminal_outcome_without_real_evidence_remains_pending() {
        let technique = golish_db::repo::coverage_truth::TECH_ENUM_DIR.to_string();
        let asset = "https://app.example.com:443".to_string();
        for evidence_refs in [Vec::new(), vec![0]] {
            let mut outcomes = BTreeMap::new();
            merge_stage_technique_outcome_row(
                StageKind::Enumeration,
                &mut outcomes,
                std::slice::from_ref(&asset),
                &BTreeSet::from([technique.clone()]),
                TechniqueOutcomeProjectionRow {
                    asset: asset.clone(),
                    technique: technique.clone(),
                    outcome: "found".to_string(),
                    source: Some("route_probe_paths".to_string()),
                    evidence_refs,
                },
                &Default::default(),
                &BTreeSet::new(),
            );
            assert!(outcomes.is_empty());
        }
    }

    #[test]
    fn enumeration_terminal_outcome_requires_matching_run_evidence_fact() {
        let technique = golish_db::repo::coverage_truth::TECH_ENUM_DIR.to_string();
        let asset = "https://app.example.com:443".to_string();
        let row = || TechniqueOutcomeProjectionRow {
            asset: asset.clone(),
            technique: technique.clone(),
            outcome: "found".to_string(),
            source: Some("route_probe_paths".to_string()),
            evidence_refs: vec![42],
        };
        let mismatched = crate::ai::db_bridge::evidence::enumeration_evidence_fact_set(vec![(
            asset.clone(),
            technique.clone(),
            "empty".to_string(),
            42,
        )]);
        let mut outcomes = BTreeMap::new();
        merge_stage_technique_outcome_row(
            StageKind::Enumeration,
            &mut outcomes,
            std::slice::from_ref(&asset),
            &BTreeSet::from([technique.clone()]),
            row(),
            &mismatched,
            &BTreeSet::new(),
        );
        assert!(outcomes.is_empty());

        let matching = crate::ai::db_bridge::evidence::enumeration_evidence_fact_set(vec![(
            "https://app.example.com/path".to_string(),
            technique.clone(),
            "found".to_string(),
            42,
        )]);
        merge_stage_technique_outcome_row(
            StageKind::Enumeration,
            &mut outcomes,
            std::slice::from_ref(&asset),
            &BTreeSet::from([technique.clone()]),
            row(),
            &matching,
            &BTreeSet::new(),
        );
        assert_eq!(outcomes[&(asset, technique)].state, "found");
    }

    #[test]
    fn enumeration_read_model_rejects_browser_owned_static_terminals() {
        let asset = "https://app.example.com:443".to_string();
        for technique in [
            golish_db::repo::coverage_truth::TECH_ENUM_JSAPI.to_string(),
            golish_db::repo::coverage_truth::TECH_ENUM_PARAM.to_string(),
        ] {
            let evidence = TargetBoundEvidenceFactSet::from([(
                asset.clone(),
                technique.clone(),
                "empty".to_string(),
                57,
            )]);
            let row = |source: &str| TechniqueOutcomeProjectionRow {
                asset: asset.clone(),
                technique: technique.clone(),
                outcome: "empty".to_string(),
                source: Some(source.to_string()),
                evidence_refs: vec![57],
            };
            let mut outcomes = BTreeMap::new();

            merge_stage_technique_outcome_row(
                StageKind::Enumeration,
                &mut outcomes,
                std::slice::from_ref(&asset),
                &BTreeSet::from([technique.clone()]),
                row("browser_collect_js_api"),
                &evidence,
                &BTreeSet::new(),
            );
            assert!(
                outcomes.is_empty(),
                "browser evidence must not close the {technique} read-model cell"
            );

            merge_stage_technique_outcome_row(
                StageKind::Enumeration,
                &mut outcomes,
                std::slice::from_ref(&asset),
                &BTreeSet::from([technique.clone()]),
                row("js_extract_apis"),
                &evidence,
                &BTreeSet::new(),
            );
            assert_eq!(outcomes[&(asset.clone(), technique)].state, "checked_empty");
        }
    }

    #[test]
    fn enumeration_blocked_projection_requires_matching_target_bound_evidence_and_owner() {
        let technique = golish_db::repo::coverage_truth::TECH_ENUM_DIR.to_string();
        let asset = "https://app.example.com:443".to_string();
        let row = |source: &str| TechniqueOutcomeProjectionRow {
            asset: asset.clone(),
            technique: technique.clone(),
            outcome: "blocked".to_string(),
            source: Some(source.to_string()),
            evidence_refs: vec![73],
        };
        let mut no_evidence = BTreeMap::new();
        merge_stage_technique_outcome_row(
            StageKind::Enumeration,
            &mut no_evidence,
            std::slice::from_ref(&asset),
            &BTreeSet::from([technique.clone()]),
            row("enum_preflight_web_origins"),
            &Default::default(),
            &BTreeSet::new(),
        );
        assert!(no_evidence.is_empty());

        let matching = TargetBoundEvidenceFactSet::from([(
            asset.clone(),
            technique.clone(),
            "blocked".to_string(),
            73,
        )]);
        let mut route_outcomes = BTreeMap::new();
        merge_stage_technique_outcome_row(
            StageKind::Enumeration,
            &mut route_outcomes,
            std::slice::from_ref(&asset),
            &BTreeSet::from([technique.clone()]),
            row("route_probe_paths"),
            &matching,
            &BTreeSet::new(),
        );
        assert_eq!(
            route_outcomes[&(asset.clone(), technique.clone())].state,
            "blocked",
            "route recovery exhaustion may own DIR blocked"
        );

        let mut forged_outcomes = BTreeMap::new();
        merge_stage_technique_outcome_row(
            StageKind::Enumeration,
            &mut forged_outcomes,
            std::slice::from_ref(&asset),
            &BTreeSet::from([technique.clone()]),
            row("js_extract_apis"),
            &matching,
            &BTreeSet::new(),
        );
        assert!(
            forged_outcomes.is_empty(),
            "a producer that owns neither preflight nor DIR recovery must not project blocked"
        );

        let mut preflight_outcomes = BTreeMap::new();
        merge_stage_technique_outcome_row(
            StageKind::Enumeration,
            &mut preflight_outcomes,
            std::slice::from_ref(&asset),
            &BTreeSet::from([technique.clone()]),
            row("enum_preflight_web_origins"),
            &matching,
            &BTreeSet::new(),
        );
        assert_eq!(preflight_outcomes[&(asset, technique)].state, "blocked");
        assert_eq!(
            preflight_outcomes.values().next().unwrap().evidence_refs,
            vec![73]
        );
    }

    #[test]
    fn enumeration_browser_recovery_blocked_projection_is_axis_scoped() {
        let asset = "https://app.example.com:443".to_string();
        for technique in [
            golish_db::repo::coverage_truth::TECH_ENUM_JS,
            golish_db::repo::coverage_truth::TECH_ENUM_JSAPI,
            golish_db::repo::coverage_truth::TECH_ENUM_PARAM,
        ] {
            let technique = technique.to_string();
            let matching = TargetBoundEvidenceFactSet::from([(
                asset.clone(),
                technique.clone(),
                "blocked".to_string(),
                74,
            )]);
            let mut outcomes = BTreeMap::new();
            merge_stage_technique_outcome_row(
                StageKind::Enumeration,
                &mut outcomes,
                std::slice::from_ref(&asset),
                &BTreeSet::from([technique.clone()]),
                TechniqueOutcomeProjectionRow {
                    asset: asset.clone(),
                    technique: technique.clone(),
                    outcome: "blocked".to_string(),
                    source: Some("browser_collect_js_api".to_string()),
                    evidence_refs: vec![74],
                },
                &matching,
                &BTreeSet::new(),
            );
            assert_eq!(outcomes[&(asset.clone(), technique)].state, "blocked");
        }

        let technique = golish_db::repo::coverage_truth::TECH_ENUM_DIR.to_string();
        let matching = TargetBoundEvidenceFactSet::from([(
            asset.clone(),
            technique.clone(),
            "blocked".to_string(),
            75,
        )]);
        let mut outcomes = BTreeMap::new();
        merge_stage_technique_outcome_row(
            StageKind::Enumeration,
            &mut outcomes,
            std::slice::from_ref(&asset),
            &BTreeSet::from([technique.clone()]),
            TechniqueOutcomeProjectionRow {
                asset: asset.clone(),
                technique: technique.clone(),
                outcome: "blocked".to_string(),
                source: Some("browser_collect_js_api".to_string()),
                evidence_refs: vec![75],
            },
            &matching,
            &BTreeSet::new(),
        );
        assert!(outcomes.is_empty(), "browser must not own DIR blocked");
    }

    #[test]
    fn eas_found_requires_business_truth_and_matching_guarded_outcome_evidence() {
        let asset = "192.0.2.10".to_string();
        let technique = golish_db::repo::coverage_truth::TECH_EAS_PORT.to_string();
        let strict = TargetBoundEvidenceFactSet::from([(
            asset.clone(),
            technique.clone(),
            "found".to_string(),
            61,
        )]);
        let row = || TechniqueOutcomeProjectionRow {
            asset: asset.clone(),
            technique: technique.clone(),
            outcome: "found".to_string(),
            source: Some("eas_discover_ports".to_string()),
            evidence_refs: vec![61],
        };

        let mut outcomes = BTreeMap::new();
        merge_stage_technique_outcome_row(
            StageKind::ExternalAttackSurface,
            &mut outcomes,
            std::slice::from_ref(&asset),
            &BTreeSet::from([technique.clone()]),
            row(),
            &strict,
            &BTreeSet::new(),
        );
        assert!(
            outcomes.is_empty(),
            "business landing is required for found"
        );

        merge_stage_technique_outcome_row(
            StageKind::ExternalAttackSurface,
            &mut outcomes,
            std::slice::from_ref(&asset),
            &BTreeSet::from([technique.clone()]),
            row(),
            &strict,
            &BTreeSet::from([(asset.clone(), technique.clone())]),
        );
        assert_eq!(outcomes[&(asset, technique)].state, "found");
    }

    #[test]
    fn eas_cidr_discovery_found_is_visible_without_child_ip_projection() {
        let asset = "192.0.2.0/24".to_string();
        let technique = golish_db::repo::coverage_truth::TECH_EAS_PORT.to_string();
        let strict = TargetBoundEvidenceFactSet::from([(
            asset.clone(),
            technique.clone(),
            "found".to_string(),
            63,
        )]);
        let mut outcomes = BTreeMap::new();

        merge_stage_technique_outcome_row(
            StageKind::ExternalAttackSurface,
            &mut outcomes,
            std::slice::from_ref(&asset),
            &BTreeSet::from([technique.clone()]),
            TechniqueOutcomeProjectionRow {
                asset: asset.clone(),
                technique: technique.clone(),
                outcome: "found".to_string(),
                source: Some("eas_discover_ports".to_string()),
                evidence_refs: vec![63],
            },
            &strict,
            &BTreeSet::new(),
        );

        assert_eq!(outcomes[&(asset, technique)].state, "found");
    }

    #[test]
    fn eas_empty_requires_matching_guarded_outcome_but_not_business_found() {
        let asset = "192.0.2.10".to_string();
        let technique = golish_db::repo::coverage_truth::TECH_EAS_PORT.to_string();
        let strict = TargetBoundEvidenceFactSet::from([(
            asset.clone(),
            technique.clone(),
            "empty".to_string(),
            62,
        )]);
        let mut outcomes = BTreeMap::new();
        merge_stage_technique_outcome_row(
            StageKind::ExternalAttackSurface,
            &mut outcomes,
            std::slice::from_ref(&asset),
            &BTreeSet::from([technique.clone()]),
            TechniqueOutcomeProjectionRow {
                asset: asset.clone(),
                technique: technique.clone(),
                outcome: "empty".to_string(),
                source: Some("eas_discover_ports".to_string()),
                evidence_refs: vec![62],
            },
            &strict,
            &BTreeSet::new(),
        );
        assert_eq!(outcomes[&(asset, technique)].state, "checked_empty");
    }

    #[test]
    fn eas_web_blocked_outcome_requires_wrapper_source_and_exact_guarded_fact() {
        let parent_asset = "app.example.com".to_string();
        let origin = "https://app.example.com:443".to_string();
        let technique = golish_db::repo::coverage_truth::TECH_EAS_WEB_FP.to_string();
        let strict = TargetBoundEvidenceFactSet::from([(
            origin.clone(),
            technique.clone(),
            "blocked".to_string(),
            81,
        )]);
        let row = |source: &str, evidence_refs: Vec<i64>| TechniqueOutcomeProjectionRow {
            asset: origin.clone(),
            technique: technique.clone(),
            outcome: "blocked".to_string(),
            source: Some(source.to_string()),
            evidence_refs,
        };

        for (source, evidence_refs) in [
            ("whatweb", vec![81]),
            ("eas_fingerprint_web_stack", vec![82]),
        ] {
            let mut outcomes = BTreeMap::new();
            merge_stage_technique_outcome_row(
                StageKind::ExternalAttackSurface,
                &mut outcomes,
                std::slice::from_ref(&parent_asset),
                &BTreeSet::from([technique.clone()]),
                row(source, evidence_refs),
                &strict,
                &BTreeSet::new(),
            );
            assert!(
                outcomes.is_empty(),
                "wrong source or evidence id must not project a blocked parent cell"
            );
        }

        let mut outcomes = BTreeMap::new();
        merge_stage_technique_outcome_row(
            StageKind::ExternalAttackSurface,
            &mut outcomes,
            std::slice::from_ref(&parent_asset),
            &BTreeSet::from([technique.clone()]),
            row("eas_fingerprint_web_stack", vec![81]),
            &strict,
            &BTreeSet::new(),
        );
        assert_eq!(outcomes[&(parent_asset, technique)].state, "blocked");
    }

    #[test]
    fn enumeration_business_rows_do_not_close_origin_cells() {
        assert!(!business_truth_closes_stage_cells(StageKind::Enumeration));
        assert!(!business_truth_closes_stage_cells(
            StageKind::ExternalAttackSurface
        ));
        assert!(business_truth_closes_stage_cells(StageKind::TargetIntel));
    }

    #[test]
    fn enumeration_coverage_lookup_uses_exact_canonical_origin() {
        assert_eq!(
            coverage_lookup_asset(
                "HTTPS://App.Example.com/login?q=1",
                golish_db::repo::coverage_truth::TECH_ENUM_JS,
            ),
            "https://app.example.com:443"
        );
    }

    #[test]
    fn eas_enumeration_and_vuln_never_fallback_to_latest_outcome() {
        assert!(!ui_allows_latest_outcome_fallback(
            StageKind::Enumeration,
            Some("active-session")
        ));
        assert!(!ui_allows_latest_outcome_fallback(
            StageKind::Enumeration,
            None
        ));
        assert!(!ui_allows_latest_outcome_fallback(
            StageKind::ExternalAttackSurface,
            Some("active-session")
        ));
        assert!(!ui_allows_latest_outcome_fallback(
            StageKind::ExternalAttackSurface,
            None
        ));
        assert!(!ui_allows_latest_outcome_fallback(
            StageKind::VulnTriage,
            Some("active-session")
        ));
        assert!(!ui_allows_latest_outcome_fallback(
            StageKind::VulnTriage,
            None
        ));
    }

    #[test]
    fn enumeration_missing_freshness_cutoff_rejects_outcome_projection() {
        assert!(!stage_accepts_outcome_projection(
            StageKind::Enumeration,
            false
        ));
        assert!(stage_accepts_outcome_projection(
            StageKind::Enumeration,
            true
        ));
        assert!(!stage_accepts_outcome_projection(
            StageKind::ExternalAttackSurface,
            false
        ));
        assert!(stage_accepts_outcome_projection(
            StageKind::ExternalAttackSurface,
            true
        ));
        assert!(!stage_accepts_outcome_projection(
            StageKind::VulnTriage,
            false
        ));
        assert!(stage_accepts_outcome_projection(
            StageKind::VulnTriage,
            true
        ));
    }

    #[test]
    fn vuln_triage_routes_general_anonymous_and_nday_cells_to_exact_wrappers() {
        let t = techniques_for_stage(StageKind::VulnTriage);
        assert_eq!(t.len(), 10);
        assert!(t.contains(&"WSTG-INPV-05"));
        assert!(t.contains(&"WSTG-ATHN-04"));
        assert!(!t.contains(&"WSTG-ATHZ-04"));
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
        for cell in &cells {
            let expected = match cell.technique.as_str() {
                "GOLISH-NDAY" => "vuln_nuclei_fingerprint_targeted",
                "WSTG-ATHN-04" => "vuln_probe_anonymous_access",
                _ => "vuln_nuclei_general",
            };
            assert_eq!(cell.suggested_tools, vec![expected.to_string()]);
        }
    }

    #[test]
    fn vuln_triage_domain_target_outcome_closes_exact_confirmed_origin_cell() {
        let mut domain = target("app.example.com", "domain");
        let target_id = domain.id;
        domain.ports = serde_json::json!([
            {
                "port": 443,
                "state": "open",
                "service": "https",
                "url": "https://app.example.com/login"
            }
        ]);

        let assets = expand_vuln_triage_web_origin_rows(vec![domain]);

        assert_eq!(assets.len(), 1);
        assert_eq!(assets[0].id, target_id);
        assert_eq!(assets[0].value, "https://app.example.com:443");
        assert!(assets[0].exact_web_origin);

        let technique = "WSTG-CONF-05";
        let mut outcomes = BTreeMap::new();
        let guarded = TargetBoundEvidenceFactSet::from([(
            "https://app.example.com:443".to_string(),
            technique.to_string(),
            "found".to_string(),
            41,
        )]);
        merge_stage_technique_outcome_row(
            StageKind::VulnTriage,
            &mut outcomes,
            &[assets[0].value.clone()],
            &BTreeSet::from([technique.to_string()]),
            TechniqueOutcomeProjectionRow {
                asset: "https://app.example.com:443".to_string(),
                technique: technique.to_string(),
                outcome: "found".to_string(),
                source: Some("vuln_nuclei_general".to_string()),
                evidence_refs: vec![41],
            },
            &guarded,
            &BTreeSet::new(),
        );

        let cells = coverage_cells(
            StageKind::VulnTriage,
            &assets[0],
            &BTreeSet::new(),
            &outcomes,
        );
        let closed = cells
            .iter()
            .find(|cell| cell.technique == technique)
            .expect("WSTG coverage cell");
        assert_eq!(closed.state, "found");
        assert_eq!(closed.source.as_deref(), Some("vuln_nuclei_general"));
        assert_eq!(closed.evidence_refs, vec![41]);
        assert!(cells
            .iter()
            .filter(|cell| cell.technique != technique)
            .all(|cell| cell.state == "pending"));
    }

    #[test]
    fn vuln_triage_url_path_target_outcome_closes_canonical_origin_cell() {
        let url = target("HTTPS://Portal.Example.com/login?q=1", "url");
        let target_id = url.id;

        let assets = expand_vuln_triage_web_origin_rows(vec![url]);

        assert_eq!(assets.len(), 1);
        assert_eq!(assets[0].id, target_id);
        assert_eq!(assets[0].value, "https://portal.example.com:443");
        assert!(assets[0].exact_web_origin);

        let technique = "GOLISH-NDAY";
        let mut outcomes = BTreeMap::new();
        let guarded = TargetBoundEvidenceFactSet::from([(
            "https://portal.example.com:443".to_string(),
            technique.to_string(),
            "empty".to_string(),
            42,
        )]);
        merge_stage_technique_outcome_row(
            StageKind::VulnTriage,
            &mut outcomes,
            &[assets[0].value.clone()],
            &BTreeSet::from([technique.to_string()]),
            TechniqueOutcomeProjectionRow {
                asset: "https://portal.example.com:443".to_string(),
                technique: technique.to_string(),
                outcome: "empty".to_string(),
                source: Some("vuln_nuclei_fingerprint_targeted".to_string()),
                evidence_refs: vec![42],
            },
            &guarded,
            &BTreeSet::new(),
        );

        let cells = coverage_cells(
            StageKind::VulnTriage,
            &assets[0],
            &BTreeSet::new(),
            &outcomes,
        );
        let closed = cells
            .iter()
            .find(|cell| cell.technique == technique)
            .expect("N-day coverage cell");
        assert_eq!(closed.state, "checked_empty");
        assert_eq!(
            closed.source.as_deref(),
            Some("vuln_nuclei_fingerprint_targeted")
        );
        assert_eq!(closed.evidence_refs, vec![42]);
        assert!(cells
            .iter()
            .filter(|cell| cell.technique != technique)
            .all(|cell| cell.state == "pending"));
    }

    #[test]
    fn vuln_triage_excludes_url_not_in_final_sealed_enumeration_surface() {
        let enumerated = target("https://enumerated.example.com/login", "url");
        let enumerated_id = enumerated.id;
        let unenumerated = target("https://raw-only.example.com/admin", "url");

        let expanded = expand_vuln_triage_web_origin_rows(vec![enumerated, unenumerated]);
        let filtered = filter_vuln_assets_by_enumeration_surface(
            expanded,
            &BTreeSet::from(["https://enumerated.example.com:443".to_string()]),
        )
        .expect("the inherited origin is still materialized by current targets");

        assert_eq!(filtered.len(), 1);
        assert_eq!(filtered[0].id, enumerated_id);
        assert_eq!(filtered[0].value, "https://enumerated.example.com:443");
        assert!(filtered[0].exact_web_origin);
    }

    #[test]
    fn vuln_triage_rejects_current_target_inventory_that_shrinks_sealed_surface() {
        let current = expand_vuln_triage_web_origin_rows(vec![target(
            "https://still-present.example.com/login",
            "url",
        )]);
        let inherited = BTreeSet::from([
            "https://still-present.example.com:443".to_string(),
            "https://deleted-or-moved.example.com:443".to_string(),
        ]);

        let error = filter_vuln_assets_by_enumeration_surface(current, &inherited)
            .expect_err("a non-empty sealed surface must not collapse to partial or zero coverage");

        assert!(error
            .to_string()
            .contains("cannot materialize the complete final-sealed Enumeration surface"));
    }

    #[test]
    fn vuln_triage_accepts_explicit_empty_sealed_surface() {
        let current = expand_vuln_triage_web_origin_rows(vec![target(
            "https://current-but-not-sealed.example.com/login",
            "url",
        )]);

        let filtered = filter_vuln_assets_by_enumeration_surface(current, &BTreeSet::new())
            .expect("an explicitly empty sealed surface is authoritative");

        assert!(filtered.is_empty());
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
    fn enumeration_worklist_read_model_is_empty_without_eas_live_truth() {
        let assets = vec![
            target("app.example.com", "domain"),
            target("203.0.113.10", "ip"),
        ];

        let filtered = filter_enumeration_assets_by_eas_found(
            assets.clone(),
            &BTreeSet::new(),
            &BTreeSet::new(),
        );

        assert!(filtered.is_empty());
    }

    #[test]
    fn enumeration_origin_expansion_drops_bare_assets_without_exact_http_origin() {
        let mut bare_domain = target("alive.example.com", "domain");
        bare_domain.http_status = Some(200);
        let mut unknown_tcp = target("tcp.example.com", "domain");
        unknown_tcp.ports = serde_json::json!([
            {"port": 80, "state": "open", "service": "unknown", "protocol": "tcp"}
        ]);
        let bare_ip = target("203.0.113.10", "ip");

        let origins = expand_enumeration_web_origin_rows(vec![bare_domain, unknown_tcp, bare_ip]);

        assert!(
            origins.is_empty(),
            "alive/http-status/bare TCP host facts do not prove an exact scheme://host:port"
        );
    }

    #[test]
    fn enumeration_origin_expansion_accepts_only_explicit_url_or_http_service_metadata() {
        let direct_url = target("HTTPS://Direct.Example/path?q=1", "url");
        let mut service_metadata = target("service.example.com", "domain");
        service_metadata.ports = serde_json::json!([
            {"port": 8080, "state": "open", "service": "http-alt", "protocol": "tcp"},
            {"port": 8443, "state": "open", "service": "https", "protocol": "tcp"},
            {"port": 9443, "state": "closed", "service": "https", "protocol": "tcp"}
        ]);
        let mut explicit_url = target("metadata.example.com", "domain");
        explicit_url.ports = serde_json::json!([
            {"port": 4443, "state": "open", "service": "unknown", "url": "https://metadata.example.com:4443/login"}
        ]);
        let mut ipv6 = target("2001:db8::1", "ip");
        ipv6.ports = serde_json::json!([
            {"port": 8443, "state": "open", "service": "https", "protocol": "tcp"}
        ]);

        let origins = expand_enumeration_web_origin_rows(vec![
            direct_url,
            service_metadata,
            explicit_url,
            ipv6,
        ]);

        assert_eq!(
            origins
                .iter()
                .map(|row| row.value.as_str())
                .collect::<Vec<_>>(),
            vec![
                "https://direct.example:443",
                "http://service.example.com:8080",
                "https://service.example.com:8443",
                "https://metadata.example.com:4443",
                "https://[2001:db8::1]:8443",
            ]
        );
        assert!(origins.iter().all(|row| row.exact_web_origin));
        assert!(origins.iter().all(|row| row.target_type == "url"));
    }

    #[test]
    fn enumeration_worklist_synthesizes_no_url_ip_ssl_http_origin() {
        let mut ip = target("203.0.113.10", "ip_address");
        let owner_id = ip.id;
        ip.ports = serde_json::json!([
            {"port": 443, "state": "open", "service": "ssl/http"},
            {"port": 8080, "state": "closed", "service": "http"},
            {"port": 22, "state": "open", "service": "ssh"}
        ]);

        let origins = expand_enumeration_web_origin_rows(vec![ip]);

        assert_eq!(origins.len(), 1);
        assert_eq!(origins[0].id, owner_id);
        assert_eq!(origins[0].value, "https://203.0.113.10:443");
        assert!(origins[0].exact_web_origin);
    }

    #[test]
    fn enumeration_eas_filter_keeps_ip_with_confirmed_http_service_without_http_status() {
        let mut ip = target("203.0.113.10", "ip_address");
        ip.http_status = None;
        ip.ports = serde_json::json!([
            {"port": 443, "state": "open", "service": "ssl/http"}
        ]);

        let filtered =
            filter_enumeration_assets_by_eas_found(vec![ip], &BTreeSet::new(), &BTreeSet::new());

        assert_eq!(filtered.len(), 1);
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
    fn enumeration_expanded_ip_origins_keep_all_four_axes_pending() {
        let mut ip = target("203.0.113.10", "ip");
        ip.ports = serde_json::json!([
            {"port": 80, "state": "open", "service": "http"},
            {"port": 443, "state": "open", "service": "ssl/http"}
        ]);
        let raw_web_capable_assets = BTreeSet::from([ip.value.clone()]);

        let origins = expand_enumeration_web_origin_rows(vec![ip]);

        assert_eq!(
            origins
                .iter()
                .map(|origin| origin.value.as_str())
                .collect::<Vec<_>>(),
            vec!["http://203.0.113.10:80", "https://203.0.113.10:443"]
        );
        for origin in origins {
            let cells = coverage_cells_with_eas_parent_ips(
                StageKind::Enumeration,
                &origin,
                &BTreeSet::new(),
                &BTreeMap::new(),
                &raw_web_capable_assets,
                &BTreeSet::new(),
            );
            assert_eq!(
                cells
                    .iter()
                    .map(|cell| (cell.technique.as_str(), cell.state.as_str()))
                    .collect::<Vec<_>>(),
                vec![
                    ("GOLISH-ENUM-JS", "pending"),
                    ("GOLISH-ENUM-DIR", "pending"),
                    ("GOLISH-ENUM-PARAM", "pending"),
                    ("GOLISH-ENUM-JSAPI", "pending"),
                ],
                "expanded exact IP origin {} must remain actionable",
                origin.value
            );
        }
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
            vec![
                "browser_collect_js_api".to_string(),
                "enum_preflight_web_origins".to_string(),
            ]
        );
        assert_eq!(
            cells[1].suggested_tools,
            vec![
                "route_probe_paths".to_string(),
                "enum_preflight_web_origins".to_string(),
            ]
        );
        assert_eq!(
            cells[2].suggested_tools,
            vec![
                "browser_collect_js_api".to_string(),
                "js_extract_apis".to_string(),
                "enum_preflight_web_origins".to_string(),
            ]
        );
        assert_eq!(
            cells[3].suggested_tools,
            vec![
                "js_extract_apis".to_string(),
                "enum_preflight_web_origins".to_string(),
            ]
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
    fn eas_ip_origin_url_asset_never_gets_port_or_service_work() {
        let asset = target("http://127.0.0.1:54537", "url");
        let cells = coverage_cells_with_eas_parent_ips(
            StageKind::ExternalAttackSurface,
            &asset,
            &BTreeSet::new(),
            &BTreeMap::new(),
            &BTreeSet::from([asset.value.clone()]),
            &BTreeSet::new(),
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
                ),
                (golish_db::repo::coverage_truth::TECH_EAS_WEB_FP, "pending"),
            ]
        );
        assert_eq!(
            cells[0].suggested_tools,
            vec!["eas_probe_http_liveness".to_string()]
        );
        assert_eq!(
            cells[3].suggested_tools,
            vec!["eas_fingerprint_web_stack".to_string()]
        );
        assert!(cells[1].suggested_tools.is_empty());
        assert!(cells[2].suggested_tools.is_empty());
    }

    #[test]
    fn wildcard_scope_pattern_has_only_passive_intel_child_expansion_work() {
        let asset = target("*.moresec.cn", "wildcard");
        let intel = coverage_cells(
            StageKind::TargetIntel,
            &asset,
            &BTreeSet::new(),
            &BTreeMap::new(),
        );
        assert_eq!(
            intel
                .iter()
                .filter(|cell| cell.state != "not_applicable")
                .map(|cell| (cell.technique.as_str(), cell.state.as_str()))
                .collect::<Vec<_>>(),
            vec![(golish_db::repo::coverage_truth::TECH_SUBDOMAIN, "pending")],
            "wildcard Target Intel must expose exactly one passive expansion responsibility"
        );

        let eas = coverage_cells(
            StageKind::ExternalAttackSurface,
            &asset,
            &BTreeSet::new(),
            &BTreeMap::new(),
        );
        assert!(
            eas.iter().all(|cell| cell.state == "not_applicable"),
            "wildcard is never an executable EAS work item: {eas:?}"
        );
    }

    #[test]
    fn target_intel_preflight_anchor_filters_promoted_wildcard_child_only() {
        let wildcard = target("*.moresec.cn", "wildcard");
        let child = target("app.moresec.cn", "domain");
        let values = |rows: Vec<TargetCoverageRow>| {
            target_intel_anchor_only_assets(rows)
                .into_iter()
                .map(|row| row.value)
                .collect::<Vec<_>>()
        };

        assert_eq!(
            values(vec![wildcard.clone(), child.clone()]),
            vec!["*.moresec.cn".to_string()]
        );
        assert_eq!(
            values(vec![wildcard.clone(), target("moresec.cn", "domain")]),
            vec!["*.moresec.cn".to_string(), "moresec.cn".to_string()],
            "wildcard must not absorb its apex"
        );
        assert_eq!(
            values(vec![wildcard.clone(), target("app.vendor.net", "domain")]),
            vec!["*.moresec.cn".to_string(), "app.vendor.net".to_string()]
        );
        assert_eq!(
            values(vec![target("moresec.cn", "organization"), child.clone()]),
            vec!["moresec.cn".to_string(), "app.moresec.cn".to_string()],
            "dotted organization display names are never coverage anchors"
        );
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
    fn eas_domain_alias_to_existing_ip_keeps_liveness_and_web_coverage() {
        let ip = target("115.28.135.55", "ip");
        let mut domain = target("moresec.cn", "domain");
        domain.real_ip = "115.28.135.55".to_string();

        assert!(counts_as_coverage_asset(&ip));
        assert!(counts_as_coverage_asset(&domain));

        let cells = coverage_cells_with_eas_parent_ips(
            StageKind::ExternalAttackSurface,
            &domain,
            &BTreeSet::new(),
            &BTreeMap::new(),
            &BTreeSet::from([domain.value.clone()]),
            &BTreeSet::new(),
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
                ),
                (golish_db::repo::coverage_truth::TECH_EAS_WEB_FP, "pending"),
            ]
        );
    }

    #[test]
    fn eas_url_alias_to_existing_ip_keeps_liveness_and_web_coverage() {
        let ip = target("115.28.135.55", "ip");
        let mut endpoint = target("https://app.moresec.cn:8443/login", "url");
        endpoint.real_ip = "115.28.135.55".to_string();

        assert!(counts_as_coverage_asset(&ip));
        assert!(counts_as_coverage_asset(&endpoint));

        let cells = coverage_cells_with_eas_parent_ips(
            StageKind::ExternalAttackSurface,
            &endpoint,
            &BTreeSet::new(),
            &BTreeMap::new(),
            &BTreeSet::from([endpoint.value.clone()]),
            &BTreeSet::new(),
        );

        assert_eq!(
            cells
                .iter()
                .map(|cell| cell.state.as_str())
                .collect::<Vec<_>>(),
            vec!["pending", "not_applicable", "not_applicable", "pending"]
        );
    }

    #[test]
    fn eas_web_cell_stays_partial_until_every_exact_origin_is_terminal() {
        let asset = target("app.moresec.cn", "domain");
        let mut cells = coverage_cells_with_eas_parent_ips(
            StageKind::ExternalAttackSurface,
            &asset,
            &BTreeSet::new(),
            &BTreeMap::new(),
            &BTreeSet::from([asset.value.clone()]),
            &BTreeSet::new(),
        );
        let required = vec![
            "http://app.moresec.cn:80".to_string(),
            "https://app.moresec.cn:443".to_string(),
        ];
        let mut origin_coverage = EasWebOriginCoverage {
            required_by_target: BTreeMap::from([(asset.id, required.clone())]),
            completed: BTreeMap::from([(
                required[0].clone(),
                OutcomeProjection {
                    state: "found".to_string(),
                    source: Some("whatweb".to_string()),
                    evidence_refs: vec![41],
                },
            )]),
        };

        apply_eas_web_origin_details(
            StageKind::ExternalAttackSurface,
            &asset,
            &mut cells,
            &origin_coverage,
        );
        let web = cells
            .iter()
            .find(|cell| cell.technique == golish_db::repo::coverage_truth::TECH_EAS_WEB_FP)
            .unwrap();
        assert_eq!(web.state, "partial");
        assert_eq!(
            web.suggested_tools,
            vec!["eas_fingerprint_web_stack".to_string()]
        );
        assert_eq!(
            web.details["missing_origins"],
            serde_json::json!(["https://app.moresec.cn:443"])
        );
        assert_eq!(
            web.details["recommended_args"]["target_urls"],
            serde_json::json!([{
                "target_id": asset.id.to_string(),
                "target_url": "https://app.moresec.cn:443"
            }])
        );

        origin_coverage.completed.insert(
            required[1].clone(),
            OutcomeProjection {
                state: "checked_empty".to_string(),
                source: Some("whatweb".to_string()),
                evidence_refs: vec![42],
            },
        );
        apply_eas_web_origin_details(
            StageKind::ExternalAttackSurface,
            &asset,
            &mut cells,
            &origin_coverage,
        );
        let web = cells
            .iter()
            .find(|cell| cell.technique == golish_db::repo::coverage_truth::TECH_EAS_WEB_FP)
            .unwrap();
        assert_eq!(web.state, "found");
        assert_eq!(web.evidence_refs, vec![41, 42]);
        assert!(web.suggested_tools.is_empty());
        assert_eq!(web.details["missing_origins"], serde_json::json!([]));
        assert_eq!(
            web.details["recommended_args"]["target_urls"],
            serde_json::json!([])
        );
    }

    #[test]
    fn eas_web_blocked_origin_is_terminal_and_visible_in_parent_rollup() {
        let asset = target("app.moresec.cn", "domain");
        let mut cells = coverage_cells_with_eas_parent_ips(
            StageKind::ExternalAttackSurface,
            &asset,
            &BTreeSet::new(),
            &BTreeMap::new(),
            &BTreeSet::from([asset.value.clone()]),
            &BTreeSet::new(),
        );
        let blocked_origin = "https://app.moresec.cn:443".to_string();
        let empty_origin = "http://app.moresec.cn:80".to_string();
        let mut origin_coverage = EasWebOriginCoverage {
            required_by_target: BTreeMap::from([(
                asset.id,
                vec![empty_origin.clone(), blocked_origin.clone()],
            )]),
            completed: BTreeMap::from([
                (
                    blocked_origin.clone(),
                    OutcomeProjection {
                        state: "blocked".to_string(),
                        source: Some("eas_fingerprint_web_stack".to_string()),
                        evidence_refs: vec![51],
                    },
                ),
                (
                    empty_origin.clone(),
                    OutcomeProjection {
                        state: "checked_empty".to_string(),
                        source: Some("eas_fingerprint_web_stack".to_string()),
                        evidence_refs: vec![52],
                    },
                ),
            ]),
        };

        apply_eas_web_origin_details(
            StageKind::ExternalAttackSurface,
            &asset,
            &mut cells,
            &origin_coverage,
        );
        let web = cells
            .iter()
            .find(|cell| cell.technique == golish_db::repo::coverage_truth::TECH_EAS_WEB_FP)
            .unwrap();
        assert_eq!(web.state, "blocked");
        assert_eq!(web.evidence_refs, vec![51, 52]);
        assert!(web.suggested_tools.is_empty());
        assert_eq!(web.details["missing_origins"], serde_json::json!([]));
        assert_eq!(
            web.details["blocked_origins"],
            serde_json::json!([blocked_origin])
        );

        origin_coverage.completed.insert(
            empty_origin,
            OutcomeProjection {
                state: "found".to_string(),
                source: Some("eas_fingerprint_web_stack".to_string()),
                evidence_refs: vec![53],
            },
        );
        apply_eas_web_origin_details(
            StageKind::ExternalAttackSurface,
            &asset,
            &mut cells,
            &origin_coverage,
        );
        let web = cells
            .iter()
            .find(|cell| cell.technique == golish_db::repo::coverage_truth::TECH_EAS_WEB_FP)
            .unwrap();
        assert_eq!(web.state, "found");
        assert_eq!(
            web.details["blocked_origins"],
            serde_json::json!(["https://app.moresec.cn:443"])
        );
        assert_eq!(web.details["missing_origins"], serde_json::json!([]));
        assert!(web.suggested_tools.is_empty());
    }

    #[test]
    fn eas_domain_without_existing_ip_remains_a_direct_coverage_asset() {
        let mut domain = target("moresec.cn", "domain");
        domain.real_ip = "115.28.135.55".to_string();

        assert!(counts_as_coverage_asset(&domain));

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
                ),
                (
                    golish_db::repo::coverage_truth::TECH_EAS_WEB_FP,
                    "not_applicable"
                )
            ]
        );
    }

    #[test]
    fn eas_ip_liveness_pending_suggests_port_discovery_first() {
        let asset = target("203.0.113.10", "ip");
        let cells = coverage_cells(
            StageKind::ExternalAttackSurface,
            &asset,
            &BTreeSet::new(),
            &BTreeMap::new(),
        );

        assert_eq!(
            cells[0].technique,
            golish_db::repo::coverage_truth::TECH_EAS_LIVENESS
        );
        assert_eq!(cells[0].state, "pending");
        assert_eq!(
            cells[0].suggested_tools,
            vec!["eas_discover_ports".to_string()]
        );
        assert!(cells[0]
            .note
            .as_deref()
            .is_some_and(|note| note.contains("run port discovery first")));
    }

    #[test]
    fn eas_web_capable_asset_gets_web_fingerprint_cell() {
        let asset = target("https://app.example.com/login", "url");
        let cells = coverage_cells_with_eas_parent_ips(
            StageKind::ExternalAttackSurface,
            &asset,
            &BTreeSet::new(),
            &BTreeMap::new(),
            &BTreeSet::from([asset.value.clone()]),
            &BTreeSet::new(),
        );

        let web_cell = cells
            .iter()
            .find(|cell| cell.technique == golish_db::repo::coverage_truth::TECH_EAS_WEB_FP)
            .expect("web fingerprint cell");
        assert_eq!(web_cell.state, "pending");
        assert_eq!(
            web_cell.suggested_tools,
            vec!["eas_fingerprint_web_stack".to_string()]
        );
    }

    #[test]
    fn whatweb_truth_fills_web_fingerprint_cell_by_host_key() {
        let asset = target("https://app.example.com/login", "url");
        let found = BTreeSet::from([(
            "app.example.com".to_string(),
            golish_db::repo::coverage_truth::TECH_EAS_WEB_FP.to_string(),
        )]);
        let cells = coverage_cells_with_eas_parent_ips(
            StageKind::ExternalAttackSurface,
            &asset,
            &found,
            &BTreeMap::new(),
            &BTreeSet::from([asset.value.clone()]),
            &BTreeSet::new(),
        );

        let web_cell = cells
            .iter()
            .find(|cell| cell.technique == golish_db::repo::coverage_truth::TECH_EAS_WEB_FP)
            .expect("web fingerprint cell");
        assert_eq!(web_cell.state, "found");
        assert!(web_cell.suggested_tools.is_empty());
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
        assert_eq!(
            cells[2].suggested_tools,
            vec!["eas_fingerprint_services".to_string()]
        );
    }

    #[test]
    fn eas_service_pending_exposes_missing_open_ports() {
        let mut asset = target("222.186.129.58", "ip");
        asset.ports = serde_json::json!([
            {"port": "80", "state": "open", "service": "http"},
            {"port": "82", "state": "open", "service": ""},
            {"port": "50002", "state": "open", "service": "open"},
            {"port": "53", "state": "open", "service": "domain"}
        ]);
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
        let service_cell = cells
            .iter()
            .find(|cell| cell.technique == golish_db::repo::coverage_truth::TECH_EAS_SERVICE_FP)
            .expect("service cell");

        assert_eq!(service_cell.state, "pending");
        assert_eq!(
            service_cell.details["missing_open_ports"],
            serde_json::json!([82, 50002])
        );
        assert_eq!(
            service_cell.details["recommended_args"]["ports"],
            serde_json::json!("82,50002")
        );
        assert!(service_cell
            .note
            .as_deref()
            .is_some_and(|note| note.contains("82,50002")));
    }

    #[test]
    fn eas_service_found_outcome_does_not_override_port_level_truth() {
        let mut outcomes = BTreeMap::new();
        let assets = vec!["115.175.6.207".to_string()];
        let techniques =
            BTreeSet::from([golish_db::repo::coverage_truth::TECH_EAS_SERVICE_FP.to_string()]);

        merge_technique_outcome_row(
            &mut outcomes,
            &assets,
            &techniques,
            TechniqueOutcomeProjectionRow {
                asset: "115.175.6.207".to_string(),
                technique: golish_db::repo::coverage_truth::TECH_EAS_SERVICE_FP.to_string(),
                outcome: "found".to_string(),
                source: Some("nmap".to_string()),
                evidence_refs: vec![1],
            },
        );

        assert!(outcomes.is_empty());

        merge_technique_outcome_row(
            &mut outcomes,
            &assets,
            &techniques,
            TechniqueOutcomeProjectionRow {
                asset: "115.175.6.207".to_string(),
                technique: golish_db::repo::coverage_truth::TECH_EAS_SERVICE_FP.to_string(),
                outcome: "empty".to_string(),
                source: Some("nmap".to_string()),
                evidence_refs: vec![2],
            },
        );

        assert_eq!(
            outcomes[&(
                "115.175.6.207".to_string(),
                golish_db::repo::coverage_truth::TECH_EAS_SERVICE_FP.to_string()
            )]
                .state,
            "checked_empty"
        );
    }

    #[test]
    fn eas_dns_only_ip_keeps_service_pending_without_port_level_service_surface() {
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
            &service_not_applicable,
        );

        assert_eq!(cells[1].state, "found");
        assert_eq!(cells[2].state, "pending");
        assert_eq!(
            cells[2].suggested_tools,
            vec!["eas_fingerprint_services".to_string()]
        );
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
    fn target_intel_partial_outcome_overrides_business_found_in_preflight() {
        let asset = target("partial.example.com", "domain");
        let technique = golish_db::repo::coverage_truth::TECH_DNS;
        let found = BTreeSet::from([(asset.value.clone(), technique.to_string())]);
        let outcomes = BTreeMap::from([(
            (asset.value.clone(), technique.to_string()),
            OutcomeProjection {
                state: outcome_state("partial"),
                source: Some("resolver".to_string()),
                evidence_refs: Vec::new(),
            },
        )]);

        let cells = coverage_cells(StageKind::TargetIntel, &asset, &found, &outcomes);
        let dns = cells
            .iter()
            .find(|cell| cell.technique == technique)
            .expect("DNS cell");
        assert_eq!(dns.state, "partial");
        assert_eq!(dns.source.as_deref(), Some("resolver"));
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
            Some("not in the current asset wave; queued for a supplemental wave")
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
    fn explicit_current_wave_assets_override_created_at_cutoff() {
        let mut asset = target("new.example.com", "domain");
        let cutoff = Utc::now();
        asset.created_at = cutoff + chrono::Duration::minutes(5);
        let current = BTreeSet::from([asset.id]);

        assert!(!is_deferred_wave_asset(
            &asset,
            Some(cutoff),
            Some(&current)
        ));
    }

    #[test]
    fn explicit_current_wave_assets_defer_assets_outside_wave() {
        let mut asset = target("old.example.com", "domain");
        let cutoff = Utc::now();
        asset.created_at = cutoff - chrono::Duration::minutes(5);
        let current = BTreeSet::from([Uuid::new_v4()]);

        assert!(is_deferred_wave_asset(&asset, Some(cutoff), Some(&current)));
    }

    #[test]
    fn explicit_current_wave_without_usable_identity_fails_closed() {
        let error = explicit_current_wave_membership(
            Some(vec![Uuid::new_v4(), Uuid::new_v4()]),
            Some(vec!["".to_string(), "  ".to_string()]),
        )
        .expect_err("an explicit but empty wave must not become an authoritative empty axis");

        assert!(error.to_string().contains("blank asset_value"));
    }

    #[test]
    fn current_wave_membership_survives_target_value_change() {
        let mut asset = target("new.example.com", "domain");
        let target_id = asset.id;
        asset.value = "renamed.example.com".to_string();
        let membership = explicit_current_wave_membership(
            Some(vec![target_id]),
            Some(vec!["old.example.com".to_string()]),
        )
        .unwrap()
        .unwrap();

        assert!(!is_deferred_wave_asset(
            &asset,
            None,
            Some(&membership.target_ids),
        ));
    }

    #[test]
    fn same_value_different_target_ids_do_not_share_wave_membership() {
        let first = target("same.example.com", "domain");
        let mut second = target("same.example.com", "domain");
        second.id = Uuid::new_v4();
        let membership = explicit_current_wave_membership(
            Some(vec![second.id]),
            Some(vec!["same.example.com".to_string()]),
        )
        .unwrap()
        .unwrap();

        assert!(is_deferred_wave_asset(
            &first,
            None,
            Some(&membership.target_ids),
        ));
        assert!(!is_deferred_wave_asset(
            &second,
            None,
            Some(&membership.target_ids),
        ));
    }

    #[test]
    fn deleted_only_wave_target_fails_closed() {
        let deleted_target_id = Uuid::new_v4();
        let membership = explicit_current_wave_membership(
            Some(vec![deleted_target_id]),
            Some(vec!["deleted.example.com".to_string()]),
        )
        .unwrap()
        .unwrap();

        let error = ensure_current_wave_targets_present(&[], Some(&membership))
            .expect_err("deleted wave target must not become an empty authoritative axis");

        assert!(error.to_string().contains(&deleted_target_id.to_string()));
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
    fn target_intel_source_error_overrides_same_run_found_projection() {
        let asset_values = vec!["moresec.cn".to_string()];
        let technique = golish_db::repo::coverage_truth::TECH_DNS.to_string();
        let stage_techniques = BTreeSet::from([technique.clone()]);
        let key = ("moresec.cn".to_string(), technique.clone());
        let mut outcomes = BTreeMap::from([(
            key.clone(),
            OutcomeProjection {
                state: "found".to_string(),
                source: Some("resolver".to_string()),
                evidence_refs: vec![10],
            },
        )]);

        merge_source_query_row(
            &mut outcomes,
            &asset_values,
            &stage_techniques,
            SourceQueryProjectionRow {
                source: "resolver".to_string(),
                target: "moresec.cn".to_string(),
                technique: Some(technique),
                status: "error".to_string(),
                evidence_refs: vec![11],
            },
            &[],
        );

        let outcome = outcomes
            .get(&key)
            .expect("source retry marker remains visible");
        assert_eq!(outcome.state, "error");
        assert_eq!(outcome.evidence_refs, vec![10, 11]);
    }

    #[test]
    fn target_intel_sibling_provider_found_preserves_business_found_projection() {
        let asset = target("广州有创网络科技有限公司", "organization");
        let asset_values = vec![asset.value.clone()];
        let organization_asset_values = asset_values.clone();
        let technique = golish_db::repo::coverage_truth::TECH_OSINT.to_string();
        let stage_techniques = BTreeSet::from([technique.clone()]);
        let key = (asset.value.clone(), technique.clone());
        let business_found = BTreeSet::from([key.clone()]);
        let source_rows = vec![
            SourceQueryProjectionRow {
                source: "enscan-go-enrichment".to_string(),
                target: asset.value.clone(),
                technique: Some(technique.clone()),
                status: "found".to_string(),
                evidence_refs: vec![10],
            },
            SourceQueryProjectionRow {
                source: "quake".to_string(),
                target: asset.value.clone(),
                technique: Some(technique.clone()),
                status: "error".to_string(),
                evidence_refs: vec![11],
            },
        ];
        let found_sources = source_query_found_sources(
            &source_rows,
            &asset_values,
            &stage_techniques,
            &organization_asset_values,
        );
        let quake_error = source_rows
            .into_iter()
            .find(|row| row.source == "quake")
            .expect("quake row");
        let mut outcomes = BTreeMap::new();

        merge_source_query_row_with_authoritative_sources(
            &mut outcomes,
            &asset_values,
            &stage_techniques,
            quake_error,
            &organization_asset_values,
            &found_sources,
            &business_found,
        );

        let cells = coverage_cells(StageKind::TargetIntel, &asset, &business_found, &outcomes);
        let osint = cells
            .iter()
            .find(|cell| cell.technique == technique)
            .expect("OSINT cell");
        assert_eq!(osint.state, "found");
    }

    #[test]
    fn source_error_does_not_reopen_gate_materialized_terminal_exception() {
        for terminal_state in ["blocked", "not_applicable"] {
            let asset_values = vec!["广州有创网络科技有限公司".to_string()];
            let technique = golish_db::repo::coverage_truth::TECH_OSINT.to_string();
            let stage_techniques = BTreeSet::from([technique.clone()]);
            let key = (asset_values[0].clone(), technique.clone());
            let mut outcomes = BTreeMap::from([(
                key.clone(),
                OutcomeProjection {
                    state: terminal_state.to_string(),
                    source: Some("submit_stage_deliverable".to_string()),
                    evidence_refs: Vec::new(),
                },
            )]);

            merge_source_query_row(
                &mut outcomes,
                &asset_values,
                &stage_techniques,
                SourceQueryProjectionRow {
                    source: "quake".to_string(),
                    target: asset_values[0].clone(),
                    technique: Some(technique),
                    status: "error".to_string(),
                    evidence_refs: vec![27784],
                },
                &asset_values,
            );

            let outcome = outcomes
                .get(&key)
                .expect("gate-materialized terminal exception remains visible");
            assert_eq!(outcome.state, terminal_state);
            assert_eq!(outcome.source.as_deref(), Some("submit_stage_deliverable"));
            assert_eq!(outcome.evidence_refs, vec![27784]);
        }
    }

    #[test]
    fn wildcard_subdomain_preflight_consumes_base_domain_source_status() {
        let asset_values = vec!["*.moresec.cn".to_string(), "moresec.cn".to_string()];
        let technique = golish_db::repo::coverage_truth::TECH_SUBDOMAIN.to_string();
        let stage_techniques = BTreeSet::from([technique.clone()]);
        let mut outcomes = BTreeMap::new();

        merge_source_query_row(
            &mut outcomes,
            &asset_values,
            &stage_techniques,
            SourceQueryProjectionRow {
                source: "domain-expansion".to_string(),
                target: "moresec.cn".to_string(),
                technique: Some(technique.clone()),
                status: "empty".to_string(),
                evidence_refs: vec![12],
            },
            &[],
        );

        for asset in asset_values {
            let key = (asset, technique.clone());
            let outcome = outcomes
                .get(&key)
                .expect("base-domain expansion result covers exact and wildcard responsibilities");
            assert_eq!(outcome.state, "checked_empty");
            assert_eq!(outcome.evidence_refs, vec![12]);
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
