//! Typed persistence boundary for Plan A coverage denominators and receipts.
//!
//! Public commands carry stable request identity and frozen source identity,
//! never caller-computed manifests or authority hashes. Every hash written by
//! this module is derived after locking the database-owned parent census.

use std::net::IpAddr;

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

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum DenominatorSourceRef {
    StageAssetWave(Uuid),
    StageTeamUnit(Uuid),
}

#[derive(Debug, Clone)]
pub struct SealSourceDenominator {
    pub stable_seal_request_id: Uuid,
    pub stage_execution_id: Uuid,
    pub source: DenominatorSourceRef,
}

/// Database-owned source member passed to the deterministic compiler while
/// the source rows remain share-locked. This type is not part of DbRepoProvider
/// and cannot be constructed by a model/tool request.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct LockedDenominatorAsset {
    pub target_id: Uuid,
    pub exact_asset: String,
    pub asset_type: String,
    pub web_capable: bool,
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct CompiledDenominatorItem {
    pub input_key: String,
    pub target_id: Uuid,
    pub exact_asset: String,
    pub technique: String,
    pub expected_capability: String,
}

#[derive(Debug, Clone)]
pub struct BeginCapabilityReceipt {
    pub id: Uuid,
    pub denominator_id: Uuid,
    pub capability: String,
    pub attempt_ordinal: i32,
}

