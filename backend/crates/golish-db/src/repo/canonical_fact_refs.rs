//! Closed canonical-fact projection contract for bounded StageHandoff payloads.
//!
//! This repository does not own a new table. It validates and projects rows
//! from the closed source catalog below; target-owned structured findings are
//! included, while prose-only claims, memory, and KG rows remain absent.

use chrono::{DateTime, Utc};
use serde::{Deserialize, Serialize};
use serde_json::Value;
use sha2::{Digest, Sha256};
use sqlx::PgConnection;
use uuid::Uuid;

pub const PROJECTION_NAME: &str = "canonical_fact_refs";
pub const CANONICAL_SOURCE_TABLES: &[&str] = &[
    "organizations",
    "targets",
    "target_assets",
    "dns_records",
    "api_endpoints",
    "directory_entries",
    "js_analysis_results",
    "fingerprints",
    "technique_outcomes",
    "attack_candidate_work_items",
    "findings",
];
pub const MAX_CANONICAL_REFS: usize = 256;
pub const MAX_TYPED_CLAIMS: usize = 128;
pub const MAX_EVIDENCE_IDS: usize = 1024;
pub const MAX_CANONICAL_PAYLOAD_BYTES: usize = 256 * 1024;

#[derive(Debug, Clone, Serialize, Deserialize, PartialEq, Eq)]
#[serde(tag = "kind", rename_all = "snake_case")]
pub enum CanonicalFactKey {
    Organization {
        organization_id: Uuid,
    },
    Target {
        target_id: Uuid,
    },
    TargetAsset {
        target_asset_id: Uuid,
    },
    DnsRecord {
        organization_id: Uuid,
        domain: String,
        record_type: String,
        value: String,
    },
    ApiEndpoint {
        api_endpoint_id: Uuid,
    },
    DirectoryEntry {
        directory_entry_id: Uuid,
    },
    JsAnalysisResult {
        js_analysis_result_id: Uuid,
    },
    Fingerprint {
        fingerprint_id: Uuid,
    },
    TechniqueOutcome {
        organization_id: Uuid,
        run_id: String,
        asset: String,
        technique: String,
    },
    TechniqueOutcomeSet {
        organization_id: Uuid,
        run_id: String,
        stage: String,
        terminal_cell_count: u32,
        outcome_set_sha256: String,
    },
    AttackCandidateWorkItem {
        work_item_id: Uuid,
    },
    Finding {
        finding_id: Uuid,
    },
}

#[derive(Debug, Clone, Serialize, Deserialize, PartialEq, Eq)]
pub struct CanonicalFactRef {
    pub key: CanonicalFactKey,
    pub organization_id: Uuid,
    pub observed_at: DateTime<Utc>,
    pub content_sha256: String,
    pub evidence_ids: Vec<i64>,
}

