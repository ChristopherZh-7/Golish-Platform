//! Typed persistence boundary for Plan A coverage denominators and receipts.
//!
//! Public commands carry stable request identity and frozen source identity,
//! never caller-computed manifests or authority hashes. Every hash written by
//! this module is derived after locking the database-owned parent census.

use chrono::{DateTime, Utc};
use golish_pentest_domain::tool_truth::ToolTruthContract;
use serde::{Deserialize, Serialize};
use sha2::{Digest, Sha256};
use sqlx::{PgPool, Postgres, Transaction};
use uuid::Uuid;

use crate::{DbError, Result};

pub const TABLE_NAME: &str = "capability_execution_receipts";

const MANIFEST_DRIFT: &str = "TOOL_TRUTH_MANIFEST_DRIFT";
const AUTHORITY_STALE: &str = "TOOL_TRUTH_AUTHORITY_STALE";
const CONTRACT_INVALID: &str = "TOOL_TRUTH_CONTRACT_INVALID";
const DENOMINATOR_UNSEALED: &str = "TOOL_TRUTH_DENOMINATOR_UNSEALED";
const RECEIPT_STALE: &str = "TOOL_TRUTH_RECEIPT_STALE";

fn fail(code: &'static str) -> DbError {
    DbError::Other(anyhow::anyhow!(code))
}

fn sha256_json(value: &serde_json::Value) -> Result<String> {
    let bytes =
        serde_json::to_vec(value).map_err(|error| DbError::Other(anyhow::Error::new(error)))?;
    let digest = Sha256::digest(bytes)
        .iter()
        .map(|byte| format!("{byte:02x}"))
        .collect::<String>();
    Ok(format!("sha256:{digest}"))
}

fn legacy_wave_hash(items: &[WaveItem]) -> String {
    let mut parts = items
        .iter()
        .map(|item| {
            format!(
                "{}\x1f{}\x1f{}\x1f{}",
                item.target_id,
                item.asset_value,
                item.asset_type,
                item.source.as_deref().unwrap_or("")
            )
        })
        .collect::<Vec<_>>();
    parts.sort();
    let mut hash = 0xcbf2_9ce4_8422_2325u64;
    for part in parts {
        for byte in part.as_bytes().iter().chain(std::iter::once(&0)) {
            hash ^= u64::from(*byte);
            hash = hash.wrapping_mul(0x0000_0100_0000_01b3);
        }
    }
    format!("{hash:016x}")
}

#[derive(Debug, Clone)]
pub struct SealWaveDenominator {
    pub stable_seal_request_id: Uuid,
    pub stage_execution_id: Uuid,
    pub scope_snapshot_id: Uuid,
    pub stage_asset_wave_id: Uuid,
    pub technique: String,
    pub expected_capability: String,
    pub contract: ToolTruthContract,
}

#[derive(Debug, Clone)]
pub struct BeginCapabilityReceipt {
    pub id: Uuid,
    pub denominator_id: Uuid,
    pub capability: String,
    pub attempt_ordinal: i32,
}

#[derive(Debug, Clone)]
pub struct AppendReconciliationFailure {
    pub id: Uuid,
    pub receipt_id: Uuid,
    pub expected_row_version: i64,
    pub state: ReconciliationFailureState,
    pub reason_code: String,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum ReconciliationFailureState {
    Orphaned,
    Superseded,
}

impl ReconciliationFailureState {
    const fn as_str(self) -> &'static str {
        match self {
            Self::Orphaned => "orphaned",
            Self::Superseded => "superseded",
        }
    }
}

#[derive(Debug, Clone, sqlx::FromRow, Serialize, Deserialize, PartialEq, Eq)]
pub struct CoverageDenominatorRow {
    pub id: Uuid,
    pub stable_seal_request_id: Uuid,
    pub execution_authority_id: Uuid,
    pub contract: String,
    pub input_manifest_hash: String,
    pub member_count: Option<i64>,
    pub member_set_hash: Option<String>,
    pub denominator_hash: String,
    pub sealed_at: Option<DateTime<Utc>>,
}

