//! Repository for `operation_state` cursor table (Doc 1 §3.4).
//!
//! 注意: 这不是 operations 表; 没有 valid_until / authz_level / scope (那些走
//! targets / organizations). 这是用户 2026-05-17 删 engagements 后唯一可接受的新表形状.
//!
//! 与现有 `repo/audit.rs` 同步: 自由函数 + `&PgPool`, 无 trait 抽象.

use crate::Result;
use chrono::{DateTime, Utc};
use golish_core::{
    ApplicationModelContract, AttackExecutionContract, InvestigationContractVersion,
    InvestigationRolloutMode, StageTopologyContract,
};
use golish_pentest_domain::tool_truth::ToolTruthContract;
use serde::{Deserialize, Serialize};
use sqlx::{Executor, PgPool, Postgres};
use uuid::Uuid;

use super::{attack_execution_rollout, operation_rollout};

#[derive(Debug, thiserror::Error, PartialEq, Eq)]
pub enum OperationContractValidationError {
    #[error("unknown runtime-memory contract: {0}")]
    UnknownRuntimeMemoryContract(String),
    #[error("attack execution v2 requires runtime_memory_contract=v2_only, got {0}")]
    RuntimeMemoryV2Required(String),
    #[error("attack execution dual-write requires a runtime-memory V2 writer, got {0}")]
    RuntimeMemoryV2WriterRequired(String),
}

impl OperationContractValidationError {
    pub const fn code(&self) -> &'static str {
        match self {
            Self::UnknownRuntimeMemoryContract(_) => "ATTACK_RUNTIME_MEMORY_CONTRACT_UNKNOWN",
            Self::RuntimeMemoryV2Required(_) => "ATTACK_RUNTIME_MEMORY_V2_REQUIRED",
            Self::RuntimeMemoryV2WriterRequired(_) => "ATTACK_RUNTIME_MEMORY_V2_WRITER_REQUIRED",
        }
    }
}

/// Validate the two operation-frozen rollout contracts without reading or
/// mutating SQL state. DB constraints and operation-creation transaction wiring
/// land in the later schema/repository tasks.
pub fn validate_operation_contracts(
    runtime_memory_contract: &str,
    attack_execution_contract: AttackExecutionContract,
) -> std::result::Result<(), OperationContractValidationError> {
    if !matches!(
        runtime_memory_contract,
        "legacy_v1" | "dual_write_legacy_read" | "dual_write_v2_preferred" | "v2_only"
    ) {
        return Err(
            OperationContractValidationError::UnknownRuntimeMemoryContract(
                runtime_memory_contract.to_string(),
            ),
        );
    }
    if attack_execution_contract.executes_v2_verifier() && runtime_memory_contract != "v2_only" {
        return Err(OperationContractValidationError::RuntimeMemoryV2Required(
            runtime_memory_contract.to_string(),
        ));
    }
    if attack_execution_contract.writes_v2() && runtime_memory_contract == "legacy_v1" {
        return Err(
            OperationContractValidationError::RuntimeMemoryV2WriterRequired(
                runtime_memory_contract.to_string(),
            ),
        );
    }
    Ok(())
}

fn parse_attack_execution_contract(value: &str) -> Result<AttackExecutionContract> {
    match value {
        "legacy" => Ok(AttackExecutionContract::Legacy),
        "dual_write_read_legacy" => Ok(AttackExecutionContract::DualWriteReadLegacy),
        "dual_write_read_v2_fallback" => Ok(AttackExecutionContract::DualWriteReadV2Fallback),
        "v2_only" => Ok(AttackExecutionContract::V2Only),
        other => Err(crate::DbError::Other(anyhow::anyhow!(
            "unknown attack-execution contract: {other}"
        ))),
    }
}

fn parse_application_model_contract(value: &str) -> Result<ApplicationModelContract> {
    ApplicationModelContract::try_from(value)
        .map_err(|error| crate::DbError::Other(anyhow::Error::new(error)))
}

fn parse_tool_truth_contract(value: &str) -> Result<ToolTruthContract> {
    ToolTruthContract::try_from(value)
        .map_err(|error| crate::DbError::Other(anyhow::Error::new(error)))
}

fn validate_frozen_operation_contracts(
    runtime_memory_contract: &str,
    attack_execution_contract: AttackExecutionContract,
) -> Result<()> {
    validate_operation_contracts(runtime_memory_contract, attack_execution_contract)
        .map_err(|error| crate::DbError::Other(anyhow::Error::new(error)))
}

fn build_clear_engagement_org_for_subtree_sql() -> String {
    "WITH RECURSIVE subtree AS ( \
         SELECT id FROM organizations WHERE id = $1 \
         UNION ALL \
         SELECT o.id FROM organizations o JOIN subtree s ON o.parent_id = s.id \
       ) \
       UPDATE operation_state \
       SET engagement_org_id = NULL \
       WHERE engagement_org_id IN (SELECT id FROM subtree)"
        .to_string()
}

const GET_OPERATION_EPOCH_SQL: &str = r#"SELECT operation_id, current_stage, stage_started_at,
              superseded_by, engagement_org_id
       FROM operation_state
       WHERE operation_id = $1"#;

pub const EAS_WEB_TRANSPORT_FAILURES_NAMESPACE: &str = "eas_web_transport_failures";
const MAX_EAS_WEB_TRANSPORT_FAILURE_SLOTS: i64 = 512;
const MAX_EAS_WEB_TRANSPORT_FAILURE_BYTES: i32 = 262_144;

