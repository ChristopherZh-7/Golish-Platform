//! Complete, per-WaveUnit Candidate reasoning manifest.

use std::collections::{BTreeMap, BTreeSet};

use chrono::{DateTime, Utc};
use sha2::{Digest, Sha256};
use sqlx::{PgConnection, Postgres, Transaction};
use uuid::Uuid;

use super::attack_candidate_seeds::{self, AttackCandidateSeedRow, NewAttackCandidateSeed};
use super::attack_waves;

pub const MAX_ATTACK_MANIFEST_ITEMS: usize = 100;
pub const MAX_ATTACK_WORK_ITEM_KEY_BYTES: usize = 256;
pub const MAX_ATTACK_TECHNIQUE_BYTES: usize = 128;
pub const MAX_ATTACK_OBSERVATION_BYTES: usize = 64 * 1024;
pub const MAX_ATTACK_OBSERVATION_EVIDENCE_IDS: usize = 64;
const MAX_TYPED_EVIDENCE_BATCH_BYTES: usize = 256 * 1024;
const FORMULAIC_TECHNIQUES: &[&str] = &[
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
];
const HOST_FORMULAIC_TECHNIQUES: &[&str] = &["WSTG-CONF-05", "WSTG-CRYP-03", "GOLISH-NDAY"];
const SURFACE_ANALYSIS_TECHNIQUE: &str = "GOLISH-SURFACE-ANALYSIS";
const NUCLEI_OBSERVATION_KIND: &str = "vuln.nuclei_observation";
const NUCLEI_BATCH_SCHEMA: &str = "nuclei_observation_batch_v1";
const NUCLEI_MATCH_SCHEMA: &str = "nuclei_match_v1";
const NUCLEI_SOURCES: &[&str] = &["vuln_nuclei_general", "vuln_nuclei_fingerprint_targeted"];
const ANONYMOUS_ACCESS_SOURCE: &str = "vuln_probe_anonymous_access";
const ANONYMOUS_ACCESS_TECHNIQUE: &str = "WSTG-ATHN-04";
const ANONYMOUS_ACCESS_KIND: &str = "vuln.anonymous_access_observation";
const ANONYMOUS_ACCESS_BATCH_SCHEMA: &str = "anonymous_access_batch_v1";
const ANONYMOUS_ACCESS_OBSERVATION_SCHEMA: &str = "anonymous_access_v1";
const DIRECTORY_ENTRY_OBSERVATION_SCHEMA: &str = "directory_entry_observation_v1";
type CandidateSeedProjection = (
    String,
    serde_json::Value,
    String,
    Option<Uuid>,
    Option<String>,
    String,
    Vec<String>,
    bool,
);
type InitialEvidenceRoleAuthority = Option<(BTreeSet<i64>, BTreeSet<i64>)>;
type FrozenEntryEvidenceAuthoritySets = (Vec<i64>, InitialEvidenceRoleAuthority);

#[derive(Debug, Clone)]
struct ExactEnumerationLineage {
    operation_id: Uuid,
    scope_snapshot_id: Uuid,
    organization_id: Uuid,
    handoff_id: Uuid,
    stage_execution_id: Uuid,
    source_stage_run_unit_id: Uuid,
    project_path: String,
    started_at: DateTime<Utc>,
    gate_passed_at: DateTime<Utc>,
}

#[derive(Debug, Clone, sqlx::FromRow)]
struct EnumerationSupportEvidenceRow {
    operation_id: Uuid,
    scope_snapshot_id: Uuid,
    organization_id: Uuid,
    handoff_id: Uuid,
    stage_execution_id: Uuid,
    source_stage_run_unit_id: Uuid,
    evidence_id: i64,
    target_id: Uuid,
    project_path: String,
    evidence_asset: String,
    evidence_technique: String,
    evidence_outcome: String,
    tool_name: String,
    evidence_kind: String,
    created_at: DateTime<Utc>,
}

#[derive(Debug, Clone, sqlx::FromRow)]
struct EnumerationDirectoryEntryRow {
    source_evidence_id: i64,
    directory_entry_id: Uuid,
    target_id: Uuid,
    project_path: String,
    url: String,
    status_code: Option<i32>,
    content_length: Option<i32>,
    content_type: Option<String>,
    source_tool: String,
    created_at: DateTime<Utc>,
}

fn merge_exact_enumeration_support(
    lineage: &ExactEnumerationLineage,
    evidence: &[EnumerationSupportEvidenceRow],
    directory_entries: &[EnumerationDirectoryEntryRow],
    observations: &mut Vec<SeedAttackObservation>,
) -> anyhow::Result<BTreeMap<String, Vec<i64>>> {
    let mut support_by_work_item = BTreeMap::<String, Vec<i64>>::new();
    let mut admitted_evidence = BTreeMap::<i64, &EnumerationSupportEvidenceRow>::new();
    for row in evidence {
        anyhow::ensure!(
            row.operation_id == lineage.operation_id
                && row.scope_snapshot_id == lineage.scope_snapshot_id
                && row.organization_id == lineage.organization_id
                && row.handoff_id == lineage.handoff_id
                && row.stage_execution_id == lineage.stage_execution_id
                && row.source_stage_run_unit_id == lineage.source_stage_run_unit_id
                && row.project_path == lineage.project_path
                && row.evidence_id > 0
                && row.created_at >= lineage.started_at
                && row.created_at <= lineage.gate_passed_at
                && !row.tool_name.trim().is_empty()
                && !row.evidence_kind.trim().is_empty()
                && matches!(
                    row.evidence_outcome.as_str(),
                    "found" | "empty" | "blocked" | "not_applicable"
                ),
            "Enumeration support escaped its exact final-sealed predecessor lineage"
        );
        let Some(evidence_origin) =
            golish_pentest_domain::canonical_web_origin(&row.evidence_asset)
        else {
            continue;
        };
        let matching_surfaces = observations
            .iter_mut()
            .filter(|observation| {
                observation.observation_kind == "surface_analysis_v1"
                    && observation.target_live_id == Some(row.target_id)
                    && golish_pentest_domain::canonical_web_origin(
                        &observation.target_value_at_time,
                    )
                    .is_some_and(|origin| origin.key == evidence_origin.key)
            })
            .collect::<Vec<_>>();
        anyhow::ensure!(
            matching_surfaces.len() <= 1,
            "Enumeration support target resolves to ambiguous Candidate surfaces"
        );
        let Some(surface) = matching_surfaces.into_iter().next() else {
            continue;
        };
        surface.evidence_ids.push(row.evidence_id);
        surface.evidence_ids.sort_unstable();
        surface.evidence_ids.dedup();
        anyhow::ensure!(
            surface.evidence_ids.len() <= MAX_ATTACK_OBSERVATION_EVIDENCE_IDS,
            "Enumeration support exceeds the Candidate evidence bound"
        );
        support_by_work_item
            .entry(surface.work_item_key.clone())
            .or_default()
            .push(row.evidence_id);
        anyhow::ensure!(
            admitted_evidence.insert(row.evidence_id, row).is_none(),
            "Enumeration final handoff contains duplicate evidence identity"
        );
    }

    let mut directory_items = BTreeMap::<String, SeedAttackObservation>::new();
    for row in directory_entries {
        let Some(source) = admitted_evidence.get(&row.source_evidence_id).copied() else {
            continue;
        };
        if source.evidence_technique != "GOLISH-ENUM-DIR"
            || source.evidence_outcome != "found"
            || source.tool_name != "route_probe_paths"
            || source.evidence_kind != "dir_entry"
            || row.target_id != source.target_id
            || row.project_path != lineage.project_path
            || row.source_tool != "route_probe"
            || row.created_at < lineage.started_at
            || row.created_at > source.created_at
            || !row
                .status_code
                .is_some_and(|status| (200..300).contains(&status))
        {
            continue;
        }
        let Some(directory_origin) = golish_pentest_domain::canonical_web_origin(&row.url) else {
            continue;
        };
        let Some(evidence_origin) =
            golish_pentest_domain::canonical_web_origin(&source.evidence_asset)
        else {
            continue;
        };
        if directory_origin.key != evidence_origin.key || http_url_path_is_root(&row.url) {
            continue;
        }
        let Some(content_length) = row.content_length.filter(|value| *value >= 0) else {
            continue;
        };
        let content_type = row.content_type.clone().unwrap_or_default();
        let row_projection = serde_json::json!({
            "content_length": content_length,
            "content_type": content_type.clone(),
            "id": row.directory_entry_id,
            "status_code": row.status_code,
            "target_id": row.target_id,
            "tool": row.source_tool,
            "url": row.url,
        });
        let directory_entry_row_sha256 =
            sha256_prefixed(serde_json::to_vec(&row_projection)?.as_slice());
        let observation = serde_json::json!({
            "schema": DIRECTORY_ENTRY_OBSERVATION_SCHEMA,
            "target_id": row.target_id,
            "directory_entry_id": row.directory_entry_id,
            "directory_entry_row_sha256": directory_entry_row_sha256,
            "url": row.url,
            "status_code": row.status_code,
            "content_length": content_length,
            "method": "GET",
            "content_type": content_type,
            "source_tool": row.source_tool,
            "source_evidence_id": row.source_evidence_id,
            "network_attempted": true,
            "authority_current_after": true,
        });
        let observation_hash = sha256_prefixed(serde_json::to_vec(&observation)?.as_slice());
        let work_item_key = format!("directory_entry:{directory_entry_row_sha256}");
        let (target_type_at_time, target_value_at_time, target_identity_hash) =
            frozen_target_snapshot(&directory_origin.key);
        let item = SeedAttackObservation {
            work_item_key: work_item_key.clone(),
            target_live_id: Some(row.target_id),
            target_type_at_time,
            target_value_at_time,
            target_identity_hash,
            technique: "WSTG-INFO".to_string(),
            observation,
            observation_hash,
            source_fact_delta_id: None,
            delta_kind: None,
            observation_kind: DIRECTORY_ENTRY_OBSERVATION_SCHEMA.to_string(),
            allowed_techniques: vec!["WSTG-INFO".to_string()],
            enrichment_required: false,
            evidence_ids: vec![row.source_evidence_id],
        };
        if let Some(existing) = directory_items.get(&work_item_key) {
            anyhow::ensure!(
                existing.observation_hash == item.observation_hash
                    && existing.evidence_ids == item.evidence_ids,
                "directory entry support identity is ambiguous"
            );
        } else {
            directory_items.insert(work_item_key.clone(), item);
        }
        support_by_work_item.insert(work_item_key, vec![row.source_evidence_id]);
    }
    observations.extend(directory_items.into_values());
    observations.sort_by(|left, right| left.work_item_key.cmp(&right.work_item_key));
    for evidence_ids in support_by_work_item.values_mut() {
        evidence_ids.sort_unstable();
        evidence_ids.dedup();
    }
    anyhow::ensure!(
        observations.len() <= MAX_ATTACK_MANIFEST_ITEMS,
        "typed Candidate manifest exceeds the frozen Wave policy"
    );
    Ok(support_by_work_item)
}

fn http_url_path_is_root(value: &str) -> bool {
    let Some((_, remainder)) = value.split_once("://") else {
        return true;
    };
    let path_start = remainder.find('/');
    let Some(path_start) = path_start else {
        return true;
    };
    let path = &remainder[path_start..];
    let path = path.split(['?', '#']).next().unwrap_or_default().trim();
    path.is_empty() || path == "/"
}

fn require_exact_predecessor<T>(rows: &[T]) -> anyhow::Result<&T> {
    match rows {
        [row] => Ok(row),
        [] => anyhow::bail!("exact predecessor final-sealed lineage is unavailable"),
        _ => anyhow::bail!("exact predecessor final-sealed lineage is ambiguous"),
    }
}

fn supported_techniques_for_target_type(target_type: &str) -> Vec<String> {
    let techniques = match target_type.trim().to_ascii_lowercase().as_str() {
        "ip" => HOST_FORMULAIC_TECHNIQUES,
        "url" | "domain" | "wildcard" | "other" => FORMULAIC_TECHNIQUES,
        _ => &[],
    };
    techniques
        .iter()
        .map(|technique| (*technique).to_string())
        .collect()
}

fn outcome_observation_kind(outcome: &FormulaicOutcomeRow) -> &'static str {
    if outcome.source.as_deref() == Some(ANONYMOUS_ACCESS_SOURCE) {
        ANONYMOUS_ACCESS_OBSERVATION_SCHEMA
    } else {
        NUCLEI_MATCH_SCHEMA
    }
}

#[derive(Debug, sqlx::FromRow)]
struct FormulaicHandoffAuthority {
    stage_execution_id: Uuid,
    source_stage_run_unit_id: Uuid,
    deliverable_submission_id: Uuid,
    scope_snapshot_id: Uuid,
    source_generation: i32,
    evidence_ids: Vec<i64>,
    coverage_watermark: serde_json::Value,
    gate_passed_at: DateTime<Utc>,
    enumeration_handoff_id: Uuid,
    enumeration_stage_execution_id: Uuid,
    enumeration_source_stage_run_unit_id: Uuid,
    enumeration_evidence_ids: Vec<i64>,
    enumeration_started_at: DateTime<Utc>,
    enumeration_gate_passed_at: DateTime<Utc>,
    project_path: String,
}

#[derive(Debug, Clone, sqlx::FromRow)]
struct FormulaicOutcomeRow {
    asset: String,
    technique: String,
    outcome: String,
    source: Option<String>,
    query: Option<String>,
    result_count: Option<i32>,
    confidence: Option<f32>,
    evidence_ids: Vec<i64>,
    collected_at: DateTime<Utc>,
}

#[derive(Debug, Clone, sqlx::FromRow)]
struct FormulaicEvidenceRow {
    id: i64,
    evidence_target_id: Option<Uuid>,
    target_live_id: Option<Uuid>,
    tool_name: String,
    detail: serde_json::Value,
    evidence_technique: Option<String>,
    evidence_asset: Option<String>,
    evidence_outcome: Option<String>,
    created_at: DateTime<Utc>,
}

fn sha256_prefixed(bytes: &[u8]) -> String {
    let hex = Sha256::digest(bytes)
        .iter()
        .map(|byte| format!("{byte:02x}"))
        .collect::<String>();
    format!("sha256:{hex}")
}

fn frozen_target_snapshot(asset: &str) -> (String, String, String) {
    let value = asset.trim().to_string();
    let target_type = if value.starts_with("http://") || value.starts_with("https://") {
        "url"
    } else if value.parse::<std::net::IpAddr>().is_ok() {
        "ip"
    } else if value.contains('/') {
        "cidr"
    } else if value.contains('.') {
        "domain"
    } else {
        "other"
    }
    .to_string();
    let identity_hash = sha256_prefixed(format!("{target_type}\u{0}{value}").as_bytes());
    (target_type, value, identity_hash)
}

fn json_uuid(value: Option<&serde_json::Value>) -> Option<Uuid> {
    value
        .and_then(serde_json::Value::as_str)
        .and_then(|value| Uuid::parse_str(value.trim()).ok())
}