#[derive(Debug, Clone, sqlx::FromRow, Serialize, Deserialize, PartialEq)]
pub struct CapabilityExecutionReceiptRow {
    pub id: Uuid,
    pub denominator_id: Uuid,
    pub execution_authority_id: Uuid,
    pub capability: String,
    pub attempt_ordinal: i32,
    pub receipt_authority_hash: String,
    pub input_manifest_hash: String,
    pub attempt_state: String,
    pub landing_state: String,
    pub observation_state: String,
    pub coverage_extent: String,
    pub coverage_gap_reason: String,
    pub reconciliation_state: String,
    pub security_interpretation: String,
    pub typed_landing: serde_json::Value,
    pub residual: Option<serde_json::Value>,
    pub current_semantic_authority_version: i64,
    pub current_semantic_reconciliation_id: Option<Uuid>,
    pub current_semantic_reconciliation_hash: Option<String>,
    pub row_version: i64,
    pub finalized_at: Option<DateTime<Utc>>,
}

#[derive(Debug, Clone, sqlx::FromRow, Serialize, Deserialize, PartialEq, Eq)]
pub struct ReconciliationRow {
    pub id: Uuid,
    pub receipt_id: Uuid,
    pub execution_authority_id: Uuid,
    pub semantic_authority_version: i64,
    pub predecessor_reconciliation_id: Option<Uuid>,
    pub reconciliation_state: String,
    pub reason_code: Option<String>,
    pub member_count: Option<i64>,
    pub member_set_hash: Option<String>,
    pub semantic_reconciliation_hash: Option<String>,
    pub sealed_at: Option<DateTime<Utc>>,
}

#[derive(Debug, sqlx::FromRow)]
struct FrozenWaveAuthority {
    operation_id: Uuid,
    project_scope_id: Uuid,
    project_path_at_freeze: String,
    organization_id: Uuid,
    stage_kind: String,
    wave_status: String,
    asset_hash: String,
    operation_contract: String,
}

#[derive(Debug, sqlx::FromRow)]
struct WaveItem {
    id: i64,
    target_id: Uuid,
    asset_value: String,
    asset_type: String,
    source: Option<String>,
}

#[derive(Debug)]
struct DerivedDenominatorItem {
    id: Uuid,
    ordinal: i32,
    input_key: String,
    target_id: Uuid,
    exact_asset: String,
    member_hash: String,
}

const DENOMINATOR_COLUMNS: &str = "id,stable_seal_request_id,execution_authority_id,contract,input_manifest_hash,member_count,member_set_hash,denominator_hash,sealed_at";
const RECEIPT_COLUMNS: &str = "id,denominator_id,execution_authority_id,capability,attempt_ordinal,receipt_authority_hash,input_manifest_hash,attempt_state,landing_state,observation_state,coverage_extent,coverage_gap_reason,reconciliation_state,security_interpretation,typed_landing,residual,current_semantic_authority_version,current_semantic_reconciliation_id,current_semantic_reconciliation_hash,row_version,finalized_at";
const RECONCILIATION_COLUMNS: &str = "id,receipt_id,execution_authority_id,semantic_authority_version,predecessor_reconciliation_id,reconciliation_state,reason_code,member_count,member_set_hash,semantic_reconciliation_hash,sealed_at";

