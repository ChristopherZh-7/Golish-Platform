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

#[derive(Debug, thiserror::Error)]
pub enum CanonicalFactRefError {
    #[error("canonical fact rejected: {code}")]
    Rejected { code: &'static str },
    #[error("canonical fact query failed: {0}")]
    Sqlx(#[from] sqlx::Error),
}

type RawProjection = (Value, Uuid, DateTime<Utc>, Vec<i64>);

fn projection(
    key: &CanonicalFactKey,
    source_table: &'static str,
    (content, organization_id, observed_at, evidence_ids): RawProjection,
) -> CanonicalFactProjectionRow {
    let content_sha256 =
        Sha256::digest(super::operation_scope_decisions::canonical_json(&content).as_bytes())
            .iter()
            .map(|byte| format!("{byte:02x}"))
            .collect();
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
    expected_organization_id: Uuid,
    project_path_at_freeze: &str,
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
            if run_id != &operation_id.to_string() {
                return Err(CanonicalFactRefError::Rejected {
                    code: "canonical_fact_foreign_operation",
                });
            }
            sqlx::query_as::<_, RawProjection>(
                r#"SELECT to_jsonb(outcome), outcome.organization_id,
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
    if keys.len() > MAX_CANONICAL_REFS {
        return Err(CanonicalFactRefError::Rejected {
            code: "canonical_fact_ref_limit_exceeded",
        });
    }
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
            expected_organization_id,
            project_path_at_freeze,
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
}
