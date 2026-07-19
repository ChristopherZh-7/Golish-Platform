//! Immutable source-operation authority for stage-test forks.
//!
//! The high-level materializer runs inside a caller-owned transaction.  It
//! derives profile/contracts, exact live final seals and current Target rows
//! from PostgreSQL; callers supply only operation/scope identities and the
//! already profile-validated stage slice.

use std::collections::{BTreeMap, BTreeSet};

use chrono::{DateTime, Utc};
use serde::{Deserialize, Serialize};
use serde_json::{json, Value};
use sqlx::{Executor, PgConnection, PgPool, Postgres};
use uuid::Uuid;

use super::operation_scope_decisions::sha256_json;

pub const FORK_TABLE_NAME: &str = "operation_stage_forks";
pub const INPUT_TABLE_NAME: &str = "operation_stage_fork_inputs";
pub const TARGET_TABLE_NAME: &str = "operation_stage_fork_targets";

#[derive(Debug, thiserror::Error)]
pub enum OperationStageForkError {
    #[error("operation stage fork identity mismatch: {code}")]
    IdentityMismatch { code: &'static str },
    #[error("operation stage fork conflict: {code}")]
    Conflict { code: &'static str },
    #[error("operation stage fork authority missing: {entity}")]
    Missing { entity: &'static str },
    #[error(transparent)]
    Sqlx(#[from] sqlx::Error),
}

impl OperationStageForkError {
    pub const fn code(&self) -> &'static str {
        match self {
            Self::IdentityMismatch { code } | Self::Conflict { code } => code,
            Self::Missing { entity } => entity,
            Self::Sqlx(_) => "operation_stage_fork_storage",
        }
    }
}

pub type OperationStageForkResult<T> = Result<T, OperationStageForkError>;

#[derive(Debug, Clone, sqlx::FromRow, Serialize, Deserialize, PartialEq, Eq)]
pub struct OperationStageForkRow {
    pub operation_id: Uuid,
    pub source_operation_id: Uuid,
    pub project_scope_id: Uuid,
    pub source_scope_snapshot_id: Uuid,
    pub target_scope_snapshot_id: Uuid,
    pub source_profile: String,
    pub target_profile: String,
    pub source_runtime_memory_contract: String,
    pub target_runtime_memory_contract: String,
    pub source_attack_execution_contract: String,
    pub target_attack_execution_contract: String,
    pub entry_stage: String,
    pub terminal_stage: String,
    pub adopted_stage_kinds: Vec<String>,
    pub expected_input_count: i32,
    pub expected_target_count: i32,
    pub manifest: Value,
    pub manifest_sha256: String,
    pub schema_version: i32,
    pub created_at: DateTime<Utc>,
}

#[derive(Debug, Clone, sqlx::FromRow, Serialize, Deserialize, PartialEq, Eq)]
pub struct OperationStageForkInputRow {
    pub id: Uuid,
    pub operation_id: Uuid,
    pub source_operation_id: Uuid,
    pub source_scope_snapshot_id: Uuid,
    pub target_scope_snapshot_id: Uuid,
    pub source_stage_kind: String,
    pub organization_id: Uuid,
    pub source_stage_execution_id: Uuid,
    pub source_stage_run_unit_id: Uuid,
    pub source_worker_run_id: Option<Uuid>,
    pub source_deliverable_submission_id: Uuid,
    pub source_handoff_id: Option<Uuid>,
    pub source_scope_hash: String,
    pub source_payload: Value,
    pub source_payload_sha256: String,
    pub source_evidence_ids: Vec<i64>,
    pub source_coverage_watermark: Value,
    pub source_unit_gate_decision_hash: String,
    pub source_aggregate_pass_token_hash: Option<String>,
    pub source_gate_passed_at: DateTime<Utc>,
    pub manifest_input_sha256: String,
    pub schema_version: i32,
    pub created_at: DateTime<Utc>,
}

#[derive(Debug, Clone, sqlx::FromRow, Serialize, Deserialize, PartialEq, Eq)]
pub struct OperationStageForkTargetRow {
    pub id: Uuid,
    pub operation_id: Uuid,
    pub scope_snapshot_id: Uuid,
    pub organization_id: Uuid,
    pub ordinal: i32,
    pub live_target_id: Uuid,
    pub target_name_at_fork: String,
    pub target_type_at_fork: String,
    pub target_value_at_fork: String,
    pub target_scope_at_fork: String,
    pub target_source_at_fork: String,
    pub project_path_at_fork: String,
    pub canonical_identity_sha256: String,
    pub schema_version: i32,
    pub frozen_at: DateTime<Utc>,
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct NewOperationStageFork {
    pub operation_id: Uuid,
    pub source_operation_id: Uuid,
    pub project_scope_id: Uuid,
    pub source_scope_snapshot_id: Uuid,
    pub target_scope_snapshot_id: Uuid,
    pub source_profile: String,
    pub target_profile: String,
    pub source_runtime_memory_contract: String,
    pub target_runtime_memory_contract: String,
    pub source_attack_execution_contract: String,
    pub target_attack_execution_contract: String,
    pub entry_stage: String,
    pub terminal_stage: String,
    pub adopted_stage_kinds: Vec<String>,
    pub expected_input_count: i32,
    pub expected_target_count: i32,
    pub manifest: Value,
    pub manifest_sha256: String,
    pub schema_version: i32,
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct NewOperationStageForkInput {
    pub id: Uuid,
    pub operation_id: Uuid,
    pub source_operation_id: Uuid,
    pub source_scope_snapshot_id: Uuid,
    pub target_scope_snapshot_id: Uuid,
    pub source_stage_kind: String,
    pub organization_id: Uuid,
    pub source_stage_execution_id: Uuid,
    pub source_stage_run_unit_id: Uuid,
    pub source_worker_run_id: Option<Uuid>,
    pub source_deliverable_submission_id: Uuid,
    pub source_handoff_id: Option<Uuid>,
    pub source_scope_hash: String,
    pub source_payload: Value,
    pub source_payload_sha256: String,
    pub source_evidence_ids: Vec<i64>,
    pub source_coverage_watermark: Value,
    pub source_unit_gate_decision_hash: String,
    pub source_aggregate_pass_token_hash: Option<String>,
    pub source_gate_passed_at: DateTime<Utc>,
    pub manifest_input_sha256: String,
    pub schema_version: i32,
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct NewOperationStageForkTarget {
    pub id: Uuid,
    pub operation_id: Uuid,
    pub scope_snapshot_id: Uuid,
    pub organization_id: Uuid,
    pub ordinal: i32,
    pub live_target_id: Uuid,
    pub target_name_at_fork: String,
    pub target_type_at_fork: String,
    pub target_value_at_fork: String,
    pub target_scope_at_fork: String,
    pub target_source_at_fork: String,
    pub project_path_at_fork: String,
    pub canonical_identity_sha256: String,
    pub schema_version: i32,
}

/// Trusted identities supplied by operation creation.  Profile, contracts,
/// final-seal content and Target content are deliberately absent: the
/// repository derives and locks them from the database.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct MaterializeOperationStageFork {
    pub operation_id: Uuid,
    pub target_scope_snapshot_id: Uuid,
    pub project_scope_id: Uuid,
    pub source_operation_id: Uuid,
    pub source_scope_snapshot_id: Uuid,
    pub entry_stage: String,
    pub terminal_stage: String,
    pub adopted_stage_kinds: Vec<String>,
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct MaterializedOperationStageFork {
    pub fork: OperationStageForkRow,
    pub inputs: Vec<OperationStageForkInputRow>,
    pub targets: Vec<OperationStageForkTargetRow>,
}

#[derive(Debug, sqlx::FromRow)]
struct LockedForkAuthority {
    source_profile: String,
    target_profile: String,
    source_runtime_memory_contract: String,
    target_runtime_memory_contract: String,
    source_attack_execution_contract: String,
    target_attack_execution_contract: String,
    source_project_scope_id: Option<Uuid>,
    target_project_scope_id: Option<Uuid>,
    source_superseded_by: Option<Uuid>,
    canonical_project_path: String,
}

#[derive(Debug, Clone, sqlx::FromRow)]
struct SourceScopeUnit {
    organization_id: Uuid,
    role: String,
    ordinal: i32,
}

#[derive(Debug, Clone, sqlx::FromRow)]
struct SourceFinalSeal {
    source_handoff_id: Option<Uuid>,
    source_stage_kind: String,
    organization_id: Uuid,
    source_stage_execution_id: Uuid,
    source_stage_run_unit_id: Uuid,
    source_worker_run_id: Option<Uuid>,
    source_deliverable_submission_id: Uuid,
    source_scope_hash: String,
    source_payload: Value,
    source_payload_sha256: String,
    source_evidence_ids: Vec<i64>,
    source_coverage_watermark: Value,
    source_unit_gate_decision_hash: String,
    source_aggregate_pass_token_hash: Option<String>,
    source_gate_passed_at: DateTime<Utc>,
}

#[derive(Debug, Clone, sqlx::FromRow)]
struct LiveTargetSnapshot {
    live_target_id: Uuid,
    organization_id: Uuid,
    target_name_at_fork: String,
    target_type_at_fork: String,
    target_value_at_fork: String,
    target_scope_at_fork: String,
    target_source_at_fork: String,
    project_path_at_fork: String,
}

fn stage_rank(stage: &str) -> Option<u8> {
    match stage {
        "scoping" => Some(1),
        "target_intel" => Some(2),
        "external_attack_surface" => Some(3),
        "enumeration" => Some(4),
        "vuln_triage" => Some(5),
        "attack_candidate" => Some(6),
        _ => None,
    }
}

fn validate_slice(input: &MaterializeOperationStageFork) -> OperationStageForkResult<()> {
    if input.operation_id.is_nil()
        || input.source_operation_id.is_nil()
        || input.target_scope_snapshot_id.is_nil()
        || input.source_scope_snapshot_id.is_nil()
        || input.project_scope_id.is_nil()
        || input.operation_id == input.source_operation_id
    {
        return Err(OperationStageForkError::IdentityMismatch {
            code: "stage_fork_identity_invalid",
        });
    }
    let entry_rank = stage_rank(&input.entry_stage)
        .filter(|rank| *rank > 1)
        .ok_or(OperationStageForkError::Conflict {
            code: "stage_fork_entry_invalid",
        })?;
    let terminal_rank =
        stage_rank(&input.terminal_stage).ok_or(OperationStageForkError::Conflict {
            code: "stage_fork_terminal_invalid",
        })?;
    if terminal_rank < entry_rank {
        return Err(OperationStageForkError::Conflict {
            code: "stage_fork_slice_reversed",
        });
    }
    if input.adopted_stage_kinds.first().map(String::as_str) != Some("scoping") {
        return Err(OperationStageForkError::Conflict {
            code: "stage_fork_prefix_missing_scoping",
        });
    }
    let mut previous_rank = 0;
    let mut unique_stages = BTreeSet::new();
    for stage in &input.adopted_stage_kinds {
        let rank = stage_rank(stage).ok_or(OperationStageForkError::Conflict {
            code: "stage_fork_prefix_stage_invalid",
        })?;
        if rank <= previous_rank || rank >= entry_rank || !unique_stages.insert(stage.as_str()) {
            return Err(OperationStageForkError::Conflict {
                code: "stage_fork_prefix_not_canonical",
            });
        }
        previous_rank = rank;
    }
    Ok(())
}

fn input_manifest(input: &NewOperationStageForkInput) -> Value {
    json!({
        "schema_version": input.schema_version,
        "source_operation_id": input.source_operation_id,
        "source_scope_snapshot_id": input.source_scope_snapshot_id,
        "source_stage_kind": input.source_stage_kind,
        "organization_id": input.organization_id,
        "source_stage_execution_id": input.source_stage_execution_id,
        "source_stage_run_unit_id": input.source_stage_run_unit_id,
        "source_worker_run_id": input.source_worker_run_id,
        "source_deliverable_submission_id": input.source_deliverable_submission_id,
        "source_handoff_id": input.source_handoff_id,
        "source_scope_hash": input.source_scope_hash,
        "source_payload_sha256": input.source_payload_sha256,
        "source_evidence_ids": input.source_evidence_ids,
        "source_coverage_watermark": input.source_coverage_watermark,
        "source_unit_gate_decision_hash": input.source_unit_gate_decision_hash,
        "source_aggregate_pass_token_hash": input.source_aggregate_pass_token_hash,
        "source_gate_passed_at": input.source_gate_passed_at,
    })
}

fn target_identity_manifest(project_scope_id: Uuid, target: &LiveTargetSnapshot) -> Value {
    json!({
        "schema_version": 1,
        "project_scope_id": project_scope_id,
        "organization_id": target.organization_id,
        "live_target_id": target.live_target_id,
        "target_type": target.target_type_at_fork,
        "target_value": target.target_value_at_fork,
        "target_scope": target.target_scope_at_fork,
        "target_source": target.target_source_at_fork,
        "project_path": target.project_path_at_fork,
    })
}

pub async fn insert_fork_with_executor<'e, E>(
    executor: E,
    input: &NewOperationStageFork,
) -> OperationStageForkResult<OperationStageForkRow>
where
    E: Executor<'e, Database = Postgres>,
{
    if input.schema_version <= 0 || sha256_json(&input.manifest) != input.manifest_sha256 {
        return Err(OperationStageForkError::Conflict {
            code: "stage_fork_manifest_hash_mismatch",
        });
    }
    sqlx::query_as::<_, OperationStageForkRow>(
        r#"INSERT INTO operation_stage_forks(
               operation_id,source_operation_id,project_scope_id,
               source_scope_snapshot_id,target_scope_snapshot_id,
               source_profile,target_profile,
               source_runtime_memory_contract,target_runtime_memory_contract,
               source_attack_execution_contract,target_attack_execution_contract,
               entry_stage,terminal_stage,adopted_stage_kinds,
               expected_input_count,expected_target_count,
               manifest,manifest_sha256,schema_version
           ) VALUES(
               $1,$2,$3,$4,$5,$6,$7,$8,$9,$10,$11,$12,$13,$14,$15,$16,$17,$18,$19
           ) RETURNING *"#,
    )
    .bind(input.operation_id)
    .bind(input.source_operation_id)
    .bind(input.project_scope_id)
    .bind(input.source_scope_snapshot_id)
    .bind(input.target_scope_snapshot_id)
    .bind(&input.source_profile)
    .bind(&input.target_profile)
    .bind(&input.source_runtime_memory_contract)
    .bind(&input.target_runtime_memory_contract)
    .bind(&input.source_attack_execution_contract)
    .bind(&input.target_attack_execution_contract)
    .bind(&input.entry_stage)
    .bind(&input.terminal_stage)
    .bind(&input.adopted_stage_kinds)
    .bind(input.expected_input_count)
    .bind(input.expected_target_count)
    .bind(&input.manifest)
    .bind(&input.manifest_sha256)
    .bind(input.schema_version)
    .fetch_one(executor)
    .await
    .map_err(Into::into)
}

pub async fn insert_input_with_executor<'e, E>(
    executor: E,
    input: &NewOperationStageForkInput,
) -> OperationStageForkResult<OperationStageForkInputRow>
where
    E: Executor<'e, Database = Postgres>,
{
    if input.schema_version <= 0
        || sha256_json(&input_manifest(input)) != input.manifest_input_sha256
    {
        return Err(OperationStageForkError::Conflict {
            code: "stage_fork_input_hash_mismatch",
        });
    }
    sqlx::query_as::<_, OperationStageForkInputRow>(
        r#"INSERT INTO operation_stage_fork_inputs(
               id,operation_id,source_operation_id,source_scope_snapshot_id,
               target_scope_snapshot_id,source_stage_kind,organization_id,
               source_stage_execution_id,source_stage_run_unit_id,
               source_worker_run_id,source_deliverable_submission_id,
               source_handoff_id,source_scope_hash,source_payload,
               source_payload_sha256,source_evidence_ids,
               source_coverage_watermark,source_unit_gate_decision_hash,
               source_aggregate_pass_token_hash,source_gate_passed_at,
               manifest_input_sha256,schema_version
           ) VALUES(
               $1,$2,$3,$4,$5,$6,$7,$8,$9,$10,$11,$12,$13,$14,$15,$16,$17,$18,$19,$20,$21,$22
           ) RETURNING *"#,
    )
    .bind(input.id)
    .bind(input.operation_id)
    .bind(input.source_operation_id)
    .bind(input.source_scope_snapshot_id)
    .bind(input.target_scope_snapshot_id)
    .bind(&input.source_stage_kind)
    .bind(input.organization_id)
    .bind(input.source_stage_execution_id)
    .bind(input.source_stage_run_unit_id)
    .bind(input.source_worker_run_id)
    .bind(input.source_deliverable_submission_id)
    .bind(input.source_handoff_id)
    .bind(&input.source_scope_hash)
    .bind(&input.source_payload)
    .bind(&input.source_payload_sha256)
    .bind(&input.source_evidence_ids)
    .bind(&input.source_coverage_watermark)
    .bind(&input.source_unit_gate_decision_hash)
    .bind(&input.source_aggregate_pass_token_hash)
    .bind(input.source_gate_passed_at)
    .bind(&input.manifest_input_sha256)
    .bind(input.schema_version)
    .fetch_one(executor)
    .await
    .map_err(Into::into)
}

pub async fn insert_target_with_executor<'e, E>(
    executor: E,
    input: &NewOperationStageForkTarget,
) -> OperationStageForkResult<OperationStageForkTargetRow>
where
    E: Executor<'e, Database = Postgres>,
{
    if input.schema_version <= 0 || input.ordinal < 0 {
        return Err(OperationStageForkError::Conflict {
            code: "stage_fork_target_identity_invalid",
        });
    }
    sqlx::query_as::<_, OperationStageForkTargetRow>(
        r#"INSERT INTO operation_stage_fork_targets(
               id,operation_id,scope_snapshot_id,organization_id,ordinal,
               live_target_id,target_name_at_fork,target_type_at_fork,
               target_value_at_fork,target_scope_at_fork,target_source_at_fork,
               project_path_at_fork,canonical_identity_sha256,schema_version
           ) VALUES($1,$2,$3,$4,$5,$6,$7,$8,$9,$10,$11,$12,$13,$14)
           RETURNING *"#,
    )
    .bind(input.id)
    .bind(input.operation_id)
    .bind(input.scope_snapshot_id)
    .bind(input.organization_id)
    .bind(input.ordinal)
    .bind(input.live_target_id)
    .bind(&input.target_name_at_fork)
    .bind(&input.target_type_at_fork)
    .bind(&input.target_value_at_fork)
    .bind(&input.target_scope_at_fork)
    .bind(&input.target_source_at_fork)
    .bind(&input.project_path_at_fork)
    .bind(&input.canonical_identity_sha256)
    .bind(input.schema_version)
    .fetch_one(executor)
    .await
    .map_err(Into::into)
}