fn read_typed_evidence_batch(
    organization_id: Uuid,
    outcome: &FormulaicOutcomeRow,
    evidence: &FormulaicEvidenceRow,
    accepted_sources: &[&str],
    expected_kind: &str,
    label: &str,
) -> anyhow::Result<serde_json::Value> {
    let source = outcome.source.as_deref().unwrap_or_default();
    anyhow::ensure!(
        accepted_sources.contains(&source)
            && evidence.tool_name == source
            && evidence.evidence_technique.as_deref() == Some(outcome.technique.as_str())
            && evidence.evidence_asset.as_deref() == Some(outcome.asset.as_str())
            && evidence.evidence_outcome.as_deref() == Some(outcome.outcome.as_str())
            && evidence.created_at <= outcome.collected_at,
        "formulaic evidence metadata does not match its canonical outcome"
    );
    anyhow::ensure!(
        evidence
            .detail
            .get("kind")
            .and_then(serde_json::Value::as_str)
            == Some(expected_kind)
            && evidence
                .detail
                .get("organization_id")
                .and_then(serde_json::Value::as_str)
                .is_some_and(|value| value == organization_id.to_string()),
        "formulaic evidence is not a target-bound {label} observation"
    );
    let raw_output = evidence
        .detail
        .get("raw_output")
        .and_then(serde_json::Value::as_str)
        .ok_or_else(|| anyhow::anyhow!("{label} evidence is missing typed raw_output"))?;
    anyhow::ensure!(
        raw_output.len() <= MAX_TYPED_EVIDENCE_BATCH_BYTES,
        "{label} evidence raw_output exceeds the Candidate entry bound"
    );
    serde_json::from_str(raw_output)
        .map_err(|error| anyhow::anyhow!("{label} evidence raw_output is malformed: {error}"))
}

fn validate_and_read_nuclei_observations(
    organization_id: Uuid,
    outcome: &FormulaicOutcomeRow,
    evidence: &FormulaicEvidenceRow,
) -> anyhow::Result<Vec<serde_json::Value>> {
    let expected_source_mode = match outcome.source.as_deref() {
        Some("vuln_nuclei_general") if outcome.technique != "GOLISH-NDAY" => "general",
        Some("vuln_nuclei_fingerprint_targeted") if outcome.technique == "GOLISH-NDAY" => {
            "fingerprint_targeted"
        }
        _ => anyhow::bail!("Nuclei outcome source does not own the frozen technique"),
    };
    let batch = read_typed_evidence_batch(
        organization_id,
        outcome,
        evidence,
        NUCLEI_SOURCES,
        NUCLEI_OBSERVATION_KIND,
        "Nuclei",
    )?;
    anyhow::ensure!(
        batch.get("schema").and_then(serde_json::Value::as_str) == Some(NUCLEI_BATCH_SCHEMA)
            && batch.get("source_mode").and_then(serde_json::Value::as_str)
                == Some(expected_source_mode)
            && batch
                .get("exact_origin")
                .and_then(serde_json::Value::as_str)
                == Some(outcome.asset.as_str())
            && batch.get("technique").and_then(serde_json::Value::as_str)
                == Some(outcome.technique.as_str()),
        "typed Nuclei batch identity does not match its canonical outcome"
    );
    if let Some(evidence_target_id) = evidence.evidence_target_id {
        anyhow::ensure!(
            json_uuid(batch.get("target_id")) == Some(evidence_target_id),
            "typed Nuclei batch target does not match evidence ownership"
        );
    }
    let observations = batch
        .get("observations")
        .and_then(serde_json::Value::as_array)
        .ok_or_else(|| anyhow::anyhow!("typed Nuclei batch is missing observations"))?;
    let matches_in_evidence = batch
        .get("matches_in_evidence")
        .and_then(serde_json::Value::as_u64)
        .and_then(|value| usize::try_from(value).ok())
        .ok_or_else(|| anyhow::anyhow!("typed Nuclei batch is missing matches_in_evidence"))?;
    let matches_total = batch
        .get("matches_total")
        .and_then(serde_json::Value::as_u64)
        .and_then(|value| usize::try_from(value).ok())
        .ok_or_else(|| anyhow::anyhow!("typed Nuclei batch is missing matches_total"))?;
    let matches_omitted = batch
        .get("matches_omitted")
        .and_then(serde_json::Value::as_u64)
        .and_then(|value| usize::try_from(value).ok())
        .ok_or_else(|| anyhow::anyhow!("typed Nuclei batch is missing matches_omitted"))?;
    anyhow::ensure!(
        matches_in_evidence == observations.len()
            && matches_total >= matches_in_evidence
            && matches_omitted == matches_total - matches_in_evidence
            && outcome.result_count == i32::try_from(matches_total).ok(),
        "typed Nuclei batch match counts are inconsistent"
    );
    anyhow::ensure!(
        (outcome.outcome == "found" && !observations.is_empty())
            || (outcome.outcome != "found" && observations.is_empty()),
        "formulaic outcome cannot be inferred from the typed Nuclei observation batch"
    );
    let canonical_asset = golish_pentest_domain::canonical_web_origin(&outcome.asset)
        .ok_or_else(|| anyhow::anyhow!("formulaic outcome asset is not an exact Web Origin"))?
        .key;
    let mut typed = Vec::with_capacity(observations.len());
    for observation in observations {
        anyhow::ensure!(
            observation.is_object()
                && observation
                    .get("schema")
                    .and_then(serde_json::Value::as_str)
                    == Some(NUCLEI_MATCH_SCHEMA)
                && observation
                    .get("technique")
                    .and_then(serde_json::Value::as_str)
                    == Some(outcome.technique.as_str())
                && observation
                    .get("source_mode")
                    .and_then(serde_json::Value::as_str)
                    == Some(expected_source_mode)
                && observation
                    .get("template_id")
                    .and_then(serde_json::Value::as_str)
                    .is_some_and(|value| !value.trim().is_empty()),
            "typed Nuclei match has an invalid schema or frozen technique"
        );
        let matched_url = observation
            .get("matched_url")
            .and_then(serde_json::Value::as_str)
            .ok_or_else(|| anyhow::anyhow!("typed Nuclei match is missing matched_url"))?;
        let matched_origin = golish_pentest_domain::canonical_web_origin(matched_url)
            .ok_or_else(|| anyhow::anyhow!("typed Nuclei match URL is not canonicalizable"))?;
        anyhow::ensure!(
            matched_origin.key == canonical_asset,
            "typed Nuclei match escaped the frozen exact origin"
        );
        if let Some(evidence_target_id) = evidence.evidence_target_id {
            anyhow::ensure!(
                json_uuid(observation.get("target_id")) == Some(evidence_target_id),
                "typed Nuclei match target does not match evidence ownership"
            );
        }
        anyhow::ensure!(
            serde_json::to_vec(observation)?.len() <= MAX_ATTACK_OBSERVATION_BYTES,
            "typed Nuclei match exceeds the Candidate observation bound"
        );
        typed.push(observation.clone());
    }
    Ok(typed)
}

fn bounded_sha256(value: Option<&serde_json::Value>) -> bool {
    value
        .and_then(serde_json::Value::as_str)
        .is_some_and(|value| {
            let digest = value.strip_prefix("sha256:").unwrap_or(value);
            digest.len() == 64
                && digest
                    .bytes()
                    .all(|byte| byte.is_ascii_digit() || matches!(byte, b'a'..=b'f'))
        })
}

fn valid_anonymous_query_binding(value: &serde_json::Value) -> bool {
    let Some(binding) = value.as_object() else {
        return false;
    };
    if binding.len() != 2 {
        return false;
    }
    let Some(name) = binding.get("name").and_then(serde_json::Value::as_str) else {
        return false;
    };
    let Some(value) = binding.get("value").and_then(serde_json::Value::as_str) else {
        return false;
    };
    let normalized_name = name.to_ascii_lowercase().replace('-', "_");
    let sensitive_name = [
        "token",
        "secret",
        "password",
        "passwd",
        "authorization",
        "cookie",
        "session",
        "api_key",
        "apikey",
        "access_key",
        "private_key",
    ]
    .iter()
    .any(|token| normalized_name.contains(token));
    let lower_value = value.to_ascii_lowercase();
    let secret_like_value = ["sk_", "ghp_", "github_pat_", "bearer", "eyj", "api_key_"]
        .iter()
        .any(|prefix| lower_value.starts_with(prefix));
    !name.is_empty()
        && name.len() <= 128
        && name
            .bytes()
            .all(|byte| byte.is_ascii_alphanumeric() || matches!(byte, b'-' | b'_' | b'.'))
        && !sensitive_name
        && !value.is_empty()
        && value.len() <= 64
        && value.is_ascii()
        && value.trim() == value
        && !value.contains("..")
        && value.parse::<std::net::IpAddr>().is_err()
        && !secret_like_value
        && (Uuid::parse_str(value).is_ok()
            || (value.len() <= 20 && value.bytes().all(|byte| byte.is_ascii_digit()))
            || (value.len() <= 32
                && value
                    .bytes()
                    .all(|byte| byte.is_ascii_alphanumeric() || matches!(byte, b'-' | b'_'))))
}

fn validate_and_read_anonymous_access_observations(
    organization_id: Uuid,
    outcome: &FormulaicOutcomeRow,
    evidence: &FormulaicEvidenceRow,
) -> anyhow::Result<Vec<serde_json::Value>> {
    anyhow::ensure!(
        outcome.technique == ANONYMOUS_ACCESS_TECHNIQUE,
        "anonymous-access evidence has the wrong frozen technique"
    );
    let batch = read_typed_evidence_batch(
        organization_id,
        outcome,
        evidence,
        &[ANONYMOUS_ACCESS_SOURCE],
        ANONYMOUS_ACCESS_KIND,
        "anonymous-access",
    )?;
    anyhow::ensure!(
        batch.get("schema").and_then(serde_json::Value::as_str)
            == Some(ANONYMOUS_ACCESS_BATCH_SCHEMA)
            && batch
                .get("exact_origin")
                .and_then(serde_json::Value::as_str)
                == Some(outcome.asset.as_str())
            && batch.get("technique").and_then(serde_json::Value::as_str)
                == Some(ANONYMOUS_ACCESS_TECHNIQUE)
            && batch
                .get("aggregate_outcome")
                .and_then(serde_json::Value::as_str)
                == Some(outcome.outcome.as_str()),
        "typed anonymous-access batch identity does not match its canonical outcome"
    );
    if let Some(evidence_target_id) = evidence.evidence_target_id {
        anyhow::ensure!(
            json_uuid(batch.get("target_id")) == Some(evidence_target_id),
            "typed anonymous-access batch target does not match evidence ownership"
        );
    }
    let completion_state = batch
        .get("completion_state")
        .and_then(serde_json::Value::as_str)
        .filter(|value| matches!(*value, "complete" | "partial"))
        .ok_or_else(|| anyhow::anyhow!("typed anonymous-access batch completion is invalid"))?;
    let reviewed_count = batch
        .get("reviewed_count")
        .and_then(serde_json::Value::as_u64)
        .and_then(|value| usize::try_from(value).ok())
        .ok_or_else(|| anyhow::anyhow!("typed anonymous-access batch is missing reviewed_count"))?;
    let selected_count = batch
        .get("selected_count")
        .and_then(serde_json::Value::as_u64)
        .and_then(|value| usize::try_from(value).ok())
        .ok_or_else(|| anyhow::anyhow!("typed anonymous-access batch is missing selected_count"))?;
    let observations = batch
        .get("observations")
        .and_then(serde_json::Value::as_array)
        .ok_or_else(|| anyhow::anyhow!("typed anonymous-access batch is missing observations"))?;
    anyhow::ensure!(
        reviewed_count >= selected_count
            && selected_count <= 16
            && observations.len() <= selected_count
            && bounded_sha256(batch.get("reviewed_set_sha256"))
            && batch
                .get("error_classes")
                .and_then(serde_json::Value::as_array)
                .is_some_and(|errors| {
                    errors.iter().all(|error| {
                        error
                            .as_str()
                            .is_some_and(|value| !value.is_empty() && value.len() <= 128)
                    })
                }),
        "typed anonymous-access batch bounds are invalid"
    );

    let mut positive = Vec::new();
    let mut verdicts = Vec::with_capacity(observations.len());
    for observation in observations {
        let verdict = observation
            .get("verdict")
            .and_then(serde_json::Value::as_str)
            .filter(|value| {
                matches!(
                    *value,
                    "controlled" | "public" | "suspicious" | "inconclusive" | "skipped"
                )
            })
            .ok_or_else(|| anyhow::anyhow!("typed anonymous-access verdict is invalid"))?;
        let method = observation
            .get("method")
            .and_then(serde_json::Value::as_str)
            .unwrap_or_default();
        let path = observation
            .get("path")
            .and_then(serde_json::Value::as_str)
            .unwrap_or_default();
        let query_bindings = observation
            .get("query_bindings")
            .and_then(serde_json::Value::as_array)
            .ok_or_else(|| anyhow::anyhow!("typed anonymous-access query bindings are missing"))?;
        let query_names = query_bindings
            .iter()
            .filter_map(|binding| binding.get("name"))
            .filter_map(serde_json::Value::as_str)
            .collect::<BTreeSet<_>>();
        anyhow::ensure!(
            observation.is_object()
                && observation
                    .get("schema")
                    .and_then(serde_json::Value::as_str)
                    == Some(ANONYMOUS_ACCESS_OBSERVATION_SCHEMA)
                && json_uuid(observation.get("endpoint_id")).is_some()
                && bounded_sha256(observation.get("endpoint_row_sha256"))
                && bounded_sha256(observation.get("request_plan_sha256"))
                && matches!(method, "GET" | "HEAD")
                && path.starts_with('/')
                && path.len() <= 2_048
                && !path.contains('?')
                && !path.contains('#')
                && query_bindings.len() <= 16
                && query_names.len() == query_bindings.len()
                && query_bindings.iter().all(valid_anonymous_query_binding)
                && observation
                    .get("no_auth")
                    .and_then(serde_json::Value::as_bool)
                    == Some(true)
                && observation
                    .get("authority_current_after")
                    .and_then(serde_json::Value::as_bool)
                    .is_some()
                && observation
                    .get("redirect")
                    .is_some_and(serde_json::Value::is_object)
                && observation
                    .get("rationale")
                    .and_then(serde_json::Value::as_str)
                    .is_some_and(|value| !value.is_empty() && value.len() <= 512)
                && serde_json::to_vec(observation)?.len() <= MAX_ATTACK_OBSERVATION_BYTES,
            "typed anonymous-access observation is malformed or unbounded"
        );
        if verdict == "suspicious" {
            anyhow::ensure!(
                observation
                    .get("network_attempted")
                    .and_then(serde_json::Value::as_bool)
                    == Some(true)
                    && observation
                        .get("authority_current_after")
                        .and_then(serde_json::Value::as_bool)
                        == Some(true)
                    && observation
                        .get("status_code")
                        .and_then(serde_json::Value::as_u64)
                        .is_some_and(|status| (200..300).contains(&status))
                    && observation
                        .get("response")
                        .is_some_and(serde_json::Value::is_object),
                "suspicious anonymous-access observation lacks a current network response"
            );
            positive.push(observation.clone());
        }
        verdicts.push(verdict);
    }
    let suspicious_count = positive.len();
    anyhow::ensure!(
        outcome.result_count == i32::try_from(suspicious_count).ok(),
        "anonymous-access result_count does not match typed suspicious observations"
    );
    match outcome.outcome.as_str() {
        "found" => anyhow::ensure!(
            suspicious_count > 0,
            "anonymous-access found outcome has no suspicious observation"
        ),
        "empty" => anyhow::ensure!(
            suspicious_count == 0
                && completion_state == "complete"
                && observations.len() == selected_count
                && verdicts
                    .iter()
                    .all(|verdict| matches!(*verdict, "controlled" | "public")),
            "anonymous-access empty outcome is not a complete negative batch"
        ),
        "not_applicable" => anyhow::ensure!(
            suspicious_count == 0
                && completion_state == "complete"
                && selected_count == 0
                && observations.is_empty(),
            "anonymous-access not-applicable outcome still contains probes"
        ),
        _ => anyhow::bail!("anonymous-access evidence cannot close this formulaic outcome"),
    }
    Ok(positive)
}

fn validate_and_read_positive_observations(
    organization_id: Uuid,
    outcome: &FormulaicOutcomeRow,
    evidence: &FormulaicEvidenceRow,
) -> anyhow::Result<Vec<serde_json::Value>> {
    match outcome.source.as_deref() {
        Some(source) if NUCLEI_SOURCES.contains(&source) => {
            validate_and_read_nuclei_observations(organization_id, outcome, evidence)
        }
        Some(ANONYMOUS_ACCESS_SOURCE) => {
            validate_and_read_anonymous_access_observations(organization_id, outcome, evidence)
        }
        _ => anyhow::bail!("formulaic outcome source has no typed Candidate adapter"),
    }
}