pub async fn seal_wave_denominator(
    pool: &PgPool,
    command: &SealWaveDenominator,
) -> Result<CoverageDenominatorRow> {
    if matches!(command.contract, ToolTruthContract::LegacyV1)
        || command.technique.trim().is_empty()
        || command.expected_capability.trim().is_empty()
    {
        return Err(fail(CONTRACT_INVALID));
    }
    let mut tx = pool.begin().await?;
    let authority = sqlx::query_as::<_, FrozenWaveAuthority>(
        r#"SELECT w.operation_id,s.project_scope_id,s.project_path_at_freeze,
                  w.organization_id,w.stage_kind,w.status AS wave_status,w.asset_hash,
                  o.tool_truth_contract AS operation_contract
             FROM stage_asset_waves w
             JOIN operation_org_scope_snapshots s
               ON s.id=$2 AND s.operation_id=w.operation_id AND s.sealed_at IS NOT NULL
             JOIN operation_org_scope_units u
               ON u.snapshot_id=s.id AND u.organization_id=w.organization_id
             JOIN stage_runs r
               ON r.id=$3 AND r.operation_id=w.operation_id AND r.stage_kind=w.stage_kind
             JOIN operation_state o ON o.operation_id=w.operation_id
            WHERE w.id=$1
            FOR SHARE OF w,s,u,r,o"#,
    )
    .bind(command.stage_asset_wave_id)
    .bind(command.scope_snapshot_id)
    .bind(command.stage_execution_id)
    .fetch_optional(&mut *tx)
    .await?
    .ok_or_else(|| fail(AUTHORITY_STALE))?;
    if authority.operation_contract != command.contract.as_str()
        || !matches!(authority.wave_status.as_str(), "running" | "completed")
    {
        return Err(fail(AUTHORITY_STALE));
    }

    let wave_items = sqlx::query_as::<_, WaveItem>(
        r#"SELECT id,target_id,asset_value,asset_type,source
             FROM stage_asset_wave_items
            WHERE wave_id=$1
            ORDER BY id
            FOR SHARE"#,
    )
    .bind(command.stage_asset_wave_id)
    .fetch_all(&mut *tx)
    .await?;
    if wave_items.is_empty() || legacy_wave_hash(&wave_items) != authority.asset_hash {
        return Err(fail(MANIFEST_DRIFT));
    }

    let binding_hash = sha256_json(&serde_json::json!({
        "stage_asset_wave_id": command.stage_asset_wave_id,
        "operation_id": authority.operation_id,
        "project_scope_id": authority.project_scope_id,
        "project_path_at_freeze": authority.project_path_at_freeze,
        "scope_snapshot_id": command.scope_snapshot_id,
        "organization_id": authority.organization_id,
        "stage_execution_id": command.stage_execution_id,
        "stage_kind": authority.stage_kind,
    }))?;
    let binding = sqlx::query_as::<_, (Uuid, String)>(
        r#"SELECT id,binding_hash FROM tool_truth_stage_wave_execution_bindings
            WHERE stage_asset_wave_id=$1 FOR SHARE"#,
    )
    .bind(command.stage_asset_wave_id)
    .fetch_optional(&mut *tx)
    .await?;
    let (binding_id, persisted_binding_hash) = if let Some(binding) = binding {
        binding
    } else {
        sqlx::query_as::<_, (Uuid, String)>(
            r#"INSERT INTO tool_truth_stage_wave_execution_bindings(
                   id,stage_asset_wave_id,operation_id,project_scope_id,project_path_at_freeze,
                   scope_snapshot_id,organization_id,stage_execution_id,stage_kind,binding_hash
               ) VALUES($1,$2,$3,$4,$5,$6,$7,$8,$9,$10)
               RETURNING id,binding_hash"#,
        )
        .bind(Uuid::new_v4())
        .bind(command.stage_asset_wave_id)
        .bind(authority.operation_id)
        .bind(authority.project_scope_id)
        .bind(&authority.project_path_at_freeze)
        .bind(command.scope_snapshot_id)
        .bind(authority.organization_id)
        .bind(command.stage_execution_id)
        .bind(&authority.stage_kind)
        .bind(&binding_hash)
        .fetch_one(&mut *tx)
        .await?
    };
    let execution_authority = sqlx::query_as::<_, (Uuid, String, Option<Uuid>)>(
        r#"SELECT id,authority_hash,stage_wave_binding_id
             FROM tool_truth_execution_authorities
            WHERE operation_id=$1 AND stable_authority_request_id=$2
            FOR SHARE"#,
    )
    .bind(authority.operation_id)
    .bind(command.stable_seal_request_id)
    .fetch_optional(&mut *tx)
    .await?;
    let (execution_authority_id, execution_authority_hash, persisted_binding_id) =
        if let Some(row) = execution_authority {
            row
        } else {
            sqlx::query_as::<_, (Uuid, String, Option<Uuid>)>(
                r#"INSERT INTO tool_truth_execution_authorities(
                       id,stable_authority_request_id,operation_id,project_scope_id,
                       project_path_at_freeze,scope_snapshot_id,organization_id,
                       stage_execution_id,stage_kind,execution_source_kind,
                       stage_wave_binding_id,stage_wave_binding_hash,
                       execution_owner_kind,authority_hash
                   ) VALUES($1,$2,$3,$4,$5,$6,$7,$8,$9,'stage_wave',$10,$11,
                            'host_stage',$12)
                   RETURNING id,authority_hash,stage_wave_binding_id"#,
            )
            .bind(Uuid::new_v4())
            .bind(command.stable_seal_request_id)
            .bind(authority.operation_id)
            .bind(authority.project_scope_id)
            .bind(&authority.project_path_at_freeze)
            .bind(command.scope_snapshot_id)
            .bind(authority.organization_id)
            .bind(command.stage_execution_id)
            .bind(&authority.stage_kind)
            .bind(binding_id)
            .bind(&persisted_binding_hash)
            .bind(sha256_json(&serde_json::json!({"server_derived": true}))?)
            .fetch_one(&mut *tx)
            .await?
        };
    if persisted_binding_id != Some(binding_id) {
        return Err(fail(MANIFEST_DRIFT));
    }

    let mut items = Vec::with_capacity(wave_items.len());
    for (ordinal, item) in wave_items.iter().enumerate() {
        let input_key = format!("{}:{}", item.target_id, command.expected_capability);
        let member_hash = sha256_json(&serde_json::json!({
            "ordinal": ordinal,
            "wave_item_id": item.id,
            "input_key": input_key,
            "target_id": item.target_id,
            "exact_asset": item.asset_value,
            "asset_type": item.asset_type,
            "technique": command.technique,
            "expected_capability": command.expected_capability,
        }))?;
        items.push(DerivedDenominatorItem {
            id: Uuid::new_v4(),
            ordinal: i32::try_from(ordinal).map_err(|_| fail(CONTRACT_INVALID))?,
            input_key,
            target_id: item.target_id,
            exact_asset: item.asset_value.clone(),
            member_hash,
        });
    }
    let input_manifest_hash = sha256_json(&serde_json::json!(items
        .iter()
        .map(|item| &item.member_hash)
        .collect::<Vec<_>>()))?;
    let denominator_hash = sha256_json(&serde_json::json!({
        "execution_authority_hash": execution_authority_hash,
        "input_manifest_hash": input_manifest_hash,
        "contract": command.contract.as_str(),
        "denominator_kind": "root",
    }))?;

    if let Some(existing) = get_denominator_on(
        &mut tx,
        execution_authority_id,
        command.stable_seal_request_id,
    )
    .await?
    {
        validate_denominator_replay_on(
            &mut tx,
            &existing,
            &input_manifest_hash,
            &denominator_hash,
            &items,
        )
        .await?;
        tx.commit().await?;
        return Ok(existing);
    }

    let denominator_id = Uuid::new_v4();
    sqlx::query(
        r#"INSERT INTO coverage_denominators(
               id,stable_seal_request_id,execution_authority_id,operation_id,
               project_scope_id,project_path_at_freeze,scope_snapshot_id,
               organization_id,stage_execution_id,stage_kind,execution_authority_hash,
               denominator_kind,contract,input_manifest_hash,denominator_hash
           ) VALUES($1,$2,$3,$4,$5,$6,$7,$8,$9,$10,$11,'root',$12,$13,$14)"#,
    )
    .bind(denominator_id)
    .bind(command.stable_seal_request_id)
    .bind(execution_authority_id)
    .bind(authority.operation_id)
    .bind(authority.project_scope_id)
    .bind(&authority.project_path_at_freeze)
    .bind(command.scope_snapshot_id)
    .bind(authority.organization_id)
    .bind(command.stage_execution_id)
    .bind(&authority.stage_kind)
    .bind(&execution_authority_hash)
    .bind(command.contract.as_str())
    .bind(&input_manifest_hash)
    .bind(&denominator_hash)
    .execute(&mut *tx)
    .await?;
    for item in &items {
        sqlx::query(
            r#"INSERT INTO coverage_denominator_items(
                   id,denominator_id,execution_authority_id,denominator_hash,ordinal,
                   input_key,target_id,exact_asset,technique,expected_capability,member_hash
               ) VALUES($1,$2,$3,$4,$5,$6,$7,$8,$9,$10,$11)"#,
        )
        .bind(item.id)
        .bind(denominator_id)
        .bind(execution_authority_id)
        .bind(&denominator_hash)
        .bind(item.ordinal)
        .bind(&item.input_key)
        .bind(item.target_id)
        .bind(&item.exact_asset)
        .bind(&command.technique)
        .bind(&command.expected_capability)
        .bind(&item.member_hash)
        .execute(&mut *tx)
        .await?;
    }
    let row = sqlx::query_as::<_, CoverageDenominatorRow>(&format!(
        "UPDATE coverage_denominators SET sealed_at=statement_timestamp() WHERE id=$1 RETURNING {DENOMINATOR_COLUMNS}"
    ))
    .bind(denominator_id)
    .fetch_one(&mut *tx)
    .await?;
    tx.commit().await?;
    Ok(row)
}