/// Lock, derive and insert one complete immutable fork materialization.
///
/// The caller must have already inserted the target operation and sealed its
/// `reuse_reconfirmed` scope clone in the same transaction.  This function
/// neither starts nor commits the transaction and performs no external work.
pub async fn materialize_with_connection(
    connection: &mut PgConnection,
    request: &MaterializeOperationStageFork,
) -> OperationStageForkResult<MaterializedOperationStageFork> {
    validate_slice(request)?;

    let authority = sqlx::query_as::<_, LockedForkAuthority>(
        r#"SELECT source.profile AS source_profile,
                  target.profile AS target_profile,
                  source.runtime_memory_contract AS source_runtime_memory_contract,
                  target.runtime_memory_contract AS target_runtime_memory_contract,
                  source.attack_execution_contract AS source_attack_execution_contract,
                  target.attack_execution_contract AS target_attack_execution_contract,
                  source.project_scope_id AS source_project_scope_id,
                  target.project_scope_id AS target_project_scope_id,
                  source.superseded_by AS source_superseded_by,
                  project.canonical_project_path
             FROM operation_state AS source
             JOIN operation_state AS target ON target.operation_id=$2
             JOIN operation_org_scope_snapshots AS source_scope
               ON source_scope.id=$3 AND source_scope.operation_id=source.operation_id
             JOIN operation_org_scope_snapshots AS target_scope
               ON target_scope.id=$4 AND target_scope.operation_id=target.operation_id
             JOIN project_scopes AS project ON project.project_scope_id=$5
            WHERE source.operation_id=$1
            FOR SHARE OF source,target,source_scope,target_scope,project"#,
    )
    .bind(request.source_operation_id)
    .bind(request.operation_id)
    .bind(request.source_scope_snapshot_id)
    .bind(request.target_scope_snapshot_id)
    .bind(request.project_scope_id)
    .fetch_optional(&mut *connection)
    .await?
    .ok_or(OperationStageForkError::Missing {
        entity: "stage_fork_operation_or_scope",
    })?;
    if authority.source_project_scope_id != Some(request.project_scope_id)
        || authority.target_project_scope_id != Some(request.project_scope_id)
    {
        return Err(OperationStageForkError::IdentityMismatch {
            code: "stage_fork_project_scope_mismatch",
        });
    }
    if authority.source_superseded_by.is_some() {
        return Err(OperationStageForkError::Conflict {
            code: "stage_fork_source_superseded",
        });
    }
    if authority.source_profile != authority.target_profile
        || authority.source_runtime_memory_contract != authority.target_runtime_memory_contract
        || authority.source_attack_execution_contract != authority.target_attack_execution_contract
    {
        return Err(OperationStageForkError::Conflict {
            code: "stage_fork_operation_contract_mismatch",
        });
    }

    let source_units = sqlx::query_as::<_, SourceScopeUnit>(
        r#"SELECT organization_id,role,ordinal
             FROM operation_org_scope_units
            WHERE snapshot_id=$1
            ORDER BY ordinal
            FOR SHARE"#,
    )
    .bind(request.source_scope_snapshot_id)
    .fetch_all(&mut *connection)
    .await?;
    if source_units.first().map(|unit| unit.role.as_str()) != Some("root") {
        return Err(OperationStageForkError::Missing {
            entity: "stage_fork_source_scope_root",
        });
    }

    // Serialize against organization deletion before reading the current
    // Target snapshot. Deletion takes these same live organization locks in
    // UPDATE mode before it freezes Targets, so either the fork commits first
    // and becomes visible to deletion preflight, or deletion commits first and
    // this materialization observes its durable job.
    let target_organization_ids = sqlx::query_scalar::<_, Uuid>(
        r#"SELECT organization_id
             FROM operation_org_scope_units
            WHERE snapshot_id=$1
            ORDER BY ordinal
            FOR SHARE"#,
    )
    .bind(request.target_scope_snapshot_id)
    .fetch_all(&mut *connection)
    .await?;
    let source_organization_ids = source_units
        .iter()
        .map(|unit| unit.organization_id)
        .collect::<BTreeSet<_>>();
    if target_organization_ids
        .iter()
        .copied()
        .collect::<BTreeSet<_>>()
        != source_organization_ids
    {
        return Err(OperationStageForkError::IdentityMismatch {
            code: "stage_fork_target_scope_units_mismatch",
        });
    }
    let locked_organization_ids = sqlx::query_scalar::<_, Uuid>(
        "SELECT id FROM organizations WHERE id=ANY($1) ORDER BY id FOR SHARE",
    )
    .bind(&target_organization_ids)
    .fetch_all(&mut *connection)
    .await?;
    if locked_organization_ids.len() != target_organization_ids.len() {
        return Err(OperationStageForkError::Missing {
            entity: "stage_fork_target_organization",
        });
    }
    let organization_deletion_active: bool = sqlx::query_scalar(
        r#"SELECT EXISTS(
               SELECT 1
                 FROM organization_deletion_job_units AS unit
                 JOIN organization_deletion_jobs AS job ON job.id=unit.job_id
                WHERE unit.organization_id_at_time=ANY($1)
                  AND job.state<>'hard_delete_committed'
           )"#,
    )
    .bind(&target_organization_ids)
    .fetch_one(&mut *connection)
    .await?;
    if organization_deletion_active {
        return Err(OperationStageForkError::Conflict {
            code: "stage_fork_target_organization_deleting",
        });
    }

    let mut source_seals = sqlx::query_as::<_, SourceFinalSeal>(
        r#"SELECT handoff.id AS source_handoff_id,
                  handoff.from_stage_kind AS source_stage_kind,
                  handoff.organization_id,
                  handoff.stage_execution_id AS source_stage_execution_id,
                  handoff.source_stage_run_unit_id,
                  submission.worker_run_id AS source_worker_run_id,
                  handoff.deliverable_submission_id AS source_deliverable_submission_id,
                  handoff.scope_hash AS source_scope_hash,
                  handoff.payload AS source_payload,
                  handoff.payload_sha256 AS source_payload_sha256,
                  handoff.evidence_ids AS source_evidence_ids,
                  handoff.coverage_watermark AS source_coverage_watermark,
                  handoff.unit_gate_decision_hash AS source_unit_gate_decision_hash,
                  handoff.aggregate_pass_token_hash AS source_aggregate_pass_token_hash,
                  handoff.gate_passed_at AS source_gate_passed_at
             FROM stage_handoffs AS handoff
             JOIN stage_runs AS run
               ON run.id=handoff.stage_execution_id
              AND run.operation_id=handoff.operation_id
              AND run.stage_kind=handoff.from_stage_kind
             JOIN stage_run_units AS unit
               ON unit.id=handoff.source_stage_run_unit_id
              AND unit.operation_id=handoff.operation_id
              AND unit.stage_execution_id=handoff.stage_execution_id
              AND unit.organization_id=handoff.organization_id
              AND unit.stage_kind=handoff.from_stage_kind
             JOIN stage_deliverable_submissions AS submission
               ON submission.id=handoff.deliverable_submission_id
              AND submission.operation_id=handoff.operation_id
              AND submission.stage_execution_id=handoff.stage_execution_id
              AND submission.stage_run_unit_id=handoff.source_stage_run_unit_id
              AND submission.organization_id=handoff.organization_id
              AND submission.stage_kind=handoff.from_stage_kind
            WHERE handoff.operation_id=$1
              AND handoff.scope_snapshot_id=$2
              AND handoff.from_stage_kind=ANY($3)
              AND handoff.invalidated_at IS NULL
            ORDER BY operation_stage_fork_stage_rank(handoff.from_stage_kind),
                     handoff.organization_id,handoff.gate_passed_at,handoff.id
            FOR SHARE OF handoff,run,unit,submission"#,
    )
    .bind(request.source_operation_id)
    .bind(request.source_scope_snapshot_id)
    .bind(&request.adopted_stage_kinds)
    .fetch_all(&mut *connection)
    .await?;

    if request
        .adopted_stage_kinds
        .iter()
        .any(|stage| stage == "scoping")
    {
        let mut scoping_seal = sqlx::query_as::<_, SourceFinalSeal>(
            r#"SELECT NULL::UUID AS source_handoff_id,
                      'scoping'::TEXT AS source_stage_kind,
                      snapshot.root_organization_id AS organization_id,
                      decision.stage_execution_id AS source_stage_execution_id,
                      unit.id AS source_stage_run_unit_id,
                      NULL::UUID AS source_worker_run_id,
                      submission.id AS source_deliverable_submission_id,
                      snapshot.scope_hash AS source_scope_hash,
                      jsonb_build_object(
                          'schema_version', 1,
                          'scope_decision_id', decision.id,
                          'scope_snapshot_id', snapshot.id,
                          'project_scope_id', snapshot.project_scope_id,
                          'root_organization_id', snapshot.root_organization_id,
                          'decision_hash', decision.decision_hash,
                          'scope_hash', snapshot.scope_hash,
                          'decision_rows', decision.decision_rows,
                          'scope_units', (
                              SELECT jsonb_agg(
                                         jsonb_build_object(
                                             'organization_id', scope_unit.organization_id,
                                             'parent_organization_id', scope_unit.parent_organization_id,
                                             'organization_name_at_freeze', scope_unit.organization_name_at_freeze,
                                             'role', scope_unit.role,
                                             'depth', scope_unit.depth,
                                             'ordinal', scope_unit.ordinal,
                                             'ownership_percent', scope_unit.ownership_percent,
                                             'decision_row_id', scope_unit.decision_row_id,
                                             'approval_source', scope_unit.approval_source
                                         ) ORDER BY scope_unit.ordinal
                                     )
                                FROM operation_org_scope_units AS scope_unit
                               WHERE scope_unit.snapshot_id=snapshot.id
                          )
                      ) AS source_payload,
                      ''::TEXT AS source_payload_sha256,
                      '{}'::BIGINT[] AS source_evidence_ids,
                      jsonb_build_object(
                          'scope_snapshot_id', snapshot.id,
                          'scope_hash', snapshot.scope_hash,
                          'sealed_at', snapshot.sealed_at
                      ) AS source_coverage_watermark,
                      decision.decision_hash AS source_unit_gate_decision_hash,
                      NULL::TEXT AS source_aggregate_pass_token_hash,
                      snapshot.sealed_at AS source_gate_passed_at
                 FROM operation_org_scope_snapshots AS snapshot
                 JOIN operation_scope_decisions AS decision
                   ON decision.id=snapshot.scope_decision_id
                  AND decision.operation_id=snapshot.operation_id
                  AND decision.project_scope_id=snapshot.project_scope_id
                 JOIN stage_runs AS run
                   ON run.id=decision.stage_execution_id
                  AND run.operation_id=snapshot.operation_id
                  AND run.stage_kind='scoping'
                  AND run.status='completed'
                 JOIN stage_run_units AS unit
                   ON unit.operation_id=snapshot.operation_id
                  AND unit.stage_execution_id=decision.stage_execution_id
                  AND unit.scope_snapshot_id=snapshot.id
                  AND unit.organization_id=snapshot.root_organization_id
                  AND unit.stage_kind='scoping'
                  AND unit.status='passed'
                 JOIN stage_deliverable_submissions AS submission
                   ON submission.operation_id=snapshot.operation_id
                  AND submission.stage_execution_id=decision.stage_execution_id
                  AND submission.stage_run_unit_id=unit.id
                  AND submission.organization_id=snapshot.root_organization_id
                  AND submission.stage_kind='scoping'
                  AND submission.worker_run_id IS NULL
                WHERE snapshot.id=$1
                  AND snapshot.operation_id=$2
                  AND snapshot.sealed_at IS NOT NULL
                FOR SHARE OF snapshot,decision,run,unit,submission"#,
        )
        .bind(request.source_scope_snapshot_id)
        .bind(request.source_operation_id)
        .fetch_optional(&mut *connection)
        .await?
        .ok_or(OperationStageForkError::Missing {
            entity: "stage_fork_source_scoping_seal",
        })?;
        scoping_seal.source_payload_sha256 = sha256_json(&scoping_seal.source_payload);
        source_seals.push(scoping_seal);
    }

    let mut seal_by_stage_org = BTreeMap::new();
    for seal in source_seals {
        let key = (seal.source_stage_kind.clone(), seal.organization_id);
        if seal_by_stage_org.insert(key, seal).is_some() {
            return Err(OperationStageForkError::Conflict {
                code: "stage_fork_source_final_seal_ambiguous",
            });
        }
    }

    let mut expected_keys = BTreeSet::new();
    for stage in &request.adopted_stage_kinds {
        for unit in &source_units {
            if stage != "scoping" || unit.role == "root" {
                expected_keys.insert((stage.clone(), unit.organization_id));
            }
        }
    }
    if seal_by_stage_org.keys().cloned().collect::<BTreeSet<_>>() != expected_keys {
        return Err(OperationStageForkError::Missing {
            entity: "stage_fork_source_final_seal_matrix",
        });
    }

    let mut new_inputs = Vec::with_capacity(expected_keys.len());
    for (stage, organization_id) in expected_keys {
        let seal = seal_by_stage_org
            .remove(&(stage.clone(), organization_id))
            .ok_or(OperationStageForkError::Missing {
                entity: "stage_fork_source_final_seal",
            })?;
        let id = Uuid::new_v5(
            &request.operation_id,
            format!("stage-fork-input:v1:{stage}:{organization_id}").as_bytes(),
        );
        let mut input = NewOperationStageForkInput {
            id,
            operation_id: request.operation_id,
            source_operation_id: request.source_operation_id,
            source_scope_snapshot_id: request.source_scope_snapshot_id,
            target_scope_snapshot_id: request.target_scope_snapshot_id,
            source_stage_kind: stage,
            organization_id,
            source_stage_execution_id: seal.source_stage_execution_id,
            source_stage_run_unit_id: seal.source_stage_run_unit_id,
            source_worker_run_id: seal.source_worker_run_id,
            source_deliverable_submission_id: seal.source_deliverable_submission_id,
            source_handoff_id: seal.source_handoff_id,
            source_scope_hash: seal.source_scope_hash,
            source_payload: seal.source_payload,
            source_payload_sha256: seal.source_payload_sha256,
            source_evidence_ids: seal.source_evidence_ids,
            source_coverage_watermark: seal.source_coverage_watermark,
            source_unit_gate_decision_hash: seal.source_unit_gate_decision_hash,
            source_aggregate_pass_token_hash: seal.source_aggregate_pass_token_hash,
            source_gate_passed_at: seal.source_gate_passed_at,
            manifest_input_sha256: String::new(),
            schema_version: 1,
        };
        input.manifest_input_sha256 = sha256_json(&input_manifest(&input));
        new_inputs.push(input);
    }
    new_inputs.sort_by_key(|input| {
        (
            stage_rank(&input.source_stage_kind).unwrap_or(u8::MAX),
            source_units
                .iter()
                .find(|unit| unit.organization_id == input.organization_id)
                .map_or(i32::MAX, |unit| unit.ordinal),
            input.organization_id,
        )
    });

    let live_targets = sqlx::query_as::<_, LiveTargetSnapshot>(
        r#"SELECT target.id AS live_target_id,
                  target.organization_id,
                  target.name AS target_name_at_fork,
                  target.target_type::TEXT AS target_type_at_fork,
                  target.value AS target_value_at_fork,
                  target.scope::TEXT AS target_scope_at_fork,
                  target.source AS target_source_at_fork,
                  target.project_path AS project_path_at_fork
             FROM targets AS target
             JOIN operation_org_scope_units AS unit
               ON unit.snapshot_id=$1
              AND unit.organization_id=target.organization_id
            WHERE target.project_path=$2
            ORDER BY unit.ordinal,target.target_type::TEXT,target.value,target.id
            FOR SHARE OF target,unit"#,
    )
    .bind(request.target_scope_snapshot_id)
    .bind(&authority.canonical_project_path)
    .fetch_all(&mut *connection)
    .await?;
    if stage_rank(&request.entry_stage).is_some_and(|rank| rank >= 3)
        && !live_targets
            .iter()
            .any(|target| target.target_scope_at_fork == "in")
    {
        return Err(OperationStageForkError::Conflict {
            code: "stage_fork_active_target_snapshot_empty",
        });
    }

    let mut organization_ordinals = BTreeMap::<Uuid, i32>::new();
    let mut new_targets = Vec::with_capacity(live_targets.len());
    for target in live_targets {
        let ordinal = organization_ordinals
            .entry(target.organization_id)
            .or_default();
        let identity_hash =
            sha256_json(&target_identity_manifest(request.project_scope_id, &target));
        new_targets.push(NewOperationStageForkTarget {
            id: Uuid::new_v5(
                &request.operation_id,
                format!("stage-fork-target:v1:{}", target.live_target_id).as_bytes(),
            ),
            operation_id: request.operation_id,
            scope_snapshot_id: request.target_scope_snapshot_id,
            organization_id: target.organization_id,
            ordinal: *ordinal,
            live_target_id: target.live_target_id,
            target_name_at_fork: target.target_name_at_fork,
            target_type_at_fork: target.target_type_at_fork,
            target_value_at_fork: target.target_value_at_fork,
            target_scope_at_fork: target.target_scope_at_fork,
            target_source_at_fork: target.target_source_at_fork,
            project_path_at_fork: target.project_path_at_fork,
            canonical_identity_sha256: identity_hash,
            schema_version: 1,
        });
        *ordinal = ordinal
            .checked_add(1)
            .ok_or(OperationStageForkError::Conflict {
                code: "stage_fork_target_ordinal_overflow",
            })?;
    }

    let input_manifest_rows = new_inputs
        .iter()
        .map(|input| {
            json!({
                "id": input.id,
                "source_stage_kind": input.source_stage_kind,
                "organization_id": input.organization_id,
                "manifest_input_sha256": input.manifest_input_sha256,
            })
        })
        .collect::<Vec<_>>();
    let target_manifest_rows = new_targets
        .iter()
        .map(|target| {
            json!({
                "id": target.id,
                "organization_id": target.organization_id,
                "ordinal": target.ordinal,
                "live_target_id": target.live_target_id,
                "canonical_identity_sha256": target.canonical_identity_sha256,
            })
        })
        .collect::<Vec<_>>();
    let manifest = json!({
        "schema_version": 1,
        "operation_id": request.operation_id,
        "source_operation_id": request.source_operation_id,
        "project_scope_id": request.project_scope_id,
        "source_scope_snapshot_id": request.source_scope_snapshot_id,
        "target_scope_snapshot_id": request.target_scope_snapshot_id,
        "profile": authority.source_profile,
        "runtime_memory_contract": authority.source_runtime_memory_contract,
        "attack_execution_contract": authority.source_attack_execution_contract,
        "entry_stage": request.entry_stage,
        "terminal_stage": request.terminal_stage,
        "adopted_stage_kinds": request.adopted_stage_kinds,
        "inputs": input_manifest_rows,
        "targets": target_manifest_rows,
    });
    let expected_input_count =
        i32::try_from(new_inputs.len()).map_err(|_| OperationStageForkError::Conflict {
            code: "stage_fork_input_count_overflow",
        })?;
    let expected_target_count =
        i32::try_from(new_targets.len()).map_err(|_| OperationStageForkError::Conflict {
            code: "stage_fork_target_count_overflow",
        })?;
    let new_fork = NewOperationStageFork {
        operation_id: request.operation_id,
        source_operation_id: request.source_operation_id,
        project_scope_id: request.project_scope_id,
        source_scope_snapshot_id: request.source_scope_snapshot_id,
        target_scope_snapshot_id: request.target_scope_snapshot_id,
        source_profile: authority.source_profile.clone(),
        target_profile: authority.target_profile,
        source_runtime_memory_contract: authority.source_runtime_memory_contract.clone(),
        target_runtime_memory_contract: authority.target_runtime_memory_contract,
        source_attack_execution_contract: authority.source_attack_execution_contract.clone(),
        target_attack_execution_contract: authority.target_attack_execution_contract,
        entry_stage: request.entry_stage.clone(),
        terminal_stage: request.terminal_stage.clone(),
        adopted_stage_kinds: request.adopted_stage_kinds.clone(),
        expected_input_count,
        expected_target_count,
        manifest_sha256: sha256_json(&manifest),
        manifest,
        schema_version: 1,
    };

    let fork = insert_fork_with_executor(&mut *connection, &new_fork).await?;
    let mut inputs = Vec::with_capacity(new_inputs.len());
    for input in &new_inputs {
        inputs.push(insert_input_with_executor(&mut *connection, input).await?);
    }
    let mut targets = Vec::with_capacity(new_targets.len());
    for target in &new_targets {
        targets.push(insert_target_with_executor(&mut *connection, target).await?);
    }
    Ok(MaterializedOperationStageFork {
        fork,
        inputs,
        targets,
    })
}