/// Uniform row shape used by catalog-specific SELECTs before key decoding.
#[derive(Debug, Clone, sqlx::FromRow, Serialize, Deserialize, PartialEq, Eq)]
pub struct CanonicalFactProjectionRow {
    pub source_table: String,
    pub source_key: Value,
    pub organization_id: Uuid,
    pub observed_at: DateTime<Utc>,
    pub content_sha256: String,
    pub evidence_ids: Vec<i64>,
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct TechniqueOutcomeSetMember {
    pub organization_id: Uuid,
    pub run_id: String,
    pub asset: String,
    pub technique: String,
    pub outcome: String,
    pub observed_at: DateTime<Utc>,
    pub evidence_ids: Vec<i64>,
    pub content: Value,
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct TechniqueOutcomeSetAttestation {
    pub terminal_cell_count: u32,
    pub outcome_set_sha256: String,
    pub content_sha256: String,
    pub observed_at: DateTime<Utc>,
    pub evidence_ids: Vec<i64>,
}

#[derive(Debug, sqlx::FromRow)]
struct TechniqueOutcomeSetRawRow {
    organization_id: Uuid,
    run_id: String,
    asset: String,
    technique: String,
    outcome: String,
    observed_at: DateTime<Utc>,
    evidence_ids: Vec<i64>,
    content: Value,
}

#[derive(Debug, thiserror::Error)]
pub enum CanonicalFactRefError {
    #[error("canonical fact rejected: {code}")]
    Rejected { code: &'static str },
    #[error("canonical fact query failed: {0}")]
    Sqlx(#[from] sqlx::Error),
}

type RawProjection = (Value, Uuid, DateTime<Utc>, Vec<i64>);

fn sha256_canonical(value: &Value) -> String {
    Sha256::digest(super::operation_scope_decisions::canonical_json(value).as_bytes())
        .iter()
        .map(|byte| format!("{byte:02x}"))
        .collect()
}

fn normalized_terminal_outcome(outcome: &str) -> Option<&'static str> {
    match outcome {
        "found" => Some("found"),
        "empty" => Some("checked_empty"),
        "blocked" => Some("blocked"),
        "not_applicable" => Some("not_applicable"),
        _ => None,
    }
}

pub fn technique_outcome_set_attestation(
    stage: &str,
    organization_id: Uuid,
    run_id: &str,
    members: &[TechniqueOutcomeSetMember],
) -> Result<TechniqueOutcomeSetAttestation, CanonicalFactRefError> {
    technique_outcome_set_attestation_at(stage, organization_id, run_id, members, None)
}

pub fn technique_outcome_set_attestation_at(
    stage: &str,
    organization_id: Uuid,
    run_id: &str,
    members: &[TechniqueOutcomeSetMember],
    empty_observed_at: Option<DateTime<Utc>>,
) -> Result<TechniqueOutcomeSetAttestation, CanonicalFactRefError> {
    if stage != "vuln_triage"
        || run_id.trim().is_empty()
        || (members.is_empty() && empty_observed_at.is_none())
    {
        return Err(CanonicalFactRefError::Rejected {
            code: "technique_outcome_set_identity_invalid",
        });
    }
    let mut ordered = members.iter().collect::<Vec<_>>();
    ordered.sort_by(|left, right| {
        left.asset
            .cmp(&right.asset)
            .then_with(|| left.technique.cmp(&right.technique))
    });
    let mut identity_cells = Vec::with_capacity(ordered.len());
    let mut contents = Vec::with_capacity(ordered.len());
    let mut evidence_ids = std::collections::BTreeSet::new();
    let mut observed_at = None;
    let mut previous_key: Option<(&str, &str)> = None;
    for member in ordered {
        let normalized = normalized_terminal_outcome(&member.outcome).ok_or(
            CanonicalFactRefError::Rejected {
                code: "technique_outcome_set_non_terminal",
            },
        )?;
        let key = (member.asset.as_str(), member.technique.as_str());
        if member.organization_id != organization_id
            || member.run_id != run_id
            || member.asset.trim().is_empty()
            || member.technique.trim().is_empty()
        {
            return Err(CanonicalFactRefError::Rejected {
                code: "technique_outcome_set_member_identity_invalid",
            });
        }
        if member.evidence_ids.is_empty()
            || member.evidence_ids.iter().any(|id| *id <= 0)
            || member
                .evidence_ids
                .iter()
                .collect::<std::collections::BTreeSet<_>>()
                .len()
                != member.evidence_ids.len()
        {
            return Err(CanonicalFactRefError::Rejected {
                code: "technique_outcome_set_member_evidence_invalid",
            });
        }
        if !member.content.is_object() {
            return Err(CanonicalFactRefError::Rejected {
                code: "technique_outcome_set_member_content_invalid",
            });
        }
        if previous_key == Some(key) {
            return Err(CanonicalFactRefError::Rejected {
                code: "technique_outcome_set_duplicate_cell",
            });
        }
        previous_key = Some(key);
        evidence_ids.extend(member.evidence_ids.iter().copied());
        observed_at = Some(
            observed_at
                .map(|current: DateTime<Utc>| current.max(member.observed_at))
                .unwrap_or(member.observed_at),
        );
        identity_cells.push(serde_json::json!({
            "asset": member.asset,
            "technique": member.technique,
            "state": normalized,
        }));
        contents.push(member.content.clone());
    }
    let terminal_cell_count =
        u32::try_from(identity_cells.len()).map_err(|_| CanonicalFactRefError::Rejected {
            code: "technique_outcome_set_too_large",
        })?;
    let outcome_set_sha256 = sha256_canonical(&serde_json::json!({
        "schema": "technique_outcome_identity_set_v1",
        "stage": stage,
        "organization_id": organization_id,
        "run_id": run_id,
        "cells": identity_cells,
    }));
    let content_sha256 = sha256_canonical(&serde_json::json!({
        "schema": "technique_outcome_content_set_v1",
        "stage": stage,
        "organization_id": organization_id,
        "run_id": run_id,
        "outcomes": contents,
    }));
    Ok(TechniqueOutcomeSetAttestation {
        terminal_cell_count,
        outcome_set_sha256,
        content_sha256,
        observed_at: observed_at
            .or(empty_observed_at)
            .expect("outcome set has a member or explicit empty-set observation time"),
        evidence_ids: evidence_ids.into_iter().collect(),
    })
}

async fn resolve_technique_outcome_set_at(
    connection: &mut PgConnection,
    operation_id: Uuid,
    expected_organization_id: Uuid,
    project_path_at_freeze: &str,
    freshness_floor: DateTime<Utc>,
    observation_ceiling: DateTime<Utc>,
    key: &CanonicalFactKey,
) -> Result<Option<CanonicalFactProjectionRow>, CanonicalFactRefError> {
    let CanonicalFactKey::TechniqueOutcomeSet {
        organization_id,
        run_id,
        stage,
        terminal_cell_count,
        outcome_set_sha256,
    } = key
    else {
        return Err(CanonicalFactRefError::Rejected {
            code: "technique_outcome_set_key_required",
        });
    };
    if *organization_id != expected_organization_id {
        return Err(CanonicalFactRefError::Rejected {
            code: "canonical_fact_foreign_organization",
        });
    }
    if run_id != &operation_id.to_string() || stage != "vuln_triage" {
        return Err(CanonicalFactRefError::Rejected {
            code: "canonical_fact_foreign_operation",
        });
    }
    if observation_ceiling < freshness_floor {
        return Err(CanonicalFactRefError::Rejected {
            code: "technique_outcome_set_time_window_invalid",
        });
    }
    let rows = sqlx::query_as::<_, TechniqueOutcomeSetRawRow>(
        r#"SELECT outcome.organization_id, outcome.run_id, outcome.asset,
                  outcome.technique, outcome.outcome,
                  outcome.collected_at AS observed_at, outcome.evidence_ids,
                  to_jsonb(outcome.*) AS content
             FROM technique_outcomes AS outcome
             JOIN organizations AS org ON org.id=outcome.organization_id
            WHERE outcome.organization_id=$1 AND outcome.run_id=$2
              AND org.project_path=$3
              AND outcome.collected_at >= $4
              AND outcome.collected_at <= $5
              AND outcome.updated_at <= $5
            ORDER BY outcome.asset, outcome.technique
            FOR SHARE OF outcome, org"#,
    )
    .bind(organization_id)
    .bind(run_id)
    .bind(project_path_at_freeze)
    .bind(freshness_floor)
    .bind(observation_ceiling)
    .fetch_all(&mut *connection)
    .await?;
    let members = rows
        .into_iter()
        .map(|row| TechniqueOutcomeSetMember {
            organization_id: row.organization_id,
            run_id: row.run_id,
            asset: row.asset,
            technique: row.technique,
            outcome: row.outcome,
            observed_at: row.observed_at,
            evidence_ids: row.evidence_ids,
            content: row.content,
        })
        .collect::<Vec<_>>();
    let attestation = technique_outcome_set_attestation_at(
        stage,
        expected_organization_id,
        run_id,
        &members,
        Some(observation_ceiling),
    )?;
    if attestation.terminal_cell_count != *terminal_cell_count
        || attestation.outcome_set_sha256 != *outcome_set_sha256
        || attestation.evidence_ids.len() > MAX_EVIDENCE_IDS
    {
        return Err(CanonicalFactRefError::Rejected {
            code: "technique_outcome_set_attestation_mismatch",
        });
    }
    Ok(Some(CanonicalFactProjectionRow {
        source_table: "technique_outcomes".to_string(),
        source_key: serde_json::to_value(key).expect("canonical fact key is serializable"),
        organization_id: expected_organization_id,
        observed_at: attestation.observed_at,
        content_sha256: attestation.content_sha256,
        evidence_ids: attestation.evidence_ids,
    }))
}

fn projection(
    key: &CanonicalFactKey,
    source_table: &'static str,
    (content, organization_id, observed_at, evidence_ids): RawProjection,
) -> CanonicalFactProjectionRow {
    let content_sha256 = sha256_canonical(&content);
    CanonicalFactProjectionRow {
        source_table: source_table.to_string(),
        source_key: serde_json::to_value(key).expect("canonical fact key is serializable"),
        organization_id,
        observed_at,
        content_sha256,
        evidence_ids,
    }
}

async fn resolve_one(
    connection: &mut PgConnection,
    operation_id: Uuid,
    operation_chat_session_key: Option<&str>,
    expected_organization_id: Uuid,
    project_path_at_freeze: &str,
    technique_outcome_set_window: Option<(DateTime<Utc>, DateTime<Utc>)>,
    key: &CanonicalFactKey,
) -> Result<Option<CanonicalFactProjectionRow>, CanonicalFactRefError> {
    let row = match key {
        CanonicalFactKey::Organization { organization_id } => {
            if *organization_id != expected_organization_id {
                return Err(CanonicalFactRefError::Rejected {
                    code: "canonical_fact_foreign_organization",
                });
            }
            sqlx::query_as::<_, RawProjection>(
                r#"SELECT to_jsonb(org), org.id, org.updated_at, '{}'::BIGINT[]
                     FROM organizations AS org
                    WHERE org.id=$1 AND org.project_path=$2
                    FOR SHARE"#,
            )
            .bind(organization_id)
            .bind(project_path_at_freeze)
            .fetch_optional(&mut *connection)
            .await?
            .map(|row| projection(key, "organizations", row))
        }
        CanonicalFactKey::Target { target_id } => sqlx::query_as::<_, RawProjection>(
            r#"SELECT to_jsonb(target), target.organization_id, target.updated_at,
                      '{}'::BIGINT[]
                 FROM targets AS target
                WHERE target.id=$1 AND target.organization_id=$2
                  AND target.project_path=$3 AND target.scope='in'
                FOR SHARE"#,
        )
        .bind(target_id)
        .bind(expected_organization_id)
        .bind(project_path_at_freeze)
        .fetch_optional(&mut *connection)
        .await?
        .map(|row| projection(key, "targets", row)),
        CanonicalFactKey::TargetAsset { target_asset_id } => sqlx::query_as::<_, RawProjection>(
            r#"SELECT to_jsonb(asset), target.organization_id, asset.updated_at,
                          '{}'::BIGINT[]
                     FROM target_assets AS asset
                     JOIN targets AS target ON target.id=asset.target_id
                    WHERE asset.id=$1 AND target.organization_id=$2
                      AND target.project_path=$3 AND asset.project_path=$3
                      AND target.scope='in'
                    FOR SHARE OF asset, target"#,
        )
        .bind(target_asset_id)
        .bind(expected_organization_id)
        .bind(project_path_at_freeze)
        .fetch_optional(&mut *connection)
        .await?
        .map(|row| projection(key, "target_assets", row)),
        CanonicalFactKey::DnsRecord {
            organization_id,
            domain,
            record_type,
            value,
        } => {
            if *organization_id != expected_organization_id {
                return Err(CanonicalFactRefError::Rejected {
                    code: "canonical_fact_foreign_organization",
                });
            }
            sqlx::query_as::<_, RawProjection>(
                r#"SELECT to_jsonb(record), target.organization_id, record.created_at,
                          '{}'::BIGINT[]
                     FROM dns_records AS record
                     JOIN targets AS target ON target.id=record.target_id
                    WHERE target.organization_id=$1 AND target.project_path=$2
                      AND record.project_path=$2 AND target.scope='in'
                      AND record.name=$3 AND record.record_type=$4 AND record.value=$5
                    FOR SHARE OF record, target"#,
            )
            .bind(expected_organization_id)
            .bind(project_path_at_freeze)
            .bind(domain)
            .bind(record_type)
            .bind(value)
            .fetch_optional(&mut *connection)
            .await?
            .map(|row| projection(key, "dns_records", row))
        }
        CanonicalFactKey::ApiEndpoint { api_endpoint_id } => sqlx::query_as::<_, RawProjection>(
            r#"SELECT to_jsonb(endpoint), target.organization_id, endpoint.updated_at,
                          '{}'::BIGINT[]
                     FROM api_endpoints AS endpoint
                     JOIN targets AS target ON target.id=endpoint.target_id
                    WHERE endpoint.id=$1 AND target.organization_id=$2
                      AND target.project_path=$3 AND endpoint.project_path=$3
                      AND target.scope='in'
                    FOR SHARE OF endpoint, target"#,
        )
        .bind(api_endpoint_id)
        .bind(expected_organization_id)
        .bind(project_path_at_freeze)
        .fetch_optional(&mut *connection)
        .await?
        .map(|row| projection(key, "api_endpoints", row)),
        CanonicalFactKey::DirectoryEntry { directory_entry_id } => {
            sqlx::query_as::<_, RawProjection>(
                r#"SELECT to_jsonb(entry), target.organization_id, entry.updated_at,
                          '{}'::BIGINT[]
                     FROM directory_entries AS entry
                     JOIN targets AS target ON target.id=entry.target_id
                    WHERE entry.id=$1 AND target.organization_id=$2
                      AND target.project_path=$3 AND entry.project_path=$3
                      AND target.scope='in'
                    FOR SHARE OF entry, target"#,
            )
            .bind(directory_entry_id)
            .bind(expected_organization_id)
            .bind(project_path_at_freeze)
            .fetch_optional(&mut *connection)
            .await?
            .map(|row| projection(key, "directory_entries", row))
        }
        CanonicalFactKey::JsAnalysisResult {
            js_analysis_result_id,
        } => sqlx::query_as::<_, RawProjection>(
            r#"SELECT to_jsonb(result), target.organization_id, result.updated_at,
                      '{}'::BIGINT[]
                 FROM js_analysis_results AS result
                 JOIN targets AS target ON target.id=result.target_id
                WHERE result.id=$1 AND target.organization_id=$2
                  AND target.project_path=$3 AND result.project_path=$3
                  AND target.scope='in'
                FOR SHARE OF result, target"#,
        )
        .bind(js_analysis_result_id)
        .bind(expected_organization_id)
        .bind(project_path_at_freeze)
        .fetch_optional(&mut *connection)
        .await?
        .map(|row| projection(key, "js_analysis_results", row)),
        CanonicalFactKey::Fingerprint { fingerprint_id } => sqlx::query_as::<_, RawProjection>(
            r#"SELECT to_jsonb(fingerprint), target.organization_id,
                          fingerprint.updated_at, '{}'::BIGINT[]
                     FROM fingerprints AS fingerprint
                     JOIN targets AS target ON target.id=fingerprint.target_id
                    WHERE fingerprint.id=$1 AND target.organization_id=$2
                      AND target.project_path=$3 AND fingerprint.project_path=$3
                      AND target.scope='in'
                    FOR SHARE OF fingerprint, target"#,
        )
        .bind(fingerprint_id)
        .bind(expected_organization_id)
        .bind(project_path_at_freeze)
        .fetch_optional(&mut *connection)
        .await?
        .map(|row| projection(key, "fingerprints", row)),
        CanonicalFactKey::TechniqueOutcome {
            organization_id,
            run_id,
            asset,
            technique,
        } => {
            if *organization_id != expected_organization_id {
                return Err(CanonicalFactRefError::Rejected {
                    code: "canonical_fact_foreign_organization",
                });
            }
            if run_id != &operation_id.to_string()
                && operation_chat_session_key != Some(run_id.as_str())
            {
                return Err(CanonicalFactRefError::Rejected {
                    code: "canonical_fact_foreign_operation",
                });
            }
            sqlx::query_as::<_, RawProjection>(
                r#"SELECT to_jsonb(outcome.*), outcome.organization_id,
                          outcome.collected_at, outcome.evidence_ids
                     FROM technique_outcomes AS outcome
                     JOIN organizations AS org ON org.id=outcome.organization_id
                    WHERE outcome.organization_id=$1 AND outcome.run_id=$2
                      AND outcome.asset=$3 AND outcome.technique=$4
                      AND outcome.collected_at IS NOT NULL AND org.project_path=$5
                    FOR SHARE OF outcome, org"#,
            )
            .bind(organization_id)
            .bind(run_id)
            .bind(asset)
            .bind(technique)
            .bind(project_path_at_freeze)
            .fetch_optional(&mut *connection)
            .await?
            .map(|row| projection(key, "technique_outcomes", row))
        }
        CanonicalFactKey::TechniqueOutcomeSet { .. } => match technique_outcome_set_window {
            Some((freshness_floor, observation_ceiling)) => {
                resolve_technique_outcome_set_at(
                    connection,
                    operation_id,
                    expected_organization_id,
                    project_path_at_freeze,
                    freshness_floor,
                    observation_ceiling,
                    key,
                )
                .await?
            }
            None => {
                return Err(CanonicalFactRefError::Rejected {
                    code: "technique_outcome_set_final_seal_only",
                });
            }
        },
        CanonicalFactKey::AttackCandidateWorkItem { work_item_id } => {
            sqlx::query_as::<_, RawProjection>(
                r#"SELECT jsonb_build_object(
                              'work_item_id', item.id,
                              'seed_id', item.seed_id,
                              'wave_unit_id', item.wave_unit_id,
                              'operation_id', item.operation_id,
                              'scope_snapshot_id', item.scope_snapshot_id,
                              'organization_id', item.organization_id,
                              'target_live_id', item.target_live_id,
                              'target_type_at_time', item.target_type_at_time,
                              'target_value_at_time', item.target_value_at_time,
                              'target_identity_hash', item.target_identity_hash,
                              'work_item_key', item.work_item_key,
                              'technique', seed.technique,
                              'observation_hash', seed.observation_hash,
                              'manifest_hash', wave.manifest_hash,
                              'manifest_count', wave.manifest_count,
                              'manifest_frozen_at', wave.manifest_frozen_at
                           ),
                          item.organization_id,
                          item.created_at,
                          ARRAY(
                              SELECT evidence_id FROM (
                                  SELECT evidence_id
                                    FROM attack_candidate_seed_evidence
                                   WHERE seed_id=item.seed_id
                                  UNION
                                  SELECT evidence_id
                                    FROM attack_candidate_work_item_evidence
                                   WHERE work_item_id=item.id
                                     AND role IN ('observation','support')
                              ) evidence ORDER BY evidence_id
                          )::BIGINT[]
                     FROM attack_candidate_work_items AS item
                     JOIN attack_candidate_seeds AS seed ON seed.id=item.seed_id
                     JOIN attack_wave_units AS wave ON wave.id=item.wave_unit_id
                     JOIN organizations AS org ON org.id=item.organization_id
                    WHERE item.id=$1 AND item.operation_id=$2
                      AND item.organization_id=$3 AND org.project_path=$4
                      AND wave.manifest_frozen_at IS NOT NULL
                      AND BTRIM(COALESCE(wave.manifest_hash, '')) <> ''
                    FOR SHARE OF item,seed,wave,org"#,
            )
            .bind(work_item_id)
            .bind(operation_id)
            .bind(expected_organization_id)
            .bind(project_path_at_freeze)
            .fetch_optional(&mut *connection)
            .await?
            .map(|row| projection(key, "attack_candidate_work_items", row))
        }
        CanonicalFactKey::Finding { finding_id } => sqlx::query_as::<_, RawProjection>(
            r#"SELECT to_jsonb(finding), target.organization_id, finding.updated_at,
                      '{}'::BIGINT[]
                 FROM findings AS finding
                 JOIN targets AS target ON target.id=finding.target_id
                WHERE finding.id=$1 AND target.organization_id=$2
                  AND target.project_path=$3 AND finding.project_path=$3
                  AND target.scope='in'
                FOR SHARE OF finding, target"#,
        )
        .bind(finding_id)
        .bind(expected_organization_id)
        .bind(project_path_at_freeze)
        .fetch_optional(&mut *connection)
        .await?
        .map(|row| projection(key, "findings", row)),
    };
    Ok(row)
}

async fn load_operation_chat_session_key(
    connection: &mut PgConnection,
    operation_id: Uuid,
) -> Result<Option<String>, CanonicalFactRefError> {
    Ok(sqlx::query_scalar::<_, Option<String>>(
        r#"SELECT session.chat_session_key
             FROM tasks AS task
             JOIN sessions AS session ON session.id=task.session_id
            WHERE task.id=$1
            FOR SHARE OF task, session"#,
    )
    .bind(operation_id)
    .fetch_optional(&mut *connection)
    .await?
    .flatten()
    .filter(|key| !key.trim().is_empty()))
}

fn fact_delta_observation_is_valid(
    delta_kind: &str,
    observed_at: DateTime<Utc>,
    attempt_started_at: DateTime<Utc>,
    attempt_terminal_at: DateTime<Utc>,
) -> bool {
    if attempt_terminal_at < attempt_started_at || observed_at > attempt_terminal_at {
        return false;
    }
    match delta_kind {
        // A refutation deliberately points at an older canonical fact.  The
        // fresh authority is the Attempt-scoped fact_delta evidence, which the
        // caller validates separately; the referenced row must merely be the
        // exact row that existed no later than terminalization.
        "refuted" => true,
        // Created/new-surface rows, and the updated projection of an older
        // subject, must have been written while this Attempt was active.
        "created" | "updated" | "new_surface" => observed_at >= attempt_started_at,
        _ => false,
    }
}

/// Resolve one FactDelta subject through the same closed canonical catalog as
/// StageHandoff, but with delta-specific time semantics.  Refutation may name
/// an older exact fact; created/updated/new-surface must project a row written
/// during the source Attempt.  Future/post-terminal rows always fail closed.
pub async fn resolve_for_fact_delta(
    connection: &mut PgConnection,
    operation_id: Uuid,
    expected_organization_id: Uuid,
    project_path_at_freeze: &str,
    attempt_started_at: DateTime<Utc>,
    attempt_terminal_at: DateTime<Utc>,
    delta_kind: &str,
    key: &CanonicalFactKey,
) -> Result<CanonicalFactRef, CanonicalFactRefError> {
    if !matches!(
        delta_kind,
        "created" | "updated" | "refuted" | "new_surface"
    ) {
        return Err(CanonicalFactRefError::Rejected {
            code: "fact_delta_kind_unsupported",
        });
    }
    if matches!(key, CanonicalFactKey::TechniqueOutcomeSet { .. }) {
        return Err(CanonicalFactRefError::Rejected {
            code: "technique_outcome_set_not_fact_delta_subject",
        });
    }
    let operation_chat_session_key = if matches!(key, CanonicalFactKey::TechniqueOutcome { .. }) {
        load_operation_chat_session_key(connection, operation_id).await?
    } else {
        None
    };
    let row = resolve_one(
        connection,
        operation_id,
        operation_chat_session_key.as_deref(),
        expected_organization_id,
        project_path_at_freeze,
        None,
        key,
    )
    .await?
    .ok_or(CanonicalFactRefError::Rejected {
        code: "canonical_fact_unknown_or_foreign",
    })?;
    if row.organization_id != expected_organization_id {
        return Err(CanonicalFactRefError::Rejected {
            code: "canonical_fact_foreign_organization",
        });
    }
    if !fact_delta_observation_is_valid(
        delta_kind,
        row.observed_at,
        attempt_started_at,
        attempt_terminal_at,
    ) {
        return Err(CanonicalFactRefError::Rejected {
            code: "canonical_fact_delta_time_mismatch",
        });
    }
    Ok(CanonicalFactRef {
        key: key.clone(),
        organization_id: row.organization_id,
        observed_at: row.observed_at,
        content_sha256: row.content_sha256,
        evidence_ids: row.evidence_ids,
    })
}

/// Resolve untrusted key hints through the closed server catalog. Returned
/// timestamp/hash/evidence fields always come from locked repository rows.
pub async fn resolve_for_handoff(
    connection: &mut PgConnection,
    operation_id: Uuid,
    expected_organization_id: Uuid,
    project_path_at_freeze: &str,
    freshness_floor: DateTime<Utc>,
    keys: &[CanonicalFactKey],
) -> Result<Vec<CanonicalFactRef>, CanonicalFactRefError> {
    resolve_for_handoff_with_set_window(
        connection,
        operation_id,
        expected_organization_id,
        project_path_at_freeze,
        freshness_floor,
        None,
        keys,
    )
    .await
}

/// Final-seal-only resolver. Aggregate outcome sets require an explicit,
/// transaction-stable upper bound so response-loss replay sees the same set.
pub async fn resolve_for_final_seal(
    connection: &mut PgConnection,
    operation_id: Uuid,
    expected_organization_id: Uuid,
    project_path_at_freeze: &str,
    freshness_floor: DateTime<Utc>,
    observation_ceiling: DateTime<Utc>,
    keys: &[CanonicalFactKey],
) -> Result<Vec<CanonicalFactRef>, CanonicalFactRefError> {
    resolve_for_handoff_with_set_window(
        connection,
        operation_id,
        expected_organization_id,
        project_path_at_freeze,
        freshness_floor,
        Some((freshness_floor, observation_ceiling)),
        keys,
    )
    .await
}

async fn resolve_for_handoff_with_set_window(
    connection: &mut PgConnection,
    operation_id: Uuid,
    expected_organization_id: Uuid,
    project_path_at_freeze: &str,
    freshness_floor: DateTime<Utc>,
    technique_outcome_set_window: Option<(DateTime<Utc>, DateTime<Utc>)>,
    keys: &[CanonicalFactKey],
) -> Result<Vec<CanonicalFactRef>, CanonicalFactRefError> {
    if keys.len() > MAX_CANONICAL_REFS {
        return Err(CanonicalFactRefError::Rejected {
            code: "canonical_fact_ref_limit_exceeded",
        });
    }
    let operation_chat_session_key = if keys
        .iter()
        .any(|key| matches!(key, CanonicalFactKey::TechniqueOutcome { .. }))
    {
        load_operation_chat_session_key(connection, operation_id).await?
    } else {
        None
    };
    let mut seen = std::collections::BTreeSet::new();
    let mut refs = Vec::with_capacity(keys.len());
    for key in keys {
        let serialized = super::operation_scope_decisions::canonical_json(
            &serde_json::to_value(key).expect("canonical fact key is serializable"),
        );
        if !seen.insert(serialized) {
            return Err(CanonicalFactRefError::Rejected {
                code: "duplicate_canonical_fact_ref",
            });
        }
        let row = resolve_one(
            connection,
            operation_id,
            operation_chat_session_key.as_deref(),
            expected_organization_id,
            project_path_at_freeze,
            technique_outcome_set_window,
            key,
        )
        .await?
        .ok_or(CanonicalFactRefError::Rejected {
            code: "canonical_fact_unknown_or_foreign",
        })?;
        if row.organization_id != expected_organization_id {
            return Err(CanonicalFactRefError::Rejected {
                code: "canonical_fact_foreign_organization",
            });
        }
        // Candidate work items are intentionally frozen from the exact
        // predecessor handoff before the current Unit starts. The final-seal
        // transaction separately proves that every such key belongs to the
        // bound CandidateAcceptance manifest before this stale exemption is
        // reachable.
        if !matches!(
            row.source_table.as_str(),
            "organizations" | "targets" | "attack_candidate_work_items"
        ) && row.observed_at < freshness_floor
        {
            return Err(CanonicalFactRefError::Rejected {
                code: "canonical_fact_stale",
            });
        }
        refs.push(CanonicalFactRef {
            key: key.clone(),
            organization_id: row.organization_id,
            observed_at: row.observed_at,
            content_sha256: row.content_sha256,
            evidence_ids: row.evidence_ids,
        });
    }
    Ok(refs)
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn technique_outcome_set_attestation_is_complete_and_order_independent() {
        let organization_id = Uuid::new_v4();
        let run_id = Uuid::new_v4().to_string();
        let observed_at = Utc::now();
        let members = (0..36)
            .flat_map(|asset_index| {
                let run_id = run_id.clone();
                (0..10).map(move |technique_index| {
                    let asset = format!("https://host-{asset_index}.example:443");
                    let technique = format!("TECH-{technique_index}");
                    let evidence_id = i64::from(asset_index * 10 + technique_index + 1);
                    TechniqueOutcomeSetMember {
                        organization_id,
                        run_id: run_id.clone(),
                        asset: asset.clone(),
                        technique: technique.clone(),
                        outcome: "blocked".to_string(),
                        observed_at,
                        evidence_ids: vec![evidence_id],
                        content: serde_json::json!({
                            "organization_id": organization_id,
                            "run_id": run_id,
                            "asset": asset,
                            "technique": technique,
                            "outcome": "blocked",
                            "evidence_ids": [evidence_id],
                        }),
                    }
                })
            })
            .collect::<Vec<_>>();

        let expected =
            technique_outcome_set_attestation("vuln_triage", organization_id, &run_id, &members)
                .expect("complete set attestation");
        let mut reversed = members;
        reversed.reverse();
        let actual =
            technique_outcome_set_attestation("vuln_triage", organization_id, &run_id, &reversed)
                .expect("reversed set attestation");

        assert_eq!(actual, expected);
        assert_eq!(actual.terminal_cell_count, 360);
        assert_eq!(actual.evidence_ids.len(), 360);
        assert_eq!(actual.outcome_set_sha256.len(), 64);
        assert_eq!(actual.content_sha256.len(), 64);
    }

    #[test]
    fn runtime_memory_repo_contract_canonical_catalog_is_closed_and_bounded() {
        let finding_id = Uuid::new_v4();
        let finding = CanonicalFactKey::Finding { finding_id };
        assert!(
            matches!(finding, CanonicalFactKey::Finding { finding_id: id } if id == finding_id)
        );
        assert_eq!(CANONICAL_SOURCE_TABLES.len(), 11);
        assert!(CANONICAL_SOURCE_TABLES.contains(&"technique_outcomes"));
        assert!(CANONICAL_SOURCE_TABLES.contains(&"attack_candidate_work_items"));
        assert!(CANONICAL_SOURCE_TABLES.contains(&"findings"));
        assert_eq!(MAX_CANONICAL_REFS, 256);
        assert_eq!(MAX_EVIDENCE_IDS, 1024);
        assert_eq!(MAX_CANONICAL_PAYLOAD_BYTES, 256 * 1024);
        assert_eq!(PROJECTION_NAME, "canonical_fact_refs");
        assert_eq!(MAX_TYPED_CLAIMS, 128);
    }

    #[test]
    fn unknown_catalog_kind_is_rejected_by_serde_before_any_repo_query() {
        let unknown = serde_json::from_value::<CanonicalFactKey>(serde_json::json!({
            "kind": "future_model_claim",
            "id": Uuid::new_v4(),
        }));
        assert!(unknown.is_err());
    }

    #[test]
    fn fact_delta_time_policy_allows_old_refutation_but_not_old_creation_or_update() {
        let started_at = Utc::now();
        let terminal_at = started_at + chrono::Duration::seconds(20);
        let old_fact = started_at - chrono::Duration::days(30);
        let during_attempt = started_at + chrono::Duration::seconds(5);
        let after_attempt = terminal_at + chrono::Duration::seconds(1);

        assert!(fact_delta_observation_is_valid(
            "refuted",
            old_fact,
            started_at,
            terminal_at,
        ));
        for kind in ["created", "updated", "new_surface"] {
            assert!(!fact_delta_observation_is_valid(
                kind,
                old_fact,
                started_at,
                terminal_at,
            ));
            assert!(fact_delta_observation_is_valid(
                kind,
                during_attempt,
                started_at,
                terminal_at,
            ));
        }
        assert!(!fact_delta_observation_is_valid(
            "refuted",
            after_attempt,
            started_at,
            terminal_at,
        ));
        assert!(!fact_delta_observation_is_valid(
            "model_invented_kind",
            during_attempt,
            started_at,
            terminal_at,
        ));
    }
}