async fn get_denominator_on(
    tx: &mut Transaction<'_, Postgres>,
    execution_authority_id: Uuid,
    stable_seal_request_id: Uuid,
) -> Result<Option<CoverageDenominatorRow>> {
    sqlx::query_as::<_, CoverageDenominatorRow>(&format!(
        "SELECT {DENOMINATOR_COLUMNS} FROM coverage_denominators WHERE execution_authority_id=$1 AND stable_seal_request_id=$2 FOR SHARE"
    ))
    .bind(execution_authority_id)
    .bind(stable_seal_request_id)
    .fetch_optional(&mut **tx)
    .await
    .map_err(Into::into)
}

async fn validate_denominator_replay_on(
    tx: &mut Transaction<'_, Postgres>,
    existing: &CoverageDenominatorRow,
    expected_manifest_hash: &str,
    expected_denominator_hash: &str,
    expected_items: &[DerivedDenominatorItem],
) -> Result<()> {
    if existing.sealed_at.is_none()
        || existing.input_manifest_hash != expected_manifest_hash
        || existing.denominator_hash != expected_denominator_hash
        || existing.member_count != Some(expected_items.len() as i64)
    {
        return Err(fail(MANIFEST_DRIFT));
    }
    let hashes = sqlx::query_scalar::<_, String>(
        "SELECT member_hash FROM coverage_denominator_items WHERE denominator_id=$1 ORDER BY ordinal",
    )
    .bind(existing.id)
    .fetch_all(&mut **tx)
    .await?;
    if hashes
        != expected_items
            .iter()
            .map(|item| item.member_hash.clone())
            .collect::<Vec<_>>()
    {
        return Err(fail(MANIFEST_DRIFT));
    }
    Ok(())
}