pub async fn get(
    pool: &PgPool,
    operation_id: Uuid,
) -> OperationStageForkResult<Option<OperationStageForkRow>> {
    sqlx::query_as("SELECT * FROM operation_stage_forks WHERE operation_id=$1")
        .bind(operation_id)
        .fetch_optional(pool)
        .await
        .map_err(Into::into)
}

pub async fn list_inputs(
    pool: &PgPool,
    operation_id: Uuid,
) -> OperationStageForkResult<Vec<OperationStageForkInputRow>> {
    sqlx::query_as(
        r#"SELECT * FROM operation_stage_fork_inputs
            WHERE operation_id=$1
            ORDER BY operation_stage_fork_stage_rank(source_stage_kind),organization_id"#,
    )
    .bind(operation_id)
    .fetch_all(pool)
    .await
    .map_err(Into::into)
}

pub async fn list_targets(
    pool: &PgPool,
    operation_id: Uuid,
) -> OperationStageForkResult<Vec<OperationStageForkTargetRow>> {
    sqlx::query_as(
        r#"SELECT * FROM operation_stage_fork_targets
            WHERE operation_id=$1
            ORDER BY organization_id,ordinal"#,
    )
    .bind(operation_id)
    .fetch_all(pool)
    .await
    .map_err(Into::into)
}