#[derive(Debug)]
struct SurfaceObservationAccumulator {
    target_type_at_time: String,
    target_value_at_time: String,
    target_identity_hash: String,
    live_target_ids: BTreeSet<Uuid>,
    evidence_ids: BTreeSet<i64>,
    coverage: Vec<serde_json::Value>,
}

fn materialize_initial_candidate_observations(
    organization_id: Uuid,
    outcomes: &[FormulaicOutcomeRow],
    evidence_rows: &[FormulaicEvidenceRow],
) -> anyhow::Result<Vec<SeedAttackObservation>> {
    if outcomes.is_empty() {
        return Ok(Vec::new());
    }
    let evidence_by_id = evidence_rows
        .iter()
        .map(|row| (row.id, row))
        .collect::<BTreeMap<_, _>>();
    anyhow::ensure!(
        evidence_by_id.len() == evidence_rows.len(),
        "formulaic evidence contains duplicate audit rows"
    );
    let mut surfaces = BTreeMap::<String, SurfaceObservationAccumulator>::new();
    let mut scanner_leads = BTreeMap::<String, SeedAttackObservation>::new();
    for outcome in outcomes {
        let (target_type_at_time, target_value_at_time, target_identity_hash) =
            frozen_target_snapshot(&outcome.asset);
        let surface = surfaces
            .entry(target_identity_hash.clone())
            .or_insert_with(|| SurfaceObservationAccumulator {
                target_type_at_time: target_type_at_time.clone(),
                target_value_at_time: target_value_at_time.clone(),
                target_identity_hash: target_identity_hash.clone(),
                live_target_ids: BTreeSet::new(),
                evidence_ids: BTreeSet::new(),
                coverage: Vec::new(),
            });
        anyhow::ensure!(
            surface.target_type_at_time == target_type_at_time
                && surface.target_value_at_time == target_value_at_time,
            "formulaic target identity hash collision"
        );
        let mut outcome_evidence = outcome.evidence_ids.clone();
        outcome_evidence.sort_unstable();
        let original_count = outcome_evidence.len();
        outcome_evidence.dedup();
        anyhow::ensure!(
            !outcome_evidence.is_empty()
                && outcome_evidence.len() == original_count
                && outcome_evidence.iter().all(|id| *id > 0),
            "formulaic outcome evidence must be unique and positive"
        );
        for evidence_id in &outcome_evidence {
            let evidence = evidence_by_id
                .get(evidence_id)
                .ok_or_else(|| anyhow::anyhow!("formulaic outcome evidence row is missing"))?;
            if let Some(target_live_id) = evidence.target_live_id {
                surface.live_target_ids.insert(target_live_id);
            }
            surface.evidence_ids.insert(*evidence_id);
            for observation in
                validate_and_read_positive_observations(organization_id, outcome, evidence)?
            {
                let observation_hash =
                    sha256_prefixed(serde_json::to_vec(&observation)?.as_slice());
                let work_item_key = format!(
                    "scanner_observation:{}:{}",
                    target_identity_hash, observation_hash
                );
                if let Some(existing) = scanner_leads.get_mut(&work_item_key) {
                    anyhow::ensure!(
                        existing.technique == outcome.technique
                            && existing.observation == observation
                            && existing.target_identity_hash == target_identity_hash,
                        "typed scanner observation identity collision"
                    );
                    if !existing.evidence_ids.contains(evidence_id) {
                        existing.evidence_ids.push(*evidence_id);
                        existing.evidence_ids.sort_unstable();
                    }
                } else {
                    scanner_leads.insert(
                        work_item_key.clone(),
                        SeedAttackObservation {
                            work_item_key,
                            target_live_id: evidence.target_live_id,
                            target_type_at_time: target_type_at_time.clone(),
                            target_value_at_time: target_value_at_time.clone(),
                            target_identity_hash: target_identity_hash.clone(),
                            technique: outcome.technique.clone(),
                            observation,
                            observation_hash,
                            source_fact_delta_id: None,
                            delta_kind: None,
                            observation_kind: outcome_observation_kind(outcome).to_string(),
                            allowed_techniques: vec![outcome.technique.clone()],
                            enrichment_required: false,
                            evidence_ids: vec![*evidence_id],
                        },
                    );
                }
            }
        }
        surface.coverage.push(serde_json::json!({
            "collected_at": outcome.collected_at,
            "confidence": outcome.confidence,
            "evidence_ids": outcome_evidence,
            "outcome": outcome.outcome,
            "query": outcome.query,
            "result_count": outcome.result_count,
            "source": outcome.source,
            "technique": outcome.technique,
        }));
    }
    anyhow::ensure!(
        evidence_by_id.keys().all(|id| outcomes
            .iter()
            .any(|outcome| outcome.evidence_ids.contains(id))),
        "formulaic handoff contains evidence outside its canonical outcomes"
    );
    let mut observations = Vec::with_capacity(surfaces.len() + scanner_leads.len());
    for (_, mut surface) in surfaces {
        surface.coverage.sort_by(|left, right| {
            left.get("technique")
                .and_then(serde_json::Value::as_str)
                .cmp(&right.get("technique").and_then(serde_json::Value::as_str))
        });
        let target_live_id = (surface.live_target_ids.len() == 1)
            .then(|| surface.live_target_ids.iter().next().copied())
            .flatten();
        let evidence_ids = surface.evidence_ids.into_iter().collect::<Vec<_>>();
        let observation = serde_json::json!({
            "schema": "surface_analysis_v1",
            "target_id": target_live_id,
            "target_identity": {
                "type": surface.target_type_at_time,
                "value": surface.target_value_at_time,
                "sha256": surface.target_identity_hash,
            },
            "formulaic_coverage": surface.coverage,
            "upstream_query_required": true,
        });
        let observation_hash = sha256_prefixed(serde_json::to_vec(&observation)?.as_slice());
        observations.push(SeedAttackObservation {
            work_item_key: format!("surface_analysis:{}", surface.target_identity_hash),
            target_live_id,
            target_type_at_time: surface.target_type_at_time.clone(),
            target_value_at_time: surface.target_value_at_time,
            target_identity_hash: surface.target_identity_hash,
            technique: SURFACE_ANALYSIS_TECHNIQUE.to_string(),
            observation,
            observation_hash,
            source_fact_delta_id: None,
            delta_kind: None,
            observation_kind: "surface_analysis_v1".to_string(),
            allowed_techniques: supported_techniques_for_target_type(&surface.target_type_at_time),
            enrichment_required: false,
            evidence_ids,
        });
    }
    observations.extend(scanner_leads.into_values());
    observations.sort_by(|left, right| left.work_item_key.cmp(&right.work_item_key));
    anyhow::ensure!(
        observations.len() <= MAX_ATTACK_MANIFEST_ITEMS,
        "typed Candidate manifest exceeds the frozen Wave policy"
    );
    Ok(observations)
}

fn watermark_usize(watermark: &serde_json::Value, key: &str) -> anyhow::Result<usize> {
    watermark
        .get(key)
        .and_then(serde_json::Value::as_u64)
        .and_then(|value| usize::try_from(value).ok())
        .ok_or_else(|| anyhow::anyhow!("vuln_triage handoff watermark missing {key}"))
}

fn watermark_strings(watermark: &serde_json::Value, key: &str) -> anyhow::Result<BTreeSet<String>> {
    watermark
        .get(key)
        .and_then(serde_json::Value::as_array)
        .ok_or_else(|| anyhow::anyhow!("vuln_triage handoff watermark missing {key}"))?
        .iter()
        .map(|value| {
            value
                .as_str()
                .map(ToString::to_string)
                .ok_or_else(|| anyhow::anyhow!("vuln_triage handoff watermark has invalid {key}"))
        })
        .collect()
}

fn attest_formulaic_outcomes(
    organization_id: Uuid,
    authority: &FormulaicHandoffAuthority,
    outcomes: &[FormulaicOutcomeRow],
) -> anyhow::Result<()> {
    let watermark = &authority.coverage_watermark;
    anyhow::ensure!(
        watermark.get("kind").and_then(serde_json::Value::as_str)
            == Some("information_coverage_v1")
            && watermark.get("stage").and_then(serde_json::Value::as_str) == Some("vuln_triage")
            && watermark
                .get("organization_id")
                .and_then(serde_json::Value::as_str)
                .is_some_and(|value| value == organization_id.to_string()),
        "vuln_triage handoff watermark identity mismatch"
    );
    for (flag, total, included) in [
        (
            "canonical_ref_truncated",
            "canonical_ref_total",
            "canonical_ref_included",
        ),
        (
            "evidence_id_truncated",
            "evidence_id_total",
            "evidence_id_included",
        ),
    ] {
        anyhow::ensure!(
            watermark.get(flag).and_then(serde_json::Value::as_bool) == Some(false)
                && watermark_usize(watermark, total)? == watermark_usize(watermark, included)?,
            "vuln_triage handoff is truncated and cannot seed an exact Candidate manifest"
        );
    }
    let terminal_cells = watermark_usize(watermark, "terminal_cells")?;
    anyhow::ensure!(
        terminal_cells == outcomes.len()
            && terminal_cells > 0
            && terminal_cells <= MAX_ATTACK_MANIFEST_ITEMS
            && watermark_usize(watermark, "canonical_ref_total")? == terminal_cells,
        "vuln_triage terminal-cell attestation mismatch"
    );
    let actual_assets = outcomes
        .iter()
        .map(|row| row.asset.clone())
        .collect::<BTreeSet<_>>();
    let actual_techniques = outcomes
        .iter()
        .map(|row| row.technique.clone())
        .collect::<BTreeSet<_>>();
    anyhow::ensure!(
        watermark_strings(watermark, "assets")? == actual_assets
            && watermark_strings(watermark, "techniques")? == actual_techniques
            && actual_techniques
                == FORMULAIC_TECHNIQUES
                    .iter()
                    .map(|technique| (*technique).to_string())
                    .collect::<BTreeSet<_>>(),
        "vuln_triage asset/technique attestation mismatch"
    );
    let handoff_evidence = authority
        .evidence_ids
        .iter()
        .copied()
        .collect::<BTreeSet<_>>();
    anyhow::ensure!(
        authority.scope_snapshot_id != Uuid::nil()
            && authority.gate_passed_at <= Utc::now()
            && outcomes.iter().all(|row| {
                matches!(
                    row.outcome.as_str(),
                    "found" | "empty" | "blocked" | "not_applicable"
                ) && !row.evidence_ids.is_empty()
                    && row.evidence_ids.len() <= MAX_ATTACK_OBSERVATION_EVIDENCE_IDS
                    && row
                        .evidence_ids
                        .iter()
                        .all(|id| *id > 0 && handoff_evidence.contains(id))
            }),
        "vuln_triage canonical outcomes are not grounded by the exact handoff"
    );
    Ok(())
}

#[derive(Debug, Clone, sqlx::FromRow, PartialEq)]
pub struct AttackCandidateWorkItemRow {
    pub id: Uuid,
    pub seed_id: Uuid,
    pub wave_unit_id: Uuid,
    pub operation_id: Uuid,
    pub scope_snapshot_id: Uuid,
    pub organization_id: Uuid,
    pub target_live_id: Option<Uuid>,
    pub target_type_at_time: String,
    pub target_value_at_time: String,
    pub target_identity_hash: String,
    pub work_item_key: String,
    pub decision_kind: Option<String>,
    pub candidate_id: Option<Uuid>,
    pub no_candidate_reason_code: Option<String>,
    pub no_candidate_detail: Option<String>,
    pub decided_at: Option<DateTime<Utc>>,
    pub row_version: i64,
    pub created_at: DateTime<Utc>,
    pub updated_at: DateTime<Utc>,
}

#[derive(Debug, Clone)]
pub struct SeedAttackObservation {
    pub work_item_key: String,
    pub target_live_id: Option<Uuid>,
    pub target_type_at_time: String,
    pub target_value_at_time: String,
    pub target_identity_hash: String,
    pub technique: String,
    pub observation: serde_json::Value,
    pub observation_hash: String,
    pub source_fact_delta_id: Option<Uuid>,
    pub delta_kind: Option<String>,
    pub observation_kind: String,
    pub allowed_techniques: Vec<String>,
    pub enrichment_required: bool,
    pub evidence_ids: Vec<i64>,
}

#[derive(Debug, Clone)]
pub struct SeedAttackWorkItems {
    pub operation_id: Uuid,
    pub scope_snapshot_id: Uuid,
    pub wave_run_id: Uuid,
    pub wave_unit_id: Uuid,
    pub organization_id: Uuid,
    pub observations: Vec<SeedAttackObservation>,
}

#[derive(Debug, Clone)]
pub struct SeededAttackWorkItem {
    pub seed: AttackCandidateSeedRow,
    pub work_item: AttackCandidateWorkItemRow,
}

#[derive(Debug, Clone)]
pub struct SeedAttackWorkItemsResult {
    pub items: Vec<SeededAttackWorkItem>,
}

#[derive(Debug, Clone, PartialEq)]
pub struct CandidateManifestItemRow {
    pub work_item: AttackCandidateWorkItemRow,
    pub technique: String,
    pub observation: serde_json::Value,
    pub observation_hash: String,
    pub source_fact_delta_id: Option<Uuid>,
    pub delta_kind: Option<String>,
    pub observation_kind: String,
    pub allowed_techniques: Vec<String>,
    pub enrichment_required: bool,
    pub evidence_ids: Vec<i64>,
}

#[derive(Debug, Clone, PartialEq)]
pub struct CandidateManifestRow {
    pub operation_id: Uuid,
    pub scope_snapshot_id: Uuid,
    pub wave_run_id: Uuid,
    pub wave_unit_id: Uuid,
    pub organization_id: Uuid,
    pub items: Vec<CandidateManifestItemRow>,
}

#[derive(Debug, sqlx::FromRow)]
struct FrozenEntryEvidenceAuthority {
    manifest_hash: Option<String>,
    manifest_count: Option<i32>,
    manifest_frozen_at: Option<DateTime<Utc>>,
    entry_consolidation_id: Option<Uuid>,
}

#[derive(Debug, sqlx::FromRow)]
struct FrozenManifestEvidenceRoleRow {
    evidence_id: i64,
    role: String,
    target_live_id: Option<Uuid>,
    evidence_run_id: Option<Uuid>,
    producer_organization_id: Option<String>,
    evidence_target_id: Option<Uuid>,
    evidence_project_path: String,
    project_path_at_freeze: String,
}

pub fn canonical_manifest_hash(manifest: &CandidateManifestRow) -> String {
    let projection = manifest
        .items
        .iter()
        .map(|item| {
            serde_json::json!({
                "evidence_ids": item.evidence_ids,
                "observation": item.observation,
                "observation_hash": item.observation_hash,
                "target_identity_hash": item.work_item.target_identity_hash,
                "technique": item.technique,
                "work_item_id": item.work_item.id,
                "work_item_key": item.work_item.work_item_key,
            })
        })
        .collect::<Vec<_>>();
    format!(
        "sha256:{}",
        super::operation_scope_decisions::sha256_json(&serde_json::Value::Array(projection))
    )
}

const COLUMNS: &str = "id,seed_id,wave_unit_id,operation_id,scope_snapshot_id,\
    organization_id,target_live_id,target_type_at_time,target_value_at_time,\
    target_identity_hash,work_item_key,decision_kind,candidate_id,no_candidate_reason_code,\
    no_candidate_detail,decided_at,row_version,created_at,updated_at";

fn invalid(message: &str) -> crate::DbError {
    crate::DbError::Other(anyhow::anyhow!(message.to_string()))
}