pub async fn begin(
    pool: &PgPool,
    command: &BeginCapabilityReceipt,
) -> Result<CapabilityExecutionReceiptRow> {
    if command.attempt_ordinal <= 0 || command.capability.trim().is_empty() {
        return Err(fail(CONTRACT_INVALID));
    }
    let mut tx = pool.begin().await?;
    let denominator = sqlx::query_as::<_, (Uuid, String, String, Option<DateTime<Utc>>)>(
        "SELECT execution_authority_id,input_manifest_hash,denominator_hash,sealed_at FROM coverage_denominators WHERE id=$1 FOR SHARE",
    )
    .bind(command.denominator_id)
    .fetch_optional(&mut *tx)
    .await?
    .ok_or_else(|| fail(AUTHORITY_STALE))?;
    if denominator.3.is_none() {
        return Err(fail(DENOMINATOR_UNSEALED));
    }
    let exact_capability: bool = sqlx::query_scalar(
        "SELECT count(*)>0 AND bool_and(expected_capability=$2) FROM coverage_denominator_items WHERE denominator_id=$1",
    )
    .bind(command.denominator_id)
    .bind(&command.capability)
    .fetch_one(&mut *tx)
    .await?;
    if !exact_capability {
        return Err(fail(MANIFEST_DRIFT));
    }

    if let Some(existing) = sqlx::query_as::<_, CapabilityExecutionReceiptRow>(&format!(
        "SELECT {RECEIPT_COLUMNS} FROM capability_execution_receipts WHERE denominator_id=$1 AND execution_authority_id=$2 AND capability=$3 AND attempt_ordinal=$4 FOR SHARE"
    ))
    .bind(command.denominator_id)
    .bind(denominator.0)
    .bind(&command.capability)
    .bind(command.attempt_ordinal)
    .fetch_optional(&mut *tx)
    .await?
    {
        if existing.input_manifest_hash != denominator.1 {
            return Err(fail(MANIFEST_DRIFT));
        }
        tx.commit().await?;
        return Ok(existing);
    }

    let destination_policy_hash = sha256_json(&serde_json::json!({
        "denominator_id": command.denominator_id,
        "execution_authority_id": denominator.0,
        "capability": command.capability,
        "execution_backend": "none_blocked",
        "governance_status": "policy_blocked",
    }))?;
    let tls_policy_hash = sha256_json(&serde_json::json!({"policy": "deny"}))?;
    let prohibited_range_policy_hash = sha256_json(&serde_json::json!({"policy": "deny"}))?;
    let destination_policy_id = if let Some(id) = sqlx::query_scalar::<_, Uuid>(
        r#"SELECT id FROM capability_execution_destination_policies
            WHERE denominator_id=$1 AND execution_authority_id=$2
              AND capability=$3 AND policy_hash=$4 AND sealed_at IS NOT NULL
            FOR SHARE"#,
    )
    .bind(command.denominator_id)
    .bind(denominator.0)
    .bind(&command.capability)
    .bind(&destination_policy_hash)
    .fetch_optional(&mut *tx)
    .await?
    {
        id
    } else {
        let id = Uuid::new_v4();
        sqlx::query(
            r#"INSERT INTO capability_execution_destination_policies(
                   id,denominator_id,execution_authority_id,capability,execution_backend,
                   governance_status,redirect_mode,max_redirect_hops,tls_policy_hash,
                   prohibited_range_policy_hash,policy_hash
               ) VALUES($1,$2,$3,$4,'none_blocked','policy_blocked','deny',0,$5,$6,$7)"#,
        )
        .bind(id)
        .bind(command.denominator_id)
        .bind(denominator.0)
        .bind(&command.capability)
        .bind(tls_policy_hash)
        .bind(prohibited_range_policy_hash)
        .bind(&destination_policy_hash)
        .execute(&mut *tx)
        .await?;
        sqlx::query(
            "UPDATE capability_execution_destination_policies SET sealed_at=statement_timestamp() WHERE id=$1",
        )
        .bind(id)
        .execute(&mut *tx)
        .await?;
        id
    };

    let temporal_member_hash = sha256_json(&serde_json::json!({
        "fact_class": "default",
        "positive_ttl_ms": 300000,
        "negative_ttl_ms": 60000,
        "refutation_ttl_ms": 60000,
        "same_epoch": true,
        "required_recheck_source": "manual_only",
    }))?;
    let temporal_policy_hash = sha256_json(&serde_json::json!({
        "execution_authority_id": denominator.0,
        "max_cross_observation_skew_ms": 30000,
        "members": [&temporal_member_hash],
    }))?;
    let temporal_policy_id = if let Some(id) = sqlx::query_scalar::<_, Uuid>(
        r#"SELECT id FROM evidence_temporal_validity_policies
            WHERE execution_authority_id=$1 AND policy_hash=$2 AND sealed_at IS NOT NULL
            ORDER BY id LIMIT 1 FOR SHARE"#,
    )
    .bind(denominator.0)
    .bind(&temporal_policy_hash)
    .fetch_optional(&mut *tx)
    .await?
    {
        id
    } else {
        let id = Uuid::new_v4();
        sqlx::query(
            r#"INSERT INTO evidence_temporal_validity_policies(
                   id,execution_authority_id,max_cross_observation_skew_ms,policy_hash
               ) VALUES($1,$2,30000,$3)"#,
        )
        .bind(id)
        .bind(denominator.0)
        .bind(&temporal_policy_hash)
        .execute(&mut *tx)
        .await?;
        sqlx::query(
            r#"INSERT INTO evidence_temporal_validity_policy_members(
                   id,policy_id,ordinal,fact_class,positive_ttl_ms,negative_ttl_ms,
                   refutation_ttl_ms,require_same_target_state_epoch,
                   required_recheck_source,member_hash
               ) VALUES($1,$2,0,'default',300000,60000,60000,TRUE,'manual_only',$3)"#,
        )
        .bind(Uuid::new_v4())
        .bind(id)
        .bind(temporal_member_hash)
        .execute(&mut *tx)
        .await?;
        sqlx::query(
            "UPDATE evidence_temporal_validity_policies SET sealed_at=statement_timestamp() WHERE id=$1",
        )
        .bind(id)
        .execute(&mut *tx)
        .await?;
        id
    };

    let receipt_authority_hash = sha256_json(&serde_json::json!({
        "denominator_id": command.denominator_id,
        "denominator_hash": denominator.2,
        "execution_authority_id": denominator.0,
        "capability": command.capability,
        "attempt_ordinal": command.attempt_ordinal,
        "input_manifest_hash": denominator.1,
        "destination_policy_hash": destination_policy_hash,
        "temporal_validity_policy_hash": temporal_policy_hash,
    }))?;
    let typed_landing = serde_json::json!({
        "capability": command.capability,
        "state": "running",
    });
    let row = sqlx::query_as::<_, CapabilityExecutionReceiptRow>(&format!(
        r#"INSERT INTO capability_execution_receipts(
               id,denominator_id,execution_authority_id,capability,attempt_ordinal,
               receipt_authority_hash,input_manifest_hash,destination_policy_id,
               destination_policy_hash,temporal_validity_policy_id,
               temporal_validity_policy_hash,attempt_state,landing_state,
               observation_state,coverage_extent,coverage_gap_reason,
               reconciliation_state,security_interpretation,typed_landing
           ) VALUES($1,$2,$3,$4,$5,$6,$7,$8,$9,$10,$11,'running',
                    'not_attempted','indeterminate','none','policy_blocked',
                    'pending','not_assessed',$12)
           RETURNING {RECEIPT_COLUMNS}"#
    ))
    .bind(command.id)
    .bind(command.denominator_id)
    .bind(denominator.0)
    .bind(&command.capability)
    .bind(command.attempt_ordinal)
    .bind(receipt_authority_hash)
    .bind(denominator.1)
    .bind(destination_policy_id)
    .bind(destination_policy_hash)
    .bind(temporal_policy_id)
    .bind(temporal_policy_hash)
    .bind(typed_landing)
    .fetch_one(&mut *tx)
    .await?;
    tx.commit().await?;
    Ok(row)
}