/// Resolve an exact adopted predecessor operation for a target fork. `None`
/// means the target operation must use its own stage truth. Multiple or stale
/// rows fail closed instead of selecting a latest source.
pub async fn source_operation_for_stage(
    pool: &PgPool,
    operation_id: Uuid,
    organization_id: Uuid,
    source_stage_kind: &str,
) -> OperationStageForkResult<Option<Uuid>> {
    let rows = sqlx::query_scalar::<_, Uuid>(
        r#"SELECT input.source_operation_id
             FROM operation_stage_fork_inputs AS input
             JOIN operation_state AS source_operation
               ON source_operation.operation_id=input.source_operation_id
              AND source_operation.superseded_by IS NULL
        LEFT JOIN stage_handoffs AS handoff
               ON handoff.id=input.source_handoff_id
              AND handoff.operation_id=input.source_operation_id
              AND handoff.organization_id=input.organization_id
              AND handoff.from_stage_kind=input.source_stage_kind
              AND handoff.invalidated_at IS NULL
        LEFT JOIN operation_org_scope_snapshots AS source_scope
               ON source_scope.id=input.source_scope_snapshot_id
              AND source_scope.operation_id=input.source_operation_id
              AND source_scope.root_organization_id=input.organization_id
              AND source_scope.scope_hash=input.source_scope_hash
              AND source_scope.sealed_at=input.source_gate_passed_at
            WHERE input.operation_id=$1
              AND input.organization_id=$2
              AND input.source_stage_kind=$3
              AND (
                   (input.source_stage_kind='scoping'
                    AND input.source_handoff_id IS NULL
                    AND source_scope.id IS NOT NULL)
                   OR
                   (input.source_stage_kind<>'scoping'
                    AND handoff.id IS NOT NULL)
              )
            FOR SHARE OF input,source_operation"#,
    )
    .bind(operation_id)
    .bind(organization_id)
    .bind(source_stage_kind)
    .fetch_all(pool)
    .await?;
    match rows.as_slice() {
        [] => Ok(None),
        [source_operation_id] => Ok(Some(*source_operation_id)),
        _ => Err(OperationStageForkError::Conflict {
            code: "stage_fork_input_ambiguous",
        }),
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn request(entry: &str, terminal: &str, adopted: &[&str]) -> MaterializeOperationStageFork {
        MaterializeOperationStageFork {
            operation_id: Uuid::new_v4(),
            target_scope_snapshot_id: Uuid::new_v4(),
            project_scope_id: Uuid::new_v4(),
            source_operation_id: Uuid::new_v4(),
            source_scope_snapshot_id: Uuid::new_v4(),
            entry_stage: entry.to_string(),
            terminal_stage: terminal.to_string(),
            adopted_stage_kinds: adopted.iter().map(|stage| (*stage).to_string()).collect(),
        }
    }

    #[test]
    fn operation_stage_fork_accepts_canonical_projected_prefix() {
        assert!(validate_slice(&request(
            "vuln_triage",
            "attack_candidate",
            &[
                "scoping",
                "target_intel",
                "external_attack_surface",
                "enumeration",
            ],
        ))
        .is_ok());
    }

    #[test]
    fn operation_stage_fork_rejects_scoping_entry_and_prefix_holes_in_order() {
        let scoping = validate_slice(&request("scoping", "scoping", &["scoping"]))
            .expect_err("Scoping must use a fresh run");
        assert_eq!(scoping.code(), "stage_fork_entry_invalid");

        let unordered = validate_slice(&request(
            "vuln_triage",
            "vuln_triage",
            &["scoping", "enumeration", "target_intel"],
        ))
        .expect_err("prefix order must be canonical");
        assert_eq!(unordered.code(), "stage_fork_prefix_not_canonical");
    }

    #[test]
    fn operation_stage_fork_manifest_hash_binds_final_seal_fields() {
        let input = NewOperationStageForkInput {
            id: Uuid::new_v4(),
            operation_id: Uuid::new_v4(),
            source_operation_id: Uuid::new_v4(),
            source_scope_snapshot_id: Uuid::new_v4(),
            target_scope_snapshot_id: Uuid::new_v4(),
            source_stage_kind: "enumeration".to_string(),
            organization_id: Uuid::new_v4(),
            source_stage_execution_id: Uuid::new_v4(),
            source_stage_run_unit_id: Uuid::new_v4(),
            source_worker_run_id: Some(Uuid::new_v4()),
            source_deliverable_submission_id: Uuid::new_v4(),
            source_handoff_id: Some(Uuid::new_v4()),
            source_scope_hash: "a".repeat(64),
            source_payload: json!({"stage_id": "enumeration"}),
            source_payload_sha256: "b".repeat(64),
            source_evidence_ids: vec![1, 3],
            source_coverage_watermark: json!({"covered": 2}),
            source_unit_gate_decision_hash: "c".repeat(64),
            source_aggregate_pass_token_hash: Some("d".repeat(64)),
            source_gate_passed_at: Utc::now(),
            manifest_input_sha256: String::new(),
            schema_version: 1,
        };
        let first = sha256_json(&input_manifest(&input));
        let mut changed = input.clone();
        changed.source_evidence_ids.push(5);
        assert_ne!(first, sha256_json(&input_manifest(&changed)));
    }
}