#[derive(Debug, Clone)]
pub struct BeginManagedCapabilityReceipt {
    pub id: Uuid,
    pub denominator_id: Uuid,
    pub capability: String,
    pub attempt_ordinal: i32,
    /// The exact enforced policy sealed before provider I/O.
    pub destination_policy_id: Uuid,
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct FixedProviderEndpoint {
    pub scheme: String,
    pub normalized_host: String,
    pub port: i32,
    pub path_prefix: String,
}

#[derive(Debug, Clone)]
pub struct SealFixedProviderDestinationPolicy {
    pub denominator_id: Uuid,
    pub capability: String,
    pub endpoints: Vec<FixedProviderEndpoint>,
}

#[derive(Debug, Clone, sqlx::FromRow, PartialEq, Eq)]
pub struct SealedDestinationPolicy {
    pub id: Uuid,
    pub execution_authority_id: Uuid,
    pub policy_hash: String,
}

#[derive(Debug, Clone)]
pub struct RawWitnessArtifactInput {
    pub artifact_id: Uuid,
    pub content_key: String,
    pub vault_object_ref_token: Vec<u8>,
    pub vault_object_ref_token_hash: String,
    pub sha256: String,
    pub ciphertext_sha256: String,
    pub operation_key_ref_hash: String,
    pub key_generation: i64,
    pub retention_policy_id: Uuid,
    pub retention_policy_hash: String,
    pub sensitivity_disposition: String,
    pub original_byte_count: i64,
    pub stored_byte_count: i64,
    pub truncated: bool,
}

#[derive(Debug, Clone)]
pub struct ObservedNetworkHopInput {
    pub hop_kind: String,
    pub scheme: String,
    pub normalized_host: String,
    pub port: i32,
    pub path_and_query: String,
    pub addresses: Vec<IpAddr>,
    pub selected_address: IpAddr,
    pub send_ordinal: i64,
}

#[derive(Debug, Clone)]
pub struct FinalizeTargetIntelReceipt {
    pub receipt_id: Uuid,
    pub expected_row_version: i64,
    pub attempt_fence: Option<TargetIntelAttemptFence>,
    pub raw_witness: RawWitnessArtifactInput,
    pub network_hops: Vec<ObservedNetworkHopInput>,
    pub request_count: i64,
    pub response_byte_count: i64,
    pub wall_clock_ms: i64,
    pub retry_count: i64,
    pub parser_complete: bool,
    pub normalized_record_count: i64,
    /// Exact server-derived input outcomes covered by this execution.
    /// Values are `found`, `no_match`, or `indeterminate`; absence means that
    /// denominator axis remains partial.
    pub input_observations: Vec<TargetIntelInputObservation>,
    pub typed_landing: serde_json::Value,
    pub failure_reason_code: Option<String>,
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct TargetIntelInputObservation {
    pub input_key: String,
    pub technique: String,
    pub observation_state: String,
}

#[derive(Debug, Clone, PartialEq)]
pub enum ManagedReceiptBeginOutcome {
    Created(CapabilityExecutionReceiptRow),
    TerminalReplay(CapabilityExecutionReceiptRow),
    InFlight(CapabilityExecutionReceiptRow),
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
    pub finalization_request_hash: Option<String>,
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

#[derive(Debug, Clone, sqlx::FromRow, PartialEq, Eq)]
pub struct TargetIntelExpectedInputRow {
    pub input_key: String,
    pub exact_asset: String,
    pub technique: String,
}

#[derive(Debug, Clone, sqlx::FromRow, PartialEq, Eq)]
pub struct TargetIntelReceiptProjectionRow {
    pub denominator_id: Uuid,
    pub stage_execution_id: Uuid,
    pub attempt_epoch: i64,
    pub input_key: String,
    pub technique: String,
    pub reconciliation_state: String,
    pub landing_state: String,
    pub observation_state: String,
    pub coverage_extent: String,
    pub authority_current: bool,
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct CurrentTargetIntelReceiptProjection {
    pub denominator_id: Uuid,
    pub denominator_hash: String,
    pub stage_execution_id: Uuid,
    pub attempt_epoch: i64,
    pub expected: Vec<TargetIntelExpectedInputRow>,
    pub receipts: Vec<TargetIntelReceiptProjectionRow>,
}

#[derive(Debug, Clone, sqlx::FromRow, PartialEq, Eq)]
pub struct CurrentTargetIntelReceiptContext {
    pub denominator_id: Uuid,
    pub execution_authority_id: Uuid,
    pub denominator_hash: String,
    pub attempt_epoch: i64,
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct TargetIntelAttemptFence {
    pub worker_run_id: Uuid,
    pub stage_run_unit_id: Uuid,
    pub lease_token: Uuid,
    pub worker_attempt_epoch: i64,
    pub source_tool_call_id: Uuid,
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

#[derive(Debug)]
struct FrozenSourceAuthority {
    operation_id: Uuid,
    project_scope_id: Uuid,
    project_path_at_freeze: String,
    scope_snapshot_id: Uuid,
    organization_id: Uuid,
    stage_kind: String,
    operation_contract: String,
    execution_source_kind: &'static str,
    stage_wave_binding_id: Option<Uuid>,
    stage_wave_binding_hash: Option<String>,
    stage_run_unit_id: Option<Uuid>,
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

/// Lock a durable source, compile its exact applicability set, and seal the
/// denominator without ever exposing a caller-authored member/count/hash seam.
pub async fn seal_source_denominator<F>(
    pool: &PgPool,
    command: &SealSourceDenominator,
    compile: F,
) -> Result<CoverageDenominatorRow>
where
    F: FnOnce(&str, &[LockedDenominatorAsset]) -> anyhow::Result<Vec<CompiledDenominatorItem>>,
{
    if command.stable_seal_request_id.is_nil() || command.stage_execution_id.is_nil() {
        return Err(fail(CONTRACT_INVALID));
    }
    let mut tx = pool.begin().await?;

    let (authority, locked_assets) = match command.source {
        DenominatorSourceRef::StageAssetWave(stage_asset_wave_id) => {
            let wave = sqlx::query_as::<_, FrozenWaveAuthority>(
                r#"SELECT w.operation_id,s.project_scope_id,s.project_path_at_freeze,
                          w.organization_id,w.stage_kind,w.status AS wave_status,w.asset_hash,
                          o.tool_truth_contract AS operation_contract
                     FROM stage_asset_waves w
                     JOIN stage_runs r
                       ON r.id=$2 AND r.operation_id=w.operation_id AND r.stage_kind=w.stage_kind
                     JOIN operation_state o ON o.operation_id=w.operation_id
                     JOIN operation_org_scope_snapshots s
                       ON s.operation_id=w.operation_id AND s.sealed_at IS NOT NULL
                     JOIN operation_org_scope_units u
                       ON u.snapshot_id=s.id AND u.organization_id=w.organization_id
                    WHERE w.id=$1
                    FOR SHARE OF w,r,o,s,u"#,
            )
            .bind(stage_asset_wave_id)
            .bind(command.stage_execution_id)
            .fetch_optional(&mut *tx)
            .await?
            .ok_or_else(|| fail(AUTHORITY_STALE))?;
            if !matches!(wave.wave_status.as_str(), "running" | "completed") {
                return Err(fail(AUTHORITY_STALE));
            }
            let wave_items = sqlx::query_as::<_, WaveItem>(
                r#"SELECT id,target_id,asset_value,asset_type,source
                     FROM stage_asset_wave_items
                    WHERE wave_id=$1
                    ORDER BY id
                    FOR SHARE"#,
            )
            .bind(stage_asset_wave_id)
            .fetch_all(&mut *tx)
            .await?;
            if wave_items.is_empty() || legacy_wave_hash(&wave_items) != wave.asset_hash {
                return Err(fail(MANIFEST_DRIFT));
            }
            let binding_hash = sha256_json(&serde_json::json!({
                "stage_asset_wave_id": stage_asset_wave_id,
                "operation_id": wave.operation_id,
                "project_scope_id": wave.project_scope_id,
                "project_path_at_freeze": wave.project_path_at_freeze,
                "scope_snapshot_id": sqlx::query_scalar::<_, Uuid>(
                    "SELECT id FROM operation_org_scope_snapshots WHERE operation_id=$1"
                ).bind(wave.operation_id).fetch_one(&mut *tx).await?,
                "organization_id": wave.organization_id,
                "stage_execution_id": command.stage_execution_id,
                "stage_kind": wave.stage_kind,
            }))?;
            let scope_snapshot_id = sqlx::query_scalar::<_, Uuid>(
                "SELECT id FROM operation_org_scope_snapshots WHERE operation_id=$1 FOR SHARE",
            )
            .bind(wave.operation_id)
            .fetch_one(&mut *tx)
            .await?;
            let binding = sqlx::query_as::<_, (Uuid, String)>(
                "SELECT id,binding_hash FROM tool_truth_stage_wave_execution_bindings WHERE stage_asset_wave_id=$1 FOR SHARE",
            )
            .bind(stage_asset_wave_id)
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
                .bind(stage_asset_wave_id)
                .bind(wave.operation_id)
                .bind(wave.project_scope_id)
                .bind(&wave.project_path_at_freeze)
                .bind(scope_snapshot_id)
                .bind(wave.organization_id)
                .bind(command.stage_execution_id)
                .bind(&wave.stage_kind)
                .bind(&binding_hash)
                .fetch_one(&mut *tx)
                .await?
            };
            let assets = wave_items
                .into_iter()
                .map(|item| LockedDenominatorAsset {
                    target_id: item.target_id,
                    exact_asset: item.asset_value,
                    asset_type: item.asset_type,
                    web_capable: false,
                })
                .collect();
            (
                FrozenSourceAuthority {
                    operation_id: wave.operation_id,
                    project_scope_id: wave.project_scope_id,
                    project_path_at_freeze: wave.project_path_at_freeze,
                    scope_snapshot_id,
                    organization_id: wave.organization_id,
                    stage_kind: wave.stage_kind,
                    operation_contract: wave.operation_contract,
                    execution_source_kind: "stage_wave",
                    stage_wave_binding_id: Some(binding_id),
                    stage_wave_binding_hash: Some(persisted_binding_hash),
                    stage_run_unit_id: None,
                },
                assets,
            )
        }
        DenominatorSourceRef::StageTeamUnit(stage_run_unit_id) => {
            let row = sqlx::query_as::<_, (Uuid, Uuid, String, Uuid, Uuid, String, String, String)>(
                r#"SELECT u.operation_id,s.project_scope_id,s.project_path_at_freeze,
                          u.scope_snapshot_id,u.organization_id,u.stage_kind,o.tool_truth_contract,u.status
                     FROM stage_run_units u
                     JOIN stage_runs r
                       ON r.id=$2 AND r.id=u.stage_execution_id
                      AND r.operation_id=u.operation_id AND r.stage_kind=u.stage_kind
                     JOIN operation_org_scope_snapshots s
                       ON s.id=u.scope_snapshot_id AND s.operation_id=u.operation_id
                      AND s.sealed_at IS NOT NULL
                     JOIN operation_org_scope_units ou
                       ON ou.snapshot_id=s.id AND ou.organization_id=u.organization_id
                     JOIN operation_state o ON o.operation_id=u.operation_id
                    WHERE u.id=$1
                    FOR SHARE OF u,r,s,ou,o"#,
            )
            .bind(stage_run_unit_id)
            .bind(command.stage_execution_id)
            .fetch_optional(&mut *tx)
            .await?
            .ok_or_else(|| fail(AUTHORITY_STALE))?;
            if matches!(row.7.as_str(), "passed" | "exhausted" | "superseded") {
                return Err(fail(AUTHORITY_STALE));
            }
            let bound_wave = sqlx::query_as::<_, (Uuid, String)>(
                r#"SELECT b.stage_asset_wave_id,w.asset_hash
                     FROM tool_truth_stage_wave_execution_bindings b
                     JOIN stage_asset_waves w ON w.id=b.stage_asset_wave_id
                    WHERE b.operation_id=$1 AND b.scope_snapshot_id=$2
                      AND b.organization_id=$3 AND b.stage_execution_id=$4
                      AND b.stage_kind=$5 AND w.status IN ('running','completed')
                    ORDER BY w.wave_index DESC LIMIT 1 FOR SHARE OF b,w"#,
            )
            .bind(row.0)
            .bind(row.3)
            .bind(row.4)
            .bind(command.stage_execution_id)
            .bind(&row.5)
            .fetch_optional(&mut *tx)
            .await?;
            let assets = if let Some((wave_id, asset_hash)) = bound_wave {
                let wave_items = sqlx::query_as::<_, WaveItem>(
                    r#"SELECT id,target_id,asset_value,asset_type,source
                         FROM stage_asset_wave_items WHERE wave_id=$1 ORDER BY id FOR SHARE"#,
                )
                .bind(wave_id)
                .fetch_all(&mut *tx)
                .await?;
                if wave_items.is_empty() || legacy_wave_hash(&wave_items) != asset_hash {
                    return Err(fail(MANIFEST_DRIFT));
                }
                wave_items
                    .into_iter()
                    .map(|asset| LockedDenominatorAsset {
                        target_id: asset.target_id,
                        exact_asset: asset.asset_value,
                        asset_type: asset.asset_type,
                        web_capable: false,
                    })
                    .collect::<Vec<_>>()
            } else {
                sqlx::query_as::<_, (Uuid, String, String, bool)>(
                    r#"SELECT id,value,target_type::text,(http_status IS NOT NULL) AS web_capable
                         FROM targets
                        WHERE organization_id=$1
                          AND project_path IS NOT DISTINCT FROM $2
                          AND scope::text='in'
                          AND created_at <= (
                              SELECT started_at FROM stage_runs WHERE id=$3
                          )
                        ORDER BY created_at,id
                        FOR SHARE"#,
                )
                .bind(row.4)
                .bind(&row.2)
                .bind(command.stage_execution_id)
                .fetch_all(&mut *tx)
                .await?
                .into_iter()
                .map(|asset| LockedDenominatorAsset {
                    target_id: asset.0,
                    exact_asset: asset.1,
                    asset_type: asset.2,
                    web_capable: asset.3,
                })
                .collect::<Vec<_>>()
            };
            (
                FrozenSourceAuthority {
                    operation_id: row.0,
                    project_scope_id: row.1,
                    project_path_at_freeze: row.2,
                    scope_snapshot_id: row.3,
                    organization_id: row.4,
                    stage_kind: row.5,
                    operation_contract: row.6,
                    execution_source_kind: "stage_unit",
                    stage_wave_binding_id: None,
                    stage_wave_binding_hash: None,
                    stage_run_unit_id: Some(stage_run_unit_id),
                },
                assets,
            )
        }
    };

    let contract = ToolTruthContract::try_from(authority.operation_contract.as_str())
        .map_err(|_| fail(CONTRACT_INVALID))?;
    if !contract.writes_receipts() || locked_assets.is_empty() {
        return Err(fail(CONTRACT_INVALID));
    }
    let compiled = compile(&authority.stage_kind, &locked_assets)
        .map_err(|error| DbError::Other(anyhow::anyhow!("{CONTRACT_INVALID}: {error}")))?;
    if compiled.is_empty() {
        return Err(fail(CONTRACT_INVALID));
    }
    let source_assets = locked_assets
        .iter()
        .map(|asset| (asset.target_id, asset.exact_asset.as_str()))
        .collect::<std::collections::HashSet<_>>();
    let mut input_keys = std::collections::HashSet::new();
    for item in &compiled {
        if item.input_key.trim().is_empty()
            || item.technique.trim().is_empty()
            || item.expected_capability.trim().is_empty()
            || !source_assets.contains(&(item.target_id, item.exact_asset.as_str()))
            || !input_keys.insert(item.input_key.as_str())
        {
            return Err(fail(MANIFEST_DRIFT));
        }
    }

    let authority_hash = sha256_json(&serde_json::json!({
        "stable_authority_request_id": command.stable_seal_request_id,
        "operation_id": authority.operation_id,
        "project_scope_id": authority.project_scope_id,
        "project_path_at_freeze": authority.project_path_at_freeze,
        "scope_snapshot_id": authority.scope_snapshot_id,
        "organization_id": authority.organization_id,
        "stage_execution_id": command.stage_execution_id,
        "stage_kind": authority.stage_kind,
        "execution_source_kind": authority.execution_source_kind,
        "stage_wave_binding_hash": authority.stage_wave_binding_hash,
        "stage_run_unit_id": authority.stage_run_unit_id,
        "execution_owner_kind": "host_stage",
        "worker_run_id": null,
        "worker_attempt_epoch": null,
        "lease_token": null,
        "source_tool_call_id": null,
    }))?;
    let existing_authority = sqlx::query_as::<_, (Uuid, String, Option<Uuid>, Option<Uuid>)>(
        r#"SELECT id,authority_hash,stage_wave_binding_id,stage_run_unit_id
             FROM tool_truth_execution_authorities
            WHERE operation_id=$1 AND stable_authority_request_id=$2
            FOR SHARE"#,
    )
    .bind(authority.operation_id)
    .bind(command.stable_seal_request_id)
    .fetch_optional(&mut *tx)
    .await?;
    let (execution_authority_id, persisted_authority_hash) =
        if let Some((id, hash, wave_binding_id, unit_id)) = existing_authority {
            if wave_binding_id != authority.stage_wave_binding_id
                || unit_id != authority.stage_run_unit_id
            {
                return Err(fail(MANIFEST_DRIFT));
            }
            (id, hash)
        } else {
            sqlx::query_as::<_, (Uuid, String)>(
                r#"INSERT INTO tool_truth_execution_authorities(
                       id,stable_authority_request_id,operation_id,project_scope_id,
                       project_path_at_freeze,scope_snapshot_id,organization_id,
                       stage_execution_id,stage_kind,execution_source_kind,
                       stage_wave_binding_id,stage_wave_binding_hash,stage_run_unit_id,
                       execution_owner_kind,authority_hash
                   ) VALUES($1,$2,$3,$4,$5,$6,$7,$8,$9,$10,$11,$12,$13,'host_stage',$14)
                   RETURNING id,authority_hash"#,
            )
            .bind(Uuid::new_v4())
            .bind(command.stable_seal_request_id)
            .bind(authority.operation_id)
            .bind(authority.project_scope_id)
            .bind(&authority.project_path_at_freeze)
            .bind(authority.scope_snapshot_id)
            .bind(authority.organization_id)
            .bind(command.stage_execution_id)
            .bind(&authority.stage_kind)
            .bind(authority.execution_source_kind)
            .bind(authority.stage_wave_binding_id)
            .bind(&authority.stage_wave_binding_hash)
            .bind(authority.stage_run_unit_id)
            .bind(&authority_hash)
            .fetch_one(&mut *tx)
            .await?
        };

    let mut derived = Vec::with_capacity(compiled.len());
    for (ordinal, item) in compiled.iter().enumerate() {
        let member_hash = sha256_json(&serde_json::json!({
            "ordinal": ordinal,
            "input_key": item.input_key,
            "target_id": item.target_id,
            "exact_asset": item.exact_asset,
            "technique": item.technique,
            "expected_capability": item.expected_capability,
        }))?;
        derived.push(DerivedDenominatorItem {
            id: Uuid::new_v4(),
            ordinal: i32::try_from(ordinal).map_err(|_| fail(CONTRACT_INVALID))?,
            input_key: item.input_key.clone(),
            target_id: item.target_id,
            exact_asset: item.exact_asset.clone(),
            member_hash,
        });
    }
    let input_manifest_hash = sha256_json(&serde_json::json!(derived
        .iter()
        .map(|item| &item.member_hash)
        .collect::<Vec<_>>()))?;
    let denominator_hash = sha256_json(&serde_json::json!({
        "execution_authority_hash": persisted_authority_hash,
        "input_manifest_hash": input_manifest_hash,
        "contract": contract.as_str(),
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
            &derived,
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
    .bind(authority.scope_snapshot_id)
    .bind(authority.organization_id)
    .bind(command.stage_execution_id)
    .bind(&authority.stage_kind)
    .bind(&persisted_authority_hash)
    .bind(contract.as_str())
    .bind(&input_manifest_hash)
    .bind(&denominator_hash)
    .execute(&mut *tx)
    .await?;
    for (compiled, item) in compiled.iter().zip(&derived) {
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
        .bind(&compiled.technique)
        .bind(&compiled.expected_capability)
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

const DENOMINATOR_COLUMNS: &str = "id,stable_seal_request_id,execution_authority_id,contract,input_manifest_hash,member_count,member_set_hash,denominator_hash,sealed_at";
const RECEIPT_COLUMNS: &str = "id,denominator_id,execution_authority_id,capability,attempt_ordinal,receipt_authority_hash,input_manifest_hash,attempt_state,landing_state,observation_state,coverage_extent,coverage_gap_reason,reconciliation_state,security_interpretation,typed_landing,residual,finalization_request_hash,current_semantic_authority_version,current_semantic_reconciliation_id,current_semantic_reconciliation_hash,row_version,finalized_at";
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

pub async fn seal_fixed_provider_destination_policy(
    pool: &PgPool,
    command: &SealFixedProviderDestinationPolicy,
) -> Result<SealedDestinationPolicy> {
    if command.capability.trim().is_empty() || command.endpoints.is_empty() {
        return Err(fail(CONTRACT_INVALID));
    }
    let mut endpoints = command.endpoints.clone();
    endpoints.sort_by(|left, right| {
        (
            &left.scheme,
            &left.normalized_host,
            left.port,
            &left.path_prefix,
        )
            .cmp(&(
                &right.scheme,
                &right.normalized_host,
                right.port,
                &right.path_prefix,
            ))
    });
    endpoints.dedup();
    if endpoints.iter().any(|endpoint| {
        endpoint.scheme != "https"
            || endpoint.normalized_host.trim().is_empty()
            || endpoint.normalized_host != endpoint.normalized_host.to_ascii_lowercase()
            || !(1..=65535).contains(&endpoint.port)
            || !endpoint.path_prefix.starts_with('/')
            || endpoint.path_prefix.contains('%')
            || endpoint.path_prefix.contains('_')
    }) {
        return Err(fail(CONTRACT_INVALID));
    }

    let mut tx = pool.begin().await?;
    let (execution_authority_id,): (Uuid,) = sqlx::query_as(
        r#"SELECT execution_authority_id
             FROM coverage_denominators d
            WHERE d.id=$1 AND d.sealed_at IS NOT NULL
              AND EXISTS(
                  SELECT 1 FROM coverage_denominator_items i
                   WHERE i.denominator_id=d.id AND i.expected_capability=$2
              )
            FOR SHARE"#,
    )
    .bind(command.denominator_id)
    .bind(&command.capability)
    .fetch_optional(&mut *tx)
    .await?
    .ok_or_else(|| fail(AUTHORITY_STALE))?;

    let tls_policy_hash = sha256_json(&serde_json::json!({
        "tls_hostname_verification": "required",
        "minimum_version": "1.2",
    }))?;
    let prohibited_range_policy_hash = sha256_json(&serde_json::json!({
        "mixed_public_private_dns": "deny",
        "loopback": "deny",
        "private": "deny",
        "link_local": "deny",
        "multicast": "deny",
        "unspecified": "deny",
        "rebind": "deny",
    }))?;
    let mut member_hashes = Vec::with_capacity(endpoints.len());
    for (ordinal, endpoint) in endpoints.iter().enumerate() {
        member_hashes.push(sha256_json(&serde_json::json!({
            "ordinal": ordinal,
            "destination_role": "fixed_provider_endpoint",
            "scheme": endpoint.scheme,
            "normalized_host": endpoint.normalized_host,
            "port": endpoint.port,
            "path_prefix": endpoint.path_prefix,
            "input_binding_mode": "escaped_parameter_only",
        }))?);
    }
    let policy_hash = sha256_json(&serde_json::json!({
        "denominator_id": command.denominator_id,
        "execution_authority_id": execution_authority_id,
        "capability": command.capability,
        "policy_contract_version": "tool_execution_destination.v1",
        "execution_backend": "fixed_provider_transport",
        "governance_status": "enforced",
        "redirect_mode": "deny",
        "max_redirect_hops": 0,
        "tls_policy_hash": tls_policy_hash,
        "prohibited_range_policy_hash": prohibited_range_policy_hash,
        "members": member_hashes,
    }))?;

    if let Some(existing) = sqlx::query_as::<_, SealedDestinationPolicy>(
        r#"SELECT id,execution_authority_id,policy_hash
             FROM capability_execution_destination_policies
            WHERE denominator_id=$1 AND capability=$2 AND policy_hash=$3
              AND sealed_at IS NOT NULL
            FOR SHARE"#,
    )
    .bind(command.denominator_id)
    .bind(&command.capability)
    .bind(&policy_hash)
    .fetch_optional(&mut *tx)
    .await?
    {
        let persisted = sqlx::query_scalar::<_, String>(
            "SELECT member_hash FROM capability_execution_destination_policy_members WHERE policy_id=$1 ORDER BY ordinal",
        )
        .bind(existing.id)
        .fetch_all(&mut *tx)
        .await?;
        if existing.execution_authority_id != execution_authority_id || persisted != member_hashes {
            return Err(fail(MANIFEST_DRIFT));
        }
        tx.commit().await?;
        return Ok(existing);
    }

    let policy_id = Uuid::new_v5(&command.denominator_id, policy_hash.as_bytes());
    sqlx::query(
        r#"INSERT INTO capability_execution_destination_policies(
               id,denominator_id,execution_authority_id,capability,
               execution_backend,governance_status,redirect_mode,max_redirect_hops,
               tls_policy_hash,prohibited_range_policy_hash,policy_hash
           ) VALUES($1,$2,$3,$4,'fixed_provider_transport','enforced','deny',0,$5,$6,$7)"#,
    )
    .bind(policy_id)
    .bind(command.denominator_id)
    .bind(execution_authority_id)
    .bind(&command.capability)
    .bind(tls_policy_hash)
    .bind(prohibited_range_policy_hash)
    .bind(&policy_hash)
    .execute(&mut *tx)
    .await?;
    for (ordinal, (endpoint, member_hash)) in endpoints.iter().zip(member_hashes.iter()).enumerate()
    {
        sqlx::query(
            r#"INSERT INTO capability_execution_destination_policy_members(
                   id,policy_id,execution_authority_id,ordinal,destination_role,
                   scheme,normalized_host,port,path_prefix,input_binding_mode,member_hash
               ) VALUES($1,$2,$3,$4,'fixed_provider_endpoint',$5,$6,$7,$8,
                        'escaped_parameter_only',$9)"#,
        )
        .bind(Uuid::new_v5(&policy_id, member_hash.as_bytes()))
        .bind(policy_id)
        .bind(execution_authority_id)
        .bind(i32::try_from(ordinal).map_err(|_| fail(CONTRACT_INVALID))?)
        .bind(&endpoint.scheme)
        .bind(&endpoint.normalized_host)
        .bind(endpoint.port)
        .bind(&endpoint.path_prefix)
        .bind(member_hash)
        .execute(&mut *tx)
        .await?;
    }
    let row = sqlx::query_as::<_, SealedDestinationPolicy>(
        r#"UPDATE capability_execution_destination_policies
              SET sealed_at=statement_timestamp()
            WHERE id=$1
            RETURNING id,execution_authority_id,policy_hash"#,
    )
    .bind(policy_id)
    .fetch_one(&mut *tx)
    .await?;
    tx.commit().await?;
    Ok(row)
}

pub async fn begin(
    pool: &PgPool,
    command: &BeginCapabilityReceipt,
) -> Result<CapabilityExecutionReceiptRow> {
    Ok(begin_on(
        pool,
        command.id,
        command.denominator_id,
        &command.capability,
        command.attempt_ordinal,
        None,
    )
    .await?
    .0)
}

pub async fn begin_managed(
    pool: &PgPool,
    command: &BeginManagedCapabilityReceipt,
) -> Result<CapabilityExecutionReceiptRow> {
    Ok(begin_on(
        pool,
        command.id,
        command.denominator_id,
        &command.capability,
        command.attempt_ordinal,
        Some(command.destination_policy_id),
    )
    .await?
    .0)
}

pub async fn begin_managed_claim(
    pool: &PgPool,
    command: &BeginManagedCapabilityReceipt,
) -> Result<ManagedReceiptBeginOutcome> {
    let (row, created) = begin_on(
        pool,
        command.id,
        command.denominator_id,
        &command.capability,
        command.attempt_ordinal,
        Some(command.destination_policy_id),
    )
    .await?;
    Ok(if created {
        ManagedReceiptBeginOutcome::Created(row)
    } else if row.finalized_at.is_some() {
        ManagedReceiptBeginOutcome::TerminalReplay(row)
    } else {
        ManagedReceiptBeginOutcome::InFlight(row)
    })
}

async fn begin_on(
    pool: &PgPool,
    receipt_id: Uuid,
    denominator_id: Uuid,
    capability: &str,
    attempt_ordinal: i32,
    managed_destination_policy_id: Option<Uuid>,
) -> Result<(CapabilityExecutionReceiptRow, bool)> {
    if attempt_ordinal <= 0 || capability.trim().is_empty() {
        return Err(fail(CONTRACT_INVALID));
    }
    let mut tx = pool.begin().await?;
    sqlx::query("SELECT pg_advisory_xact_lock(hashtextextended($1,0))")
        .bind(format!(
            "tool-truth-receipt:{denominator_id}:{capability}:{attempt_ordinal}"
        ))
        .execute(&mut *tx)
        .await?;
    let denominator = sqlx::query_as::<_, (Uuid, String, String, Option<DateTime<Utc>>)>(
        "SELECT execution_authority_id,input_manifest_hash,denominator_hash,sealed_at FROM coverage_denominators WHERE id=$1 FOR SHARE",
    )
    .bind(denominator_id)
    .fetch_optional(&mut *tx)
    .await?
    .ok_or_else(|| fail(AUTHORITY_STALE))?;
    if denominator.3.is_none() {
        return Err(fail(DENOMINATOR_UNSEALED));
    }
    let exact_capability: bool = sqlx::query_scalar(
        "SELECT EXISTS(SELECT 1 FROM coverage_denominator_items WHERE denominator_id=$1 AND expected_capability=$2)",
    )
    .bind(denominator_id)
    .bind(capability)
    .fetch_one(&mut *tx)
    .await?;
    if !exact_capability {
        return Err(fail(MANIFEST_DRIFT));
    }

    let (destination_policy_id, destination_policy_hash) = if let Some(policy_id) =
        managed_destination_policy_id
    {
        sqlx::query_as::<_, (Uuid, String)>(
            r#"SELECT id,policy_hash FROM capability_execution_destination_policies
                    WHERE id=$1 AND denominator_id=$2 AND execution_authority_id=$3
                      AND capability=$4 AND execution_backend='fixed_provider_transport'
                      AND governance_status='enforced' AND sealed_at IS NOT NULL
                    FOR SHARE"#,
        )
        .bind(policy_id)
        .bind(denominator_id)
        .bind(denominator.0)
        .bind(capability)
        .fetch_optional(&mut *tx)
        .await?
        .ok_or_else(|| fail(AUTHORITY_STALE))?
    } else {
        let policy_hash = sha256_json(&serde_json::json!({
            "denominator_id": denominator_id,
            "execution_authority_id": denominator.0,
            "capability": capability,
            "execution_backend": "none_blocked",
            "governance_status": "policy_blocked",
        }))?;
        let tls_policy_hash = sha256_json(&serde_json::json!({"policy": "deny"}))?;
        let prohibited_range_policy_hash = sha256_json(&serde_json::json!({"policy": "deny"}))?;
        let id = if let Some(id) = sqlx::query_scalar::<_, Uuid>(
            r#"SELECT id FROM capability_execution_destination_policies
                    WHERE denominator_id=$1 AND execution_authority_id=$2
                      AND capability=$3 AND policy_hash=$4 AND sealed_at IS NOT NULL
                    FOR SHARE"#,
        )
        .bind(denominator_id)
        .bind(denominator.0)
        .bind(capability)
        .bind(&policy_hash)
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
            .bind(denominator_id)
            .bind(denominator.0)
            .bind(capability)
            .bind(tls_policy_hash)
            .bind(prohibited_range_policy_hash)
            .bind(&policy_hash)
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
        (id, policy_hash)
    };

    if let Some(existing) = sqlx::query_as::<_, CapabilityExecutionReceiptRow>(&format!(
        "SELECT {RECEIPT_COLUMNS} FROM capability_execution_receipts WHERE denominator_id=$1 AND execution_authority_id=$2 AND capability=$3 AND attempt_ordinal=$4 FOR SHARE"
    ))
    .bind(denominator_id)
    .bind(denominator.0)
    .bind(capability)
    .bind(attempt_ordinal)
    .fetch_optional(&mut *tx)
    .await?
    {
        let (existing_policy_id, existing_policy_hash): (Uuid, String) = sqlx::query_as(
            "SELECT destination_policy_id,destination_policy_hash FROM capability_execution_receipts WHERE id=$1",
        )
        .bind(existing.id)
        .fetch_one(&mut *tx)
        .await?;
        if existing.input_manifest_hash != denominator.1
            || existing_policy_id != destination_policy_id
            || existing_policy_hash != destination_policy_hash
        {
            return Err(fail(MANIFEST_DRIFT));
        }
        tx.commit().await?;
        return Ok((existing, false));
    }

    supersede_prior_attempts_on(
        &mut tx,
        denominator_id,
        denominator.0,
        capability,
        attempt_ordinal,
    )
    .await?;

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
        "denominator_id": denominator_id,
        "denominator_hash": denominator.2,
        "execution_authority_id": denominator.0,
        "capability": capability,
        "attempt_ordinal": attempt_ordinal,
        "input_manifest_hash": denominator.1,
        "destination_policy_hash": destination_policy_hash,
        "temporal_validity_policy_hash": temporal_policy_hash,
    }))?;
    let typed_landing = serde_json::json!({
        "capability": capability,
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
    .bind(receipt_id)
    .bind(denominator_id)
    .bind(denominator.0)
    .bind(capability)
    .bind(attempt_ordinal)
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
    Ok((row, true))
}

async fn supersede_prior_attempts_on(
    tx: &mut Transaction<'_, Postgres>,
    denominator_id: Uuid,
    execution_authority_id: Uuid,
    capability: &str,
    current_attempt_ordinal: i32,
) -> Result<()> {
    let prior = sqlx::query_as::<_, (Uuid, i64, i64, Option<Uuid>)>(
        r#"SELECT id,row_version,current_semantic_authority_version,current_semantic_reconciliation_id
             FROM capability_execution_receipts
            WHERE denominator_id=$1 AND execution_authority_id=$2 AND capability=$3
              AND attempt_ordinal<$4 AND reconciliation_state<>'superseded'
            ORDER BY attempt_ordinal,id FOR UPDATE"#,
    )
    .bind(denominator_id)
    .bind(execution_authority_id)
    .bind(capability)
    .bind(current_attempt_ordinal)
    .fetch_all(&mut **tx)
    .await?;
    for (receipt_id, row_version, semantic_version, predecessor_id) in prior {
        let reconciliation_id = Uuid::new_v5(
            &receipt_id,
            format!("superseded-by-attempt:{current_attempt_ordinal}").as_bytes(),
        );
        let next_version = semantic_version + 1;
        sqlx::query(
            r#"INSERT INTO capability_execution_reconciliations(
                   id,receipt_id,execution_authority_id,semantic_authority_version,
                   predecessor_reconciliation_id,reconciliation_state
               ) VALUES($1,$2,$3,$4,$5,'pending')"#,
        )
        .bind(reconciliation_id)
        .bind(receipt_id)
        .bind(execution_authority_id)
        .bind(next_version)
        .bind(predecessor_id)
        .execute(&mut **tx)
        .await?;
        let semantic_hash: String = sqlx::query_scalar(
            r#"UPDATE capability_execution_reconciliations
                  SET reconciliation_state='superseded',reason_code='newer_attempt_started',
                      sealed_at=statement_timestamp()
                WHERE id=$1 RETURNING semantic_reconciliation_hash"#,
        )
        .bind(reconciliation_id)
        .fetch_one(&mut **tx)
        .await?;
        let updated = sqlx::query(
            r#"UPDATE capability_execution_receipts
                  SET attempt_state='superseded',landing_state=CASE
                          WHEN landing_state='committed' THEN 'partial' ELSE landing_state END,
                      observation_state=CASE WHEN observation_state='found' THEN 'found'
                          ELSE 'indeterminate' END,
                      coverage_extent=CASE WHEN coverage_extent='complete' THEN 'partial'
                          ELSE coverage_extent END,
                      coverage_gap_reason=CASE WHEN coverage_gap_reason='none'
                          THEN 'source_unavailable' ELSE coverage_gap_reason END,
                      reconciliation_state='superseded',
                      security_interpretation=CASE WHEN observation_state='found'
                          THEN 'signal' ELSE 'inconclusive' END,
                      current_semantic_authority_version=$3,
                      current_semantic_reconciliation_id=$4,
                      current_semantic_reconciliation_hash=$5,
                      row_version=row_version+1,finalized_at=COALESCE(finalized_at,statement_timestamp())
                WHERE id=$1 AND row_version=$2"#,
        )
        .bind(receipt_id)
        .bind(row_version)
        .bind(next_version)
        .bind(reconciliation_id)
        .bind(semantic_hash)
        .execute(&mut **tx)
        .await?;
        if updated.rows_affected() != 1 {
            return Err(fail(RECEIPT_STALE));
        }
    }
    Ok(())
}

pub async fn current_target_intel_receipt_context(
    pool: &PgPool,
    operation_id: Uuid,
    organization_id: Uuid,
    stage_execution_id: Uuid,
    capability: &str,
    attempt_fence: Option<&TargetIntelAttemptFence>,
) -> Result<Option<CurrentTargetIntelReceiptContext>> {
    if operation_id.is_nil()
        || organization_id.is_nil()
        || stage_execution_id.is_nil()
        || capability.trim().is_empty()
    {
        return Err(fail(CONTRACT_INVALID));
    }
    let attempt_ordinal = match attempt_fence {
        Some(fence) => fence
            .worker_attempt_epoch
            .checked_add(1)
            .ok_or_else(|| fail(CONTRACT_INVALID))?,
        None => 1,
    };
    if attempt_ordinal <= 0 {
        return Err(fail(CONTRACT_INVALID));
    }
    let worker_run_id = attempt_fence.map(|fence| fence.worker_run_id);
    let stage_run_unit_id = attempt_fence.map(|fence| fence.stage_run_unit_id);
    let lease_token = attempt_fence.map(|fence| fence.lease_token);
    let worker_attempt_epoch = attempt_fence.map(|fence| fence.worker_attempt_epoch);
    let source_tool_call_id = attempt_fence.map(|fence| fence.source_tool_call_id);
    let contexts = sqlx::query_as::<_, CurrentTargetIntelReceiptContext>(
        r#"SELECT d.id AS denominator_id,d.execution_authority_id,d.denominator_hash,
                  $5::bigint AS attempt_epoch
             FROM coverage_denominators d
             JOIN stage_runs r
               ON r.id=d.stage_execution_id AND r.operation_id=d.operation_id
              AND r.stage_kind='target_intel' AND r.status='started'
             JOIN operation_state o
               ON o.operation_id=d.operation_id
              AND o.tool_truth_contract IN ('shadow_v1','receipt_v1')
            WHERE d.operation_id=$1 AND d.organization_id=$2
              AND d.stage_execution_id=$3 AND d.stage_kind='target_intel'
              AND d.denominator_kind='root' AND d.sealed_at IS NOT NULL
              AND EXISTS(
                  SELECT 1 FROM coverage_denominator_items i
                   WHERE i.denominator_id=d.id AND i.expected_capability=$4
              )
              AND (
                  $6::uuid IS NULL
                  OR EXISTS(
                      SELECT 1
                        FROM stage_worker_runs w
                        JOIN tool_calls t
                          ON t.id=$10
                         AND t.worker_run_id=w.id
                         AND t.stage_run_unit_id=w.stage_run_unit_id
                         AND t.attempt_epoch=w.attempt_epoch
                         AND t.lease_token=w.lease_token
                       WHERE w.id=$6 AND w.stage_run_unit_id=$7
                         AND w.operation_id=$1 AND w.organization_id=$2
                         AND w.stage_execution_id=$3 AND w.lease_token=$8
                         AND w.attempt_epoch=$9
                         AND w.status='running'
                         AND w.lease_expires_at>statement_timestamp()
                  )
              )
            ORDER BY d.sealed_at DESC,d.id"#,
    )
    .bind(operation_id)
    .bind(organization_id)
    .bind(stage_execution_id)
    .bind(capability)
    .bind(attempt_ordinal)
    .bind(worker_run_id)
    .bind(stage_run_unit_id)
    .bind(lease_token)
    .bind(worker_attempt_epoch)
    .bind(source_tool_call_id)
    .fetch_all(pool)
    .await?;
    match contexts.as_slice() {
        [] => Ok(None),
        [context] => Ok(Some(context.clone())),
        _ => Err(fail(AUTHORITY_STALE)),
    }
}

pub async fn current_target_intel_projection(
    pool: &PgPool,
    operation_id: Uuid,
    organization_id: Uuid,
) -> Result<Option<CurrentTargetIntelReceiptProjection>> {
    let stage_execution_ids = sqlx::query_scalar::<_, Uuid>(
        r#"SELECT id FROM stage_runs
            WHERE operation_id=$1 AND stage_kind='target_intel' AND status='started'
            ORDER BY started_at,id"#,
    )
    .bind(operation_id)
    .fetch_all(pool)
    .await?;
    let stage_execution_id = match stage_execution_ids.as_slice() {
        [] => return Ok(None),
        [stage_execution_id] => *stage_execution_id,
        _ => return Err(fail(AUTHORITY_STALE)),
    };
    let denominators = sqlx::query_as::<_, (Uuid, String)>(
        r#"SELECT d.id,d.denominator_hash
             FROM coverage_denominators d
             JOIN tool_truth_execution_authorities a
               ON a.id=d.execution_authority_id
            WHERE a.operation_id=$1 AND a.organization_id=$2
              AND a.stage_execution_id=$3 AND a.stage_kind='target_intel'
              AND d.denominator_kind='root'
              AND d.sealed_at IS NOT NULL
            ORDER BY (a.execution_source_kind='stage_unit') DESC,d.sealed_at DESC,d.id"#,
    )
    .bind(operation_id)
    .bind(organization_id)
    .bind(stage_execution_id)
    .fetch_all(pool)
    .await?;
    let Some((denominator_id, denominator_hash)) = denominators.first().cloned() else {
        return Ok(None);
    };
    if denominators
        .iter()
        .skip(1)
        .any(|row| row.0 != denominator_id || row.1 != denominator_hash)
    {
        return Err(fail(MANIFEST_DRIFT));
    }
    let expected = sqlx::query_as::<_, TargetIntelExpectedInputRow>(
        r#"SELECT input_key,exact_asset,technique FROM coverage_denominator_items
            WHERE denominator_id=$1 ORDER BY ordinal"#,
    )
    .bind(denominator_id)
    .fetch_all(pool)
    .await?;
    let attempt_epoch = sqlx::query_scalar::<_, Option<i64>>(
        "SELECT max(attempt_ordinal)::bigint FROM capability_execution_receipts WHERE denominator_id=$1",
    )
    .bind(denominator_id)
    .fetch_one(pool)
    .await?
    .unwrap_or(0);
    let receipts = sqlx::query_as::<_, TargetIntelReceiptProjectionRow>(
        r#"SELECT r.denominator_id,a.stage_execution_id,r.attempt_ordinal::bigint AS attempt_epoch,
                  i.input_key,di.technique,r.reconciliation_state,i.landing_state,
                  i.observation_state,i.coverage_extent,
                  (r.attempt_ordinal=$2 AND r.reconciliation_state='consistent'
                   AND r.finalized_at IS NOT NULL
                   AND r.valid_until>statement_timestamp()
                   AND r.current_semantic_reconciliation_id IS NOT NULL) AS authority_current
             FROM capability_execution_receipts r
             JOIN tool_truth_execution_authorities a ON a.id=r.execution_authority_id
             JOIN capability_execution_receipt_inputs i ON i.receipt_id=r.id
             JOIN coverage_denominator_items di
               ON di.id=i.denominator_item_id AND di.denominator_id=i.denominator_id
            WHERE r.denominator_id=$1 AND i.sealed_at IS NOT NULL
            ORDER BY r.attempt_ordinal,i.input_key,r.id"#,
    )
    .bind(denominator_id)
    .bind(i32::try_from(attempt_epoch).unwrap_or(i32::MAX))
    .fetch_all(pool)
    .await?;
    Ok(Some(CurrentTargetIntelReceiptProjection {
        denominator_id,
        denominator_hash,
        stage_execution_id,
        attempt_epoch,
        expected,
        receipts,
    }))
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

fn network_address_is_public(address: IpAddr) -> bool {
    match address {
        IpAddr::V4(address) => {
            let octets = address.octets();
            !matches!(
                octets,
                [0, ..]
                    | [10, ..]
                    | [100, 64..=127, ..]
                    | [127, ..]
                    | [169, 254, ..]
                    | [172, 16..=31, ..]
                    | [192, 0, 0, ..]
                    | [192, 0, 2, ..]
                    | [192, 168, ..]
                    | [198, 18..=19, ..]
                    | [198, 51, 100, ..]
                    | [203, 0, 113, ..]
                    | [224..=255, ..]
            )
        }
        IpAddr::V6(address) => {
            if let Some(mapped) = address.to_ipv4_mapped() {
                return network_address_is_public(IpAddr::V4(mapped));
            }
            let segments = address.segments();
            !(address.is_unspecified()
                || address.is_loopback()
                || address.is_multicast()
                || (segments[0] & 0xfe00) == 0xfc00
                || (segments[0] & 0xffc0) == 0xfe80
                || (segments[0] == 0x2001 && segments[1] == 0x0db8))
        }
    }
}

fn target_intel_finalization_request_hash(command: &FinalizeTargetIntelReceipt) -> Result<String> {
    let mut observations = command
        .input_observations
        .iter()
        .map(|observation| {
            serde_json::json!({
                "input_key": observation.input_key,
                "technique": observation.technique,
                "observation_state": observation.observation_state,
            })
        })
        .collect::<Vec<_>>();
    observations.sort_by_key(|value| value.to_string());
    let mut hops = command
        .network_hops
        .iter()
        .map(|hop| {
            let mut addresses = hop
                .addresses
                .iter()
                .map(ToString::to_string)
                .collect::<Vec<_>>();
            addresses.sort();
            addresses.dedup();
            serde_json::json!({
                "hop_kind": hop.hop_kind,
                "scheme": hop.scheme,
                "normalized_host": hop.normalized_host,
                "port": hop.port,
                "path_and_query": hop.path_and_query,
                "addresses": addresses,
                "selected_address": hop.selected_address.to_string(),
                "send_ordinal": hop.send_ordinal,
            })
        })
        .collect::<Vec<_>>();
    hops.sort_by_key(|value| value.to_string());
    sha256_json(&serde_json::json!({
        "receipt_id": command.receipt_id,
        "attempt_fence": command.attempt_fence.as_ref().map(|fence| serde_json::json!({
            "worker_run_id": fence.worker_run_id,
            "stage_run_unit_id": fence.stage_run_unit_id,
            "lease_token": fence.lease_token,
            "worker_attempt_epoch": fence.worker_attempt_epoch,
            "source_tool_call_id": fence.source_tool_call_id,
        })),
        "raw_witness": {
            "artifact_id": command.raw_witness.artifact_id,
            "content_key": command.raw_witness.content_key,
            "vault_object_ref_token": command.raw_witness.vault_object_ref_token,
            "vault_object_ref_token_hash": command.raw_witness.vault_object_ref_token_hash,
            "sha256": command.raw_witness.sha256,
            "ciphertext_sha256": command.raw_witness.ciphertext_sha256,
            "operation_key_ref_hash": command.raw_witness.operation_key_ref_hash,
            "key_generation": command.raw_witness.key_generation,
            "retention_policy_id": command.raw_witness.retention_policy_id,
            "retention_policy_hash": command.raw_witness.retention_policy_hash,
            "sensitivity_disposition": command.raw_witness.sensitivity_disposition,
            "original_byte_count": command.raw_witness.original_byte_count,
            "stored_byte_count": command.raw_witness.stored_byte_count,
            "truncated": command.raw_witness.truncated,
        },
        "network_hops": hops,
        "request_count": command.request_count,
        "response_byte_count": command.response_byte_count,
        "wall_clock_ms": command.wall_clock_ms,
        "retry_count": command.retry_count,
        "parser_complete": command.parser_complete,
        "normalized_record_count": command.normalized_record_count,
        "input_observations": observations,
        "typed_landing": command.typed_landing,
        "failure_reason_code": command.failure_reason_code,
    }))
}

/// Atomically seals the server-observed TargetIntel lifecycle.  The command
/// carries observations, never authority hashes for sets or semantic heads;
/// every census/hash is recomputed while the receipt is locked.
pub async fn finalize_target_intel_receipt(
    pool: &PgPool,
    command: &FinalizeTargetIntelReceipt,
) -> Result<CapabilityExecutionReceiptRow> {
    if command.receipt_id.is_nil()
        || command.request_count < 0
        || command.response_byte_count < 0
        || command.wall_clock_ms < 0
        || command.retry_count < 0
        || command.normalized_record_count < 0
        || !command.typed_landing.is_object()
        || command.raw_witness.vault_object_ref_token.len() < 32
        || command.raw_witness.key_generation <= 0
        || command.raw_witness.original_byte_count < command.raw_witness.stored_byte_count
    {
        return Err(fail(CONTRACT_INVALID));
    }
    let mut observations = command.input_observations.clone();
    observations.sort_by(|left, right| {
        (&left.input_key, &left.technique).cmp(&(&right.input_key, &right.technique))
    });
    observations.dedup_by(|left, right| {
        left.input_key == right.input_key && left.technique == right.technique
    });
    if observations.iter().any(|observation| {
        observation.input_key.trim().is_empty()
            || observation.technique.trim().is_empty()
            || !matches!(
                observation.observation_state.as_str(),
                "found" | "no_match" | "indeterminate"
            )
    }) || observations.len() != command.input_observations.len()
    {
        return Err(fail(CONTRACT_INVALID));
    }
    if command.network_hops.iter().any(|hop| {
        !matches!(hop.hop_kind.as_str(), "initial" | "redirect" | "retry")
            || hop.scheme != "https"
            || hop.normalized_host.trim().is_empty()
            || hop.normalized_host != hop.normalized_host.to_ascii_lowercase()
            || !(1..=65535).contains(&hop.port)
            || !hop.path_and_query.starts_with('/')
            || hop.send_ordinal <= 0
            || hop.addresses.is_empty()
            || !hop.addresses.contains(&hop.selected_address)
            || hop
                .addresses
                .iter()
                .any(|address| !network_address_is_public(*address))
    }) {
        return Err(fail(CONTRACT_INVALID));
    }
    let mut send_ordinals = command
        .network_hops
        .iter()
        .map(|hop| hop.send_ordinal)
        .collect::<Vec<_>>();
    send_ordinals.sort_unstable();
    send_ordinals.dedup();
    if send_ordinals.len() != command.network_hops.len()
        || usize::try_from(command.request_count).ok() != Some(command.network_hops.len())
        || command.response_byte_count != command.raw_witness.stored_byte_count
    {
        return Err(fail(CONTRACT_INVALID));
    }
    let finalization_request_hash = target_intel_finalization_request_hash(command)?;
    let mut fence_current = true;

    let mut tx = pool.begin().await?;
    let receipt = sqlx::query_as::<
        _,
        (
            Uuid,
            Uuid,
            String,
            String,
            Uuid,
            String,
            Uuid,
            String,
            i64,
            i64,
            Option<Uuid>,
            Option<Uuid>,
            String,
            String,
            Option<DateTime<Utc>>,
            Option<String>,
        ),
    >(
        r#"SELECT denominator_id,execution_authority_id,capability,receipt_authority_hash,
                  destination_policy_id,destination_policy_hash,temporal_validity_policy_id,
                  temporal_validity_policy_hash,row_version,current_semantic_authority_version,
                  current_semantic_reconciliation_id,raw_witness_artifact_id,
                  attempt_state,reconciliation_state,finalized_at,finalization_request_hash
             FROM capability_execution_receipts
            WHERE id=$1 FOR UPDATE"#,
    )
    .bind(command.receipt_id)
    .fetch_optional(&mut *tx)
    .await?
    .ok_or_else(|| fail(AUTHORITY_STALE))?;
    if let Some(existing_artifact_id) = receipt.11 {
        if existing_artifact_id != command.raw_witness.artifact_id
            || receipt.15.as_deref() != Some(finalization_request_hash.as_str())
        {
            return Err(fail(MANIFEST_DRIFT));
        }
        let row = sqlx::query_as::<_, CapabilityExecutionReceiptRow>(&format!(
            "SELECT {RECEIPT_COLUMNS} FROM capability_execution_receipts WHERE id=$1"
        ))
        .bind(command.receipt_id)
        .fetch_one(&mut *tx)
        .await?;
        tx.commit().await?;
        return Ok(row);
    }
    let superseded = receipt.12 == "superseded" || receipt.13 == "superseded";
    if receipt.14.is_some() && !superseded {
        return Err(fail(MANIFEST_DRIFT));
    }
    if !superseded && receipt.8 != command.expected_row_version {
        return Err(fail(RECEIPT_STALE));
    }
    let destination_enforced: bool = sqlx::query_scalar(
        r#"SELECT EXISTS(
              SELECT 1 FROM capability_execution_destination_policies
               WHERE id=$1 AND execution_authority_id=$2 AND policy_hash=$3
                 AND execution_backend='fixed_provider_transport'
                 AND governance_status='enforced' AND sealed_at IS NOT NULL
           )"#,
    )
    .bind(receipt.4)
    .bind(receipt.1)
    .bind(&receipt.5)
    .fetch_one(&mut *tx)
    .await?;
    if !destination_enforced {
        return Err(fail(AUTHORITY_STALE));
    }
    for hop in &command.network_hops {
        let endpoint_allowed: bool = sqlx::query_scalar(
            r#"SELECT EXISTS(
                  SELECT 1 FROM capability_execution_destination_policy_members
                   WHERE policy_id=$1 AND scheme=$2 AND normalized_host=$3 AND port=$4
                     AND (split_part($5,'?',1)=path_prefix
                          OR (right(path_prefix,1)='/'
                              AND left(split_part($5,'?',1),length(path_prefix))=path_prefix))
               )"#,
        )
        .bind(receipt.4)
        .bind(&hop.scheme)
        .bind(&hop.normalized_host)
        .bind(hop.port)
        .bind(&hop.path_and_query)
        .fetch_one(&mut *tx)
        .await?;
        if !endpoint_allowed {
            return Err(fail(AUTHORITY_STALE));
        }
    }

    sqlx::query(
        r#"INSERT INTO capability_raw_witness_artifacts(
               id,receipt_id,execution_authority_id,receipt_authority_hash,content_key,
               vault_object_ref_token,vault_object_ref_token_hash,sha256,ciphertext_sha256,
               operation_key_ref_hash,key_generation,retention_policy_id,retention_policy_hash,
               sensitivity_disposition,original_byte_count,stored_byte_count,truncated
           ) VALUES($1,$2,$3,$4,$5,$6,$7,$8,$9,$10,$11,$12,$13,$14,$15,$16,$17)"#,
    )
    .bind(command.raw_witness.artifact_id)
    .bind(command.receipt_id)
    .bind(receipt.1)
    .bind(&receipt.3)
    .bind(&command.raw_witness.content_key)
    .bind(&command.raw_witness.vault_object_ref_token)
    .bind(&command.raw_witness.vault_object_ref_token_hash)
    .bind(&command.raw_witness.sha256)
    .bind(&command.raw_witness.ciphertext_sha256)
    .bind(&command.raw_witness.operation_key_ref_hash)
    .bind(command.raw_witness.key_generation)
    .bind(command.raw_witness.retention_policy_id)
    .bind(&command.raw_witness.retention_policy_hash)
    .bind(&command.raw_witness.sensitivity_disposition)
    .bind(command.raw_witness.original_byte_count)
    .bind(command.raw_witness.stored_byte_count)
    .bind(command.raw_witness.truncated)
    .execute(&mut *tx)
    .await?;