pub async fn get(pool: &PgPool, receipt_id: Uuid) -> Result<Option<CapabilityExecutionReceiptRow>> {
    sqlx::query_as::<_, CapabilityExecutionReceiptRow>(&format!(
        "SELECT {RECEIPT_COLUMNS} FROM capability_execution_receipts WHERE id=$1"
    ))
    .bind(receipt_id)
    .fetch_optional(pool)
    .await
    .map_err(Into::into)
}

pub async fn append_reconciliation_failure(
    pool: &PgPool,
    command: &AppendReconciliationFailure,
) -> Result<ReconciliationRow> {
    if command.reason_code.trim().is_empty() {
        return Err(fail(CONTRACT_INVALID));
    }
    let mut tx = pool.begin().await?;
    if let Some(existing) = sqlx::query_as::<_, ReconciliationRow>(&format!(
        "SELECT {RECONCILIATION_COLUMNS} FROM capability_execution_reconciliations WHERE id=$1 FOR SHARE"
    ))
    .bind(command.id)
    .fetch_optional(&mut *tx)
    .await?
    {
        if existing.receipt_id != command.receipt_id
            || existing.reconciliation_state != command.state.as_str()
            || existing.reason_code.as_deref() != Some(command.reason_code.as_str())
        {
            return Err(fail(MANIFEST_DRIFT));
        }
        tx.commit().await?;
        return Ok(existing);
    }
    let receipt = sqlx::query_as::<_, (Uuid, i64, i64, Option<Uuid>)>(
        "SELECT execution_authority_id,row_version,current_semantic_authority_version,current_semantic_reconciliation_id FROM capability_execution_receipts WHERE id=$1 FOR UPDATE",
    )
    .bind(command.receipt_id)
    .fetch_optional(&mut *tx)
    .await?
    .ok_or_else(|| fail(AUTHORITY_STALE))?;
    if receipt.1 != command.expected_row_version {
        return Err(fail(RECEIPT_STALE));
    }
    let semantic_version = receipt.2 + 1;
    sqlx::query(
        r#"INSERT INTO capability_execution_reconciliations(
               id,receipt_id,execution_authority_id,semantic_authority_version,
               predecessor_reconciliation_id,reconciliation_state
           ) VALUES($1,$2,$3,$4,$5,'pending')"#,
    )
    .bind(command.id)
    .bind(command.receipt_id)
    .bind(receipt.0)
    .bind(semantic_version)
    .bind(receipt.3)
    .execute(&mut *tx)
    .await?;
    let reconciliation = sqlx::query_as::<_, ReconciliationRow>(&format!(
        "UPDATE capability_execution_reconciliations SET reconciliation_state=$2,reason_code=$3,sealed_at=statement_timestamp() WHERE id=$1 RETURNING {RECONCILIATION_COLUMNS}"
    ))
    .bind(command.id)
    .bind(command.state.as_str())
    .bind(&command.reason_code)
    .fetch_one(&mut *tx)
    .await?;
    let semantic_hash = reconciliation
        .semantic_reconciliation_hash
        .as_deref()
        .ok_or_else(|| fail(MANIFEST_DRIFT))?;
    let affected = sqlx::query(
        r#"UPDATE capability_execution_receipts
              SET attempt_state=CASE WHEN attempt_state='running' THEN 'outcome_unknown' ELSE attempt_state END,
                  landing_state=CASE WHEN landing_state='not_attempted' THEN 'failed' ELSE landing_state END,
                  coverage_extent=CASE WHEN coverage_extent='complete' THEN 'partial' ELSE coverage_extent END,
                  coverage_gap_reason=CASE WHEN coverage_gap_reason='none' THEN 'source_unavailable' ELSE coverage_gap_reason END,
                  reconciliation_state=$3,security_interpretation='inconclusive',
                  current_semantic_authority_version=$4,
                  current_semantic_reconciliation_id=$5,
                  current_semantic_reconciliation_hash=$6,
                  row_version=row_version+1,finalized_at=COALESCE(finalized_at,statement_timestamp())
            WHERE id=$1 AND row_version=$2"#,
    )
    .bind(command.receipt_id)
    .bind(command.expected_row_version)
    .bind(command.state.as_str())
    .bind(semantic_version)
    .bind(command.id)
    .bind(semantic_hash)
    .execute(&mut *tx)
    .await?;
    if affected.rows_affected() != 1 {
        return Err(fail(RECEIPT_STALE));
    }
    tx.commit().await?;
    Ok(reconciliation)
}