/// Seed the complete reasoning manifest for one exact frozen organization unit.
/// Natural-key conflicts are read back and compared, so a replay is idempotent
/// but cannot silently rewrite a frozen observation or target identity.
pub async fn seed_wave_work_items(
    tx: &mut Transaction<'_, Postgres>,
    command: SeedAttackWorkItems,
) -> crate::Result<SeedAttackWorkItemsResult> {
    seed_wave_work_items_with_support(tx, command, &BTreeMap::new()).await
}

async fn seed_wave_work_items_with_support(
    tx: &mut Transaction<'_, Postgres>,
    command: SeedAttackWorkItems,
    support_by_work_item: &BTreeMap<String, Vec<i64>>,
) -> crate::Result<SeedAttackWorkItemsResult> {
    if command.observations.is_empty() || command.observations.len() > MAX_ATTACK_MANIFEST_ITEMS {
        return Err(invalid("attack work-item manifest cannot be empty"));
    }
    let submitted_keys = command
        .observations
        .iter()
        .map(|observation| observation.work_item_key.as_str())
        .collect::<BTreeSet<_>>();
    if support_by_work_item
        .keys()
        .any(|work_item_key| !submitted_keys.contains(work_item_key.as_str()))
    {
        return Err(invalid(
            "attack support evidence references an unknown work item",
        ));
    }
    let submitted_count = i32::try_from(command.observations.len())
        .map_err(|_| invalid("attack work-item manifest is too large"))?;
    let operation_contracts: Option<(String, String)> = sqlx::query_as(
        "SELECT runtime_memory_contract,attack_execution_contract
         FROM operation_state WHERE operation_id=$1 FOR UPDATE",
    )
    .bind(command.operation_id)
    .fetch_optional(&mut **tx)
    .await?;
    let (_, attack_contract) = operation_contracts
        .ok_or_else(|| crate::DbError::NotFound("operation_state".to_string()))?;
    if attack_contract == "legacy" {
        return Err(invalid(
            "legacy operation cannot seed Candidate V2 work-items",
        ));
    }
    let wave = attack_waves::lock_wave(
        tx,
        command.operation_id,
        command.scope_snapshot_id,
        command.wave_run_id,
    )
    .await?;
    if submitted_count > wave.max_candidates_total {
        return Err(invalid(
            "attack work-item manifest exceeds its frozen Wave policy",
        ));
    }
    let wave_unit = attack_waves::lock_wave_unit(
        tx,
        command.operation_id,
        command.scope_snapshot_id,
        command.wave_run_id,
        command.wave_unit_id,
        command.organization_id,
    )
    .await?;
    if wave_unit.review_closed || wave_unit.verification_closed || wave_unit.terminal_at.is_some() {
        return Err(invalid(
            "closed WaveUnit cannot accept new reasoning work-items",
        ));
    }

    let mut items = Vec::with_capacity(command.observations.len());
    for observation in command.observations {
        if observation.work_item_key.trim().is_empty()
            || observation.work_item_key.len() > MAX_ATTACK_WORK_ITEM_KEY_BYTES
            || observation.target_type_at_time.trim().is_empty()
            || observation.target_value_at_time.trim().is_empty()
            || observation.target_identity_hash.trim().is_empty()
            || observation.technique.trim().is_empty()
            || observation.technique.len() > MAX_ATTACK_TECHNIQUE_BYTES
            || observation.observation_hash.trim().is_empty()
            || observation.observation_kind.trim().is_empty()
            || observation.observation_kind.len() > MAX_ATTACK_TECHNIQUE_BYTES
            || observation.allowed_techniques.is_empty()
            || observation.allowed_techniques.len() > FORMULAIC_TECHNIQUES.len()
            || observation.allowed_techniques.iter().any(|technique| {
                technique.trim().is_empty() || technique.len() > MAX_ATTACK_TECHNIQUE_BYTES
            })
            || !observation.observation.is_object()
            || serde_json::to_vec(&observation.observation)?.len() > MAX_ATTACK_OBSERVATION_BYTES
            || observation.evidence_ids.is_empty()
            || observation.evidence_ids.len() > MAX_ATTACK_OBSERVATION_EVIDENCE_IDS
        {
            return Err(invalid("invalid or ungrounded attack observation"));
        }
        let unique_allowed = observation
            .allowed_techniques
            .iter()
            .collect::<BTreeSet<_>>();
        if unique_allowed.len() != observation.allowed_techniques.len()
            || observation.source_fact_delta_id.is_some() != observation.delta_kind.is_some()
            || observation.enrichment_required && observation.source_fact_delta_id.is_none()
        {
            return Err(invalid("invalid attack observation route metadata"));
        }
        let seed = attack_candidate_seeds::insert_or_get_exact(
            tx,
            command.operation_id,
            command.scope_snapshot_id,
            command.wave_unit_id,
            command.organization_id,
            &NewAttackCandidateSeed {
                id: Uuid::new_v4(),
                target_live_id: observation.target_live_id,
                target_type_at_time: observation.target_type_at_time.clone(),
                target_value_at_time: observation.target_value_at_time.clone(),
                target_identity_hash: observation.target_identity_hash.clone(),
                technique: observation.technique,
                observation: observation.observation,
                observation_hash: observation.observation_hash,
                source_fact_delta_id: observation.source_fact_delta_id,
                delta_kind: observation.delta_kind,
                observation_kind: observation.observation_kind,
                allowed_techniques: observation.allowed_techniques,
                enrichment_required: observation.enrichment_required,
            },
        )
        .await?;
        let work_item_id = Uuid::new_v4();
        let insert_sql = format!(
            "INSERT INTO attack_candidate_work_items(
                 id,seed_id,wave_unit_id,operation_id,scope_snapshot_id,organization_id,
                 target_live_id,target_type_at_time,target_value_at_time,target_identity_hash,
                 work_item_key)
             VALUES($1,$2,$3,$4,$5,$6,$7,$8,$9,$10,$11)
             ON CONFLICT(wave_unit_id,work_item_key) DO NOTHING RETURNING {COLUMNS}"
        );
        let inserted = sqlx::query_as::<_, AttackCandidateWorkItemRow>(&insert_sql)
            .bind(work_item_id)
            .bind(seed.id)
            .bind(command.wave_unit_id)
            .bind(command.operation_id)
            .bind(command.scope_snapshot_id)
            .bind(command.organization_id)
            .bind(seed.target_live_id)
            .bind(&seed.target_type_at_time)
            .bind(&seed.target_value_at_time)
            .bind(&seed.target_identity_hash)
            .bind(&observation.work_item_key)
            .fetch_optional(&mut **tx)
            .await?;
        let work_item = if let Some(row) = inserted {
            row
        } else {
            let select_sql = format!(
                "SELECT {COLUMNS} FROM attack_candidate_work_items
                 WHERE wave_unit_id=$1 AND work_item_key=$2 FOR UPDATE"
            );
            sqlx::query_as::<_, AttackCandidateWorkItemRow>(&select_sql)
                .bind(command.wave_unit_id)
                .bind(&observation.work_item_key)
                .fetch_one(&mut **tx)
                .await?
        };
        if work_item.seed_id != seed.id
            || work_item.operation_id != command.operation_id
            || work_item.scope_snapshot_id != command.scope_snapshot_id
            || work_item.organization_id != command.organization_id
            || work_item.target_identity_hash != seed.target_identity_hash
        {
            return Err(invalid("attack work-item idempotency identity mismatch"));
        }
        let mut expected_all_evidence = observation.evidence_ids;
        expected_all_evidence.sort_unstable();
        let expected_len = expected_all_evidence.len();
        expected_all_evidence.dedup();
        let mut expected_support_evidence = support_by_work_item
            .get(&observation.work_item_key)
            .cloned()
            .unwrap_or_default();
        expected_support_evidence.sort_unstable();
        let expected_support_len = expected_support_evidence.len();
        expected_support_evidence.dedup();
        if expected_all_evidence.len() != expected_len
            || expected_support_evidence.len() != expected_support_len
            || expected_all_evidence
                .iter()
                .any(|evidence_id| *evidence_id <= 0)
            || expected_support_evidence
                .iter()
                .any(|evidence_id| !expected_all_evidence.contains(evidence_id))
        {
            return Err(invalid(
                "attack observation evidence must be unique and positive",
            ));
        }
        let expected_observation_evidence = expected_all_evidence
            .iter()
            .filter(|evidence_id| !expected_support_evidence.contains(evidence_id))
            .copied()
            .collect::<Vec<_>>();
        let existing_seed_evidence: Vec<i64> = sqlx::query_scalar(
            "SELECT evidence_id FROM attack_candidate_seed_evidence
             WHERE seed_id=$1 AND role='observation' ORDER BY evidence_id",
        )
        .bind(seed.id)
        .fetch_all(&mut **tx)
        .await?;
        let existing_work_item_evidence: Vec<i64> = sqlx::query_scalar(
            "SELECT evidence_id FROM attack_candidate_work_item_evidence
             WHERE work_item_id=$1 AND role='observation' ORDER BY evidence_id",
        )
        .bind(work_item.id)
        .fetch_all(&mut **tx)
        .await?;
        let existing_seed_support: Vec<i64> = sqlx::query_scalar(
            "SELECT evidence_id FROM attack_candidate_seed_evidence
             WHERE seed_id=$1 AND role='support' ORDER BY evidence_id",
        )
        .bind(seed.id)
        .fetch_all(&mut **tx)
        .await?;
        let existing_work_item_support: Vec<i64> = sqlx::query_scalar(
            "SELECT evidence_id FROM attack_candidate_work_item_evidence
             WHERE work_item_id=$1 AND role='support' ORDER BY evidence_id",
        )
        .bind(work_item.id)
        .fetch_all(&mut **tx)
        .await?;
        let replay = !existing_seed_evidence.is_empty()
            || !existing_work_item_evidence.is_empty()
            || !existing_seed_support.is_empty()
            || !existing_work_item_support.is_empty();
        if replay
            && (existing_seed_evidence != expected_observation_evidence
                || existing_work_item_evidence != expected_observation_evidence
                || existing_seed_support != expected_support_evidence
                || existing_work_item_support != expected_support_evidence)
        {
            return Err(invalid("attack observation evidence replay drift"));
        }
        for evidence_id in expected_observation_evidence {
            sqlx::query(
                "INSERT INTO attack_candidate_seed_evidence(seed_id,evidence_id,role)
                 VALUES($1,$2,'observation') ON CONFLICT DO NOTHING",
            )
            .bind(seed.id)
            .bind(evidence_id)
            .execute(&mut **tx)
            .await?;
            sqlx::query(
                "INSERT INTO attack_candidate_work_item_evidence(work_item_id,evidence_id,role)
                 VALUES($1,$2,'observation') ON CONFLICT DO NOTHING",
            )
            .bind(work_item.id)
            .bind(evidence_id)
            .execute(&mut **tx)
            .await?;
        }
        for evidence_id in expected_support_evidence {
            sqlx::query(
                "INSERT INTO attack_candidate_seed_evidence(seed_id,evidence_id,role)
                 VALUES($1,$2,'support') ON CONFLICT DO NOTHING",
            )
            .bind(seed.id)
            .bind(evidence_id)
            .execute(&mut **tx)
            .await?;
            sqlx::query(
                "INSERT INTO attack_candidate_work_item_evidence(work_item_id,evidence_id,role)
                 VALUES($1,$2,'support') ON CONFLICT DO NOTHING",
            )
            .bind(work_item.id)
            .bind(evidence_id)
            .execute(&mut **tx)
            .await?;
        }
        items.push(SeededAttackWorkItem { seed, work_item });
    }
    let frozen_manifest = load_for_wave_unit_in_transaction(
        tx,
        command.operation_id,
        command.scope_snapshot_id,
        command.wave_run_id,
        command.wave_unit_id,
        command.organization_id,
    )
    .await?;
    if frozen_manifest.items.len() != submitted_count as usize {
        return Err(invalid(
            "attack manifest replay must provide the exact complete work-item set",
        ));
    }
    let manifest_hash = canonical_manifest_hash(&frozen_manifest);
    match (
        wave_unit.manifest_hash.as_deref(),
        wave_unit.manifest_count,
        wave_unit.manifest_frozen_at,
    ) {
        (Some(existing_hash), Some(existing_count), Some(_)) => {
            if existing_hash != manifest_hash || existing_count != submitted_count {
                return Err(invalid("attack manifest attestation replay drift"));
            }
        }
        (None, None, None) => {
            let frozen = sqlx::query(
                r#"UPDATE attack_wave_units
                      SET manifest_hash=$2,manifest_count=$3,manifest_frozen_at=NOW(),
                          row_version=row_version+1,updated_at=NOW()
                    WHERE id=$1 AND manifest_hash IS NULL
                      AND manifest_count IS NULL AND manifest_frozen_at IS NULL"#,
            )
            .bind(command.wave_unit_id)
            .bind(&manifest_hash)
            .bind(submitted_count)
            .execute(&mut **tx)
            .await?;
            if frozen.rows_affected() != 1 {
                return Err(invalid("attack manifest freeze CAS lost"));
            }
        }
        _ => return Err(invalid("attack manifest attestation is partially written")),
    }
    Ok(SeedAttackWorkItemsResult { items })
}

async fn load_for_wave_unit_in_transaction(
    tx: &mut Transaction<'_, Postgres>,
    operation_id: Uuid,
    scope_snapshot_id: Uuid,
    wave_run_id: Uuid,
    wave_unit_id: Uuid,
    organization_id: Uuid,
) -> crate::Result<CandidateManifestRow> {
    let sql = format!(
        "SELECT {COLUMNS} FROM attack_candidate_work_items
         WHERE wave_unit_id=$1 AND operation_id=$2 AND scope_snapshot_id=$3
           AND organization_id=$4 ORDER BY work_item_key,id FOR UPDATE"
    );
    let work_items = sqlx::query_as::<_, AttackCandidateWorkItemRow>(&sql)
        .bind(wave_unit_id)
        .bind(operation_id)
        .bind(scope_snapshot_id)
        .bind(organization_id)
        .fetch_all(&mut **tx)
        .await?;
    let mut items = Vec::with_capacity(work_items.len());
    for work_item in work_items {
        let (
            technique,
            observation,
            observation_hash,
            source_fact_delta_id,
            delta_kind,
            observation_kind,
            allowed_techniques,
            enrichment_required,
        ): CandidateSeedProjection = sqlx::query_as(
            "SELECT technique,observation,observation_hash,source_fact_delta_id,
                    delta_kind,observation_kind,allowed_techniques,enrichment_required
               FROM attack_candidate_seeds WHERE id=$1 FOR SHARE",
        )
        .bind(work_item.seed_id)
        .fetch_one(&mut **tx)
        .await?;
        let evidence_ids = sqlx::query_scalar(
            r#"SELECT evidence_id FROM (
                   SELECT evidence_id FROM attack_candidate_seed_evidence WHERE seed_id=$1
                   UNION
                   SELECT evidence_id FROM attack_candidate_work_item_evidence
                    WHERE work_item_id=$2 AND role IN ('observation','support')
               ) evidence ORDER BY evidence_id"#,
        )
        .bind(work_item.seed_id)
        .bind(work_item.id)
        .fetch_all(&mut **tx)
        .await?;
        items.push(CandidateManifestItemRow {
            work_item,
            technique,
            observation,
            observation_hash,
            source_fact_delta_id,
            delta_kind,
            observation_kind,
            allowed_techniques,
            enrichment_required,
            evidence_ids,
        });
    }
    Ok(CandidateManifestRow {
        operation_id,
        scope_snapshot_id,
        wave_run_id,
        wave_unit_id,
        organization_id,
        items,
    })
}