    let authority = sqlx::query_as::<
        _,
        (
            Uuid,
            Uuid,
            String,
            Uuid,
            Uuid,
            Uuid,
            String,
            String,
            String,
            Option<Uuid>,
            Option<i64>,
            Option<Uuid>,
            Option<Uuid>,
        ),
    >(
        r#"SELECT operation_id,project_scope_id,project_path_at_freeze,scope_snapshot_id,
                  organization_id,stage_execution_id,stage_kind,authority_hash,
                  execution_owner_kind,worker_run_id,worker_attempt_epoch,lease_token,
                  source_tool_call_id
             FROM tool_truth_execution_authorities
            WHERE id=$1 FOR SHARE"#,
    )
    .bind(receipt.1)
    .fetch_one(&mut *tx)
    .await?;
    let mut producer = serde_json::json!({
        "organization_id": authority.4,
        "stage_execution_id": authority.5,
        "receipt_id": command.receipt_id,
        "raw_witness_sha256": command.raw_witness.sha256,
        "finalization_request_hash": finalization_request_hash,
    });
    if authority.8 == "worker_tool" {
        let fence = command
            .attempt_fence
            .as_ref()
            .ok_or_else(|| fail(AUTHORITY_STALE))?;
        if authority.9 != Some(fence.worker_run_id)
            || authority.10 != Some(fence.worker_attempt_epoch)
            || authority.11 != Some(fence.lease_token)
            || authority.12 != Some(fence.source_tool_call_id)
        {
            return Err(fail(AUTHORITY_STALE));
        }
        fence_current = sqlx::query_scalar(
            r#"SELECT EXISTS(
                  SELECT 1 FROM stage_worker_runs w
                  JOIN tool_calls t ON t.id=$5
                   AND t.worker_run_id=w.id AND t.stage_run_unit_id=w.stage_run_unit_id
                   AND t.attempt_epoch=w.attempt_epoch AND t.lease_token=w.lease_token
                 WHERE w.id=$1 AND w.stage_run_unit_id=$2 AND w.lease_token=$3
                   AND w.attempt_epoch=$4 AND w.status='running'
                   AND w.lease_expires_at>statement_timestamp()
               )"#,
        )
        .bind(fence.worker_run_id)
        .bind(fence.stage_run_unit_id)
        .bind(fence.lease_token)
        .bind(fence.worker_attempt_epoch)
        .bind(fence.source_tool_call_id)
        .fetch_one(&mut *tx)
        .await?;
        let producer = producer
            .as_object_mut()
            .ok_or_else(|| fail(CONTRACT_INVALID))?;
        producer.insert("worker_run_id".to_string(), serde_json::json!(authority.9));
        producer.insert(
            "worker_attempt_epoch".to_string(),
            serde_json::json!(authority.10),
        );
        producer.insert("lease_token".to_string(), serde_json::json!(authority.11));
        producer.insert(
            "source_tool_call_id".to_string(),
            serde_json::json!(authority.12),
        );
    } else if command.attempt_fence.is_some() {
        return Err(fail(AUTHORITY_STALE));
    }
    let audit_id: i64 = sqlx::query_scalar(
        r#"INSERT INTO audit_log(
               action,category,details,project_path,source,status,detail,run_id,audit_role,
               evidence_technique,evidence_outcome
           ) VALUES('target_intel_receipt_observed','tool_truth',
                    'Canonical TargetIntel provider execution',$1,'tool_truth_receipt',
                    'completed',$2,$3,'evidence',$4,$5)
           RETURNING id"#,
    )
    .bind(&authority.2)
    .bind(serde_json::json!({"tool_truth_producer": producer}))
    .bind(authority.0)
    .bind(&receipt.2)
    .bind(if command.normalized_record_count > 0 {
        "found"
    } else {
        "indeterminate"
    })
    .fetch_one(&mut *tx)
    .await?;
    let scope_version: i64 =
        sqlx::query_scalar("SELECT scope_rules_version FROM organizations WHERE id=$1")
            .bind(authority.4)
            .fetch_one(&mut *tx)
            .await?;
    let classification_id: i64 = sqlx::query_scalar(
        r#"INSERT INTO evidence_classifications(
               evidence_audit_id,classification,scope_version,reason,
               classified_by_session,producing_stage_run_id
           ) VALUES($1,'in_scope',$2,'sealed TargetIntel execution','tool_truth_receipt',$3)
           RETURNING id"#,
    )
    .bind(audit_id)
    .bind(scope_version)
    .bind(authority.5)
    .fetch_one(&mut *tx)
    .await?;
    let production_binding_id = Uuid::new_v5(&command.receipt_id, b"evidence-production:v1");
    sqlx::query(
        r#"INSERT INTO tool_truth_evidence_production_bindings(
               id,execution_authority_id,operation_id,project_scope_id,
               project_path_at_freeze,scope_snapshot_id,organization_id,
               stage_execution_id,stage_kind,execution_authority_hash,
               evidence_audit_id,evidence_classification_id,production_binding_hash
           ) VALUES($1,$2,$3,$4,$5,$6,$7,$8,$9,$10,$11,$12,$13)"#,
    )
    .bind(production_binding_id)
    .bind(receipt.1)
    .bind(authority.0)
    .bind(authority.1)
    .bind(&authority.2)
    .bind(authority.3)
    .bind(authority.4)
    .bind(authority.5)
    .bind(&authority.6)
    .bind(&authority.7)
    .bind(audit_id)
    .bind(classification_id)
    .bind(sha256_json(
        &serde_json::json!({"untrusted": "server_recomputes"}),
    )?)
    .execute(&mut *tx)
    .await?;
    let evidence_authority_id = Uuid::new_v5(&command.receipt_id, b"evidence-authority:v1");
    let placeholder_hash = sha256_json(&serde_json::json!({"untrusted": "server_recomputes"}))?;
    sqlx::query(
        r#"INSERT INTO tool_truth_evidence_authorities(
               id,production_binding_id,execution_authority_id,operation_id,
               project_scope_id,project_path_at_freeze,scope_snapshot_id,organization_id,
               stage_execution_id,stage_kind,execution_authority_hash,evidence_audit_id,
               evidence_classification_id,audit_row_hash,classification_row_hash,
               evidence_chain_hash,authority_hash
           ) VALUES($1,$2,$3,$4,$5,$6,$7,$8,$9,$10,$11,$12,$13,$14,$14,$14,$14)"#,
    )
    .bind(evidence_authority_id)
    .bind(production_binding_id)
    .bind(receipt.1)
    .bind(authority.0)
    .bind(authority.1)
    .bind(&authority.2)
    .bind(authority.3)
    .bind(authority.4)
    .bind(authority.5)
    .bind(&authority.6)
    .bind(&authority.7)
    .bind(audit_id)
    .bind(classification_id)
    .bind(placeholder_hash)
    .execute(&mut *tx)
    .await?;

    for (hop_ordinal, hop) in command.network_hops.iter().enumerate() {
        let mut addresses = hop.addresses.clone();
        addresses.sort();
        addresses.dedup();
        let hop_id = Uuid::new_v5(
            &command.receipt_id,
            format!("network-hop:{hop_ordinal}:{}", hop.send_ordinal).as_bytes(),
        );
        let path_and_query_hash = sha256_json(&serde_json::json!(hop.path_and_query))?;
        let hop_hash = sha256_json(&serde_json::json!({
            "receipt_id": command.receipt_id,
            "hop_ordinal": hop_ordinal,
            "hop_kind": hop.hop_kind,
            "scheme": hop.scheme,
            "normalized_host": hop.normalized_host,
            "port": hop.port,
            "path_and_query_hash": path_and_query_hash,
            "destination_policy_hash": receipt.5,
            "send_ordinal": hop.send_ordinal,
        }))?;
        sqlx::query(
            r#"INSERT INTO capability_execution_network_hops(
                   id,receipt_id,execution_authority_id,receipt_authority_hash,hop_ordinal,
                   hop_kind,scheme,normalized_host,port,path_and_query_hash,
                   destination_policy_id,destination_policy_hash,transport_decision,
                   send_ordinal,hop_hash
               ) VALUES($1,$2,$3,$4,$5,$6,$7,$8,$9,$10,$11,$12,
                        'authorized_and_sent',$13,$14)"#,
        )
        .bind(hop_id)
        .bind(command.receipt_id)
        .bind(receipt.1)
        .bind(&receipt.3)
        .bind(i32::try_from(hop_ordinal).map_err(|_| fail(CONTRACT_INVALID))?)
        .bind(&hop.hop_kind)
        .bind(&hop.scheme)
        .bind(&hop.normalized_host)
        .bind(hop.port)
        .bind(path_and_query_hash)
        .bind(receipt.4)
        .bind(&receipt.5)
        .bind(hop.send_ordinal)
        .bind(hop_hash)
        .execute(&mut *tx)
        .await?;
        for (ordinal, address) in addresses.iter().enumerate() {
            let member_hash = sha256_json(&serde_json::json!({
                "ordinal": ordinal,
                "address": address.to_string(),
                "selected_for_pin": *address == hop.selected_address,
            }))?;
            sqlx::query(
                r#"INSERT INTO capability_execution_network_hop_addresses(
                       id,network_hop_id,receipt_id,execution_authority_id,ordinal,address,
                       address_class,selected_for_pin,member_hash
                   ) VALUES($1,$2,$3,$4,$5,$6::inet,'public',$7,$8)"#,
            )
            .bind(Uuid::new_v5(&hop_id, member_hash.as_bytes()))
            .bind(hop_id)
            .bind(command.receipt_id)
            .bind(receipt.1)
            .bind(i32::try_from(ordinal).map_err(|_| fail(CONTRACT_INVALID))?)
            .bind(address.to_string())
            .bind(*address == hop.selected_address)
            .bind(member_hash)
            .execute(&mut *tx)
            .await?;
        }
        sqlx::query(
            "UPDATE capability_execution_network_hops SET sealed_at=statement_timestamp() WHERE id=$1",
        )
        .bind(hop_id)
        .execute(&mut *tx)
        .await?;
    }

    let parser_census_id = Uuid::new_v5(&command.receipt_id, b"parser-census:v1");
    let parser_digest = sha256_json(&serde_json::json!({
        "parser": "target_intel_provider_result",
        "version": 1,
    }))?;
    let framing_manifest_hash = sha256_json(&serde_json::json!({
        "stored_byte_count": command.raw_witness.stored_byte_count,
        "complete": command.parser_complete,
    }))?;
    sqlx::query(
        r#"INSERT INTO capability_parser_censuses(
               id,receipt_id,execution_authority_id,receipt_authority_hash,
               raw_witness_artifact_id,framer_contract_id,framer_contract_version,
               framer_digest,framing_manifest_hash,parser_contract_id,
               parser_contract_version,parser_digest,parse_domain_byte_count,
               framed_record_count,unaccounted_nonempty_record_count,sealed_empty
           ) VALUES($1,$2,$3,$4,$5,'target_intel.length_prefixed','1',$6,$7,
                    'target_intel.provider_result','1',$8,$9,$10,0,FALSE)"#,
    )
    .bind(parser_census_id)
    .bind(command.receipt_id)
    .bind(receipt.1)
    .bind(&receipt.3)
    .bind(command.raw_witness.artifact_id)
    .bind(&parser_digest)
    .bind(framing_manifest_hash)
    .bind(&parser_digest)
    .bind(command.raw_witness.stored_byte_count)
    .bind(if command.raw_witness.stored_byte_count > 0 {
        1_i64
    } else {
        0_i64
    })
    .execute(&mut *tx)
    .await?;
    let parser_member_id = if command.raw_witness.stored_byte_count > 0 {
        let member_id = Uuid::new_v5(&parser_census_id, b"raw-envelope:0");
        let record_hash = command.raw_witness.sha256.clone();
        let member_hash = sha256_json(&serde_json::json!({
            "record_hash": record_hash,
            "raw_start": 0,
            "raw_end": command.raw_witness.stored_byte_count,
            "disposition": "parsed_observation",
        }))?;
        sqlx::query(
            r#"INSERT INTO capability_parser_census_members(
                   id,census_id,receipt_id,execution_authority_id,ordinal,stream_kind,
                   raw_start,raw_end,record_hash,disposition,member_hash
               ) VALUES($1,$2,$3,$4,0,'envelope',0,$5,$6,'parsed_observation',$7)"#,
        )
        .bind(member_id)
        .bind(parser_census_id)
        .bind(command.receipt_id)
        .bind(receipt.1)
        .bind(command.raw_witness.stored_byte_count)
        .bind(record_hash)
        .bind(member_hash)
        .execute(&mut *tx)
        .await?;
        Some(member_id)
    } else {
        None
    };
    sqlx::query(
        "UPDATE capability_parser_censuses SET sealed_at=statement_timestamp() WHERE id=$1",
    )
    .bind(parser_census_id)
    .execute(&mut *tx)
    .await?;

    let items = sqlx::query_as::<_, (Uuid, String, String)>(
        r#"SELECT id,input_key,technique FROM coverage_denominator_items
            WHERE denominator_id=$1 AND expected_capability=$2 ORDER BY ordinal FOR SHARE"#,
    )
    .bind(receipt.0)
    .bind(&receipt.2)
    .fetch_all(&mut *tx)
    .await?;
    if items.is_empty() {
        return Err(fail(MANIFEST_DRIFT));
    }
    let observation_map = observations
        .into_iter()
        .map(|observation| {
            (
                (observation.input_key, observation.technique),
                observation.observation_state,
            )
        })
        .collect::<std::collections::BTreeMap<_, _>>();
    if observation_map.keys().any(|key| {
        !items
            .iter()
            .any(|(_, input_key, technique)| key == &(input_key.clone(), technique.clone()))
    }) {
        return Err(fail(MANIFEST_DRIFT));
    }
    let complete = !superseded
        && fence_current
        && command.parser_complete
        && command.failure_reason_code.is_none()
        && !command.raw_witness.truncated
        && !command.network_hops.is_empty()
        && items.iter().all(|(_, input_key, technique)| {
            matches!(
                observation_map
                    .get(&(input_key.clone(), technique.clone()))
                    .map(String::as_str),
                Some("found" | "no_match")
            )
        });

    for (ordinal, (item_id, input_key, technique)) in items.iter().enumerate() {
        if let Some(parser_member_id) = parser_member_id {
            sqlx::query(
                r#"INSERT INTO capability_typed_landing_source_members(
                       id,receipt_id,execution_authority_id,ordinal,input_key,source_kind,
                       raw_start,raw_end,parser_census_member_id,normalized_observation_hash
                   ) VALUES($1,$2,$3,$4,$5,'raw_range',0,$6,$7,$8)"#,
            )
            .bind(Uuid::new_v5(
                &command.receipt_id,
                format!("typed-source:{ordinal}").as_bytes(),
            ))
            .bind(command.receipt_id)
            .bind(receipt.1)
            .bind(i32::try_from(ordinal).map_err(|_| fail(CONTRACT_INVALID))?)
            .bind(input_key)
            .bind(command.raw_witness.stored_byte_count)
            .bind(parser_member_id)
            .bind(sha256_json(&serde_json::json!({
                "input_key": input_key,
                "technique": technique,
                "observation": observation_map.get(&(input_key.clone(), technique.clone())),
            }))?)
            .execute(&mut *tx)
            .await?;
        }
        let observation = observation_map
            .get(&(input_key.clone(), technique.clone()))
            .map(String::as_str)
            .unwrap_or("indeterminate");
        let terminal = command.parser_complete
            && command.failure_reason_code.is_none()
            && !command.raw_witness.truncated
            && matches!(observation, "found" | "no_match");
        let input_id = Uuid::new_v5(&command.receipt_id, input_key.as_bytes());
        sqlx::query(
            r#"INSERT INTO capability_execution_receipt_inputs(
                   id,receipt_id,denominator_id,denominator_item_id,execution_authority_id,
                   input_key,attempt_state,landing_state,observation_state,coverage_extent,
                   coverage_gap_reason
               ) VALUES($1,$2,$3,$4,$5,$6,$7,$8,$9,$10,$11)"#,
        )
        .bind(input_id)
        .bind(command.receipt_id)
        .bind(receipt.0)
        .bind(*item_id)
        .bind(receipt.1)
        .bind(input_key)
        .bind(if terminal {
            "succeeded"
        } else {
            "outcome_unknown"
        })
        .bind(if terminal { "committed" } else { "partial" })
        .bind(if terminal {
            observation
        } else {
            "indeterminate"
        })
        .bind(if terminal { "complete" } else { "partial" })
        .bind(if terminal {
            "none"
        } else {
            "source_unavailable"
        })
        .execute(&mut *tx)
        .await?;
        let lineage_hash = sha256_json(&serde_json::json!({
            "input_key": input_key,
            "technique": technique,
            "evidence_authority_id": evidence_authority_id,
            "observation": observation,
        }))?;
        sqlx::query(
            r#"INSERT INTO capability_execution_input_evidence_members(
                   id,input_id,receipt_id,denominator_item_id,execution_authority_id,
                   evidence_authority_id,ordinal,member_hash
               ) VALUES($1,$2,$3,$4,$5,$6,0,$7)"#,
        )
        .bind(Uuid::new_v5(&input_id, b"evidence-authority:v1"))
        .bind(input_id)
        .bind(command.receipt_id)
        .bind(*item_id)
        .bind(receipt.1)
        .bind(evidence_authority_id)
        .bind(lineage_hash)
        .execute(&mut *tx)
        .await?;
        sqlx::query(
            "UPDATE capability_execution_receipt_inputs SET sealed_at=statement_timestamp() WHERE id=$1",
        )
        .bind(input_id)
        .execute(&mut *tx)
        .await?;
    }

    for (axis, value, source) in [
        ("requests", command.request_count, "host_governor"),
        (
            "response_bytes",
            command.response_byte_count,
            "adapter_instrumentation",
        ),
        ("wall_clock_ms", command.wall_clock_ms, "host_governor"),
        ("retries", command.retry_count, "host_governor"),
    ] {
        sqlx::query(
            r#"INSERT INTO capability_execution_budget_contract_axes(
                   receipt_id,execution_authority_id,axis,required_for_complete,
                   planned_limit,required_observation_source
               ) VALUES($1,$2,$3,TRUE,NULL,$4)"#,
        )
        .bind(command.receipt_id)
        .bind(receipt.1)
        .bind(axis)
        .bind(source)
        .execute(&mut *tx)
        .await?;
        sqlx::query(
            r#"INSERT INTO capability_execution_budget_observations(
                   receipt_id,execution_authority_id,axis,actual_value,observed,observation_source
               ) VALUES($1,$2,$3,$4,TRUE,$5)"#,
        )
        .bind(command.receipt_id)
        .bind(receipt.1)
        .bind(axis)
        .bind(value)
        .bind(source)
        .execute(&mut *tx)
        .await?;
    }

    let temporal_census_id = Uuid::new_v5(&command.receipt_id, b"temporal-census:v1");
    sqlx::query(
        r#"INSERT INTO capability_execution_temporal_censuses(
               id,receipt_id,execution_authority_id,receipt_authority_hash,
               temporal_validity_policy_id,temporal_validity_policy_hash,
               observation_window_started_at,observation_window_completed_at,effective_valid_until
           ) SELECT $1,r.id,r.execution_authority_id,r.receipt_authority_hash,
                    r.temporal_validity_policy_id,r.temporal_validity_policy_hash,
                    r.observation_started_at,statement_timestamp(),
                    statement_timestamp()+INTERVAL '60 seconds'
               FROM capability_execution_receipts r WHERE r.id=$2"#,
    )
    .bind(temporal_census_id)
    .bind(command.receipt_id)
    .execute(&mut *tx)
    .await?;
    for (ordinal, (_, input_key, technique)) in items.iter().enumerate() {
        let observation = observation_map
            .get(&(input_key.clone(), technique.clone()))
            .map(String::as_str)
            .unwrap_or("indeterminate");
        let polarity = match observation {
            "found" => "positive",
            "no_match" => "negative",
            _ => "inconclusive",
        };
        let member_hash = sha256_json(&serde_json::json!({
            "input_key": input_key,
            "technique": technique,
            "polarity": polarity,
            "ttl_ms": 60000,
        }))?;
        sqlx::query(
            r#"INSERT INTO capability_execution_temporal_census_members(
                   id,census_id,receipt_id,execution_authority_id,ordinal,input_key,
                   observation_identity_hash,temporal_fact_class,observation_polarity,
                   mapping_rule_id,mapping_rule_version,mapping_rule_digest,selected_ttl_ms,
                   observed_at,effective_valid_until,member_hash
               ) VALUES($1,$2,$3,$4,$5,$6,$7,'target_intel',$8,
                        'target_intel.default_ttl','1',$9,60000,statement_timestamp(),
                        statement_timestamp()+INTERVAL '60 seconds',$10)"#,
        )
        .bind(Uuid::new_v5(&temporal_census_id, input_key.as_bytes()))
        .bind(temporal_census_id)
        .bind(command.receipt_id)
        .bind(receipt.1)
        .bind(i32::try_from(ordinal).map_err(|_| fail(CONTRACT_INVALID))?)
        .bind(input_key)
        .bind(sha256_json(&serde_json::json!({
            "receipt_id": command.receipt_id,
            "input_key": input_key,
            "observation": observation,
        }))?)
        .bind(polarity)
        .bind(sha256_json(&serde_json::json!({
            "rule": "target_intel.default_ttl",
            "version": 1,
        }))?)
        .bind(member_hash)
        .execute(&mut *tx)
        .await?;
    }
    sqlx::query(
        "UPDATE capability_execution_temporal_censuses SET sealed_at=statement_timestamp() WHERE id=$1",
    )
    .bind(temporal_census_id)
    .execute(&mut *tx)
    .await?;

    let reconciliation_id = Uuid::new_v5(
        &command.receipt_id,
        format!("semantic-reconciliation:{}", receipt.9 + 1).as_bytes(),
    );
    sqlx::query(
        r#"INSERT INTO capability_execution_reconciliations(
               id,receipt_id,execution_authority_id,semantic_authority_version,
               predecessor_reconciliation_id,reconciliation_state
           ) VALUES($1,$2,$3,$4,$5,'pending')"#,
    )
    .bind(reconciliation_id)
    .bind(command.receipt_id)
    .bind(receipt.1)
    .bind(receipt.9 + 1)
    .bind(receipt.10)
    .execute(&mut *tx)
    .await?;
    let reconciliation_member_hash = sha256_json(&serde_json::json!({
        "source_kind": "evidence",
        "evidence_authority_id": evidence_authority_id,
        "receipt_id": command.receipt_id,
    }))?;
    sqlx::query(
        r#"INSERT INTO capability_execution_reconciliation_members(
               id,reconciliation_id,receipt_id,execution_authority_id,ordinal,
               source_kind,evidence_authority_id,member_hash
           ) VALUES($1,$2,$3,$4,0,'evidence',$5,$6)"#,
    )
    .bind(Uuid::new_v5(&reconciliation_id, b"evidence-authority:v1"))
    .bind(reconciliation_id)
    .bind(command.receipt_id)
    .bind(receipt.1)
    .bind(evidence_authority_id)
    .bind(reconciliation_member_hash)
    .execute(&mut *tx)
    .await?;
    let reconciliation_state = if superseded {
        "superseded"
    } else if complete {
        "consistent"
    } else {
        "orphaned"
    };
    let reason_code = if complete {
        None
    } else if superseded {
        Some("late_superseded_attempt_closeout".to_string())
    } else if !fence_current {
        Some("worker_fence_stale".to_string())
    } else {
        Some(
            command
                .failure_reason_code
                .clone()
                .unwrap_or_else(|| "target_intel_incomplete".to_string()),
        )
    };
    let semantic_hash: String = sqlx::query_scalar(
        r#"UPDATE capability_execution_reconciliations
              SET reconciliation_state=$2,reason_code=$3,
                  observed_artifact_sha256=$4,observed_artifact_byte_count=$5,
                  sealed_at=statement_timestamp()
            WHERE id=$1 RETURNING semantic_reconciliation_hash"#,
    )
    .bind(reconciliation_id)
    .bind(reconciliation_state)
    .bind(reason_code)
    .bind(&command.raw_witness.sha256)
    .bind(command.raw_witness.stored_byte_count)
    .fetch_one(&mut *tx)
    .await?;
    let receipt_observation = if complete
        && items.iter().any(|(_, input_key, technique)| {
            observation_map
                .get(&(input_key.clone(), technique.clone()))
                .map(String::as_str)
                == Some("found")
        }) {
        "found"
    } else if complete {
        "no_match"
    } else {
        "indeterminate"
    };
    let row = sqlx::query_as::<_, CapabilityExecutionReceiptRow>(&format!(
        r#"UPDATE capability_execution_receipts
              SET attempt_state=$3,landing_state=$4,observation_state=$5,
                  coverage_extent=$6,coverage_gap_reason=$7,reconciliation_state=$8,
                  security_interpretation=$9,typed_landing=$10,
                  residual=$11,raw_witness_artifact_id=$12,parser_census_id=$13,
                  temporal_census_id=$14,current_semantic_authority_version=$15,
                  current_semantic_reconciliation_id=$16,
                  current_semantic_reconciliation_hash=$17,
                  finalization_request_hash=$18,row_version=row_version+1,
                  observation_completed_at=statement_timestamp(),
                  valid_until=statement_timestamp()+INTERVAL '60 seconds',
                  finalized_at=COALESCE(finalized_at,statement_timestamp())
            WHERE id=$1 AND row_version=$2
            RETURNING {RECEIPT_COLUMNS}"#
    ))
    .bind(command.receipt_id)
    .bind(if superseded {
        receipt.8
    } else {
        command.expected_row_version
    })
    .bind(if superseded {
        "superseded"
    } else if complete {
        "succeeded"
    } else {
        "outcome_unknown"
    })
    .bind(if complete { "committed" } else { "partial" })
    .bind(receipt_observation)
    .bind(if complete { "complete" } else { "partial" })
    .bind(if complete {
        "none"
    } else {
        "source_unavailable"
    })
    .bind(reconciliation_state)
    .bind(if receipt_observation == "found" {
        "signal"
    } else if complete {
        "not_assessed"
    } else {
        "inconclusive"
    })
    .bind(&command.typed_landing)
    .bind((!complete).then(|| {
        serde_json::json!({
            "code": "TOOL_TRUTH_TARGET_INTEL_INCOMPLETE",
            "missing_inputs": items
                .iter()
                .filter(|(_, input_key, technique)| {
                    !matches!(observation_map
                        .get(&(input_key.clone(), technique.clone()))
                        .map(String::as_str), Some("found" | "no_match"))
                })
                .map(|(_, input_key, technique)| serde_json::json!({
                    "input_key": input_key,
                    "technique": technique,
                }))
                .collect::<Vec<_>>(),
        })
    }))
    .bind(command.raw_witness.artifact_id)
    .bind(parser_census_id)
    .bind(temporal_census_id)
    .bind(receipt.9 + 1)
    .bind(reconciliation_id)
    .bind(semantic_hash)
    .bind(finalization_request_hash)
    .fetch_optional(&mut *tx)
    .await?
    .ok_or_else(|| fail(RECEIPT_STALE))?;
    tx.commit().await?;
    Ok(row)
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
