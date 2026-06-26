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
use golish_agent_kit::harness::StageKind;

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
    pub source: String,
    pub discovered_phase: String,
    pub created_at: String,
    pub parent_id: Option<String>,
    pub coverage: Vec<StageAssetCoverageCell>,
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
    pub suggested_tools: Vec<String>,
}

#[derive(Debug, FromRow)]
struct TargetCoverageRow {
    id: Uuid,
    value: String,
    target_type: String,
    source: Option<String>,
    parent_id: Option<Uuid>,
    created_at: DateTime<Utc>,
}

#[derive(Debug, Clone)]
struct OutcomeProjection {
    state: String,
    source: Option<String>,
    evidence_refs: Vec<i64>,
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

    let assets = list_stage_targets(&state.db_pool, org_id, stage_kind).await?;
    let asset_values: Vec<String> = assets.iter().map(|a| a.value.clone()).collect();
    let asset_types: Vec<String> = assets.iter().map(|a| a.target_type.clone()).collect();

    let found_facts = golish_db::repo::coverage_truth::coverage_truth_facts(
        &state.db_pool,
        Some(org_id),
        &asset_values,
        &asset_types,
        run_start,
    )
    .await?;
    let found: BTreeSet<(String, String)> = found_facts
        .into_iter()
        .map(|(asset, technique)| (asset, technique.to_string()))
        .collect();

    let outcomes = if let Some(run_id) = session_id.as_deref() {
        stage_outcomes(&state.db_pool, org_id, stage_kind, run_id, &asset_values).await?
    } else {
        BTreeMap::new()
    };

    let mut rows = Vec::with_capacity(assets.len());
    let mut done_assets = 0usize;
    let mut pending_assets = 0usize;
    let mut blocked_assets = 0usize;
    let mut seed_assets = 0usize;
    let mut new_assets = 0usize;

    for asset in assets {
        let phase = discovered_phase(&asset, run_start);
        if phase == "new_in_stage" {
            new_assets += 1;
        } else {
            seed_assets += 1;
        }
        let coverage = coverage_cells(stage_kind, &asset, &found, &outcomes);
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
        rows.push(StageAssetCoverageRow {
            target_id: asset.id.to_string(),
            value: asset.value,
            target_type: asset.target_type,
            source: asset.source.unwrap_or_default(),
            discovered_phase: phase,
            created_at: asset.created_at.to_rfc3339(),
            parent_id: asset.parent_id.map(|id| id.to_string()),
            coverage,
        });
    }