const INCREMENT_EAS_WEB_TRANSPORT_FAILURE_SQL: &str = r#"UPDATE operation_state
SET state_blob = jsonb_set(
    jsonb_set(
        CASE WHEN jsonb_typeof(state_blob) = 'object' THEN state_blob ELSE '{}'::jsonb END,
        ARRAY[$3],
        CASE
            WHEN jsonb_typeof(state_blob -> $3) = 'object' THEN state_blob -> $3
            ELSE '{}'::jsonb
        END,
        true
    ),
    ARRAY[$3, $4],
    jsonb_build_object(
        'epoch_started_at', to_jsonb($2::timestamptz),
        'organization_id', $5::text,
        'target_id', $6::text,
        'origin', $7::text,
        'technique', $8::text,
        'failure_class', $9::text,
        'attempts', CASE
            WHEN state_blob #> ARRAY[$3, $4, 'epoch_started_at'] = to_jsonb($2::timestamptz)
             AND state_blob #>> ARRAY[$3, $4, 'organization_id'] = $5::text
             AND state_blob #>> ARRAY[$3, $4, 'target_id'] = $6::text
             AND state_blob #>> ARRAY[$3, $4, 'origin'] = $7::text
             AND state_blob #>> ARRAY[$3, $4, 'technique'] = $8::text
             AND state_blob #>> ARRAY[$3, $4, 'failure_class'] = $9::text
            THEN COALESCE((state_blob #>> ARRAY[$3, $4, 'attempts'])::int, 0) + 1
            ELSE 1
        END,
        'independently_confirmed', false,
        'updated_at', to_jsonb(NOW())
    ),
    true
)
WHERE operation_id = $1
  AND current_stage = 'external_attack_surface'
  AND stage_started_at = $2
  AND superseded_by IS NULL
  AND (
      COALESCE(state_blob -> $3, '{}'::jsonb) ? $4
      OR (
          (SELECT COUNT(*) FROM jsonb_object_keys(
              CASE WHEN jsonb_typeof(state_blob -> $3) = 'object'
                   THEN state_blob -> $3 ELSE '{}'::jsonb END
          )) < $10
          AND pg_column_size(COALESCE(state_blob -> $3, '{}'::jsonb)) < $11
      )
  )
RETURNING (state_blob #>> ARRAY[$3, $4, 'attempts'])::int"#;

const CLEAR_EAS_WEB_TRANSPORT_FAILURES_SQL: &str = r#"UPDATE operation_state
SET state_blob = jsonb_set(
    CASE WHEN jsonb_typeof(state_blob) = 'object' THEN state_blob ELSE '{}'::jsonb END,
    ARRAY[$3],
    CASE WHEN jsonb_typeof(state_blob -> $3) = 'object'
         THEN (state_blob -> $3) - $4::text[] ELSE '{}'::jsonb END,
    true
)
WHERE operation_id = $1
  AND current_stage = 'external_attack_surface'
  AND stage_started_at = $2
  AND superseded_by IS NULL"#;

const MARK_EAS_WEB_FINGERPRINT_PRODUCER_BLOCKED_SQL: &str = r#"UPDATE operation_state
SET state_blob = jsonb_set(
    CASE WHEN jsonb_typeof(state_blob) = 'object' THEN state_blob ELSE '{}'::jsonb END,
    ARRAY[$3, $4],
    (state_blob #> ARRAY[$3, $4]) || jsonb_build_object(
        'producer_blocked', true,
        'producer_evidence_id', $10::bigint,
        'producer_run_id', $11::text,
        'producer_blocked_at', to_jsonb(NOW())
    ) || CASE WHEN $12::bigint IS NOT NULL THEN jsonb_build_object(
        'independently_confirmed', true,
        'evidence_id', $12::bigint,
        'producer', $13::text,
        'kind', $14::text,
        'confirmed_at', to_jsonb(NOW())
    ) ELSE '{}'::jsonb END,
    true
)
WHERE operation_id = $1
  AND current_stage = 'external_attack_surface'
  AND stage_started_at = $2
  AND superseded_by IS NULL
  AND state_blob #> ARRAY[$3, $4, 'epoch_started_at'] = to_jsonb($2::timestamptz)
  AND state_blob #>> ARRAY[$3, $4, 'organization_id'] = $5::text
  AND state_blob #>> ARRAY[$3, $4, 'target_id'] = $6::text
  AND state_blob #>> ARRAY[$3, $4, 'origin'] = $7::text
  AND state_blob #>> ARRAY[$3, $4, 'technique'] = $8::text
  AND state_blob #>> ARRAY[$3, $4, 'failure_class'] = $9::text
  AND COALESCE((state_blob #>> ARRAY[$3, $4, 'attempts'])::int, 0) >= 3
RETURNING 1"#;

const LIST_EAS_WEB_FINGERPRINT_PRODUCER_BLOCKED_SQL: &str = r#"SELECT entry.value->>'target_id' AS target_id,
       entry.value->>'origin' AS origin,
       entry.value->>'producer_evidence_id' AS producer_evidence_id
FROM operation_state os
CROSS JOIN LATERAL jsonb_each(
    CASE WHEN jsonb_typeof(os.state_blob -> $4) = 'object'
         THEN os.state_blob -> $4 ELSE '{}'::jsonb END
) AS entry(key, value)
JOIN audit_log al
  ON al.id = CASE
      WHEN entry.value->>'producer_evidence_id' ~ '^[1-9][0-9]*$'
      THEN (entry.value->>'producer_evidence_id')::bigint
      ELSE NULL
  END
 AND al.audit_role = 'evidence'
 AND al.run_id = os.operation_id
 AND al.target_id::text = entry.value->>'target_id'
 AND al.tool_name = 'whatweb'
 AND al.detail->>'kind' = 'eas.fingerprint_web_stack'
 AND al.detail->>'organization_id' = entry.value->>'organization_id'
 AND al.evidence_asset = entry.value->>'origin'
 AND al.evidence_technique = entry.value->>'technique'
 AND al.evidence_outcome = 'blocked'
 AND al.created_at >= $2
JOIN targets t
  ON t.id::text = entry.value->>'target_id'
 AND t.scope::text = 'in'
 AND t.organization_id = $3
 AND t.project_path IS NOT DISTINCT FROM al.project_path
JOIN technique_outcomes outcome
  ON outcome.organization_id = $3
 AND outcome.run_id = entry.value->>'producer_run_id'
 AND outcome.asset = entry.value->>'origin'
 AND outcome.technique = entry.value->>'technique'
 AND outcome.outcome = 'blocked'
 AND outcome.source = 'eas_fingerprint_web_stack'
 AND al.id = ANY(outcome.evidence_ids)
 AND outcome.collected_at >= $2
WHERE os.operation_id = $1
  AND os.current_stage = 'external_attack_surface'
  AND os.stage_started_at = $2
  AND os.superseded_by IS NULL
  AND entry.value->>'organization_id' = $3::text
  AND entry.value #> ARRAY['epoch_started_at'] = to_jsonb($2::timestamptz)
  AND entry.value->>'producer_blocked' = 'true'
  AND COALESCE((entry.value->>'attempts')::int, 0) >= 3"#;

const LIST_EAS_WEB_TRANSPORT_BLOCKED_SQL: &str = r#"SELECT entry.value->>'target_id' AS target_id,
       entry.value->>'origin' AS origin
FROM operation_state os
CROSS JOIN LATERAL jsonb_each(
    CASE WHEN jsonb_typeof(os.state_blob -> $3) = 'object'
         THEN os.state_blob -> $3 ELSE '{}'::jsonb END
) AS entry(key, value)
JOIN audit_log al
  ON al.id = CASE
      WHEN entry.value->>'evidence_id' ~ '^[1-9][0-9]*$'
      THEN (entry.value->>'evidence_id')::bigint
      ELSE NULL
  END
 AND al.id > 0
 AND al.audit_role = 'evidence'
 AND al.run_id = os.operation_id
 AND al.target_id::text = entry.value->>'target_id'
 AND al.tool_name = entry.value->>'producer'
 AND al.detail->>'kind' = entry.value->>'kind'
 AND al.detail->>'organization_id' = entry.value->>'organization_id'
 AND al.evidence_asset = entry.value->>'origin'
 AND al.evidence_technique = entry.value->>'technique'
 AND al.evidence_outcome = 'blocked'
JOIN targets t
  ON t.id::text = entry.value->>'target_id'
 AND t.scope::text = 'in'
 AND t.organization_id = $2
 AND t.project_path IS NOT DISTINCT FROM al.project_path
JOIN audit_log producer_al
  ON producer_al.id = CASE
      WHEN entry.value->>'producer_evidence_id' ~ '^[1-9][0-9]*$'
      THEN (entry.value->>'producer_evidence_id')::bigint
      ELSE NULL
  END
 AND producer_al.audit_role = 'evidence'
 AND producer_al.run_id = os.operation_id
 AND producer_al.target_id = t.id
 AND producer_al.tool_name = 'whatweb'
 AND producer_al.detail->>'kind' = 'eas.fingerprint_web_stack'
 AND producer_al.evidence_asset = entry.value->>'origin'
 AND producer_al.evidence_technique = entry.value->>'technique'
 AND producer_al.evidence_outcome = 'blocked'
 AND producer_al.project_path IS NOT DISTINCT FROM t.project_path
JOIN technique_outcomes producer_outcome
  ON producer_outcome.organization_id = $2
 AND producer_outcome.run_id = entry.value->>'producer_run_id'
 AND producer_outcome.asset = entry.value->>'origin'
 AND producer_outcome.technique = entry.value->>'technique'
 AND producer_outcome.outcome = 'blocked'
 AND producer_outcome.source = 'eas_fingerprint_web_stack'
 AND producer_al.id = ANY(producer_outcome.evidence_ids)
WHERE os.operation_id = $1
  AND os.current_stage = 'enumeration'
  AND os.superseded_by IS NULL
  AND entry.value->>'organization_id' = $2::text
  AND entry.value->>'independently_confirmed' = 'true'
  AND entry.value->>'producer_blocked' = 'true'
  AND COALESCE((entry.value->>'attempts')::int, 0) >= 3
  AND entry.value->>'producer' = 'eas_transport_preflight'
  AND entry.value->>'kind' = 'eas_transport_preflight_blocked'
  AND entry.value->>'technique' = 'GOLISH-EAS-WEB-FINGERPRINT'"#;

const WRITE_STATE_BLOB_SQL: &str = r#"UPDATE operation_state
SET state_blob = jsonb_set(
    CASE WHEN jsonb_typeof($2::jsonb) = 'object' THEN $2::jsonb ELSE '{}'::jsonb END,
    ARRAY[$3],
    CASE WHEN jsonb_typeof(state_blob -> $3) = 'object'
         THEN state_blob -> $3 ELSE '{}'::jsonb END,
    true
)
WHERE operation_id = $1
  AND runtime_memory_contract <> 'v2_only'"#;

const ADVANCE_STAGE_SQL: &str = r#"UPDATE operation_state
SET current_stage = $2,
    stage_started_at = NOW(),
    state_blob = CASE WHEN $2 = 'external_attack_surface'
        THEN jsonb_set(
            CASE WHEN jsonb_typeof(state_blob) = 'object' THEN state_blob ELSE '{}'::jsonb END,
            ARRAY[$3],
            '{}'::jsonb,
            true
        )
        ELSE state_blob
    END
WHERE operation_id = $1"#;

#[derive(Debug, Clone)]
pub struct EasWebTransportFailureInput {
    pub operation_id: Uuid,
    pub stage_started_at: DateTime<Utc>,
    pub slot_key: String,
    pub organization_id: Uuid,
    pub target_id: Uuid,
    pub origin: String,
    pub technique: String,
    pub failure_class: String,
}

/// `operation_state` 行映射 (`sqlx::FromRow`).
#[derive(Debug, Clone, sqlx::FromRow, Serialize, Deserialize)]
pub struct OperationStateRow {
    pub operation_id: Uuid,
    pub profile: String,
    pub current_stage: String,
    pub runtime_memory_contract: String,
    /// Immutable Tool Truth execution/evidence contract selected at operation
    /// creation. Existing rows and the Plan A deployment default are legacy_v1.
    pub tool_truth_contract: String,
    /// Immutable Application Understanding/Candidate topology. Historical
    /// operations remain `legacy_no_model`.
    pub application_model_contract: String,
    /// Immutable Candidate/Hypothesis Registry schema contract selected in the
    /// same transaction as the Tool Truth contract.
    pub investigation_contract_version: String,
    /// Immutable five-state rollout mode paired with the schema contract.
    pub investigation_rollout_mode: String,
    /// Immutable operation graph selected from the server-owned Investigation
    /// rollout pair. Existing rows are frozen to the historical graph.
    pub stage_topology_contract: String,
    /// Exact canonical material and domain-separated hash used by resume,
    /// reporting, fork, and projection consumers.
    pub stage_topology_canonical_json: String,
    pub stage_topology_sha256: String,
    pub stage_topology_freeze_source: String,
    /// Stable workspace identity for runtime-memory V2. Legacy rows remain
    /// nullable; every newly created runtime operation supplies this value.
    pub project_scope_id: Option<Uuid>,
    pub stage_started_at: DateTime<Utc>,
    pub last_evidence_audit_id: Option<i64>,
    pub last_classification_id: Option<i64>,
    pub last_scope_version: Option<i64>,
    pub state_blob: serde_json::Value,
    pub superseded_by: Option<Uuid>,
    /// Engagement-org isolation (设计 2026-06-15-engagement-org-isolation): the
    /// scoping-confirmed root organization id this operation is bound to. Fan-out
    /// / in-scope reads confine to its subtree (root + subsidiaries). `None` = not
    /// yet bound (legacy whole-DB axis).
    pub engagement_org_id: Option<Uuid>,
}

/// Lightweight operation epoch used by hot-path validity checks.
///
/// This intentionally excludes `state_blob` and cursor payload columns so a
/// stage guard does not deserialize the potentially large resume document.
#[derive(Debug, Clone, sqlx::FromRow, Serialize, Deserialize)]
pub struct OperationEpochRow {
    pub operation_id: Uuid,
    pub current_stage: String,
    pub stage_started_at: DateTime<Utc>,
    pub superseded_by: Option<Uuid>,
    pub engagement_org_id: Option<Uuid>,
}

/// 创建一个新 operation_state 行 (新 operation 入口).
const INSERT_OPERATION_SQL: &str = r#"INSERT INTO operation_state
        (operation_id, profile, current_stage, runtime_memory_contract,
         attack_execution_contract, application_model_contract, tool_truth_contract,
         investigation_contract_version, investigation_rollout_mode)
    VALUES ($1, $2, $3, $4, $5, $6, $7, $8, $9)"#;

#[cfg(test)]
const OPERATION_STATE_ROW_COLUMNS: &str = r#"operation_id, profile, current_stage,
    runtime_memory_contract, tool_truth_contract, application_model_contract,
    investigation_contract_version,
    investigation_rollout_mode, stage_topology_contract,
    stage_topology_canonical_json, stage_topology_sha256,
    stage_topology_freeze_source, project_scope_id, stage_started_at,
    last_evidence_audit_id, last_classification_id, last_scope_version,
    state_blob, superseded_by, engagement_org_id"#;

const INSERT_OPERATION_WITH_EXECUTOR_SQL: &str = r#"INSERT INTO operation_state
        (operation_id, profile, current_stage, runtime_memory_contract, project_scope_id,
         attack_execution_contract, application_model_contract, tool_truth_contract,
         investigation_contract_version, investigation_rollout_mode)
    VALUES ($1, $2, $3, $4, $5, $6, $7, $8, $9, $10)
    RETURNING operation_id, profile, current_stage, runtime_memory_contract,
              tool_truth_contract, application_model_contract, investigation_contract_version,
              investigation_rollout_mode, stage_topology_contract,
              stage_topology_canonical_json, stage_topology_sha256,
              stage_topology_freeze_source, project_scope_id, stage_started_at, last_evidence_audit_id,
              last_classification_id, last_scope_version, state_blob,
              superseded_by, engagement_org_id"#;

pub async fn insert(
    pool: &PgPool,
    operation_id: Uuid,
    profile: &str,
    current_stage: &str,
    runtime_memory_contract: &str,
    application_model_contract: ApplicationModelContract,
) -> Result<()> {
    let mut tx = pool.begin().await?;
    let attack_rollout = attack_execution_rollout::get_for_share(&mut tx).await?;
    let joint_contract = operation_rollout::deployment_pair_for_share(&mut tx)
        .await
        .map_err(operation_rollout::map_to_db_error)?;
    let attack_contract = parse_attack_execution_contract(&attack_rollout.contract)?;
    validate_frozen_operation_contracts(runtime_memory_contract, attack_contract)?;
    sqlx::query(INSERT_OPERATION_SQL)
        .bind(operation_id)
        .bind(profile)
        .bind(current_stage)
        .bind(runtime_memory_contract)
        .bind(attack_contract.as_str())
        .bind(application_model_contract.as_str())
        .bind(joint_contract.tool_truth_contract().as_str())
        .bind(joint_contract.investigation_contract_version().as_str())
        .bind(joint_contract.investigation_rollout_mode().as_str())
        .execute(&mut *tx)
        .await?;
    tx.commit().await?;
    Ok(())
}

/// Insert a fully bound runtime operation through the caller's executor and
/// return the frozen row. The mandatory `project_scope_id` is enforced by the
/// signature even though the migration keeps the column nullable for legacy
/// operations.
pub async fn insert_with_executor<'e, E>(
    executor: E,
    operation_id: Uuid,
    profile: &str,
    current_stage: &str,
    runtime_memory_contract: &str,
    project_scope_id: Uuid,
    attack_execution_contract: AttackExecutionContract,
    application_model_contract: ApplicationModelContract,
    tool_truth_contract: ToolTruthContract,
    investigation_contract_version: InvestigationContractVersion,
    investigation_rollout_mode: InvestigationRolloutMode,
) -> Result<OperationStateRow>
where
    E: Executor<'e, Database = Postgres>,
{
    validate_frozen_operation_contracts(runtime_memory_contract, attack_execution_contract)?;
    operation_rollout::validate_joint_pair(
        tool_truth_contract,
        investigation_contract_version,
        investigation_rollout_mode,
    )
    .map_err(operation_rollout::map_to_db_error)?;
    let row = sqlx::query_as::<_, OperationStateRow>(INSERT_OPERATION_WITH_EXECUTOR_SQL)
        .bind(operation_id)
        .bind(profile)
        .bind(current_stage)
        .bind(runtime_memory_contract)
        .bind(project_scope_id)
        .bind(attack_execution_contract.as_str())
        .bind(application_model_contract.as_str())
        .bind(tool_truth_contract.as_str())
        .bind(investigation_contract_version.as_str())
        .bind(investigation_rollout_mode.as_str())
        .fetch_one(executor)
        .await?;
    Ok(row)
}

/// 读 operation_state · 主 lookup.
pub async fn get(pool: &PgPool, operation_id: Uuid) -> Result<Option<OperationStateRow>> {
    let row = sqlx::query_as::<_, OperationStateRow>(
        r#"SELECT operation_id, profile, current_stage, runtime_memory_contract,
                  tool_truth_contract, application_model_contract, investigation_contract_version,
                  investigation_rollout_mode, stage_topology_contract,
                  stage_topology_canonical_json, stage_topology_sha256,
                  stage_topology_freeze_source, project_scope_id, stage_started_at,
                  last_evidence_audit_id, last_classification_id,
                  last_scope_version, state_blob, superseded_by, engagement_org_id
           FROM operation_state
           WHERE operation_id = $1"#,
    )
    .bind(operation_id)
    .fetch_optional(pool)
    .await?;
    Ok(row)
}

/// Read and strictly decode the immutable Application Model contract. Unknown
/// persisted values fail closed rather than selecting a fallback topology.
pub async fn get_application_model_contract(
    pool: &PgPool,
    operation_id: Uuid,
) -> Result<Option<ApplicationModelContract>> {
    let value: Option<String> = sqlx::query_scalar(
        "SELECT application_model_contract FROM operation_state WHERE operation_id=$1",
    )
    .bind(operation_id)
    .fetch_optional(pool)
    .await?;
    value
        .map(|value| parse_application_model_contract(&value))
        .transpose()
}

/// Strictly decoded operation-frozen graph material. Unknown values never
/// fall back to legacy or the current deployment pair.
#[derive(Debug, Clone, sqlx::FromRow, Serialize, Deserialize, PartialEq, Eq)]
pub struct FrozenStageTopologyRow {
    pub stage_topology_contract: String,
    pub stage_topology_canonical_json: String,
    pub stage_topology_sha256: String,
    pub stage_topology_freeze_source: String,
}

impl FrozenStageTopologyRow {
    pub fn topology(&self) -> Result<StageTopologyContract> {
        let topology = StageTopologyContract::try_parse(&self.stage_topology_contract)
            .map_err(|error| crate::DbError::Other(anyhow::Error::new(error)))?;
        let material = topology.freeze_material();
        if self.stage_topology_canonical_json != material.canonical_json
            || self.stage_topology_sha256 != material.sha256
        {
            return Err(crate::DbError::Other(anyhow::anyhow!(
                "stage topology canonical material mismatch"
            )));
        }
        Ok(topology)
    }
}

pub async fn get_stage_topology(
    pool: &PgPool,
    operation_id: Uuid,
) -> Result<Option<FrozenStageTopologyRow>> {
    let row = sqlx::query_as::<_, FrozenStageTopologyRow>(
        r#"SELECT stage_topology_contract,stage_topology_canonical_json,
                  stage_topology_sha256,stage_topology_freeze_source
             FROM operation_state
            WHERE operation_id=$1"#,
    )
    .bind(operation_id)
    .fetch_optional(pool)
    .await?;
    if let Some(row) = &row {
        row.topology()?;
    }
    Ok(row)
}

/// Read and strictly decode the immutable Tool Truth contract. Unknown values
/// fail closed instead of projecting legacy semantics.
pub async fn get_tool_truth_contract(
    pool: &PgPool,
    operation_id: Uuid,
) -> Result<Option<ToolTruthContract>> {
    let value: Option<String> =
        sqlx::query_scalar("SELECT tool_truth_contract FROM operation_state WHERE operation_id=$1")
            .bind(operation_id)
            .fetch_optional(pool)
            .await?;
    value
        .map(|value| parse_tool_truth_contract(&value))
        .transpose()
}

/// Read the server-frozen Enumeration analyzer contract. Unknown values fail
/// closed instead of silently selecting legacy or production behavior.
pub async fn get_enumeration_analysis_contract(
    pool: &PgPool,
    operation_id: Uuid,
) -> Result<Option<String>> {
    let value: Option<String> = sqlx::query_scalar(
        "SELECT enumeration_analysis_contract FROM operation_state WHERE operation_id=$1",
    )
    .bind(operation_id)
    .fetch_optional(pool)
    .await?;
    match value.as_deref() {
        None | Some("legacy_v1") | Some("agent_team_v2_shadow") | Some("agent_team_v2") => {
            Ok(value)
        }
        Some(other) => Err(crate::DbError::Other(anyhow::anyhow!(
            "unknown enumeration analysis contract: {other}"
        ))),
    }
}

/// Read and strictly decode the immutable Candidate execution contract without
/// widening the hot-path [`OperationStateRow`] projection.
pub async fn get_attack_execution_contract(
    pool: &PgPool,
    operation_id: Uuid,
) -> Result<Option<AttackExecutionContract>> {
    let value: Option<String> = sqlx::query_scalar(
        "SELECT attack_execution_contract FROM operation_state WHERE operation_id=$1",
    )
    .bind(operation_id)
    .fetch_optional(pool)
    .await?;
    value
        .map(|value| parse_attack_execution_contract(&value))
        .transpose()
}

/// Read only the operation epoch fields required by stage-validity guards.
pub async fn get_epoch(pool: &PgPool, operation_id: Uuid) -> Result<Option<OperationEpochRow>> {
    let row = sqlx::query_as::<_, OperationEpochRow>(GET_OPERATION_EPOCH_SQL)
        .bind(operation_id)
        .fetch_optional(pool)
        .await?;
    Ok(row)
}

/// 推进 cursor (resume 时更新最新 evidence + classification + scope_version 锚).
pub async fn advance_cursor(
    pool: &PgPool,
    operation_id: Uuid,
    last_evidence_audit_id: Option<i64>,
    last_classification_id: Option<i64>,
    last_scope_version: Option<i64>,
) -> Result<()> {
    sqlx::query(
        r#"UPDATE operation_state
           SET last_evidence_audit_id = $2,
               last_classification_id = $3,
               last_scope_version = $4
           WHERE operation_id = $1"#,
    )
    .bind(operation_id)
    .bind(last_evidence_audit_id)
    .bind(last_classification_id)
    .bind(last_scope_version)
    .execute(pool)
    .await?;
    Ok(())
}

/// 切换 current_stage + 写新 stage_started_at = NOW().
pub async fn advance_stage(pool: &PgPool, operation_id: Uuid, new_stage: &str) -> Result<()> {
    sqlx::query(ADVANCE_STAGE_SQL)
        .bind(operation_id)
        .bind(new_stage)
        .bind(EAS_WEB_TRANSPORT_FAILURES_NAMESPACE)
        .execute(pool)
        .await?;
    Ok(())
}

/// cross-profile transition (assessment → pentest 等) · 标 superseded_by 但不删原行.
///
/// 调用方应已经先插入新 operation_state(new_operation_id), 再调本 fn 把
/// 老 operation 的 superseded_by 指向新 operation.
pub async fn supersede(
    pool: &PgPool,
    operation_id: Uuid,
    superseded_by_new_operation: Uuid,
) -> Result<()> {
    sqlx::query(
        r#"UPDATE operation_state
           SET superseded_by = $2
           WHERE operation_id = $1"#,
    )
    .bind(operation_id)
    .bind(superseded_by_new_operation)
    .execute(pool)
    .await?;
    Ok(())
}

/// 写入 harness 私有 resume 状态，同时原子保留 EAS transport breaker 的
/// reserved namespace；其它 checkpoint 字段仍以调用方 payload 为准。
pub async fn write_state_blob(
    pool: &PgPool,
    operation_id: Uuid,
    state_blob: serde_json::Value,
) -> Result<()> {
    sqlx::query(WRITE_STATE_BLOB_SQL)
        .bind(operation_id)
        .bind(state_blob)
        .bind(EAS_WEB_TRANSPORT_FAILURES_NAMESPACE)
        .execute(pool)
        .await?;
    Ok(())
}

/// Atomically advance one EAS exact-origin transport failure counter without
/// replacing any sibling `state_blob` namespace. `None` means the trusted EAS
/// operation epoch changed or the bounded namespace refused a new slot.
pub async fn increment_eas_web_transport_failure(
    pool: &PgPool,
    input: &EasWebTransportFailureInput,
) -> Result<Option<i32>> {
    let attempts = sqlx::query_scalar::<_, i32>(INCREMENT_EAS_WEB_TRANSPORT_FAILURE_SQL)
        .bind(input.operation_id)
        .bind(input.stage_started_at)
        .bind(EAS_WEB_TRANSPORT_FAILURES_NAMESPACE)
        .bind(&input.slot_key)
        .bind(input.organization_id)
        .bind(input.target_id)
        .bind(&input.origin)
        .bind(&input.technique)
        .bind(&input.failure_class)
        .bind(MAX_EAS_WEB_TRANSPORT_FAILURE_SLOTS)
        .bind(MAX_EAS_WEB_TRANSPORT_FAILURE_BYTES)
        .fetch_optional(pool)
        .await?;
    Ok(attempts)
}

/// Clear all known failure-class slots for one exact owner/origin after an HTTP
/// response or successful producer observation. The epoch guard prevents a
/// late completion from deleting counters from a newer EAS attempt.
pub async fn clear_eas_web_transport_failures(
    pool: &PgPool,
    operation_id: Uuid,
    stage_started_at: DateTime<Utc>,
    slot_keys: &[String],
) -> Result<bool> {
    let result = sqlx::query(CLEAR_EAS_WEB_TRANSPORT_FAILURES_SQL)
        .bind(operation_id)
        .bind(stage_started_at)
        .bind(EAS_WEB_TRANSPORT_FAILURES_NAMESPACE)
        .bind(slot_keys)
        .execute(pool)
        .await?;
    Ok(result.rows_affected() == 1)
}

pub async fn mark_eas_web_fingerprint_producer_blocked(
    pool: &PgPool,
    input: &EasWebTransportFailureInput,
    evidence_id: i64,
    run_id: &str,
    independent_handoff: Option<(i64, &str, &str)>,
) -> Result<bool> {
    if evidence_id <= 0 || run_id.trim().is_empty() {
        return Ok(false);
    }
    let (independent_evidence_id, independent_producer, independent_kind) = independent_handoff
        .map(|(id, producer, kind)| (Some(id), Some(producer), Some(kind)))
        .unwrap_or((None, None, None));
    if independent_evidence_id.is_some_and(|id| id <= 0) {
        return Ok(false);
    }
    let applied = sqlx::query_scalar::<_, i32>(MARK_EAS_WEB_FINGERPRINT_PRODUCER_BLOCKED_SQL)
        .bind(input.operation_id)
        .bind(input.stage_started_at)
        .bind(EAS_WEB_TRANSPORT_FAILURES_NAMESPACE)
        .bind(&input.slot_key)
        .bind(input.organization_id)
        .bind(input.target_id)
        .bind(&input.origin)
        .bind(&input.technique)
        .bind(&input.failure_class)
        .bind(evidence_id)
        .bind(run_id)
        .bind(independent_evidence_id)
        .bind(independent_producer)
        .bind(independent_kind)
        .fetch_optional(pool)
        .await?;
    Ok(applied.is_some())
}

pub async fn list_eas_web_fingerprint_producer_blocked_origins(
    pool: &PgPool,
    operation_id: Uuid,
    stage_started_at: DateTime<Utc>,
    organization_id: Uuid,
) -> Result<Vec<(Uuid, String, i64)>> {
    let rows = sqlx::query_as::<_, (String, String, String)>(
        LIST_EAS_WEB_FINGERPRINT_PRODUCER_BLOCKED_SQL,
    )
    .bind(operation_id)
    .bind(stage_started_at)
    .bind(organization_id)
    .bind(EAS_WEB_TRANSPORT_FAILURES_NAMESPACE)
    .fetch_all(pool)
    .await?;
    Ok(rows
        .into_iter()
        .filter_map(|(target_id, origin, evidence_id)| {
            Some((
                Uuid::parse_str(&target_id).ok()?,
                origin,
                evidence_id.parse::<i64>().ok().filter(|id| *id > 0)?,
            ))
        })
        .collect())
}

/// Read exact operation/org/target/origin exclusions after the operation has
/// entered Enumeration. Every row is joined back to its guarded audit evidence;
/// a bare JSON boolean or malformed evidence id is never trusted.
pub async fn list_eas_web_transport_blocked_origins(
    pool: &PgPool,
    operation_id: Uuid,
    organization_id: Uuid,
) -> Result<Vec<(Uuid, String)>> {
    let rows = sqlx::query_as::<_, (String, String)>(LIST_EAS_WEB_TRANSPORT_BLOCKED_SQL)
        .bind(operation_id)
        .bind(organization_id)
        .bind(EAS_WEB_TRANSPORT_FAILURES_NAMESPACE)
        .fetch_all(pool)
        .await?;
    Ok(rows
        .into_iter()
        .filter_map(|(target_id, origin)| Uuid::parse_str(&target_id).ok().map(|id| (id, origin)))
        .collect())
}

/// Engagement-org isolation (设计 2026-06-15-engagement-org-isolation): bind this
/// operation to its scoping-confirmed engagement root org (or clear with `None`).
/// Read back via [`get`] → `OperationStateRow::engagement_org_id`.
pub async fn set_engagement_org(
    pool: &PgPool,
    operation_id: Uuid,
    engagement_org_id: Option<Uuid>,
) -> Result<()> {
    sqlx::query(
        r#"UPDATE operation_state
           SET engagement_org_id = $2
           WHERE operation_id = $1"#,
    )
    .bind(operation_id)
    .bind(engagement_org_id)
    .execute(pool)
    .await?;
    Ok(())
}

/// Clear operation bindings that point at a deleted organization subtree.
///
/// `operation_state.engagement_org_id` is intentionally a soft cursor, not a FK,
/// so organization delete has to null it explicitly before the org row is gone.
pub async fn clear_engagement_org_for_subtree(pool: &PgPool, root_org_id: Uuid) -> Result<u64> {
    let res = sqlx::query(&build_clear_engagement_org_for_subtree_sql())
        .bind(root_org_id)
        .execute(pool)
        .await?;
    Ok(res.rows_affected())
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn operation_state_row_serde_roundtrip() {
        let row = OperationStateRow {
            operation_id: Uuid::new_v4(),
            profile: "assessment".to_string(),
            current_stage: "external_attack_surface".to_string(),
            runtime_memory_contract: "dual_write_legacy_read".to_string(),
            tool_truth_contract: "legacy_v1".to_string(),
            application_model_contract: "legacy_no_model".to_string(),
            investigation_contract_version: "legacy_candidate_v1".to_string(),
            investigation_rollout_mode: "legacy_only".to_string(),
            stage_topology_contract: "legacy_candidate_verification_v1".to_string(),
            stage_topology_canonical_json: "{\"contract_version\":\"stage_topology.v1\"}"
                .to_string(),
            stage_topology_sha256: format!("sha256:{}", "0".repeat(64)),
            stage_topology_freeze_source: "legacy_backfill_v1".to_string(),
            project_scope_id: Some(Uuid::new_v4()),
            stage_started_at: Utc::now(),
            last_evidence_audit_id: Some(42),
            last_classification_id: Some(7),
            last_scope_version: Some(3),
            state_blob: serde_json::json!({"sprint_id": "abc"}),
            superseded_by: None,
            engagement_org_id: Some(Uuid::new_v4()),
        };
        let json = serde_json::to_string(&row).expect("serialize");
        let back: OperationStateRow = serde_json::from_str(&json).expect("deserialize");
        assert_eq!(row.operation_id, back.operation_id);
        assert_eq!(row.current_stage, back.current_stage);
        assert_eq!(row.runtime_memory_contract, back.runtime_memory_contract);
        assert_eq!(row.tool_truth_contract, back.tool_truth_contract);
        assert_eq!(row.stage_topology_contract, back.stage_topology_contract);
        assert_eq!(
            row.investigation_contract_version,
            back.investigation_contract_version
        );
        assert_eq!(
            row.investigation_rollout_mode,
            back.investigation_rollout_mode
        );
        assert_eq!(row.state_blob, back.state_blob);
    }

    #[test]
    fn operation_insert_freezes_runtime_memory_contract() {
        assert!(INSERT_OPERATION_SQL.contains("runtime_memory_contract"));
        assert!(INSERT_OPERATION_SQL.contains("$4"));
    }

    #[test]
    fn operation_insert_freezes_tool_truth_contract() {
        assert!(INSERT_OPERATION_SQL.contains("tool_truth_contract"));
        assert!(INSERT_OPERATION_WITH_EXECUTOR_SQL.contains("tool_truth_contract"));
        assert!(OPERATION_STATE_ROW_COLUMNS.contains("tool_truth_contract"));
    }

    #[test]
    fn operation_insert_freezes_investigation_contract() {
        assert!(INSERT_OPERATION_SQL.contains("investigation_contract_version"));
        assert!(INSERT_OPERATION_SQL.contains("investigation_rollout_mode"));
        assert!(INSERT_OPERATION_WITH_EXECUTOR_SQL.contains("investigation_contract_version"));
        assert!(OPERATION_STATE_ROW_COLUMNS.contains("investigation_rollout_mode"));
    }

    #[test]
    fn tool_truth_contract_does_not_fallback_on_unknown_value() {
        let error = parse_tool_truth_contract("future_contract")
            .expect_err("unknown persisted contract must fail closed");
        assert_eq!(
            error.to_string(),
            "unknown tool-truth contract: future_contract"
        );
    }

    #[test]
    fn runtime_operation_creation_locks_tool_truth_rollout() {
        let source = include_str!("runtime_memory_tx.rs");
        assert!(source.contains("operation_rollout::deployment_pair_for_share"));
        assert!(source.contains("operation_rollout::choose_stage_fork_pair_and_write_adoption"));
    }

    #[test]
    fn runtime_memory_store_operation_insert_requires_project_scope_and_returns_row() {
        assert!(INSERT_OPERATION_WITH_EXECUTOR_SQL.contains("project_scope_id"));
        assert!(INSERT_OPERATION_WITH_EXECUTOR_SQL.contains("$5"));
        assert!(INSERT_OPERATION_WITH_EXECUTOR_SQL.contains("RETURNING"));
        assert!(OPERATION_STATE_ROW_COLUMNS.contains("project_scope_id"));
    }

    #[test]
    fn attack_contract_cannot_enable_v2_on_non_v2_runtime_memory() {
        assert!(validate_operation_contracts(
            "dual_write_read_v2_fallback",
            golish_core::AttackExecutionContract::V2Only,
        )
        .is_err());
        assert!(validate_operation_contracts(
            "v2_only",
            golish_core::AttackExecutionContract::V2Only,
        )
        .is_ok());
    }

    #[test]
    fn operation_epoch_query_excludes_large_state_blob_and_keeps_epoch_contract() {
        assert!(GET_OPERATION_EPOCH_SQL.contains("operation_id"));
        assert!(GET_OPERATION_EPOCH_SQL.contains("current_stage"));
        assert!(GET_OPERATION_EPOCH_SQL.contains("stage_started_at"));
        assert!(GET_OPERATION_EPOCH_SQL.contains("superseded_by"));
        assert!(GET_OPERATION_EPOCH_SQL.contains("engagement_org_id"));
        assert!(!GET_OPERATION_EPOCH_SQL.contains("state_blob"));
        assert!(!GET_OPERATION_EPOCH_SQL.contains("profile"));
        assert!(!GET_OPERATION_EPOCH_SQL.contains("last_evidence_audit_id"));
        assert!(!GET_OPERATION_EPOCH_SQL.contains("last_classification_id"));
        assert!(!GET_OPERATION_EPOCH_SQL.contains("last_scope_version"));
    }

    #[test]
    fn generic_state_blob_checkpoint_preserves_reserved_transport_namespace() {
        assert!(WRITE_STATE_BLOB_SQL.contains("jsonb_set"));
        assert!(WRITE_STATE_BLOB_SQL.contains("state_blob -> $3"));
        assert!(WRITE_STATE_BLOB_SQL.contains("runtime_memory_contract <> 'v2_only'"));
        assert!(!WRITE_STATE_BLOB_SQL.contains("SET state_blob = $2"));
    }

    #[test]
    fn stage_advance_resets_breaker_only_on_new_eas_epoch() {
        assert!(ADVANCE_STAGE_SQL.contains("$2 = 'external_attack_surface'"));
        assert!(ADVANCE_STAGE_SQL.contains("ARRAY[$3]"));
        assert!(ADVANCE_STAGE_SQL.contains("ELSE state_blob"));
    }

    #[test]
    fn operation_epoch_row_serde_roundtrip() {
        let row = OperationEpochRow {
            operation_id: Uuid::new_v4(),
            current_stage: "enumeration".to_string(),
            stage_started_at: Utc::now(),
            superseded_by: Some(Uuid::new_v4()),
            engagement_org_id: Some(Uuid::new_v4()),
        };

        let json = serde_json::to_value(&row).expect("serialize epoch row");
        let back: OperationEpochRow =
            serde_json::from_value(json.clone()).expect("deserialize epoch row");

        assert_eq!(row.operation_id, back.operation_id);
        assert_eq!(row.current_stage, back.current_stage);
        assert_eq!(row.stage_started_at, back.stage_started_at);
        assert_eq!(row.superseded_by, back.superseded_by);
        assert_eq!(row.engagement_org_id, back.engagement_org_id);
        assert!(json.get("state_blob").is_none());
    }

    #[test]
    fn clear_engagement_org_for_subtree_sql_recurses_org_tree() {
        let sql = build_clear_engagement_org_for_subtree_sql();
        assert!(sql.contains("WITH RECURSIVE subtree"));
        assert!(sql.contains("JOIN subtree s ON o.parent_id = s.id"));
        assert!(sql.contains("UPDATE operation_state"));
        assert!(sql.contains("SET engagement_org_id = NULL"));
        assert!(sql.contains("engagement_org_id IN (SELECT id FROM subtree)"));
    }

    #[test]
    fn eas_transport_counter_sql_is_epoch_guarded_and_preserves_sibling_namespaces() {
        assert!(INCREMENT_EAS_WEB_TRANSPORT_FAILURE_SQL.contains("jsonb_set"));
        assert!(INCREMENT_EAS_WEB_TRANSPORT_FAILURE_SQL.contains("state_blob"));
        assert!(INCREMENT_EAS_WEB_TRANSPORT_FAILURE_SQL
            .contains("current_stage = 'external_attack_surface'"));
        assert!(INCREMENT_EAS_WEB_TRANSPORT_FAILURE_SQL.contains("stage_started_at = $2"));
        assert!(INCREMENT_EAS_WEB_TRANSPORT_FAILURE_SQL.contains("superseded_by IS NULL"));
        assert!(INCREMENT_EAS_WEB_TRANSPORT_FAILURE_SQL.contains("epoch_started_at"));
        assert!(INCREMENT_EAS_WEB_TRANSPORT_FAILURE_SQL.contains("failure_class'] = $9::text"));
        assert!(INCREMENT_EAS_WEB_TRANSPORT_FAILURE_SQL.contains("jsonb_object_keys"));
        assert!(INCREMENT_EAS_WEB_TRANSPORT_FAILURE_SQL.contains("pg_column_size"));
        assert!(!INCREMENT_EAS_WEB_TRANSPORT_FAILURE_SQL.contains("SET state_blob = $"));
    }

    #[test]
    fn independent_transport_handoff_read_is_operation_org_target_and_origin_scoped() {
        assert!(LIST_EAS_WEB_TRANSPORT_BLOCKED_SQL.contains("operation_id = $1"));
        assert!(LIST_EAS_WEB_TRANSPORT_BLOCKED_SQL.contains("current_stage = 'enumeration'"));
        assert!(LIST_EAS_WEB_TRANSPORT_BLOCKED_SQL.contains("organization_id"));
        assert!(LIST_EAS_WEB_TRANSPORT_BLOCKED_SQL.contains("target_id"));
        assert!(LIST_EAS_WEB_TRANSPORT_BLOCKED_SQL.contains("origin"));
        assert!(LIST_EAS_WEB_TRANSPORT_BLOCKED_SQL.contains("independently_confirmed"));
    }

    #[tokio::test]
    async fn transport_state_sql_explains_against_migrated_db_when_configured() {
        let Ok(database_url) = std::env::var("GOLISH_TEST_DATABASE_URL") else {
            return;
        };
        let pool = PgPool::connect(&database_url)
            .await
            .expect("connect GOLISH_TEST_DATABASE_URL");
        let operation_id = Uuid::new_v4();
        let organization_id = Uuid::new_v4();
        let target_id = Uuid::new_v4();
        let epoch = Utc::now();
        let slot = "explain-only-slot";
        let namespace = EAS_WEB_TRANSPORT_FAILURES_NAMESPACE;

        let sql = format!("EXPLAIN {INCREMENT_EAS_WEB_TRANSPORT_FAILURE_SQL}");
        sqlx::query(&sql)
            .bind(operation_id)
            .bind(epoch)
            .bind(namespace)
            .bind(slot)
            .bind(organization_id)
            .bind(target_id)
            .bind("https://example.test:443")
            .bind("GOLISH-EAS-WEB-FINGERPRINT")
            .bind("timeout")
            .bind(MAX_EAS_WEB_TRANSPORT_FAILURE_SLOTS)
            .bind(MAX_EAS_WEB_TRANSPORT_FAILURE_BYTES)
            .fetch_all(&pool)
            .await
            .expect("EXPLAIN counter increment");

        let sql = format!("EXPLAIN {CLEAR_EAS_WEB_TRANSPORT_FAILURES_SQL}");
        sqlx::query(&sql)
            .bind(operation_id)
            .bind(epoch)
            .bind(namespace)
            .bind(vec![slot.to_string()])
            .fetch_all(&pool)
            .await
            .expect("EXPLAIN counter clear");

        let sql = format!("EXPLAIN {MARK_EAS_WEB_FINGERPRINT_PRODUCER_BLOCKED_SQL}");
        sqlx::query(&sql)
            .bind(operation_id)
            .bind(epoch)
            .bind(namespace)
            .bind(slot)
            .bind(organization_id)
            .bind(target_id)
            .bind("https://example.test:443")
            .bind("GOLISH-EAS-WEB-FINGERPRINT")
            .bind("timeout")
            .bind(1_i64)
            .bind("run")
            .bind(Option::<i64>::None)
            .bind(Option::<String>::None)
            .bind(Option::<String>::None)
            .fetch_all(&pool)
            .await
            .expect("EXPLAIN atomic producer/handoff seal");

        let sql = format!("EXPLAIN {LIST_EAS_WEB_FINGERPRINT_PRODUCER_BLOCKED_SQL}");
        sqlx::query(&sql)
            .bind(operation_id)
            .bind(epoch)
            .bind(organization_id)
            .bind(namespace)
            .fetch_all(&pool)
            .await
            .expect("EXPLAIN producer-blocked read");

        let sql = format!("EXPLAIN {LIST_EAS_WEB_TRANSPORT_BLOCKED_SQL}");
        sqlx::query(&sql)
            .bind(operation_id)
            .bind(organization_id)
            .bind(namespace)
            .fetch_all(&pool)
            .await
            .expect("EXPLAIN Enumeration handoff read");

        let sql = format!("EXPLAIN {WRITE_STATE_BLOB_SQL}");
        sqlx::query(&sql)
            .bind(operation_id)
            .bind(serde_json::json!({"graph_flow": {}}))
            .bind(namespace)
            .fetch_all(&pool)
            .await
            .expect("EXPLAIN namespace-preserving checkpoint write");

        let sql = format!("EXPLAIN {ADVANCE_STAGE_SQL}");
        sqlx::query(&sql)
            .bind(operation_id)
            .bind("enumeration")
            .bind(namespace)
            .fetch_all(&pool)
            .await
            .expect("EXPLAIN stage transition");
    }
}