/// Materialize and freeze the exact Candidate reasoning manifest for one
/// `attack_candidate` runtime Unit. The complete canonical `vuln_triage`
/// outcome set is re-read under the predecessor handoff/watermark locks; a
/// truncated or drifted handoff fails closed. Live targets are optional hints —
/// the frozen target type/value/hash remains authoritative after deletion.
pub async fn seed_from_final_vuln_triage_handoff(
    pool: &sqlx::PgPool,
    operation_id: Uuid,
    stage_run_unit_id: Uuid,
    organization_id: Uuid,
) -> crate::Result<CandidateManifestRow> {
    let mut tx = pool.begin().await?;
    let unit: (Uuid, i32, Uuid) = sqlx::query_as(
        r#"SELECT scope_snapshot_id,generation,stage_execution_id FROM stage_run_units
            WHERE id=$1 AND operation_id=$2 AND organization_id=$3
              AND stage_kind='attack_candidate'
            FOR SHARE"#,
    )
    .bind(stage_run_unit_id)
    .bind(operation_id)
    .bind(organization_id)
    .fetch_optional(&mut *tx)
    .await?
    .ok_or_else(|| invalid("attack_candidate StageRunUnit identity mismatch"))?;
    let authorities = sqlx::query_as::<_, FormulaicHandoffAuthority>(
        r#"SELECT handoff.stage_execution_id,handoff.source_stage_run_unit_id,
                  handoff.deliverable_submission_id,handoff.scope_snapshot_id,
                  source_unit.generation AS source_generation,handoff.evidence_ids,
                  handoff.coverage_watermark,handoff.gate_passed_at,
                  enumeration_handoff.id AS enumeration_handoff_id,
                  enumeration_handoff.stage_execution_id AS enumeration_stage_execution_id,
                  enumeration_handoff.source_stage_run_unit_id
                      AS enumeration_source_stage_run_unit_id,
                  enumeration_handoff.evidence_ids AS enumeration_evidence_ids,
                  enumeration_run.started_at AS enumeration_started_at,
                  enumeration_handoff.gate_passed_at AS enumeration_gate_passed_at,
                  scope_snapshot.project_path_at_freeze AS project_path
             FROM stage_handoffs AS handoff
             JOIN stage_run_units AS source_unit
               ON source_unit.id=handoff.source_stage_run_unit_id
              AND source_unit.operation_id=handoff.operation_id
              AND source_unit.stage_execution_id=handoff.stage_execution_id
              AND source_unit.scope_snapshot_id=handoff.scope_snapshot_id
              AND source_unit.organization_id=handoff.organization_id
              AND source_unit.stage_kind=handoff.from_stage_kind
             JOIN stage_runs AS vuln_run
               ON vuln_run.id=handoff.stage_execution_id
              AND vuln_run.operation_id=handoff.operation_id
              AND vuln_run.stage_kind='vuln_triage'
              AND vuln_run.status='completed'
              AND vuln_run.completed_at IS NOT NULL
             JOIN stage_runs AS candidate_run
               ON candidate_run.id=$4
              AND candidate_run.operation_id=handoff.operation_id
              AND candidate_run.stage_kind='attack_candidate'
              AND candidate_run.status IN ('started','completed')
              AND candidate_run.started_at=vuln_run.completed_at
             JOIN stage_runs AS enumeration_run
               ON enumeration_run.operation_id=handoff.operation_id
              AND enumeration_run.stage_kind='enumeration'
              AND enumeration_run.status='completed'
              AND enumeration_run.completed_at=vuln_run.started_at
             JOIN stage_handoffs AS enumeration_handoff
               ON enumeration_handoff.operation_id=handoff.operation_id
              AND enumeration_handoff.organization_id=handoff.organization_id
              AND enumeration_handoff.scope_snapshot_id=handoff.scope_snapshot_id
              AND enumeration_handoff.from_stage_kind='enumeration'
              AND enumeration_handoff.stage_execution_id=enumeration_run.id
              AND enumeration_handoff.invalidated_at IS NULL
             JOIN stage_run_units AS enumeration_unit
               ON enumeration_unit.id=enumeration_handoff.source_stage_run_unit_id
              AND enumeration_unit.operation_id=enumeration_handoff.operation_id
              AND enumeration_unit.stage_execution_id=enumeration_handoff.stage_execution_id
              AND enumeration_unit.scope_snapshot_id=enumeration_handoff.scope_snapshot_id
              AND enumeration_unit.organization_id=enumeration_handoff.organization_id
              AND enumeration_unit.stage_kind=enumeration_handoff.from_stage_kind
              AND enumeration_unit.status='passed'
              AND enumeration_unit.terminal_at IS NOT NULL
             JOIN operation_org_scope_snapshots AS scope_snapshot
               ON scope_snapshot.id=handoff.scope_snapshot_id
              AND scope_snapshot.operation_id=handoff.operation_id
              AND scope_snapshot.sealed_at IS NOT NULL
            WHERE handoff.operation_id=$1 AND handoff.organization_id=$2
              AND handoff.scope_snapshot_id=$3
              AND handoff.from_stage_kind='vuln_triage'
              AND handoff.invalidated_at IS NULL
              AND source_unit.status='passed' AND source_unit.terminal_at IS NOT NULL
            ORDER BY handoff.id,enumeration_handoff.id
            FOR SHARE OF handoff,source_unit,vuln_run,candidate_run,enumeration_run,
                         enumeration_handoff,enumeration_unit,scope_snapshot"#,
    )
    .bind(operation_id)
    .bind(organization_id)
    .bind(unit.0)
    .bind(unit.2)
    .fetch_all(&mut *tx)
    .await?;
    let authority = require_exact_predecessor(&authorities).map_err(crate::DbError::Other)?;
    if authority.scope_snapshot_id != unit.0
        || authority.source_generation < 0
        || unit.1 != 0
        || authority.evidence_ids.is_empty()
        || authority.enumeration_evidence_ids.is_empty()
    {
        return Err(invalid(
            "initial Candidate Wave generation or vuln_triage handoff authority mismatch",
        ));
    }
    let ordinal: i32 = sqlx::query_scalar(
        "SELECT ordinal FROM operation_org_scope_units
         WHERE snapshot_id=$1 AND organization_id=$2",
    )
    .bind(unit.0)
    .bind(organization_id)
    .fetch_one(&mut *tx)
    .await?;
    let outcomes = sqlx::query_as::<_, FormulaicOutcomeRow>(
        r#"SELECT asset,technique,outcome,source,query,result_count,confidence,
                  evidence_ids,collected_at
             FROM technique_outcomes
            WHERE organization_id=$1 AND run_id=$2
              AND technique=ANY($3)
              AND outcome IN ('found','empty','blocked','not_applicable')
              AND collected_at IS NOT NULL AND collected_at<=$4
              AND updated_at<=$4
            ORDER BY asset,technique
            FOR SHARE"#,
    )
    .bind(organization_id)
    .bind(operation_id.to_string())
    .bind(FORMULAIC_TECHNIQUES)
    .bind(authority.gate_passed_at)
    .fetch_all(&mut *tx)
    .await?;
    attest_formulaic_outcomes(organization_id, authority, &outcomes)
        .map_err(crate::DbError::Other)?;
    let outcome_evidence_ids = outcomes
        .iter()
        .flat_map(|outcome| outcome.evidence_ids.iter().copied())
        .collect::<BTreeSet<_>>()
        .into_iter()
        .collect::<Vec<_>>();
    let evidence_rows = sqlx::query_as::<_, FormulaicEvidenceRow>(
        r#"SELECT evidence.id,evidence.target_id AS evidence_target_id,
                  CASE WHEN current_target.id IS NULL THEN NULL
                       ELSE current_target.id END AS target_live_id,
                  COALESCE(evidence.tool_name,'') AS tool_name,evidence.detail,
                  evidence.evidence_technique,evidence.evidence_asset,
                  evidence.evidence_outcome,evidence.created_at
             FROM audit_log AS evidence
             JOIN operation_org_scope_snapshots AS scope_snapshot
               ON scope_snapshot.id=$2 AND scope_snapshot.operation_id=$3
             LEFT JOIN targets AS current_target
               ON current_target.id=evidence.target_id
              AND current_target.organization_id=$1
              AND current_target.project_path=scope_snapshot.project_path_at_freeze
              AND current_target.scope='in'
            WHERE evidence.id=ANY($4) AND evidence.audit_role='evidence'
              AND evidence.created_at<=$5
            ORDER BY evidence.id
            FOR SHARE OF evidence,scope_snapshot"#,
    )
    .bind(organization_id)
    .bind(unit.0)
    .bind(operation_id)
    .bind(&outcome_evidence_ids)
    .bind(authority.gate_passed_at)
    .fetch_all(&mut *tx)
    .await?;
    let mut observations =
        materialize_initial_candidate_observations(organization_id, &outcomes, &evidence_rows)
            .map_err(crate::DbError::Other)?;
    let enumeration_support = sqlx::query_as::<_, EnumerationSupportEvidenceRow>(
        r#"SELECT evidence.run_id AS operation_id,
                  scope_snapshot.id AS scope_snapshot_id,
                  enumeration_handoff.organization_id,
                  enumeration_handoff.id AS handoff_id,
                  enumeration_handoff.stage_execution_id,
                  enumeration_handoff.source_stage_run_unit_id,
                  evidence.id AS evidence_id,evidence.target_id,
                  evidence.project_path,
                  COALESCE(evidence.evidence_asset,'') AS evidence_asset,
                  COALESCE(evidence.evidence_technique,'') AS evidence_technique,
                  COALESCE(evidence.evidence_outcome,'') AS evidence_outcome,
                  COALESCE(evidence.tool_name,'') AS tool_name,
                  COALESCE(evidence.detail->>'kind','') AS evidence_kind,
                  evidence.created_at
             FROM stage_handoffs AS enumeration_handoff
             JOIN operation_org_scope_snapshots AS scope_snapshot
               ON scope_snapshot.id=enumeration_handoff.scope_snapshot_id
              AND scope_snapshot.operation_id=enumeration_handoff.operation_id
              AND scope_snapshot.sealed_at IS NOT NULL
             CROSS JOIN LATERAL UNNEST(enumeration_handoff.evidence_ids)
                  AS linked(evidence_id)
             JOIN audit_log AS evidence
               ON evidence.id=linked.evidence_id
              AND evidence.run_id=enumeration_handoff.operation_id
              AND evidence.audit_role='evidence'
              AND evidence.project_path=scope_snapshot.project_path_at_freeze
              AND evidence.detail->>'organization_id'=
                  enumeration_handoff.organization_id::TEXT
             JOIN targets AS current_target
               ON current_target.id=evidence.target_id
              AND current_target.organization_id=enumeration_handoff.organization_id
              AND current_target.project_path=scope_snapshot.project_path_at_freeze
              AND current_target.scope='in'
            WHERE enumeration_handoff.id=$1
              AND enumeration_handoff.operation_id=$2
              AND enumeration_handoff.scope_snapshot_id=$3
              AND enumeration_handoff.organization_id=$4
              AND enumeration_handoff.from_stage_kind='enumeration'
              AND enumeration_handoff.stage_execution_id=$5
              AND enumeration_handoff.source_stage_run_unit_id=$6
              AND enumeration_handoff.invalidated_at IS NULL
              AND evidence.created_at>=$7 AND evidence.created_at<=$8
            ORDER BY evidence.id
            FOR SHARE OF enumeration_handoff,scope_snapshot,evidence,current_target"#,
    )
    .bind(authority.enumeration_handoff_id)
    .bind(operation_id)
    .bind(unit.0)
    .bind(organization_id)
    .bind(authority.enumeration_stage_execution_id)
    .bind(authority.enumeration_source_stage_run_unit_id)
    .bind(authority.enumeration_started_at)
    .bind(authority.enumeration_gate_passed_at)
    .fetch_all(&mut *tx)
    .await?;
    let support_evidence_ids = enumeration_support
        .iter()
        .map(|row| row.evidence_id)
        .collect::<Vec<_>>();
    let enumeration_directory_entries = sqlx::query_as::<_, EnumerationDirectoryEntryRow>(
        r#"SELECT evidence.id AS source_evidence_id,
                  entry.id AS directory_entry_id,entry.target_id,entry.project_path,
                  entry.url,entry.status_code,entry.content_length,
                  NULLIF(entry.content_type,'') AS content_type,
                  entry.tool AS source_tool,entry.created_at
             FROM audit_log AS evidence
             JOIN operation_org_scope_snapshots AS scope_snapshot
               ON scope_snapshot.id=$2 AND scope_snapshot.operation_id=$3
              AND scope_snapshot.sealed_at IS NOT NULL
             JOIN targets AS current_target
               ON current_target.id=evidence.target_id
              AND current_target.organization_id=$4
              AND current_target.project_path=scope_snapshot.project_path_at_freeze
              AND current_target.scope='in'
             JOIN directory_entries AS entry
               ON entry.target_id=current_target.id
              AND entry.project_path=scope_snapshot.project_path_at_freeze
              AND entry.created_at>=$5 AND entry.created_at<=evidence.created_at
            WHERE evidence.id=ANY($1)
              AND evidence.run_id=$3 AND evidence.audit_role='evidence'
              AND evidence.project_path=scope_snapshot.project_path_at_freeze
              AND evidence.detail->>'organization_id'=$4::TEXT
              AND evidence.evidence_technique='GOLISH-ENUM-DIR'
              AND evidence.evidence_outcome='found'
              AND evidence.tool_name='route_probe_paths'
              AND evidence.detail->>'kind'='dir_entry'
            ORDER BY evidence.id,entry.id
            FOR SHARE OF evidence,scope_snapshot,current_target,entry"#,
    )
    .bind(&support_evidence_ids)
    .bind(unit.0)
    .bind(operation_id)
    .bind(organization_id)
    .bind(authority.enumeration_started_at)
    .fetch_all(&mut *tx)
    .await?;
    let support_by_work_item = merge_exact_enumeration_support(
        &ExactEnumerationLineage {
            operation_id,
            scope_snapshot_id: unit.0,
            organization_id,
            handoff_id: authority.enumeration_handoff_id,
            stage_execution_id: authority.enumeration_stage_execution_id,
            source_stage_run_unit_id: authority.enumeration_source_stage_run_unit_id,
            project_path: authority.project_path.clone(),
            started_at: authority.enumeration_started_at,
            gate_passed_at: authority.enumeration_gate_passed_at,
        },
        &enumeration_support,
        &enumeration_directory_entries,
        &mut observations,
    )
    .map_err(crate::DbError::Other)?;
    let wave_run_id = attack_waves::deterministic_initial_wave_run_id(operation_id, unit.1);
    let wave_unit_id =
        attack_waves::deterministic_initial_wave_unit_id(wave_run_id, organization_id);
    let (policy_snapshot, policy_hash) = attack_waves::deterministic_initial_policy()?;
    attack_waves::open_from_vuln_triage_handoff(
        &mut tx,
        &attack_waves::OpenAttackWaveUnit {
            wave_run_id,
            wave_unit_id,
            operation_id,
            scope_snapshot_id: unit.0,
            organization_id,
            entry_stage_execution_id: authority.stage_execution_id,
            entry_stage_run_unit_id: authority.source_stage_run_unit_id,
            entry_deliverable_submission_id: authority.deliverable_submission_id,
            generation: unit.1,
            ordinal,
            policy_snapshot,
            policy_hash,
            max_waves: 3,
            max_candidates_total: 100,
            max_chain_depth: 3,
            max_attempts_total: 200,
        },
    )
    .await?;
    seed_wave_work_items_with_support(
        &mut tx,
        SeedAttackWorkItems {
            operation_id,
            scope_snapshot_id: unit.0,
            wave_run_id,
            wave_unit_id,
            organization_id,
            observations,
        },
        &support_by_work_item,
    )
    .await?;
    tx.commit().await?;
    load_for_wave_unit(
        pool,
        operation_id,
        unit.0,
        wave_run_id,
        wave_unit_id,
        organization_id,
    )
    .await
}