    Ok(StageAssetCoverageSnapshot {
        stage,
        organization_id,
        session_id,
        summary: StageAssetCoverageSummary {
            total_assets: rows.len(),
            seed_assets,
            new_assets,
            done_assets,
            pending_assets,
            blocked_assets,
        },
        assets: rows,
    })
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

async fn get_organization_row(
    pool: &sqlx::PgPool,
    organization_id: Uuid,
) -> anyhow::Result<Option<TargetCoverageRow>> {
    Ok(sqlx::query_as::<_, TargetCoverageRow>(
        r#"SELECT id,
                  name AS value,
                  'organization'::text AS target_type,
                  'engagement_org'::text AS source,
                  parent_id,
                  created_at
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
        r#"SELECT id, value, target_type::text AS target_type, source, parent_id, created_at
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
    run_id: &str,
    asset_values: &[String],
) -> anyhow::Result<BTreeMap<(String, String), OutcomeProjection>> {
    let mut out = BTreeMap::new();
    let stage_techniques: BTreeSet<String> = techniques_for_stage(stage)
        .into_iter()
        .map(str::to_string)
        .collect();
    for row in
        golish_db::repo::technique_outcomes::list_for_run(pool, organization_id, run_id).await?
    {
        if !stage_techniques.contains(&row.technique) {
            continue;
        }
        merge_outcome(
            &mut out,
            (row.asset, row.technique),
            OutcomeProjection {
                state: outcome_state(&row.outcome),
                source: row.source,
                evidence_refs: row.evidence_ids,
            },
        );
    }
    for row in
        golish_db::repo::source_query_log::list_for_run(pool, organization_id, run_id).await?
    {
        let Some(technique) = row.technique else {
            continue;
        };
        if !stage_techniques.contains(&technique) {
            continue;
        }
        let Some(state) = source_query_terminal_state(&row.status) else {
            continue;
        };
        let projection = OutcomeProjection {
            state,
            source: Some(row.source),
            evidence_refs: row.evidence_ids,
        };
        if row.target.is_empty() {
            for asset in asset_values {
                merge_outcome(
                    &mut out,
                    (asset.clone(), technique.clone()),
                    projection.clone(),
                );
            }
        } else if asset_values.iter().any(|asset| asset == &row.target) {
            merge_outcome(&mut out, (row.target, technique), projection);
        }
    }
    Ok(out)
}

fn coverage_cells(
    stage: StageKind,
    asset: &TargetCoverageRow,
    found: &BTreeSet<(String, String)>,
    outcomes: &BTreeMap<(String, String), OutcomeProjection>,
) -> Vec<StageAssetCoverageCell> {
    let class = golish_agent_kit::harness::technique_resolver::AssetClass::from_target_type(
        &asset.target_type,
    );
    techniques_for_stage(stage)
        .into_iter()
        .map(|technique| {
            if !golish_agent_kit::harness::technique_resolver::technique_applies_to_value(
                stage,
                class,
                &asset.value,
                technique,
            ) {
                return cell(
                    technique,
                    "not_applicable",
                    None,
                    Vec::new(),
                    Some("not applicable to this asset type".to_string()),
                );
            }
            if found.contains(&(asset.value.clone(), technique.to_string())) {
                return cell(technique, "found", None, Vec::new(), None);
            }
            if let Some(outcome) = outcomes.get(&(asset.value.clone(), technique.to_string())) {
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
        .collect()
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
            golish_db::repo::coverage_truth::TECH_ENUM_DIR,
            golish_db::repo::coverage_truth::TECH_ENUM_PARAM,
            golish_db::repo::coverage_truth::TECH_ENUM_JSAPI,
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
    StageAssetCoverageCell {
        technique: technique.to_string(),
        label: technique_label(technique).to_string(),
        state: state.to_string(),
        source,
        evidence_refs,
        note,
        suggested_tools: if state == "pending" {
            suggested_tools(technique)
        } else {
            Vec::new()
        },
    }
}

fn outcome_state(outcome: &str) -> String {
    match outcome {
        "found" => "found",
        "empty" => "checked_empty",
        "blocked" => "blocked",
        "error" => "error",
        _ => "pending",
    }
    .to_string()
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
        golish_db::repo::coverage_truth::TECH_ENUM_DIR => "Directory",
        golish_db::repo::coverage_truth::TECH_ENUM_PARAM => "Parameter",
        golish_db::repo::coverage_truth::TECH_ENUM_JSAPI => "JS/API",
        _ => "Coverage",
    }
}

fn suggested_tools(technique: &str) -> Vec<String> {
    match technique {
        golish_db::repo::coverage_truth::TECH_DNS
        | golish_db::repo::coverage_truth::TECH_ASN
        | golish_db::repo::coverage_truth::TECH_CT
        | golish_db::repo::coverage_truth::TECH_SUBDOMAIN
        | golish_db::repo::coverage_truth::TECH_OSINT => vec!["recon_map_assets".to_string()],
        golish_db::repo::coverage_truth::TECH_WHOIS => {
            vec!["recon_lookup_whois".to_string()]
        }
        golish_db::repo::coverage_truth::TECH_EAS_LIVENESS => {
            vec!["httpx".to_string(), "nmap -sn".to_string()]
        }
        golish_db::repo::coverage_truth::TECH_EAS_PORT => {
            vec!["naabu".to_string(), "nmap".to_string()]
        }
        golish_db::repo::coverage_truth::TECH_EAS_SERVICE_FP => {
            vec!["nmap -sV".to_string(), "whatweb".to_string()]
        }
        golish_db::repo::coverage_truth::TECH_ENUM_DIR => {
            vec!["route_probe_paths".to_string(), "ffuf".to_string()]
        }
        golish_db::repo::coverage_truth::TECH_ENUM_PARAM => {
            vec!["arjun".to_string(), "js_extract_apis".to_string()]
        }
        golish_db::repo::coverage_truth::TECH_ENUM_JSAPI => {
            vec![
                "browser_collect_js_api".to_string(),
                "js_extract_apis".to_string(),
            ]
        }
        _ => Vec::new(),
    }
}

fn discovered_phase(asset: &TargetCoverageRow, run_start: Option<DateTime<Utc>>) -> String {
    if run_start.is_some_and(|started_at| asset.created_at >= started_at)
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
            source: Some("asset_intel".to_string()),
            parent_id: None,
            created_at: Utc::now(),
        }
    }

    #[test]
    fn eas_url_asset_only_requires_liveness() {
        let asset = target("https://app.example.com/login", "url");
        let found = BTreeSet::from([(
            asset.value.clone(),
            golish_db::repo::coverage_truth::TECH_EAS_LIVENESS.to_string(),
        )]);

        let cells = coverage_cells(
            golish_agent_kit::harness::StageKind::ExternalAttackSurface,
            &asset,
            &found,
            &BTreeMap::new(),
        );

        assert_eq!(cells[0].state, "found");
        assert_eq!(cells[1].state, "not_applicable");
        assert_eq!(cells[2].state, "not_applicable");
    }

    #[test]
    fn empty_outcome_is_checked_empty() {
        let asset = target("app.example.com", "domain");
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
    fn error_outcome_is_distinct_from_blocked() {
        let asset = target("app.example.com", "domain");
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
    fn target_intel_org_row_exposes_six_passive_dimensions() {
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
        assert!(cells.iter().all(|cell| cell.state == "pending"));
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
