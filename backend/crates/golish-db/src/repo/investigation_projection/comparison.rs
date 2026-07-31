//! Whole-record comparison persistence for Investigation rollout.
//!
//! This is the only writer for `investigation_projection_compare_samples`.
//! It accepts two already-complete V1 semantic records, compares their final
//! hashes without inspecting or merging fields, and derives the frozen
//! operation contract from database authority.

use chrono::{DateTime, Utc};
use golish_core::investigation_comparison::{
    compare_whole_records_v1, InvestigationComparisonRecordInputV1,
    InvestigationComparisonRecordV1, WholeRecordComparisonStateV1,
};
use golish_core::investigation_projection::ProjectionEntityV1;
use golish_core::ComparePolicy;
use serde::Deserialize;
use serde_json::{json, Value};
use sqlx::PgPool;
use uuid::Uuid;

use super::types::{InvestigationProjectionError, InvestigationProjectionResult};

const COMPARISON_CONTRACT_INVALID: &str = "INVESTIGATION_COMPARISON_CONTRACT_INVALID";
const COMPARISON_REPLAY_DRIFT: &str = "INVESTIGATION_COMPARISON_REPLAY_DRIFT";
const COMPARISON_HEAD_INVALID: &str = "INVESTIGATION_COMPARISON_HEAD_INVALID";

#[derive(Debug, Clone)]
pub struct CompareAndRecordV1Input {
    pub operation_id: Uuid,
    pub organization_id: Option<Uuid>,
    pub as_of_change_seq: i64,
    pub record_kind: String,
    pub record_key: String,
    pub legacy: Option<InvestigationComparisonRecordV1>,
    pub registry: Option<InvestigationComparisonRecordV1>,
}

#[derive(Debug, Clone, sqlx::FromRow, PartialEq)]
pub struct InvestigationComparisonSampleV1 {
    pub comparison_id: Uuid,
    pub operation_id: Uuid,
    pub project_scope_id: Uuid,
    pub organization_id: Option<Uuid>,
    pub projection_schema_version: i32,
    pub as_of_change_seq: i64,
    pub comparison_contract_version: String,
    pub tool_truth_contract: String,
    pub investigation_contract_version: String,
    pub investigation_rollout_mode: String,
    pub record_kind: String,
    pub record_key: String,
    pub legacy_hash: Option<String>,
    pub registry_hash: Option<String>,
    pub comparison_state: String,
    pub diff_summary: Value,
    pub compared_at: DateTime<Utc>,
}

#[derive(Debug, sqlx::FromRow)]
struct ComparisonAuthorityRow {
    project_scope_id: Option<Uuid>,
    tool_truth_contract: String,
    investigation_contract_version: String,
    investigation_rollout_mode: String,
    projection_schema_version: i32,
    change_seq: i64,
}

fn contract(code: &'static str) -> InvestigationProjectionError {
    InvestigationProjectionError::Contract(code)
}

fn comparison_state(value: WholeRecordComparisonStateV1) -> &'static str {
    match value {
        WholeRecordComparisonStateV1::Match => "match",
        WholeRecordComparisonStateV1::Mismatch => "mismatch",
        WholeRecordComparisonStateV1::Incomplete => "incomplete",
    }
}

#[derive(Debug, Deserialize)]
struct FrozenComparisonRecordEnvelopeV1 {
    legacy: Option<Value>,
    registry: Option<Value>,
}