/// Load the exact manifest consumed by one current attack_candidate runtime
/// Unit. Unit generation is the server-owned bridge to the immutable WaveRun;
/// zero/multiple work is never collapsed into an "unavailable means empty"
/// result.
pub async fn load_for_runtime_unit(
    pool: &sqlx::PgPool,
    operation_id: Uuid,
    stage_run_unit_id: Uuid,
    organization_id: Uuid,
) -> crate::Result<CandidateManifestRow> {
    let identity: (Uuid, Uuid, Uuid) = sqlx::query_as(
        r#"SELECT run.scope_snapshot_id,run.id,wave_unit.id
             FROM stage_run_units AS decision_unit
             JOIN attack_wave_runs AS run
               ON run.operation_id=decision_unit.operation_id
              AND run.scope_snapshot_id=decision_unit.scope_snapshot_id
              AND run.generation=decision_unit.generation
             JOIN attack_wave_units AS wave_unit
               ON wave_unit.wave_run_id=run.id
              AND wave_unit.operation_id=run.operation_id
              AND wave_unit.scope_snapshot_id=run.scope_snapshot_id
              AND wave_unit.organization_id=decision_unit.organization_id
            WHERE decision_unit.id=$1
              AND decision_unit.operation_id=$2
              AND decision_unit.organization_id=$3
              AND decision_unit.stage_kind='attack_candidate'"#,
    )
    .bind(stage_run_unit_id)
    .bind(operation_id)
    .bind(organization_id)
    .fetch_optional(pool)
    .await?
    .ok_or_else(|| crate::DbError::NotFound("attack_candidate_manifest".to_string()))?;
    load_for_wave_unit(
        pool,
        operation_id,
        identity.0,
        identity.1,
        identity.2,
        organization_id,
    )
    .await
}

async fn load_manifest_with_connection(
    connection: &mut PgConnection,
    operation_id: Uuid,
    scope_snapshot_id: Uuid,
    wave_run_id: Uuid,
    wave_unit_id: Uuid,
    organization_id: Uuid,
) -> crate::Result<CandidateManifestRow> {
    let sql = format!(
        "SELECT {COLUMNS} FROM attack_candidate_work_items
         WHERE wave_unit_id=$1 AND operation_id=$2 AND scope_snapshot_id=$3
           AND organization_id=$4 ORDER BY work_item_key,id FOR SHARE"
    );
    let work_items = sqlx::query_as::<_, AttackCandidateWorkItemRow>(&sql)
        .bind(wave_unit_id)
        .bind(operation_id)
        .bind(scope_snapshot_id)
        .bind(organization_id)
        .fetch_all(&mut *connection)
        .await?;
    let mut items = Vec::with_capacity(work_items.len());
    for work_item in work_items {
        let (
            technique,
            observation,
            observation_hash,
            source_fact_delta_id,
            delta_kind,
            observation_kind,
            allowed_techniques,
            enrichment_required,
        ): CandidateSeedProjection = sqlx::query_as(
            "SELECT technique,observation,observation_hash,source_fact_delta_id,
                    delta_kind,observation_kind,allowed_techniques,enrichment_required
               FROM attack_candidate_seeds WHERE id=$1 FOR SHARE",
        )
        .bind(work_item.seed_id)
        .fetch_one(&mut *connection)
        .await?;
        let seed_evidence: Vec<i64> = sqlx::query_scalar(
            "SELECT evidence_id FROM attack_candidate_seed_evidence
             WHERE seed_id=$1 AND role IN ('observation','support')
             ORDER BY evidence_id FOR SHARE",
        )
        .bind(work_item.seed_id)
        .fetch_all(&mut *connection)
        .await?;
        let work_item_evidence: Vec<i64> = sqlx::query_scalar(
            "SELECT evidence_id FROM attack_candidate_work_item_evidence
             WHERE work_item_id=$1 AND role IN ('observation','support')
             ORDER BY evidence_id FOR SHARE",
        )
        .bind(work_item.id)
        .fetch_all(&mut *connection)
        .await?;
        let evidence_ids = seed_evidence
            .into_iter()
            .chain(work_item_evidence)
            .collect::<BTreeSet<_>>()
            .into_iter()
            .collect();
        items.push(CandidateManifestItemRow {
            work_item,
            technique,
            observation,
            observation_hash,
            source_fact_delta_id,
            delta_kind,
            observation_kind,
            allowed_techniques,
            enrichment_required,
            evidence_ids,
        });
    }
    Ok(CandidateManifestRow {
        operation_id,
        scope_snapshot_id,
        wave_run_id,
        wave_unit_id,
        organization_id,
        items,
    })
}

/// Resolve the historical evidence that an `attack_candidate` final seal may
/// carry across its own Unit freshness boundary. Authority is the exact frozen
/// `vuln_triage` entry handoff plus the immutable manifest attestation; merely
/// belonging to the same operation or organization is insufficient.
///
/// This is connection-based so callers can keep the proof under the same
/// final-seal transaction and locks as Candidate acceptance.
pub async fn load_frozen_entry_evidence_ids_with_connection(
    connection: &mut PgConnection,
    operation_id: Uuid,
    scope_snapshot_id: Uuid,
    wave_run_id: Uuid,
    wave_unit_id: Uuid,
    organization_id: Uuid,
) -> crate::Result<Vec<i64>> {
    let authority = sqlx::query_as::<_, FrozenEntryEvidenceAuthority>(
        r#"SELECT wave_unit.manifest_hash,wave_unit.manifest_count,
                  wave_unit.manifest_frozen_at,wave_unit.entry_consolidation_id
             FROM attack_wave_units AS wave_unit
             JOIN attack_wave_runs AS wave
               ON wave.id=wave_unit.wave_run_id
              AND wave.operation_id=wave_unit.operation_id
              AND wave.scope_snapshot_id=wave_unit.scope_snapshot_id
            WHERE wave_unit.id=$1
              AND wave_unit.wave_run_id=$2
              AND wave_unit.operation_id=$3
              AND wave_unit.scope_snapshot_id=$4
              AND wave_unit.organization_id=$5
            FOR SHARE OF wave_unit,wave"#,
    )
    .bind(wave_unit_id)
    .bind(wave_run_id)
    .bind(operation_id)
    .bind(scope_snapshot_id)
    .bind(organization_id)
    .fetch_optional(&mut *connection)
    .await?
    .ok_or_else(|| invalid("attack candidate entry authority mismatch"))?;
    let (entry_evidence_ids, initial_role_authority): FrozenEntryEvidenceAuthoritySets =
        if let Some(consolidation_id) = authority.entry_consolidation_id {
            (
                sqlx::query_scalar(
                    r#"SELECT DISTINCT evidence.evidence_id
                 FROM attack_wave_consolidations AS consolidation
                 JOIN attack_wave_consolidation_members AS member
                   ON member.consolidation_id=consolidation.id
                  AND member.operation_id=consolidation.operation_id
                  AND member.scope_snapshot_id=consolidation.scope_snapshot_id
                  AND member.source_wave_run_id=consolidation.source_wave_run_id
                 JOIN attack_fact_delta_decisions AS decision
                   ON decision.fact_delta_id=member.fact_delta_id
                  AND decision.disposition='accepted'
                 JOIN attack_fact_delta_evidence AS evidence
                   ON evidence.fact_delta_id=member.fact_delta_id
                  AND evidence.role='fact_delta'
                WHERE consolidation.id=$1
                  AND consolidation.decision_kind='opened_next_wave'
                  AND consolidation.target_wave_run_id=$2
                  AND consolidation.operation_id=$3
                  AND consolidation.scope_snapshot_id=$4
                  AND member.target_wave_unit_id=$5
                  AND member.organization_id=$6
                  AND member.target_work_item_id IS NOT NULL
                ORDER BY evidence.evidence_id"#,
                )
                .bind(consolidation_id)
                .bind(wave_run_id)
                .bind(operation_id)
                .bind(scope_snapshot_id)
                .bind(wave_unit_id)
                .bind(organization_id)
                .fetch_all(&mut *connection)
                .await?,
                None,
            )
        } else {
            let predecessor_evidence = sqlx::query_as::<_, (Vec<i64>, Vec<i64>)>(
                r#"SELECT handoff.evidence_ids,enumeration_handoff.evidence_ids
                 FROM attack_wave_units AS wave_unit
                 JOIN attack_wave_runs AS wave
                   ON wave.id=wave_unit.wave_run_id
                  AND wave.operation_id=wave_unit.operation_id
                  AND wave.scope_snapshot_id=wave_unit.scope_snapshot_id
                 JOIN stage_run_units AS entry_unit
                   ON entry_unit.id=wave_unit.entry_stage_run_unit_id
                  AND entry_unit.operation_id=wave_unit.operation_id
                  AND entry_unit.stage_execution_id=wave_unit.entry_stage_execution_id
                  AND entry_unit.scope_snapshot_id=wave_unit.scope_snapshot_id
                  AND entry_unit.organization_id=wave_unit.organization_id
                  AND entry_unit.stage_kind=wave_unit.entry_stage_kind
                 JOIN stage_handoffs AS handoff
                   ON handoff.operation_id=entry_unit.operation_id
                  AND handoff.scope_snapshot_id=entry_unit.scope_snapshot_id
                  AND handoff.organization_id=entry_unit.organization_id
                  AND handoff.stage_execution_id=entry_unit.stage_execution_id
                  AND handoff.source_stage_run_unit_id=entry_unit.id
                  AND handoff.deliverable_submission_id=wave_unit.entry_deliverable_submission_id
                  AND handoff.from_stage_kind=entry_unit.stage_kind
                 JOIN stage_runs AS vuln_run
                   ON vuln_run.id=entry_unit.stage_execution_id
                  AND vuln_run.operation_id=entry_unit.operation_id
                  AND vuln_run.stage_kind='vuln_triage'
                  AND vuln_run.status='completed'
                  AND vuln_run.completed_at IS NOT NULL
                 JOIN stage_run_units AS candidate_unit
                   ON candidate_unit.operation_id=wave_unit.operation_id
                  AND candidate_unit.scope_snapshot_id=wave_unit.scope_snapshot_id
                  AND candidate_unit.organization_id=wave_unit.organization_id
                  AND candidate_unit.stage_kind='attack_candidate'
                  AND candidate_unit.generation=wave.generation
                 JOIN stage_runs AS candidate_run
                   ON candidate_run.id=candidate_unit.stage_execution_id
                  AND candidate_run.operation_id=candidate_unit.operation_id
                  AND candidate_run.stage_kind=candidate_unit.stage_kind
                  AND candidate_run.status IN ('started','completed')
                  AND candidate_run.started_at=vuln_run.completed_at
                 JOIN stage_runs AS enumeration_run
                   ON enumeration_run.operation_id=vuln_run.operation_id
                  AND enumeration_run.stage_kind='enumeration'
                  AND enumeration_run.status='completed'
                  AND enumeration_run.completed_at=vuln_run.started_at
                 JOIN stage_handoffs AS enumeration_handoff
                   ON enumeration_handoff.operation_id=handoff.operation_id
                  AND enumeration_handoff.scope_snapshot_id=handoff.scope_snapshot_id
                  AND enumeration_handoff.organization_id=handoff.organization_id
                  AND enumeration_handoff.stage_execution_id=enumeration_run.id
                  AND enumeration_handoff.from_stage_kind='enumeration'
                  AND enumeration_handoff.invalidated_at IS NULL
                 JOIN stage_run_units AS enumeration_unit
                   ON enumeration_unit.id=enumeration_handoff.source_stage_run_unit_id
                  AND enumeration_unit.operation_id=enumeration_handoff.operation_id
                  AND enumeration_unit.stage_execution_id=enumeration_handoff.stage_execution_id
                  AND enumeration_unit.scope_snapshot_id=enumeration_handoff.scope_snapshot_id
                  AND enumeration_unit.organization_id=enumeration_handoff.organization_id
                  AND enumeration_unit.stage_kind=enumeration_handoff.from_stage_kind
                 WHERE wave_unit.id=$1
                  AND wave_unit.wave_run_id=$2
                  AND wave_unit.operation_id=$3
                  AND wave_unit.scope_snapshot_id=$4
                  AND wave_unit.organization_id=$5
                  AND wave_unit.entry_stage_kind='vuln_triage'
                  AND entry_unit.status='passed'
                  AND entry_unit.terminal_at IS NOT NULL
                  AND handoff.invalidated_at IS NULL
                  AND enumeration_unit.status='passed'
                  AND enumeration_unit.terminal_at IS NOT NULL
                ORDER BY handoff.id,enumeration_handoff.id
                FOR SHARE OF wave_unit,wave,entry_unit,handoff,vuln_run,candidate_unit,
                             candidate_run,enumeration_run,enumeration_handoff,
                             enumeration_unit"#,
            )
            .bind(wave_unit_id)
            .bind(wave_run_id)
            .bind(operation_id)
            .bind(scope_snapshot_id)
            .bind(organization_id)
            .fetch_all(&mut *connection)
            .await?;
            let (entry, enumeration) =
                require_exact_predecessor(&predecessor_evidence).map_err(crate::DbError::Other)?;
            let entry = entry.iter().copied().collect::<BTreeSet<_>>();
            let enumeration = enumeration.iter().copied().collect::<BTreeSet<_>>();
            (
                entry
                    .iter()
                    .chain(&enumeration)
                    .copied()
                    .collect::<BTreeSet<_>>()
                    .into_iter()
                    .collect(),
                Some((entry, enumeration)),
            )
        };
    if entry_evidence_ids.is_empty() {
        return Err(invalid("attack candidate entry evidence is empty"));
    }
    let (manifest_hash, manifest_count, _manifest_frozen_at) = match (
        authority.manifest_hash,
        authority.manifest_count,
        authority.manifest_frozen_at,
    ) {
        (Some(hash), Some(count), Some(frozen_at)) if !hash.trim().is_empty() && count > 0 => {
            (hash, count, frozen_at)
        }
        _ => return Err(invalid("attack candidate entry manifest is not frozen")),
    };
    let manifest = load_manifest_with_connection(
        connection,
        operation_id,
        scope_snapshot_id,
        wave_run_id,
        wave_unit_id,
        organization_id,
    )
    .await?;
    if manifest.items.len() != manifest_count as usize
        || canonical_manifest_hash(&manifest) != manifest_hash
        || manifest
            .items
            .iter()
            .any(|item| item.evidence_ids.is_empty())
    {
        return Err(invalid(
            "attack candidate frozen manifest attestation mismatch",
        ));
    }
    let evidence_ids = manifest
        .items
        .iter()
        .flat_map(|item| item.evidence_ids.iter().copied())
        .collect::<BTreeSet<_>>();
    if let Some((observation_authority, support_authority)) = initial_role_authority {
        let role_rows = sqlx::query_as::<_, FrozenManifestEvidenceRoleRow>(
            r#"WITH manifest_links AS (
                    SELECT item.target_live_id,link.evidence_id,link.role
                      FROM attack_candidate_work_items AS item
                      JOIN attack_candidate_seed_evidence AS link
                        ON link.seed_id=item.seed_id
                     WHERE item.wave_unit_id=$1 AND item.operation_id=$2
                       AND item.scope_snapshot_id=$3 AND item.organization_id=$4
                       AND link.role IN ('observation','support')
                    UNION ALL
                    SELECT item.target_live_id,link.evidence_id,link.role
                      FROM attack_candidate_work_items AS item
                      JOIN attack_candidate_work_item_evidence AS link
                        ON link.work_item_id=item.id
                     WHERE item.wave_unit_id=$1 AND item.operation_id=$2
                       AND item.scope_snapshot_id=$3 AND item.organization_id=$4
                       AND link.role IN ('observation','support')
                 )
                 SELECT link.evidence_id,link.role,link.target_live_id,
                        evidence.run_id AS evidence_run_id,
                        evidence.detail->>'organization_id' AS producer_organization_id,
                        evidence.target_id AS evidence_target_id,
                        evidence.project_path AS evidence_project_path,
                        snapshot.project_path_at_freeze
                   FROM manifest_links AS link
                   JOIN audit_log AS evidence ON evidence.id=link.evidence_id
                   JOIN operation_org_scope_snapshots AS snapshot
                     ON snapshot.id=$3 AND snapshot.operation_id=$2
                    AND snapshot.sealed_at IS NOT NULL
                  ORDER BY link.evidence_id,link.role,link.target_live_id"#,
        )
        .bind(wave_unit_id)
        .bind(operation_id)
        .bind(scope_snapshot_id)
        .bind(organization_id)
        .fetch_all(&mut *connection)
        .await?;
        let role_evidence_ids = role_rows
            .iter()
            .map(|row| row.evidence_id)
            .collect::<BTreeSet<_>>();
        let exact_org = organization_id.to_string();
        if role_rows.is_empty()
            || role_evidence_ids != evidence_ids
            || role_rows.iter().any(|row| {
                row.evidence_run_id != Some(operation_id)
                    || row.producer_organization_id.as_deref() != Some(exact_org.as_str())
                    || row.evidence_project_path != row.project_path_at_freeze
                    || match row.role.as_str() {
                        "observation" => !observation_authority.contains(&row.evidence_id),
                        "support" => {
                            !support_authority.contains(&row.evidence_id)
                                || row.target_live_id.is_none()
                                || row.evidence_target_id != row.target_live_id
                        }
                        _ => true,
                    }
            })
        {
            return Err(invalid(
                "attack candidate frozen manifest evidence role or target authority mismatch",
            ));
        }
    }
    let entry_evidence = entry_evidence_ids.into_iter().collect::<BTreeSet<_>>();
    if evidence_ids.iter().any(|id| !entry_evidence.contains(id)) {
        return Err(invalid(
            "attack candidate manifest evidence is not linked by its exact sealed entry",
        ));
    }
    Ok(evidence_ids.into_iter().collect())
}

