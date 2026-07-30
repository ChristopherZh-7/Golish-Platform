use std::collections::{BTreeMap, BTreeSet};

use golish_core::{InvestigationContractVersion, InvestigationRolloutMode};
use golish_pentest_domain::tool_truth::ToolTruthContract;
use serde_json::{json, Value};
use sqlx::{PgConnection, Postgres, Transaction};
use uuid::Uuid;

use super::{investigation_rollout, operation_scope_decisions::sha256_json, tool_truth_rollout};

pub const ADOPTION_TABLE_NAME: &str = "operation_contract_adoptions";

#[derive(Debug, thiserror::Error)]
pub enum OperationRolloutError {
    #[error("operation rollout conflict: {code}")]
    Conflict { code: &'static str },
    #[error("operation rollout identity mismatch: {code}")]
    IdentityMismatch { code: &'static str },
    #[error("operation rollout row missing: {entity}")]
    Missing { entity: &'static str },
    #[error(transparent)]
    Sqlx(#[from] sqlx::Error),
}

impl OperationRolloutError {
    pub const fn code(&self) -> &'static str {
        match self {
            Self::Conflict { code } | Self::IdentityMismatch { code } => code,
            Self::Missing { entity } => entity,
            Self::Sqlx(_) => "OPERATION_ROLLOUT_STORAGE",
        }
    }
}

pub type OperationRolloutResult<T> = Result<T, OperationRolloutError>;

fn tagged_sha256_json(value: &Value) -> String {
    format!("sha256:{}", sha256_json(value))
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct FrozenOperationJointContract {
    pub tool_truth_contract: ToolTruthContract,
    pub investigation_contract_version: InvestigationContractVersion,
    pub investigation_rollout_mode: InvestigationRolloutMode,
    pub joint_rank: i16,
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct OperationContractForkAdoptionRow {
    pub request_id: String,
    pub target_tool_truth_contract: ToolTruthContract,
    pub target_investigation_contract_version: InvestigationContractVersion,
    pub target_investigation_rollout_mode: InvestigationRolloutMode,
    pub source_final_seal_hash: String,
    pub adoption_exact_set_hash: String,
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct OperationContractAdoptionWitness {
    pub source_final_seal_hash: String,
    pub adoption_exact_set_hash: String,
}

#[derive(Debug, Clone, sqlx::FromRow, PartialEq, Eq)]
pub struct OperationContractAdoptionReceiptRow {
    pub adoption_id: Uuid,
    pub source_operation_id: Uuid,
    pub target_operation_id: Uuid,
    pub source_tool_truth_contract: String,
    pub source_investigation_contract_version: String,
    pub source_investigation_rollout_mode: String,
    pub source_joint_rank: i16,
    pub target_tool_truth_contract: String,
    pub target_investigation_contract_version: String,
    pub target_investigation_rollout_mode: String,
    pub target_joint_rank: i16,
    pub source_final_seal_hash: String,
    pub adoption_set_hash: String,
    pub stable_request_id: Uuid,
    pub receipt_hash: String,
}

#[derive(Debug, sqlx::FromRow)]
struct FrozenOperationJointContractRow {
    tool_truth_contract: String,
    investigation_contract_version: String,
    investigation_rollout_mode: String,
}

#[derive(Debug, sqlx::FromRow)]
struct SourceScopeUnitRow {
    organization_id: Uuid,
    role: String,
    ordinal: i32,
}

#[derive(Debug, sqlx::FromRow)]
struct SourceHandoffSealRow {
    handoff_id: Uuid,
    stage_kind: String,
    organization_id: Uuid,
    stage_execution_id: Uuid,
    stage_run_unit_id: Uuid,
    worker_run_id: Option<Uuid>,
    deliverable_submission_id: Uuid,
    source_scope_hash: String,
    payload_sha256: String,
    evidence_ids: Vec<i64>,
    coverage_watermark: Value,
    unit_gate_decision_hash: String,
    aggregate_pass_token_hash: Option<String>,
    gate_passed_at: chrono::DateTime<chrono::Utc>,
}

pub const fn joint_contract_rank(
    tool_truth_contract: ToolTruthContract,
    investigation_contract_version: InvestigationContractVersion,
    investigation_rollout_mode: InvestigationRolloutMode,
) -> Option<i16> {
    use InvestigationContractVersion::{HypothesisRegistryV1, LegacyCandidateV1};
    use InvestigationRolloutMode::{
        DualReadCompare, LegacyOnly, NewOnly, RegistryAuthoritativeLegacyProjection, ShadowRegistry,
    };
    use ToolTruthContract::{LegacyV1, ReceiptV1, ShadowV1};

    match (
        tool_truth_contract,
        investigation_contract_version,
        investigation_rollout_mode,
    ) {
        (LegacyV1, LegacyCandidateV1, LegacyOnly) => Some(0),
        (ShadowV1, LegacyCandidateV1, LegacyOnly) => Some(1),
        (ShadowV1, HypothesisRegistryV1, ShadowRegistry) => Some(2),
        (ShadowV1, HypothesisRegistryV1, DualReadCompare) => Some(3),
        (ReceiptV1, HypothesisRegistryV1, DualReadCompare) => Some(4),
        (ReceiptV1, HypothesisRegistryV1, RegistryAuthoritativeLegacyProjection) => Some(5),
        (ReceiptV1, HypothesisRegistryV1, NewOnly) => Some(6),
        _ => None,
    }
}

pub fn validate_joint_pair(
    tool_truth_contract: ToolTruthContract,
    investigation_contract_version: InvestigationContractVersion,
    investigation_rollout_mode: InvestigationRolloutMode,
) -> OperationRolloutResult<i16> {
    joint_contract_rank(
        tool_truth_contract,
        investigation_contract_version,
        investigation_rollout_mode,
    )
    .ok_or(OperationRolloutError::Conflict {
        code: "OPERATION_JOINT_CONTRACT_PAIR_INVALID",
    })
}

fn decode_joint_contract(
    row: FrozenOperationJointContractRow,
) -> OperationRolloutResult<FrozenOperationJointContract> {
    let tool_truth_contract = ToolTruthContract::try_from(row.tool_truth_contract.as_str())
        .map_err(|_| OperationRolloutError::Conflict {
            code: "OPERATION_TOOL_TRUTH_CONTRACT_UNKNOWN",
        })?;
    let (investigation_contract_version, investigation_rollout_mode) =
        investigation_rollout::parse_frozen_pair(
            &row.investigation_contract_version,
            &row.investigation_rollout_mode,
        )
        .map_err(|_| OperationRolloutError::Conflict {
            code: "OPERATION_INVESTIGATION_CONTRACT_UNKNOWN",
        })?;
    let joint_rank = validate_joint_pair(
        tool_truth_contract,
        investigation_contract_version,
        investigation_rollout_mode,
    )?;
    Ok(FrozenOperationJointContract {
        tool_truth_contract,
        investigation_contract_version,
        investigation_rollout_mode,
        joint_rank,
    })
}

/// Freeze the two deployment singletons in the only allowed order: Tool Truth
/// first, then Investigation. Both locks live in the caller transaction.
pub async fn deployment_pair_for_share(
    transaction: &mut Transaction<'_, Postgres>,
) -> OperationRolloutResult<FrozenOperationJointContract> {
    let tool_truth_contract = tool_truth_rollout::get_for_share(transaction)
        .await
        .map_err(|_| OperationRolloutError::Conflict {
            code: "OPERATION_TOOL_TRUTH_ROLLOUT_UNAVAILABLE",
        })?;
    let investigation = investigation_rollout::get_for_share(transaction)
        .await
        .map_err(|_| OperationRolloutError::Conflict {
            code: "OPERATION_INVESTIGATION_ROLLOUT_UNAVAILABLE",
        })?;
    let (investigation_contract_version, investigation_rollout_mode) =
        investigation_rollout::parse_frozen_pair(
            &investigation.contract_version,
            &investigation.rollout_mode,
        )
        .map_err(|_| OperationRolloutError::Conflict {
            code: "OPERATION_INVESTIGATION_CONTRACT_UNKNOWN",
        })?;
    if investigation.mode_rank != investigation_rollout_mode.mode_rank() {
        return Err(OperationRolloutError::Conflict {
            code: "OPERATION_INVESTIGATION_ROLLOUT_RANK_MISMATCH",
        });
    }
    let joint_rank = validate_joint_pair(
        tool_truth_contract,
        investigation_contract_version,
        investigation_rollout_mode,
    )?;
    Ok(FrozenOperationJointContract {
        tool_truth_contract,
        investigation_contract_version,
        investigation_rollout_mode,
        joint_rank,
    })
}

pub async fn source_pair_for_share(
    connection: &mut PgConnection,
    source_operation_id: Uuid,
) -> OperationRolloutResult<FrozenOperationJointContract> {
    let row = sqlx::query_as::<_, FrozenOperationJointContractRow>(
        r#"SELECT tool_truth_contract,investigation_contract_version,
                  investigation_rollout_mode
             FROM operation_state
            WHERE operation_id=$1
            FOR SHARE"#,
    )
    .bind(source_operation_id)
    .fetch_optional(connection)
    .await?
    .ok_or(OperationRolloutError::Missing {
        entity: "STAGE_FORK_SOURCE_OPERATION",
    })?;
    decode_joint_contract(row)
}

fn stage_rank(stage: &str) -> Option<i16> {
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

fn validate_adopted_stages(adopted_stage_kinds: &[String]) -> OperationRolloutResult<()> {
    if adopted_stage_kinds.first().map(String::as_str) != Some("scoping") {
        return Err(OperationRolloutError::Conflict {
            code: "STAGE_FORK_ADOPTION_PREFIX_INVALID",
        });
    }
    let mut previous = 0;
    for stage in adopted_stage_kinds {
        let rank = stage_rank(stage).ok_or(OperationRolloutError::Conflict {
            code: "STAGE_FORK_ADOPTION_STAGE_UNKNOWN",
        })?;
        if rank != previous + 1 {
            return Err(OperationRolloutError::Conflict {
                code: "STAGE_FORK_ADOPTION_STAGE_SET_INVALID",
            });
        }
        previous = rank;
    }
    Ok(())
}

/// Recompute the exact source final-seal witness used by a contract adoption.
/// The source operation, frozen scope, scope units, stage handoffs, Units and
/// submissions are all share-locked before the digest is returned.
pub async fn source_final_seal_hash_for_share(
    connection: &mut PgConnection,
    source_operation_id: Uuid,
    source_scope_snapshot_id: Uuid,
    adopted_stage_kinds: &[String],
) -> OperationRolloutResult<String> {
    validate_adopted_stages(adopted_stage_kinds)?;
    let scope: Option<Value> = sqlx::query_scalar(
        r#"SELECT jsonb_build_object(
                   'scope_snapshot_id',snapshot.id,
                   'scope_hash',snapshot.scope_hash,
                   'scope_decision_id',snapshot.scope_decision_id,
                   'sealed_at',snapshot.sealed_at
               )
             FROM operation_org_scope_snapshots AS snapshot
             JOIN operation_scope_decisions AS decision
               ON decision.id=snapshot.scope_decision_id
              AND decision.operation_id=snapshot.operation_id
             JOIN stage_runs AS run
               ON run.id=decision.stage_execution_id
              AND run.operation_id=snapshot.operation_id
              AND run.stage_kind='scoping'
              AND run.status='completed'
            WHERE snapshot.id=$1
              AND snapshot.operation_id=$2
              AND snapshot.sealed_at IS NOT NULL
            FOR SHARE OF snapshot,decision,run"#,
    )
    .bind(source_scope_snapshot_id)
    .bind(source_operation_id)
    .fetch_optional(&mut *connection)
    .await?
    .ok_or(OperationRolloutError::Missing {
        entity: "STAGE_FORK_SOURCE_SCOPING_SEAL",
    })?;
    let units = sqlx::query_as::<_, SourceScopeUnitRow>(
        r#"SELECT organization_id,role,ordinal
             FROM operation_org_scope_units
            WHERE snapshot_id=$1
            ORDER BY ordinal
            FOR SHARE"#,
    )
    .bind(source_scope_snapshot_id)
    .fetch_all(&mut *connection)
    .await?;
    if units.first().map(|unit| unit.role.as_str()) != Some("root") {
        return Err(OperationRolloutError::Missing {
            entity: "STAGE_FORK_SOURCE_SCOPE_ROOT",
        });
    }

    let non_scoping = adopted_stage_kinds
        .iter()
        .filter(|stage| stage.as_str() != "scoping")
        .cloned()
        .collect::<Vec<_>>();
    let handoffs = if non_scoping.is_empty() {
        Vec::new()
    } else {
        sqlx::query_as::<_, SourceHandoffSealRow>(
            r#"SELECT handoff.id AS handoff_id,
                      handoff.from_stage_kind AS stage_kind,
                      handoff.organization_id,
                      handoff.stage_execution_id,
                      handoff.source_stage_run_unit_id AS stage_run_unit_id,
                      submission.worker_run_id,
                      handoff.deliverable_submission_id,
                      handoff.scope_hash AS source_scope_hash,
                      handoff.payload_sha256,
                      handoff.evidence_ids,
                      handoff.coverage_watermark,
                      handoff.unit_gate_decision_hash,
                      handoff.aggregate_pass_token_hash,
                      handoff.gate_passed_at
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
                         handoff.organization_id,handoff.id
                FOR SHARE OF handoff,run,unit,submission"#,
        )
        .bind(source_operation_id)
        .bind(source_scope_snapshot_id)
        .bind(&non_scoping)
        .fetch_all(&mut *connection)
        .await?
    };

    let mut expected = BTreeSet::new();
    for stage in &non_scoping {
        for unit in &units {
            expected.insert((stage.clone(), unit.organization_id));
        }
    }
    let mut actual = BTreeMap::new();
    for handoff in handoffs {
        let key = (handoff.stage_kind.clone(), handoff.organization_id);
        if actual.insert(key, handoff).is_some() {
            return Err(OperationRolloutError::Conflict {
                code: "STAGE_FORK_SOURCE_FINAL_SEAL_AMBIGUOUS",
            });
        }
    }
    if actual.keys().cloned().collect::<BTreeSet<_>>() != expected {
        return Err(OperationRolloutError::Missing {
            entity: "STAGE_FORK_SOURCE_FINAL_SEAL_MATRIX",
        });
    }

    let unit_ordinals = units
        .iter()
        .map(|unit| (unit.organization_id, unit.ordinal))
        .collect::<BTreeMap<_, _>>();
    let mut seal_rows = actual.into_values().collect::<Vec<_>>();
    seal_rows.sort_by_key(|row| {
        (
            stage_rank(&row.stage_kind).unwrap_or(i16::MAX),
            unit_ordinals
                .get(&row.organization_id)
                .copied()
                .unwrap_or(i32::MAX),
            row.organization_id,
        )
    });
    let seals = seal_rows
        .into_iter()
        .map(|row| {
            json!({
                "handoff_id": row.handoff_id,
                "stage_kind": row.stage_kind,
                "organization_id": row.organization_id,
                "stage_execution_id": row.stage_execution_id,
                "stage_run_unit_id": row.stage_run_unit_id,
                "worker_run_id": row.worker_run_id,
                "deliverable_submission_id": row.deliverable_submission_id,
                "source_scope_hash": row.source_scope_hash,
                "payload_sha256": row.payload_sha256,
                "evidence_ids": row.evidence_ids,
                "coverage_watermark": row.coverage_watermark,
                "unit_gate_decision_hash": row.unit_gate_decision_hash,
                "aggregate_pass_token_hash": row.aggregate_pass_token_hash,
                "gate_passed_at": row.gate_passed_at,
            })
        })
        .collect::<Vec<_>>();
    let scope_units = units
        .iter()
        .map(|unit| {
            json!({
                "organization_id": unit.organization_id,
                "role": unit.role,
                "ordinal": unit.ordinal,
            })
        })
        .collect::<Vec<_>>();
    Ok(tagged_sha256_json(&json!({
        "schema": "operation-contract-source-final-seals.v1",
        "source_operation_id": source_operation_id,
        "source_scope_snapshot_id": source_scope_snapshot_id,
        "adopted_stage_kinds": adopted_stage_kinds,
        "scoping_seal": scope,
        "scope_units": scope_units,
        "stage_seals": seals,
    })))
}

fn adoption_set_hash(
    source_operation_id: Uuid,
    source_scope_snapshot_id: Uuid,
    adopted_stage_kinds: &[String],
    source: FrozenOperationJointContract,
    target: FrozenOperationJointContract,
    source_final_seal_hash: &str,
) -> String {
    tagged_sha256_json(&json!({
        "schema": "operation-contract-adoption-set.v1",
        "source_operation_id": source_operation_id,
        "source_scope_snapshot_id": source_scope_snapshot_id,
        "adopted_stage_kinds": adopted_stage_kinds,
        "source_tool_truth_contract": source.tool_truth_contract.as_str(),
        "source_investigation_contract_version": source.investigation_contract_version.as_str(),
        "source_investigation_rollout_mode": source.investigation_rollout_mode.as_str(),
        "source_joint_rank": source.joint_rank,
        "target_tool_truth_contract": target.tool_truth_contract.as_str(),
        "target_investigation_contract_version": target.investigation_contract_version.as_str(),
        "target_investigation_rollout_mode": target.investigation_rollout_mode.as_str(),
        "target_joint_rank": target.joint_rank,
        "source_final_seal_hash": source_final_seal_hash,
    }))
}

pub async fn prepare_stage_fork_adoption_witness(
    connection: &mut PgConnection,
    source_operation_id: Uuid,
    source_scope_snapshot_id: Uuid,
    adopted_stage_kinds: &[String],
    target_tool_truth_contract: ToolTruthContract,
    target_investigation_contract_version: InvestigationContractVersion,
    target_investigation_rollout_mode: InvestigationRolloutMode,
) -> OperationRolloutResult<OperationContractAdoptionWitness> {
    let source = source_pair_for_share(&mut *connection, source_operation_id).await?;
    let target_rank = validate_joint_pair(
        target_tool_truth_contract,
        target_investigation_contract_version,
        target_investigation_rollout_mode,
    )?;
    if target_rank != source.joint_rank + 1 {
        return Err(OperationRolloutError::Conflict {
            code: "STAGE_FORK_OPERATION_CONTRACT_ADOPTION_NOT_ADJACENT",
        });
    }
    let target = FrozenOperationJointContract {
        tool_truth_contract: target_tool_truth_contract,
        investigation_contract_version: target_investigation_contract_version,
        investigation_rollout_mode: target_investigation_rollout_mode,
        joint_rank: target_rank,
    };
    let source_final_seal_hash = source_final_seal_hash_for_share(
        &mut *connection,
        source_operation_id,
        source_scope_snapshot_id,
        adopted_stage_kinds,
    )
    .await?;
    let adoption_exact_set_hash = adoption_set_hash(
        source_operation_id,
        source_scope_snapshot_id,
        adopted_stage_kinds,
        source,
        target,
        &source_final_seal_hash,
    );
    Ok(OperationContractAdoptionWitness {
        source_final_seal_hash,
        adoption_exact_set_hash,
    })
}

fn adoption_receipt_hash(
    stable_request_id: Uuid,
    source_operation_id: Uuid,
    target_operation_id: Uuid,
    adoption_set_hash: &str,
) -> String {
    tagged_sha256_json(&json!({
        "schema": "operation-contract-adoption-receipt.v1",
        "stable_request_id": stable_request_id,
        "source_operation_id": source_operation_id,
        "target_operation_id": target_operation_id,
        "adoption_set_hash": adoption_set_hash,
    }))
}

pub async fn choose_stage_fork_pair_and_write_adoption(
    connection: &mut PgConnection,
    source_operation_id: Uuid,
    target_operation_id: Uuid,
    source_scope_snapshot_id: Uuid,
    adopted_stage_kinds: &[String],
    adoption: Option<&OperationContractForkAdoptionRow>,
) -> OperationRolloutResult<FrozenOperationJointContract> {
    let source = source_pair_for_share(&mut *connection, source_operation_id).await?;
    let Some(adoption) = adoption else {
        return Ok(source);
    };
    let request_id = Uuid::parse_str(&adoption.request_id).map_err(|_| {
        OperationRolloutError::IdentityMismatch {
            code: "STAGE_FORK_OPERATION_CONTRACT_ADOPTION_REQUEST_INVALID",
        }
    })?;
    if target_operation_id.is_nil() || source_operation_id == target_operation_id {
        return Err(OperationRolloutError::IdentityMismatch {
            code: "STAGE_FORK_OPERATION_CONTRACT_ADOPTION_IDENTITY_INVALID",
        });
    }
    let target_rank = validate_joint_pair(
        adoption.target_tool_truth_contract,
        adoption.target_investigation_contract_version,
        adoption.target_investigation_rollout_mode,
    )?;
    if target_rank != source.joint_rank + 1 {
        return Err(OperationRolloutError::Conflict {
            code: "STAGE_FORK_OPERATION_CONTRACT_ADOPTION_NOT_ADJACENT",
        });
    }
    let target = FrozenOperationJointContract {
        tool_truth_contract: adoption.target_tool_truth_contract,
        investigation_contract_version: adoption.target_investigation_contract_version,
        investigation_rollout_mode: adoption.target_investigation_rollout_mode,
        joint_rank: target_rank,
    };
    let source_final_seal_hash = source_final_seal_hash_for_share(
        &mut *connection,
        source_operation_id,
        source_scope_snapshot_id,
        adopted_stage_kinds,
    )
    .await?;
    if source_final_seal_hash != adoption.source_final_seal_hash {
        return Err(OperationRolloutError::IdentityMismatch {
            code: "STAGE_FORK_OPERATION_CONTRACT_SOURCE_SEAL_DRIFT",
        });
    }
    let expected_adoption_set_hash = adoption_set_hash(
        source_operation_id,
        source_scope_snapshot_id,
        adopted_stage_kinds,
        source,
        target,
        &source_final_seal_hash,
    );
    if expected_adoption_set_hash != adoption.adoption_exact_set_hash {
        return Err(OperationRolloutError::IdentityMismatch {
            code: "STAGE_FORK_OPERATION_CONTRACT_ADOPTION_SET_DRIFT",
        });
    }
    let receipt_hash = adoption_receipt_hash(
        request_id,
        source_operation_id,
        target_operation_id,
        &expected_adoption_set_hash,
    );
    let adoption_id = Uuid::new_v5(
        &target_operation_id,
        format!("operation-contract-adoption:v1:{request_id}").as_bytes(),
    );
    let inserted = sqlx::query_as::<_, OperationContractAdoptionReceiptRow>(
        r#"INSERT INTO operation_contract_adoptions(
               adoption_id,source_operation_id,target_operation_id,
               source_tool_truth_contract,source_investigation_contract_version,
               source_investigation_rollout_mode,source_joint_rank,
               target_tool_truth_contract,target_investigation_contract_version,
               target_investigation_rollout_mode,target_joint_rank,
               source_final_seal_hash,adoption_set_hash,stable_request_id,receipt_hash
           ) VALUES($1,$2,$3,$4,$5,$6,$7,$8,$9,$10,$11,$12,$13,$14,$15)
           ON CONFLICT(stable_request_id) DO NOTHING
           RETURNING adoption_id,source_operation_id,target_operation_id,
                     source_tool_truth_contract,source_investigation_contract_version,
                     source_investigation_rollout_mode,source_joint_rank,
                     target_tool_truth_contract,target_investigation_contract_version,
                     target_investigation_rollout_mode,target_joint_rank,
                     source_final_seal_hash,adoption_set_hash,stable_request_id,receipt_hash"#,
    )
    .bind(adoption_id)
    .bind(source_operation_id)
    .bind(target_operation_id)
    .bind(source.tool_truth_contract.as_str())
    .bind(source.investigation_contract_version.as_str())
    .bind(source.investigation_rollout_mode.as_str())
    .bind(source.joint_rank)
    .bind(target.tool_truth_contract.as_str())
    .bind(target.investigation_contract_version.as_str())
    .bind(target.investigation_rollout_mode.as_str())
    .bind(target.joint_rank)
    .bind(&source_final_seal_hash)
    .bind(&expected_adoption_set_hash)
    .bind(request_id)
    .bind(&receipt_hash)
    .fetch_optional(&mut *connection)
    .await?;
    if inserted.is_none() {
        let existing = sqlx::query_as::<_, OperationContractAdoptionReceiptRow>(
            r#"SELECT adoption_id,source_operation_id,target_operation_id,
                      source_tool_truth_contract,source_investigation_contract_version,
                      source_investigation_rollout_mode,source_joint_rank,
                      target_tool_truth_contract,target_investigation_contract_version,
                      target_investigation_rollout_mode,target_joint_rank,
                      source_final_seal_hash,adoption_set_hash,stable_request_id,receipt_hash
                 FROM operation_contract_adoptions
                WHERE stable_request_id=$1
                FOR SHARE"#,
        )
        .bind(request_id)
        .fetch_one(&mut *connection)
        .await?;
        let exact = existing.adoption_id == adoption_id
            && existing.source_operation_id == source_operation_id
            && existing.target_operation_id == target_operation_id
            && existing.source_tool_truth_contract == source.tool_truth_contract.as_str()
            && existing.source_investigation_contract_version
                == source.investigation_contract_version.as_str()
            && existing.source_investigation_rollout_mode
                == source.investigation_rollout_mode.as_str()
            && existing.source_joint_rank == source.joint_rank
            && existing.target_tool_truth_contract == target.tool_truth_contract.as_str()
            && existing.target_investigation_contract_version
                == target.investigation_contract_version.as_str()
            && existing.target_investigation_rollout_mode
                == target.investigation_rollout_mode.as_str()
            && existing.target_joint_rank == target.joint_rank
            && existing.source_final_seal_hash == source_final_seal_hash
            && existing.adoption_set_hash == expected_adoption_set_hash
            && existing.receipt_hash == receipt_hash;
        if !exact {
            return Err(OperationRolloutError::Conflict {
                code: "STAGE_FORK_OPERATION_CONTRACT_ADOPTION_REQUEST_DRIFT",
            });
        }
    }
    Ok(target)
}

pub fn map_to_db_error(error: OperationRolloutError) -> crate::DbError {
    crate::DbError::Other(anyhow::Error::new(error))
}