/// Assemble two independently frozen complete records from the materialized
/// projection entity. A missing, malformed, or semantically invalid side is
/// deliberately returned as absent; the caller will persist `incomplete`.
/// The assembler never copies a field from one side into the other.
pub(super) async fn assemble_frozen_comparison_records_v1(
    pool: &PgPool,
    operation_id: Uuid,
    batch_id: Uuid,
    entity_kind: &str,
    entity_id: &str,
    entity_version: i64,
) -> InvestigationProjectionResult<(
    Option<InvestigationComparisonRecordV1>,
    Option<InvestigationComparisonRecordV1>,
)> {
    let projection_body: Option<Value> = sqlx::query_scalar(
        r#"SELECT projection_body
             FROM investigation_projection_entity_versions
            WHERE operation_id=$1 AND batch_id=$2 AND entity_kind=$3
              AND entity_id=$4 AND entity_version=$5"#,
    )
    .bind(operation_id)
    .bind(batch_id)
    .bind(entity_kind)
    .bind(entity_id)
    .bind(entity_version)
    .fetch_optional(pool)
    .await?;
    let Some(projection_body) = projection_body else {
        return Ok((None, None));
    };
    let Ok(entity) = serde_json::from_value::<ProjectionEntityV1>(projection_body) else {
        return Ok((None, None));
    };
    let Some(envelope) = entity
        .record()
        .canonical_redacted_body()
        .as_value()
        .get("comparison_record_v1")
        .cloned()
        .and_then(|value| serde_json::from_value::<FrozenComparisonRecordEnvelopeV1>(value).ok())
    else {
        return Ok((None, None));
    };
    Ok((
        envelope
            .legacy
            .and_then(|value| {
                serde_json::from_value::<InvestigationComparisonRecordInputV1>(value).ok()
            })
            .and_then(|input| InvestigationComparisonRecordV1::compile(input).ok()),
        envelope
            .registry
            .and_then(|value| {
                serde_json::from_value::<InvestigationComparisonRecordInputV1>(value).ok()
            })
            .and_then(|input| InvestigationComparisonRecordV1::compile(input).ok()),
    ))
}