pub async fn load_for_wave_unit(
    pool: &sqlx::PgPool,
    operation_id: Uuid,
    scope_snapshot_id: Uuid,
    wave_run_id: Uuid,
    wave_unit_id: Uuid,
    organization_id: Uuid,
) -> crate::Result<CandidateManifestRow> {
    let attestation: (String, i32, chrono::DateTime<Utc>) = sqlx::query_as(
        r#"SELECT manifest_hash,manifest_count,manifest_frozen_at
             FROM attack_wave_units
            WHERE id=$1 AND wave_run_id=$2 AND operation_id=$3
              AND scope_snapshot_id=$4 AND organization_id=$5"#,
    )
    .bind(wave_unit_id)
    .bind(wave_run_id)
    .bind(operation_id)
    .bind(scope_snapshot_id)
    .bind(organization_id)
    .fetch_optional(pool)
    .await?
    .ok_or_else(|| crate::DbError::Other(anyhow::anyhow!("attack manifest is not frozen")))?;
    let sql = format!(
        "SELECT {COLUMNS} FROM attack_candidate_work_items
         WHERE wave_unit_id=$1 AND operation_id=$2 AND scope_snapshot_id=$3
           AND organization_id=$4 ORDER BY work_item_key,id"
    );
    let work_items = sqlx::query_as::<_, AttackCandidateWorkItemRow>(&sql)
        .bind(wave_unit_id)
        .bind(operation_id)
        .bind(scope_snapshot_id)
        .bind(organization_id)
        .fetch_all(pool)
        .await?;
    let mut items = Vec::with_capacity(work_items.len());
    for work_item in work_items {
        let (
            technique,
            observation,
            observation_hash,
            source_fact_delta_id,
            delta_kind,
            observation_kind,
            allowed_techniques,
            enrichment_required,
        ): CandidateSeedProjection = sqlx::query_as(
            "SELECT technique,observation,observation_hash,source_fact_delta_id,
                    delta_kind,observation_kind,allowed_techniques,enrichment_required
               FROM attack_candidate_seeds WHERE id=$1",
        )
        .bind(work_item.seed_id)
        .fetch_one(pool)
        .await?;
        let evidence_ids = sqlx::query_scalar(
            r#"SELECT evidence_id FROM (
                   SELECT evidence_id FROM attack_candidate_seed_evidence WHERE seed_id=$1
                   UNION
                   SELECT evidence_id FROM attack_candidate_work_item_evidence
                    WHERE work_item_id=$2 AND role IN ('observation','support')
               ) evidence ORDER BY evidence_id"#,
        )
        .bind(work_item.seed_id)
        .bind(work_item.id)
        .fetch_all(pool)
        .await?;
        items.push(CandidateManifestItemRow {
            work_item,
            technique,
            observation,
            observation_hash,
            source_fact_delta_id,
            delta_kind,
            observation_kind,
            allowed_techniques,
            enrichment_required,
            evidence_ids,
        });
    }
    let manifest = CandidateManifestRow {
        operation_id,
        scope_snapshot_id,
        wave_run_id,
        wave_unit_id,
        organization_id,
        items,
    };
    let actual_hash = canonical_manifest_hash(&manifest);
    if attestation.0 != actual_hash || attestation.1 != manifest.items.len() as i32 {
        return Err(invalid("attack manifest attestation mismatch"));
    }
    Ok(manifest)
}

#[cfg(test)]
mod tests {
    use super::*;

    fn typed_evidence(
        organization_id: Uuid,
        target_live_id: Uuid,
        id: i64,
        asset: &str,
        technique: &str,
        observations: Vec<serde_json::Value>,
    ) -> FormulaicEvidenceRow {
        let targeted = technique == "GOLISH-NDAY";
        let tool_name = if targeted {
            "vuln_nuclei_fingerprint_targeted"
        } else {
            "vuln_nuclei_general"
        };
        let source_mode = if targeted {
            "fingerprint_targeted"
        } else {
            "general"
        };
        FormulaicEvidenceRow {
            id,
            evidence_target_id: Some(target_live_id),
            target_live_id: Some(target_live_id),
            tool_name: tool_name.to_string(),
            detail: serde_json::json!({
                "kind": "vuln.nuclei_observation",
                "organization_id": organization_id,
                "raw_output": serde_json::json!({
                    "schema": "nuclei_observation_batch_v1",
                    "source_mode": source_mode,
                    "target_id": target_live_id,
                    "exact_origin": asset,
                    "technique": technique,
                    "matches_total": observations.len(),
                    "matches_in_evidence": observations.len(),
                    "matches_omitted": 0,
                    "observations": observations,
                }).to_string(),
            }),
            evidence_technique: Some(technique.to_string()),
            evidence_asset: Some(asset.to_string()),
            evidence_outcome: Some("found".to_string()),
            created_at: Utc::now() - chrono::Duration::seconds(2),
        }
    }

    fn typed_match(target_live_id: Uuid, technique: &str, template_id: &str) -> serde_json::Value {
        let source_mode = if technique == "GOLISH-NDAY" {
            "fingerprint_targeted"
        } else {
            "general"
        };
        serde_json::json!({
            "schema": "nuclei_match_v1",
            "source_mode": source_mode,
            "target_id": target_live_id,
            "matched_url": "https://app.example.test:443/login",
            "template_id": template_id,
            "matcher_name": "body",
            "template_name": "bounded fixture",
            "severity": "high",
            "technique": technique,
            "fingerprint_refs": [],
            "observed_at": Utc::now() - chrono::Duration::seconds(3),
        })
    }

    fn anonymous_access_evidence(
        organization_id: Uuid,
        target_live_id: Uuid,
        id: i64,
        asset: &str,
        outcome: &str,
        observations: Vec<serde_json::Value>,
    ) -> FormulaicEvidenceRow {
        let completion_state = if observations.iter().any(|observation| {
            matches!(
                observation["verdict"].as_str(),
                Some("inconclusive" | "skipped")
            )
        }) {
            "partial"
        } else {
            "complete"
        };
        FormulaicEvidenceRow {
            id,
            evidence_target_id: Some(target_live_id),
            target_live_id: Some(target_live_id),
            tool_name: "vuln_probe_anonymous_access".to_string(),
            detail: serde_json::json!({
                "kind": "vuln.anonymous_access_observation",
                "organization_id": organization_id,
                "raw_output": serde_json::json!({
                    "schema": "anonymous_access_batch_v1",
                    "technique": "WSTG-ATHN-04",
                    "target_id": target_live_id,
                    "exact_origin": asset,
                    "reviewed_count": observations.len(),
                    "selected_count": observations.len(),
                    "reviewed_set_sha256": "1111111111111111111111111111111111111111111111111111111111111111",
                    "completion_state": completion_state,
                    "aggregate_outcome": outcome,
                    "observations": observations,
                    "error_classes": [],
                }).to_string(),
            }),
            evidence_technique: Some("WSTG-ATHN-04".to_string()),
            evidence_asset: Some(asset.to_string()),
            evidence_outcome: Some(outcome.to_string()),
            created_at: Utc::now() - chrono::Duration::seconds(2),
        }
    }

    fn anonymous_observation(endpoint_id: Uuid, verdict: &str) -> serde_json::Value {
        serde_json::json!({
            "schema": "anonymous_access_v1",
            "endpoint_id": endpoint_id,
            "endpoint_row_sha256": "2222222222222222222222222222222222222222222222222222222222222222",
            "request_plan_sha256": "3333333333333333333333333333333333333333333333333333333333333333",
            "method": "GET",
            "path": "/api/private/profile",
            "query_bindings": [{"name": "id", "value": "42"}],
            "persisted_auth_hint": "unknown",
            "persisted_headers_present": false,
            "source": "browser_js",
            "no_auth": true,
            "network_attempted": true,
            "status_code": 200,
            "response": {
                "content_type_family": "json",
                "declared_length": 128,
                "captured_length": 128,
                "prefix_sha256": "sha256:response",
                "truncated": false,
            },
            "redirect": {
                "present": false,
                "same_origin": null,
                "login_like": false,
            },
            "verdict": verdict,
            "rationale": "bounded fixture",
            "error_class": null,
            "authority_current_after": true,
        })
    }

    fn outcomes(count: usize) -> Vec<FormulaicOutcomeRow> {
        (0..count)
            .map(|index| FormulaicOutcomeRow {
                asset: format!(
                    "https://app-{}.example.test",
                    index / FORMULAIC_TECHNIQUES.len()
                ),
                technique: FORMULAIC_TECHNIQUES[index % FORMULAIC_TECHNIQUES.len()].to_string(),
                outcome: if index % 2 == 0 { "found" } else { "empty" }.to_string(),
                source: Some("formulaic_sweep".to_string()),
                query: None,
                result_count: Some(i32::from(index % 2 == 0)),
                confidence: Some(1.0),
                evidence_ids: vec![index as i64 + 1],
                collected_at: Utc::now() - chrono::Duration::seconds(1),
            })
            .collect()
    }

    fn authority(organization_id: Uuid, rows: &[FormulaicOutcomeRow]) -> FormulaicHandoffAuthority {
        let assets = rows
            .iter()
            .map(|row| row.asset.clone())
            .collect::<BTreeSet<_>>();
        let techniques = rows
            .iter()
            .map(|row| row.technique.clone())
            .collect::<BTreeSet<_>>();
        FormulaicHandoffAuthority {
            stage_execution_id: Uuid::new_v4(),
            source_stage_run_unit_id: Uuid::new_v4(),
            deliverable_submission_id: Uuid::new_v4(),
            scope_snapshot_id: Uuid::new_v4(),
            source_generation: 0,
            evidence_ids: rows
                .iter()
                .flat_map(|row| row.evidence_ids.iter().copied())
                .collect(),
            coverage_watermark: serde_json::json!({
                "kind": "information_coverage_v1",
                "stage": "vuln_triage",
                "organization_id": organization_id,
                "terminal_cells": rows.len(),
                "canonical_ref_total": rows.len(),
                "canonical_ref_included": rows.len(),
                "canonical_ref_truncated": false,
                "evidence_id_total": rows.len(),
                "evidence_id_included": rows.len(),
                "evidence_id_truncated": false,
                "assets": assets,
                "techniques": techniques,
            }),
            gate_passed_at: Utc::now(),
            enumeration_handoff_id: Uuid::new_v4(),
            enumeration_stage_execution_id: Uuid::new_v4(),
            enumeration_source_stage_run_unit_id: Uuid::new_v4(),
            enumeration_evidence_ids: vec![1],
            enumeration_started_at: Utc::now() - chrono::Duration::seconds(2),
            enumeration_gate_passed_at: Utc::now() - chrono::Duration::seconds(1),
            project_path: "/tmp/fixture".to_string(),
        }
    }

    #[test]
    fn exact_formulaic_watermark_passes_but_truncation_fails_closed() {
        let organization_id = Uuid::new_v4();
        let rows = outcomes(FORMULAIC_TECHNIQUES.len());
        let mut exact = authority(organization_id, &rows);
        attest_formulaic_outcomes(organization_id, &exact, &rows)
            .expect("exact canonical formulaic cells");
        exact.coverage_watermark["canonical_ref_truncated"] = serde_json::json!(true);
        exact.coverage_watermark["canonical_ref_included"] = serde_json::json!(rows.len() - 1);
        assert!(attest_formulaic_outcomes(organization_id, &exact, &rows).is_err());
    }

    #[test]
    fn formulaic_manifest_over_policy_limit_fails_before_seeding() {
        let organization_id = Uuid::new_v4();
        let rows = outcomes(MAX_ATTACK_MANIFEST_ITEMS + 1);
        let exact = authority(organization_id, &rows);
        assert!(attest_formulaic_outcomes(organization_id, &exact, &rows).is_err());
    }

    #[test]
    fn frozen_target_snapshot_does_not_require_a_live_target_row() {
        let (target_type, value, hash) = frozen_target_snapshot("https://example.test/login");
        assert_eq!(target_type, "url");
        assert_eq!(value, "https://example.test/login");
        assert!(hash.starts_with("sha256:"));
    }

    #[test]
    fn two_typed_nuclei_matches_become_two_leads_plus_one_surface_item() {
        let organization_id = Uuid::new_v4();
        let target_live_id = Uuid::new_v4();
        let asset = "https://app.example.test:443";
        let technique = "WSTG-INPV-05";
        let rows = vec![FormulaicOutcomeRow {
            asset: asset.to_string(),
            technique: technique.to_string(),
            outcome: "found".to_string(),
            source: Some("vuln_nuclei_general".to_string()),
            query: Some("nuclei:general:fixture".to_string()),
            result_count: Some(2),
            confidence: None,
            evidence_ids: vec![41],
            collected_at: Utc::now() - chrono::Duration::seconds(1),
        }];
        let evidence = vec![typed_evidence(
            organization_id,
            target_live_id,
            41,
            asset,
            technique,
            vec![
                typed_match(target_live_id, technique, "fixture-one"),
                typed_match(target_live_id, technique, "fixture-two"),
            ],
        )];

        let observations =
            materialize_initial_candidate_observations(organization_id, &rows, &evidence)
                .expect("typed observations must materialize");

        assert_eq!(observations.len(), 3);
        assert_eq!(
            observations
                .iter()
                .filter(|item| item.work_item_key.starts_with("scanner_observation:"))
                .count(),
            2
        );
        assert_eq!(
            observations
                .iter()
                .filter(|item| item.observation["schema"] == "surface_analysis_v1")
                .count(),
            1
        );
        let template_ids = observations
            .iter()
            .filter_map(|item| item.observation["template_id"].as_str())
            .collect::<BTreeSet<_>>();
        assert_eq!(template_ids, BTreeSet::from(["fixture-one", "fixture-two"]));
    }

    #[test]
    fn negative_cells_are_context_not_individual_work_items() {
        let organization_id = Uuid::new_v4();
        let asset = "https://app.example.test:443";
        let rows = ["empty", "blocked", "not_applicable"]
            .into_iter()
            .enumerate()
            .map(|(index, outcome)| FormulaicOutcomeRow {
                asset: asset.to_string(),
                technique: FORMULAIC_TECHNIQUES[index].to_string(),
                outcome: outcome.to_string(),
                source: Some("vuln_nuclei_general".to_string()),
                query: None,
                result_count: Some(0),
                confidence: None,
                evidence_ids: vec![index as i64 + 1],
                collected_at: Utc::now() - chrono::Duration::seconds(1),
            })
            .collect::<Vec<_>>();
        let evidence = rows
            .iter()
            .enumerate()
            .map(|(index, row)| {
                let mut evidence = typed_evidence(
                    organization_id,
                    Uuid::new_v4(),
                    index as i64 + 1,
                    asset,
                    &row.technique,
                    Vec::new(),
                );
                evidence.evidence_outcome = Some(row.outcome.clone());
                evidence
            })
            .collect::<Vec<_>>();

        let observations =
            materialize_initial_candidate_observations(organization_id, &rows, &evidence)
                .expect("negative coverage remains bounded target context");

        assert_eq!(observations.len(), 1);
        assert!(observations[0]
            .work_item_key
            .starts_with("surface_analysis:"));
        assert_eq!(observations[0].observation["schema"], "surface_analysis_v1");
        assert_eq!(
            observations[0].observation["formulaic_coverage"]
                .as_array()
                .unwrap()
                .len(),
            3
        );
    }

    #[test]
    fn coarse_found_without_a_typed_positive_observation_fails_closed() {
        let organization_id = Uuid::new_v4();
        let target_live_id = Uuid::new_v4();
        let asset = "https://app.example.test:443";
        let technique = "GOLISH-NDAY";
        let rows = vec![FormulaicOutcomeRow {
            asset: asset.to_string(),
            technique: technique.to_string(),
            outcome: "found".to_string(),
            source: Some("vuln_nuclei_fingerprint_targeted".to_string()),
            query: None,
            result_count: Some(1),
            confidence: None,
            evidence_ids: vec![77],
            collected_at: Utc::now() - chrono::Duration::seconds(1),
        }];
        let mut evidence = typed_evidence(
            organization_id,
            target_live_id,
            77,
            asset,
            technique,
            Vec::new(),
        );
        evidence.tool_name = "vuln_nuclei_fingerprint_targeted".to_string();

        assert!(
            materialize_initial_candidate_observations(organization_id, &rows, &[evidence],)
                .is_err()
        );
    }

    #[test]
    fn nuclei_result_count_drift_from_typed_observations_fails_closed() {
        let organization_id = Uuid::new_v4();
        let target_live_id = Uuid::new_v4();
        let asset = "https://app.example.test:443";
        let technique = "WSTG-INPV-05";
        let rows = vec![FormulaicOutcomeRow {
            asset: asset.to_string(),
            technique: technique.to_string(),
            outcome: "found".to_string(),
            source: Some("vuln_nuclei_general".to_string()),
            query: None,
            result_count: Some(2),
            confidence: None,
            evidence_ids: vec![79],
            collected_at: Utc::now() - chrono::Duration::seconds(1),
        }];
        let evidence = typed_evidence(
            organization_id,
            target_live_id,
            79,
            asset,
            technique,
            vec![typed_match(target_live_id, technique, "fixture-one")],
        );

        assert!(
            materialize_initial_candidate_observations(organization_id, &rows, &[evidence],)
                .is_err()
        );
    }

    #[test]
    fn suspicious_anonymous_access_observation_becomes_a_lead_but_controlled_does_not() {
        let organization_id = Uuid::new_v4();
        let target_live_id = Uuid::new_v4();
        let asset = "https://app.example.test:443";
        let suspicious = anonymous_observation(Uuid::new_v4(), "suspicious");
        let negative = ["controlled", "public", "inconclusive", "skipped"]
            .into_iter()
            .map(|verdict| anonymous_observation(Uuid::new_v4(), verdict))
            .collect::<Vec<_>>();
        let rows = vec![FormulaicOutcomeRow {
            asset: asset.to_string(),
            technique: "WSTG-ATHN-04".to_string(),
            outcome: "found".to_string(),
            source: Some("vuln_probe_anonymous_access".to_string()),
            query: Some("generation:fixture".to_string()),
            result_count: Some(1),
            confidence: None,
            evidence_ids: vec![88],
            collected_at: Utc::now() - chrono::Duration::seconds(1),
        }];
        let evidence = vec![anonymous_access_evidence(
            organization_id,
            target_live_id,
            88,
            asset,
            "found",
            std::iter::once(suspicious.clone())
                .chain(negative)
                .collect(),
        )];

        let materialized =
            materialize_initial_candidate_observations(organization_id, &rows, &evidence)
                .expect("typed anonymous-access evidence must materialize");

        assert_eq!(materialized.len(), 2);
        let lead = materialized
            .iter()
            .find(|item| item.work_item_key.starts_with("scanner_observation:"))
            .expect("one concrete positive lead");
        assert_eq!(lead.technique, "WSTG-ATHN-04");
        assert_eq!(lead.observation, suspicious);
        assert_eq!(
            materialized
                .iter()
                .filter(|item| item.observation["schema"] == "surface_analysis_v1")
                .count(),
            1
        );
    }

    #[test]
    fn canonical_manifest_hash_covers_observation_and_declared_hash() {
        let now = Utc::now();
        let work_item = AttackCandidateWorkItemRow {
            id: Uuid::new_v4(),
            seed_id: Uuid::new_v4(),
            wave_unit_id: Uuid::new_v4(),
            operation_id: Uuid::new_v4(),
            scope_snapshot_id: Uuid::new_v4(),
            organization_id: Uuid::new_v4(),
            target_live_id: None,
            target_type_at_time: "url".to_string(),
            target_value_at_time: "https://app.example.test:443".to_string(),
            target_identity_hash: "sha256:target".to_string(),
            work_item_key: "scanner_observation:fixture".to_string(),
            decision_kind: None,
            candidate_id: None,
            no_candidate_reason_code: None,
            no_candidate_detail: None,
            decided_at: None,
            row_version: 0,
            created_at: now,
            updated_at: now,
        };
        let base_item = CandidateManifestItemRow {
            work_item,
            technique: "GOLISH-NDAY".to_string(),
            observation: serde_json::json!({"schema": "nuclei_match_v1", "template_id": "one"}),
            observation_hash: "sha256:one".to_string(),
            source_fact_delta_id: None,
            delta_kind: None,
            observation_kind: "nuclei_match_v1".to_string(),
            allowed_techniques: vec!["GOLISH-NDAY".to_string()],
            enrichment_required: false,
            evidence_ids: vec![9],
        };
        let manifest = |item: CandidateManifestItemRow| CandidateManifestRow {
            operation_id: item.work_item.operation_id,
            scope_snapshot_id: item.work_item.scope_snapshot_id,
            wave_run_id: Uuid::new_v4(),
            wave_unit_id: item.work_item.wave_unit_id,
            organization_id: item.work_item.organization_id,
            items: vec![item],
        };
        let base_hash = canonical_manifest_hash(&manifest(base_item.clone()));
        let mut observation_drift = base_item.clone();
        observation_drift.observation["template_id"] = serde_json::json!("two");
        assert_ne!(
            base_hash,
            canonical_manifest_hash(&manifest(observation_drift))
        );
        let mut declared_hash_drift = base_item;
        declared_hash_drift.observation_hash = "sha256:two".to_string();
        assert_ne!(
            base_hash,
            canonical_manifest_hash(&manifest(declared_hash_drift))
        );
    }

    fn surface_observation(target_id: Uuid, origin: &str) -> SeedAttackObservation {
        let (target_type_at_time, target_value_at_time, target_identity_hash) =
            frozen_target_snapshot(origin);
        let observation = serde_json::json!({
            "schema": "surface_analysis_v1",
            "target_id": target_id,
            "target_identity": {
                "type": target_type_at_time,
                "value": target_value_at_time,
                "sha256": target_identity_hash,
            },
            "formulaic_coverage": [],
            "upstream_query_required": true,
        });
        SeedAttackObservation {
            work_item_key: format!("surface_analysis:{target_identity_hash}"),
            target_live_id: Some(target_id),
            target_type_at_time,
            target_value_at_time,
            target_identity_hash,
            technique: SURFACE_ANALYSIS_TECHNIQUE.to_string(),
            observation_hash: sha256_prefixed(
                serde_json::to_vec(&observation)
                    .expect("serialize surface")
                    .as_slice(),
            ),
            observation,
            source_fact_delta_id: None,
            delta_kind: None,
            observation_kind: "surface_analysis_v1".to_string(),
            allowed_techniques: FORMULAIC_TECHNIQUES
                .iter()
                .map(|value| (*value).to_string())
                .collect(),
            enrichment_required: false,
            evidence_ids: vec![41],
        }
    }

    fn enumeration_support_fixture() -> (
        ExactEnumerationLineage,
        EnumerationSupportEvidenceRow,
        EnumerationDirectoryEntryRow,
        SeedAttackObservation,
    ) {
        let operation_id = Uuid::new_v4();
        let scope_snapshot_id = Uuid::new_v4();
        let organization_id = Uuid::new_v4();
        let handoff_id = Uuid::new_v4();
        let stage_execution_id = Uuid::new_v4();
        let source_stage_run_unit_id = Uuid::new_v4();
        let target_id = Uuid::new_v4();
        let started_at = Utc::now() - chrono::Duration::minutes(2);
        let gate_passed_at = Utc::now() - chrono::Duration::minutes(1);
        let project_path = "/tmp/exact-project".to_string();
        (
            ExactEnumerationLineage {
                operation_id,
                scope_snapshot_id,
                organization_id,
                handoff_id,
                stage_execution_id,
                source_stage_run_unit_id,
                project_path: project_path.clone(),
                started_at,
                gate_passed_at,
            },
            EnumerationSupportEvidenceRow {
                operation_id,
                scope_snapshot_id,
                organization_id,
                handoff_id,
                stage_execution_id,
                source_stage_run_unit_id,
                evidence_id: 20,
                target_id,
                project_path: project_path.clone(),
                evidence_asset: "https://example.test:443".to_string(),
                evidence_technique: "GOLISH-ENUM-DIR".to_string(),
                evidence_outcome: "found".to_string(),
                tool_name: "route_probe_paths".to_string(),
                evidence_kind: "dir_entry".to_string(),
                created_at: gate_passed_at - chrono::Duration::seconds(10),
            },
            EnumerationDirectoryEntryRow {
                source_evidence_id: 20,
                directory_entry_id: Uuid::new_v4(),
                target_id,
                project_path,
                url: "https://example.test/README.md".to_string(),
                status_code: Some(200),
                content_length: Some(512),
                content_type: Some("text/markdown".to_string()),
                source_tool: "route_probe".to_string(),
                created_at: gate_passed_at - chrono::Duration::seconds(20),
            },
            surface_observation(target_id, "https://example.test:443"),
        )
    }

    #[test]
    fn exact_enumeration_support_is_frozen_on_surface_and_typed_directory_item() {
        let (lineage, evidence, directory, surface) = enumeration_support_fixture();
        let surface_key = surface.work_item_key.clone();
        let mut observations = vec![surface];

        let support = merge_exact_enumeration_support(
            &lineage,
            &[evidence],
            std::slice::from_ref(&directory),
            &mut observations,
        )
        .expect("exact predecessor support must materialize");

        assert_eq!(observations.len(), 2);
        assert_eq!(support.get(&surface_key), Some(&vec![20]));
        assert!(observations[0].evidence_ids.contains(&20));
        let directory_item = observations
            .iter()
            .find(|item| item.observation_kind == DIRECTORY_ENTRY_OBSERVATION_SCHEMA)
            .expect("2xx non-root directory observation");
        assert_eq!(support.get(&directory_item.work_item_key), Some(&vec![20]));
        assert_eq!(directory_item.technique, "WSTG-INFO");
        assert_eq!(directory_item.allowed_techniques, vec!["WSTG-INFO"]);
        assert_eq!(directory_item.target_live_id, Some(directory.target_id));
        assert_eq!(
            directory_item.observation["directory_entry_id"],
            directory.directory_entry_id.to_string()
        );
        assert_eq!(directory_item.observation["source_evidence_id"], 20);
        assert!(directory_item.observation["directory_entry_row_sha256"]
            .as_str()
            .is_some_and(|value| value.starts_with("sha256:")));
    }

    #[test]
    fn enumeration_support_lineage_and_owner_drift_fail_closed() {
        let (lineage, evidence, directory, surface) = enumeration_support_fixture();
        for mutate in [
            |row: &mut EnumerationSupportEvidenceRow| row.operation_id = Uuid::new_v4(),
            |row: &mut EnumerationSupportEvidenceRow| row.scope_snapshot_id = Uuid::new_v4(),
            |row: &mut EnumerationSupportEvidenceRow| row.organization_id = Uuid::new_v4(),
            |row: &mut EnumerationSupportEvidenceRow| row.handoff_id = Uuid::new_v4(),
            |row: &mut EnumerationSupportEvidenceRow| row.stage_execution_id = Uuid::new_v4(),
            |row: &mut EnumerationSupportEvidenceRow| row.source_stage_run_unit_id = Uuid::new_v4(),
        ] {
            let mut drifted = evidence.clone();
            mutate(&mut drifted);
            let mut observations = vec![surface.clone()];
            assert!(merge_exact_enumeration_support(
                &lineage,
                &[drifted],
                std::slice::from_ref(&directory),
                &mut observations,
            )
            .is_err());
        }

        let mut project_drift = evidence.clone();
        project_drift.project_path = "/tmp/foreign-project".to_string();
        let mut observations = vec![surface.clone()];
        assert!(merge_exact_enumeration_support(
            &lineage,
            &[project_drift],
            std::slice::from_ref(&directory),
            &mut observations,
        )
        .is_err());

        let mut foreign_target = evidence;
        foreign_target.target_id = Uuid::new_v4();
        let mut observations = vec![surface];
        let support = merge_exact_enumeration_support(
            &lineage,
            &[foreign_target],
            &[directory],
            &mut observations,
        )
        .expect("foreign target is ignored, never admitted");
        assert!(support.is_empty());
        assert_eq!(observations.len(), 1);
        assert_eq!(observations[0].evidence_ids, vec![41]);
    }

    #[test]
    fn directory_support_requires_exact_target_origin_time_and_2xx_route_probe() {
        let (lineage, evidence, directory, surface) = enumeration_support_fixture();
        let cases = [
            {
                let mut row = directory.clone();
                row.target_id = Uuid::new_v4();
                row
            },
            {
                let mut row = directory.clone();
                row.project_path = "/tmp/foreign-project".to_string();
                row
            },
            {
                let mut row = directory.clone();
                row.url = "https://foreign.example/README.md".to_string();
                row
            },
            {
                let mut row = directory.clone();
                row.url = "https://example.test/".to_string();
                row
            },
            {
                let mut row = directory.clone();
                row.status_code = Some(403);
                row
            },
            {
                let mut row = directory.clone();
                row.source_tool = "arbitrary_tool".to_string();
                row
            },
            {
                let mut row = directory.clone();
                row.created_at = lineage.started_at - chrono::Duration::seconds(1);
                row
            },
        ];
        for row in cases {
            let mut observations = vec![surface.clone()];
            let support = merge_exact_enumeration_support(
                &lineage,
                std::slice::from_ref(&evidence),
                &[row],
                &mut observations,
            )
            .expect("invalid directory row is excluded");
            assert_eq!(observations.len(), 1);
            assert_eq!(support.len(), 1, "surface support remains exact");
        }
    }

    #[test]
    fn missing_or_ambiguous_atomic_predecessor_fails_closed() {
        assert!(require_exact_predecessor::<()>(&[]).is_err());
        assert!(require_exact_predecessor(&[(), ()]).is_err());
        assert!(require_exact_predecessor(&[()]).is_ok());
    }
}