/// Compare two complete semantic records and append the unique V1 sample.
///
/// A missing side is always persisted as `incomplete`. No per-field diff or
/// fallback representation exists in either the input or the stored summary.
pub async fn compare_and_record_v1(
    pool: &PgPool,
    input: CompareAndRecordV1Input,
) -> InvestigationProjectionResult<InvestigationComparisonSampleV1> {
    if input.as_of_change_seq < 0
        || input.record_kind.trim().is_empty()
        || input.record_key.trim().is_empty()
    {
        return Err(contract(COMPARISON_CONTRACT_INVALID));
    }

    let mut tx = pool.begin().await?;
    let authority = sqlx::query_as::<_, ComparisonAuthorityRow>(
        r#"SELECT state.project_scope_id,state.tool_truth_contract,
                  state.investigation_contract_version,state.investigation_rollout_mode,
                  head.projection_schema_version,head.change_seq
             FROM operation_state state
             JOIN investigation_projection_heads head USING(operation_id)
            WHERE state.operation_id=$1
            FOR SHARE OF state,head"#,
    )
    .bind(input.operation_id)
    .fetch_one(&mut *tx)
    .await?;
    let (_, mode) = crate::repo::investigation_rollout::parse_frozen_pair(
        &authority.investigation_contract_version,
        &authority.investigation_rollout_mode,
    )
    .map_err(|_| contract(COMPARISON_CONTRACT_INVALID))?;
    if mode.policy().compare_policy == ComparePolicy::Off {
        return Err(contract(COMPARISON_CONTRACT_INVALID));
    }
    if authority.change_seq < input.as_of_change_seq {
        return Err(contract(COMPARISON_HEAD_INVALID));
    }
    let project_scope_id = authority
        .project_scope_id
        .ok_or_else(|| contract(COMPARISON_CONTRACT_INVALID))?;
    if let Some(organization_id) = input.organization_id {
        let organization_in_scope: bool = sqlx::query_scalar(
            r#"SELECT EXISTS(
                   SELECT 1
                     FROM operation_org_scope_units unit
                     JOIN operation_org_scope_snapshots snapshot
                       ON snapshot.id=unit.snapshot_id
                    WHERE snapshot.operation_id=$1
                      AND snapshot.project_scope_id=$2
                      AND snapshot.sealed_at IS NOT NULL
                      AND unit.organization_id=$3
               )"#,
        )
        .bind(input.operation_id)
        .bind(project_scope_id)
        .bind(organization_id)
        .fetch_one(&mut *tx)
        .await?;
        if !organization_in_scope {
            return Err(contract(COMPARISON_CONTRACT_INVALID));
        }
    }

    let comparison = compare_whole_records_v1(input.legacy.as_ref(), input.registry.as_ref());
    let state = comparison_state(comparison.state);
    let comparison_identity = format!(
        "comparison_record.v1:{}:{}:{}:{}",
        input.operation_id, input.as_of_change_seq, input.record_kind, input.record_key
    );
    let comparison_id = Uuid::new_v5(&Uuid::NAMESPACE_URL, comparison_identity.as_bytes());
    let diff_summary = json!({
        "schema": "whole_record_comparison.v1",
        "field_fallback": false,
        "legacy_complete": input.legacy.is_some(),
        "registry_complete": input.registry.is_some(),
    });

    if let Some(existing) = sqlx::query_as::<_, InvestigationComparisonSampleV1>(
        r#"SELECT comparison_id,operation_id,project_scope_id,organization_id,
                  projection_schema_version,as_of_change_seq,comparison_contract_version,
                  tool_truth_contract,investigation_contract_version,
                  investigation_rollout_mode,record_kind,record_key,legacy_hash,
                  registry_hash,comparison_state,diff_summary,compared_at
             FROM investigation_projection_compare_samples
            WHERE operation_id=$1 AND as_of_change_seq=$2
              AND record_kind=$3 AND record_key=$4 FOR SHARE"#,
    )
    .bind(input.operation_id)
    .bind(input.as_of_change_seq)
    .bind(&input.record_kind)
    .bind(&input.record_key)
    .fetch_optional(&mut *tx)
    .await?
    {
        if existing.comparison_id != comparison_id
            || existing.project_scope_id != project_scope_id
            || existing.organization_id != input.organization_id
            || existing.projection_schema_version != authority.projection_schema_version
            || existing.tool_truth_contract != authority.tool_truth_contract
            || existing.investigation_contract_version != authority.investigation_contract_version
            || existing.investigation_rollout_mode != authority.investigation_rollout_mode
            || existing.legacy_hash != comparison.legacy_hash
            || existing.registry_hash != comparison.registry_hash
            || existing.comparison_state != state
            || existing.diff_summary != diff_summary
        {
            return Err(contract(COMPARISON_REPLAY_DRIFT));
        }
        tx.commit().await?;
        return Ok(existing);
    }

    let inserted = sqlx::query_as::<_, InvestigationComparisonSampleV1>(
        r#"INSERT INTO investigation_projection_compare_samples(
               comparison_id,operation_id,project_scope_id,organization_id,
               projection_schema_version,as_of_change_seq,tool_truth_contract,
               investigation_contract_version,investigation_rollout_mode,
               record_kind,record_key,legacy_hash,registry_hash,comparison_state,diff_summary
           ) VALUES($1,$2,$3,$4,$5,$6,$7,$8,$9,$10,$11,$12,$13,$14,$15)
           RETURNING comparison_id,operation_id,project_scope_id,organization_id,
                     projection_schema_version,as_of_change_seq,comparison_contract_version,
                     tool_truth_contract,investigation_contract_version,
                     investigation_rollout_mode,record_kind,record_key,legacy_hash,
                     registry_hash,comparison_state,diff_summary,compared_at"#,
    )
    .bind(comparison_id)
    .bind(input.operation_id)
    .bind(project_scope_id)
    .bind(input.organization_id)
    .bind(authority.projection_schema_version)
    .bind(input.as_of_change_seq)
    .bind(&authority.tool_truth_contract)
    .bind(&authority.investigation_contract_version)
    .bind(&authority.investigation_rollout_mode)
    .bind(&input.record_kind)
    .bind(&input.record_key)
    .bind(comparison.legacy_hash)
    .bind(comparison.registry_hash)
    .bind(state)
    .bind(diff_summary)
    .fetch_one(&mut *tx)
    .await?;
    tx.commit().await?;
    Ok(inserted)
}
