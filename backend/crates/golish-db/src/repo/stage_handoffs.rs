//! Bounded, final-PASS StageHandoff schema contract.

use std::collections::BTreeSet;

use chrono::{DateTime, Utc};
use serde::{Deserialize, Serialize};
use serde_json::Value;
use sqlx::{Executor, PgPool, Postgres};
use uuid::Uuid;

use super::runtime_memory_tx::{RuntimeMemoryStoreError, RuntimeMemoryStoreResult};

pub const TABLE_NAME: &str = "stage_handoffs";
pub const SOURCE_UNIT_UNIQUE_SQL: &str = "UNIQUE(source_stage_run_unit_id)";
pub const DELIVERABLE_UNIQUE_SQL: &str = "UNIQUE(deliverable_submission_id)";
pub const EXECUTION_ORG_UNIQUE_SQL: &str = "UNIQUE(stage_execution_id, organization_id)";
pub const SCOPE_OPERATION_FK_SQL: &str = "FOREIGN KEY(scope_snapshot_id, operation_id) \
     REFERENCES operation_org_scope_snapshots(id, operation_id)";
pub const SCOPE_MEMBERSHIP_FK_SQL: &str = "FOREIGN KEY(scope_snapshot_id, organization_id) \
     REFERENCES operation_org_scope_units(snapshot_id, organization_id)";
pub const SOURCE_UNIT_OWNER_FK_SQL: &str =
    "FOREIGN KEY(source_stage_run_unit_id, operation_id, stage_execution_id, organization_id) \
     REFERENCES stage_run_units(id, operation_id, stage_execution_id, organization_id)";
pub const DELIVERABLE_OWNER_FK_SQL: &str =
    "FOREIGN KEY(deliverable_submission_id, operation_id, stage_execution_id) \
     REFERENCES stage_deliverable_submissions(id, operation_id, stage_execution_id)";

#[derive(Debug, Clone, sqlx::FromRow, Serialize, Deserialize, PartialEq, Eq)]
pub struct StageHandoffRow {
    pub id: Uuid,
    pub operation_id: Uuid,
    pub organization_id: Uuid,
    pub scope_snapshot_id: Uuid,
    pub from_stage_kind: String,
    pub stage_execution_id: Uuid,
    pub source_stage_run_unit_id: Uuid,
    pub deliverable_submission_id: Uuid,
    pub scope_hash: String,
    pub payload: Value,
    pub payload_sha256: String,
    pub evidence_ids: Vec<i64>,
    pub coverage_watermark: Value,
    pub unit_gate_decision_hash: String,
    pub aggregate_pass_token_hash: Option<String>,
    pub gate_passed_at: DateTime<Utc>,
    pub invalidated_at: Option<DateTime<Utc>>,
    pub schema_version: i32,
}

/// Server-authored final seal for Candidate V2 Verification. It deliberately
/// has no deliverable submission: exact terminal Candidate truth and the
/// durable WaveUnit are its authority.
#[derive(Debug, Clone, sqlx::FromRow, Serialize, Deserialize, PartialEq, Eq)]
pub struct VerificationStageHandoffRow {
    pub id: Uuid,
    pub operation_id: Uuid,
    pub scope_snapshot_id: Uuid,
    pub wave_run_id: Uuid,
    pub wave_unit_id: Uuid,
    pub organization_id: Uuid,
    pub stage_execution_id: Uuid,
    pub source_stage_run_unit_id: Uuid,
    pub primary_worker_run_id: Uuid,
    pub wave_generation: i32,
    pub wave_unit_row_version_after_close: i64,
    pub from_stage_kind: String,
    pub authority_kind: String,
    pub payload: Value,
    pub payload_sha256: String,
    pub evidence_ids: Vec<i64>,
    pub coverage_watermark: Value,
    pub verification_truth_hash: String,
    pub gate_passed_at: DateTime<Utc>,
    pub schema_version: i32,
}

#[derive(Debug, Clone)]
pub(crate) struct NewVerificationStageHandoffRow {
    pub id: Uuid,
    pub operation_id: Uuid,
    pub scope_snapshot_id: Uuid,
    pub wave_run_id: Uuid,
    pub wave_unit_id: Uuid,
    pub organization_id: Uuid,
    pub stage_execution_id: Uuid,
    pub source_stage_run_unit_id: Uuid,
    pub primary_worker_run_id: Uuid,
    pub wave_generation: i32,
    pub wave_unit_row_version_after_close: i64,
    pub payload: Value,
    pub payload_sha256: String,
    pub evidence_ids: Vec<i64>,
    pub coverage_watermark: Value,
    pub verification_truth_hash: String,
}

fn is_sha256_hex(value: &str) -> bool {
    value.len() == 64
        && value
            .bytes()
            .all(|byte| byte.is_ascii_digit() || (b'a'..=b'f').contains(&byte))
}

fn json_evidence_ids(value: Option<&Value>) -> Option<Vec<i64>> {
    value?
        .as_array()?
        .iter()
        .map(Value::as_i64)
        .collect::<Option<Vec<_>>>()
}

fn evidence_ids_are_canonical(evidence_ids: &[i64]) -> bool {
    evidence_ids.iter().all(|evidence_id| *evidence_id > 0)
        && evidence_ids.windows(2).all(|pair| pair[0] < pair[1])
}

fn verification_claim_order_key(claim: &Value) -> Option<(u8, Uuid, u8)> {
    let kind = claim.get("kind")?.as_str()?;
    let (group, identity_pointer, kind_order) = match kind {
        "candidate_attempt_terminal" => (0, "/payload/candidate_id", 0),
        "verified_candidate_attempt" => (0, "/payload/candidate_id", 1),
        "attack_no_candidate_decision" => (1, "/payload/work_item_id", 0),
        "attack_fact_delta_proposal" => (2, "/payload/fact_delta_id", 0),
        _ => return None,
    };
    let identity = Uuid::parse_str(claim.pointer(identity_pointer)?.as_str()?).ok()?;
    Some((group, identity, kind_order))
}

fn verification_handoff_input_is_valid(input: &NewVerificationStageHandoffRow) -> bool {
    let evidence_ids = &input.evidence_ids;
    if !evidence_ids_are_canonical(evidence_ids)
        || input.id != Uuid::new_v5(&input.wave_unit_id, b"verification-stage-handoff:v1")
        || input.wave_generation < 0
        || input.wave_unit_row_version_after_close <= 0
        || !is_sha256_hex(&input.payload_sha256)
        || !is_sha256_hex(&input.verification_truth_hash)
        || super::operation_scope_decisions::sha256_json(&input.payload) != input.payload_sha256
        || input.payload.get("schema_version").and_then(Value::as_i64) != Some(1)
        || input
            .payload
            .get("verification_truth_hash")
            .and_then(Value::as_str)
            != Some(input.verification_truth_hash.as_str())
        || input.payload.get("coverage_watermark") != Some(&input.coverage_watermark)
        || json_evidence_ids(input.payload.get("evidence_ids")) != Some(evidence_ids.clone())
    {
        return false;
    }
    let Some(typed_claims) = input.payload.get("typed_claims").and_then(Value::as_array) else {
        return false;
    };
    let Some(canonical_refs) = input
        .payload
        .get("canonical_fact_refs")
        .and_then(Value::as_array)
    else {
        return false;
    };
    let Some(claim_order) = typed_claims
        .iter()
        .map(verification_claim_order_key)
        .collect::<Option<Vec<_>>>()
    else {
        return false;
    };
    if !claim_order.windows(2).all(|pair| pair[0] < pair[1]) {
        return false;
    }
    let mut projected_evidence_ids = std::collections::BTreeSet::new();
    for claim in typed_claims {
        let Some(kind) = claim.get("kind").and_then(Value::as_str) else {
            return false;
        };
        if !matches!(
            kind,
            "candidate_attempt_terminal"
                | "verified_candidate_attempt"
                | "attack_fact_delta_proposal"
                | "attack_no_candidate_decision"
        ) {
            return false;
        }
        let Some(claim_evidence_ids) = json_evidence_ids(claim.pointer("/payload/evidence_ids"))
        else {
            return false;
        };
        if !evidence_ids_are_canonical(&claim_evidence_ids) {
            return false;
        }
        projected_evidence_ids.extend(claim_evidence_ids);
    }
    let mut previous_finding_id = None;
    for canonical_ref in canonical_refs {
        if canonical_ref.pointer("/key/kind").and_then(Value::as_str) != Some("finding")
            || canonical_ref.get("source_table").and_then(Value::as_str) != Some("findings")
            || !canonical_ref
                .get("content_sha256")
                .and_then(Value::as_str)
                .is_some_and(is_sha256_hex)
        {
            return false;
        }
        let Some(finding_id) = canonical_ref
            .pointer("/key/finding_id")
            .and_then(Value::as_str)
            .and_then(|value| Uuid::parse_str(value).ok())
        else {
            return false;
        };
        if previous_finding_id.is_some_and(|previous| previous >= finding_id) {
            return false;
        }
        previous_finding_id = Some(finding_id);
        let Some(ref_evidence_ids) = json_evidence_ids(canonical_ref.get("evidence_ids")) else {
            return false;
        };
        if !evidence_ids_are_canonical(&ref_evidence_ids) {
            return false;
        }
        projected_evidence_ids.extend(ref_evidence_ids);
    }
    projected_evidence_ids.into_iter().collect::<Vec<_>>() == *evidence_ids
}

/// Common downstream projection for ordinary deliverable final seals and the
/// server-authored Verification Wave close exception.
#[derive(Debug, Clone, sqlx::FromRow, Serialize, Deserialize, PartialEq, Eq)]
pub struct FinalSealedStageHandoffRow {
    pub id: Uuid,
    pub operation_id: Uuid,
    pub organization_id: Uuid,
    pub scope_snapshot_id: Uuid,
    pub from_stage_kind: String,
    pub stage_execution_id: Uuid,
    pub source_stage_run_unit_id: Uuid,
    pub deliverable_submission_id: Option<Uuid>,
    pub authority_kind: String,
    pub scope_hash: String,
    pub payload: Value,
    pub payload_sha256: String,
    pub evidence_ids: Vec<i64>,
    pub coverage_watermark: Value,
    pub unit_gate_decision_hash: String,
    pub aggregate_pass_token_hash: Option<String>,
    pub gate_passed_at: DateTime<Utc>,
    pub schema_version: i32,
}

const COLUMNS: &str = r#"id, operation_id, organization_id, scope_snapshot_id,
    from_stage_kind, stage_execution_id, source_stage_run_unit_id,
    deliverable_submission_id, scope_hash, payload, payload_sha256,
    evidence_ids, coverage_watermark, unit_gate_decision_hash,
    aggregate_pass_token_hash, gate_passed_at, invalidated_at, schema_version"#;

#[derive(Debug, Clone)]
pub(crate) struct NewStageHandoffRow {
    pub id: Uuid,
    pub operation_id: Uuid,
    pub organization_id: Uuid,
    pub scope_snapshot_id: Uuid,
    pub from_stage_kind: String,
    pub stage_execution_id: Uuid,
    pub source_stage_run_unit_id: Uuid,
    pub deliverable_submission_id: Uuid,
    pub scope_hash: String,
    pub payload: Value,
    pub payload_sha256: String,
    pub evidence_ids: Vec<i64>,
    pub coverage_watermark: Value,
    pub unit_gate_decision_hash: String,
    pub aggregate_pass_token_hash: Option<String>,
    pub schema_version: i32,
}

pub(crate) async fn insert_with_executor<'e, E>(
    executor: E,
    input: &NewStageHandoffRow,
) -> RuntimeMemoryStoreResult<StageHandoffRow>
where
    E: Executor<'e, Database = Postgres>,
{
    if input.schema_version <= 0 {
        return Err(RuntimeMemoryStoreError::Conflict {
            code: "invalid_stage_handoff_schema_version",
        });
    }
    let sql = format!(
        r#"INSERT INTO stage_handoffs (
               id, operation_id, organization_id, scope_snapshot_id,
               from_stage_kind, stage_execution_id, source_stage_run_unit_id,
               deliverable_submission_id, scope_hash, payload, payload_sha256,
               evidence_ids, coverage_watermark, unit_gate_decision_hash,
               aggregate_pass_token_hash, gate_passed_at, schema_version
           ) VALUES ($1,$2,$3,$4,$5,$6,$7,$8,$9,$10,$11,$12,$13,$14,$15,NOW(),$16)
           RETURNING {COLUMNS}"#
    );
    Ok(sqlx::query_as::<_, StageHandoffRow>(&sql)
        .bind(input.id)
        .bind(input.operation_id)
        .bind(input.organization_id)
        .bind(input.scope_snapshot_id)
        .bind(&input.from_stage_kind)
        .bind(input.stage_execution_id)
        .bind(input.source_stage_run_unit_id)
        .bind(input.deliverable_submission_id)
        .bind(&input.scope_hash)
        .bind(&input.payload)
        .bind(&input.payload_sha256)
        .bind(&input.evidence_ids)
        .bind(&input.coverage_watermark)
        .bind(&input.unit_gate_decision_hash)
        .bind(&input.aggregate_pass_token_hash)
        .bind(input.schema_version)
        .fetch_one(executor)
        .await?)
}

pub(crate) async fn insert_verification_with_executor<'e, E>(
    executor: E,
    input: &NewVerificationStageHandoffRow,
) -> RuntimeMemoryStoreResult<VerificationStageHandoffRow>
where
    E: Executor<'e, Database = Postgres>,
{
    if !verification_handoff_input_is_valid(input) {
        return Err(RuntimeMemoryStoreError::Conflict {
            code: "invalid_verification_stage_handoff",
        });
    }
    sqlx::query_as::<_, VerificationStageHandoffRow>(
        r#"INSERT INTO verification_stage_handoffs(
               id,operation_id,scope_snapshot_id,wave_run_id,wave_unit_id,
               organization_id,stage_execution_id,source_stage_run_unit_id,
               primary_worker_run_id,wave_generation,
               wave_unit_row_version_after_close,payload,payload_sha256,
               evidence_ids,coverage_watermark,verification_truth_hash
           ) VALUES($1,$2,$3,$4,$5,$6,$7,$8,$9,$10,$11,$12,$13,$14,$15,$16)
           RETURNING *"#,
    )
    .bind(input.id)
    .bind(input.operation_id)
    .bind(input.scope_snapshot_id)
    .bind(input.wave_run_id)
    .bind(input.wave_unit_id)
    .bind(input.organization_id)
    .bind(input.stage_execution_id)
    .bind(input.source_stage_run_unit_id)
    .bind(input.primary_worker_run_id)
    .bind(input.wave_generation)
    .bind(input.wave_unit_row_version_after_close)
    .bind(&input.payload)
    .bind(&input.payload_sha256)
    .bind(&input.evidence_ids)
    .bind(&input.coverage_watermark)
    .bind(&input.verification_truth_hash)
    .fetch_one(executor)
    .await
    .map_err(Into::into)
}

pub(crate) async fn get_verification_with_executor<'e, E>(
    executor: E,
    source_stage_run_unit_id: Uuid,
) -> RuntimeMemoryStoreResult<Option<VerificationStageHandoffRow>>
where
    E: Executor<'e, Database = Postgres>,
{
    sqlx::query_as::<_, VerificationStageHandoffRow>(
        "SELECT * FROM verification_stage_handoffs
         WHERE source_stage_run_unit_id=$1 FOR SHARE",
    )
    .bind(source_stage_run_unit_id)
    .fetch_optional(executor)
    .await
    .map_err(Into::into)
}

/// Read one newest immutable, non-invalidated final seal per inherited source
/// stage. The join back to passed Unit and Worker rows prevents a detached or
/// partially written projection from becoming downstream evidence.
pub async fn list_latest_final_sealed_for_sources(
    pool: &PgPool,
    operation_id: Uuid,
    organization_id: Uuid,
    source_stage_kinds: &[String],
) -> RuntimeMemoryStoreResult<Vec<FinalSealedStageHandoffRow>> {
    if source_stage_kinds.is_empty() {
        return Ok(Vec::new());
    }
    let sql = r#"WITH final_seals AS (
                    SELECT handoff.id,handoff.operation_id,handoff.organization_id,
                           handoff.scope_snapshot_id,handoff.from_stage_kind,
                           handoff.stage_execution_id,handoff.source_stage_run_unit_id,
                           handoff.deliverable_submission_id,
                           'deliverable_final_seal'::TEXT AS authority_kind,
                           handoff.scope_hash,handoff.payload,handoff.payload_sha256,
                           handoff.evidence_ids,handoff.coverage_watermark,
                           handoff.unit_gate_decision_hash,
                           handoff.aggregate_pass_token_hash,handoff.gate_passed_at,
                           handoff.schema_version
                      FROM stage_handoffs AS handoff
                      JOIN stage_run_units AS unit
                        ON unit.id=handoff.source_stage_run_unit_id
                       AND unit.operation_id=handoff.operation_id
                       AND unit.stage_execution_id=handoff.stage_execution_id
                       AND unit.organization_id=handoff.organization_id
                       AND unit.status='passed'
                      JOIN stage_deliverable_submissions AS submission
                        ON submission.id=handoff.deliverable_submission_id
                       AND submission.stage_run_unit_id=unit.id
                      LEFT JOIN stage_worker_runs AS worker
                        ON worker.id=submission.worker_run_id
                       AND worker.operation_id=handoff.operation_id
                       AND worker.stage_execution_id=handoff.stage_execution_id
                       AND worker.stage_run_unit_id=unit.id
                       AND worker.organization_id=handoff.organization_id
                     WHERE handoff.operation_id=$1 AND handoff.organization_id=$2
                       AND handoff.from_stage_kind=ANY($3)
                       AND handoff.invalidated_at IS NULL
                       AND (
                             (submission.worker_run_id IS NULL
                              AND handoff.from_stage_kind='scoping')
                             OR worker.status='passed'
                           )
                    UNION ALL
                    SELECT handoff.id,handoff.operation_id,handoff.organization_id,
                           handoff.scope_snapshot_id,handoff.from_stage_kind,
                           handoff.stage_execution_id,handoff.source_stage_run_unit_id,
                           NULL::UUID AS deliverable_submission_id,handoff.authority_kind,
                           snapshot.scope_hash,handoff.payload,handoff.payload_sha256,
                           handoff.evidence_ids,handoff.coverage_watermark,
                           handoff.verification_truth_hash AS unit_gate_decision_hash,
                           NULL::TEXT AS aggregate_pass_token_hash,handoff.gate_passed_at,
                           handoff.schema_version
                      FROM verification_stage_handoffs AS handoff
                      JOIN operation_org_scope_snapshots AS snapshot
                        ON snapshot.id=handoff.scope_snapshot_id
                       AND snapshot.operation_id=handoff.operation_id
                       AND snapshot.sealed_at IS NOT NULL
                      JOIN stage_run_units AS unit
                        ON unit.id=handoff.source_stage_run_unit_id
                       AND unit.operation_id=handoff.operation_id
                       AND unit.stage_execution_id=handoff.stage_execution_id
                       AND unit.organization_id=handoff.organization_id
                       AND unit.stage_kind='verification'
                       AND unit.status='passed'
                       AND unit.terminal_at IS NOT NULL
                      JOIN stage_worker_runs AS worker
                        ON worker.id=handoff.primary_worker_run_id
                       AND worker.operation_id=handoff.operation_id
                       AND worker.stage_execution_id=handoff.stage_execution_id
                       AND worker.stage_run_unit_id=unit.id
                       AND worker.organization_id=handoff.organization_id
                       AND worker.status='passed'
                       AND worker.terminal_at IS NOT NULL
                     WHERE handoff.operation_id=$1 AND handoff.organization_id=$2
                       AND handoff.from_stage_kind=ANY($3)
                )
                SELECT DISTINCT ON (from_stage_kind) *
                  FROM final_seals
                 ORDER BY from_stage_kind,gate_passed_at DESC,id DESC"#;
    let mut rows = sqlx::query_as::<_, FinalSealedStageHandoffRow>(sql)
        .bind(operation_id)
        .bind(organization_id)
        .bind(source_stage_kinds)
        .fetch_all(pool)
        .await?;
    let current_stages = rows
        .iter()
        .map(|row| row.from_stage_kind.as_str())
        .collect::<BTreeSet<_>>();
    let missing = source_stage_kinds
        .iter()
        .filter(|stage| !current_stages.contains(stage.as_str()))
        .cloned()
        .collect::<Vec<_>>();
    if !missing.is_empty() {
        let fork_sql = r#"SELECT COALESCE(input.source_handoff_id,input.id) AS id,
                   input.operation_id,input.organization_id,
                   input.target_scope_snapshot_id AS scope_snapshot_id,
                   input.source_stage_kind AS from_stage_kind,
                   input.source_stage_execution_id AS stage_execution_id,
                   input.source_stage_run_unit_id,
                   input.source_deliverable_submission_id AS deliverable_submission_id,
                   'stage_fork_final_seal'::TEXT AS authority_kind,
                   input.source_scope_hash AS scope_hash,
                   input.source_payload AS payload,
                   input.source_payload_sha256 AS payload_sha256,
                   input.source_evidence_ids AS evidence_ids,
                   input.source_coverage_watermark AS coverage_watermark,
                   input.source_unit_gate_decision_hash AS unit_gate_decision_hash,
                   input.source_aggregate_pass_token_hash AS aggregate_pass_token_hash,
                   input.source_gate_passed_at AS gate_passed_at,
                   input.schema_version
              FROM operation_stage_fork_inputs AS input
              JOIN operation_stage_forks AS fork
                ON fork.operation_id=input.operation_id
               AND fork.source_operation_id=input.source_operation_id
              JOIN operation_state AS source_operation
                ON source_operation.operation_id=input.source_operation_id
               AND source_operation.superseded_by IS NULL
         LEFT JOIN stage_handoffs AS handoff
                ON handoff.id=input.source_handoff_id
               AND handoff.operation_id=input.source_operation_id
               AND handoff.scope_snapshot_id=input.source_scope_snapshot_id
               AND handoff.organization_id=input.organization_id
               AND handoff.from_stage_kind=input.source_stage_kind
               AND handoff.stage_execution_id=input.source_stage_execution_id
               AND handoff.source_stage_run_unit_id=input.source_stage_run_unit_id
               AND handoff.deliverable_submission_id=input.source_deliverable_submission_id
               AND handoff.scope_hash=input.source_scope_hash
               AND handoff.payload=input.source_payload
               AND handoff.payload_sha256=input.source_payload_sha256
               AND handoff.evidence_ids=input.source_evidence_ids
               AND handoff.coverage_watermark=input.source_coverage_watermark
               AND handoff.unit_gate_decision_hash=input.source_unit_gate_decision_hash
               AND handoff.aggregate_pass_token_hash IS NOT DISTINCT FROM input.source_aggregate_pass_token_hash
               AND handoff.gate_passed_at=input.source_gate_passed_at
               AND handoff.invalidated_at IS NULL
         LEFT JOIN operation_org_scope_snapshots AS source_scope
                ON source_scope.id=input.source_scope_snapshot_id
               AND source_scope.operation_id=input.source_operation_id
               AND source_scope.root_organization_id=input.organization_id
               AND source_scope.scope_hash=input.source_scope_hash
               AND source_scope.sealed_at=input.source_gate_passed_at
             WHERE input.operation_id=$1
               AND input.organization_id=$2
               AND input.source_stage_kind=ANY($3)
               AND (
                    (
                        input.source_stage_kind='scoping'
                        AND input.source_handoff_id IS NULL
                        AND source_scope.id IS NOT NULL
                    )
                    OR (
                        input.source_stage_kind<>'scoping'
                        AND handoff.id IS NOT NULL
                    )
               )
             ORDER BY operation_stage_fork_stage_rank(input.source_stage_kind)"#;
        let inherited = sqlx::query_as::<_, FinalSealedStageHandoffRow>(fork_sql)
            .bind(operation_id)
            .bind(organization_id)
            .bind(&missing)
            .fetch_all(pool)
            .await?;
        rows.extend(inherited);
    }
    rows.sort_by(|left, right| left.from_stage_kind.cmp(&right.from_stage_kind));
    Ok(rows)
}

/// Canonical exact-origin surface inherited from the immutable, final-sealed
/// Enumeration handoff for one operation/org. Downstream active scanners and
/// their coverage denominator must both use this helper; a raw URL-shaped
/// target row is not proof that Enumeration authorized that origin.
pub async fn list_final_sealed_enumeration_origins(
    pool: &PgPool,
    operation_id: Uuid,
    organization_id: Uuid,
) -> anyhow::Result<BTreeSet<String>> {
    let handoffs = list_latest_final_sealed_for_sources(
        pool,
        operation_id,
        organization_id,
        &["enumeration".to_string()],
    )
    .await
    .map_err(|error| anyhow::anyhow!(error.to_string()))?;
    enumeration_origins_from_optional_final_sealed_watermark(
        handoffs.first().map(|handoff| &handoff.coverage_watermark),
        organization_id,
    )
}

fn enumeration_origins_from_optional_final_sealed_watermark(
    watermark: Option<&Value>,
    organization_id: Uuid,
) -> anyhow::Result<BTreeSet<String>> {
    let watermark = watermark.ok_or_else(|| {
        anyhow::anyhow!(
            "missing final-sealed Enumeration handoff for operation organization {organization_id}"
        )
    })?;
    enumeration_origins_from_final_sealed_watermark(watermark, organization_id)
}

fn enumeration_origins_from_final_sealed_watermark(
    watermark: &Value,
    organization_id: Uuid,
) -> anyhow::Result<BTreeSet<String>> {
    anyhow::ensure!(
        watermark.get("kind").and_then(Value::as_str) == Some("information_coverage_v1")
            && watermark.get("stage").and_then(Value::as_str) == Some("enumeration")
            && watermark
                .get("organization_id")
                .and_then(Value::as_str)
                .and_then(|value| Uuid::parse_str(value).ok())
                == Some(organization_id),
        "final-sealed Enumeration handoff has an invalid coverage watermark"
    );
    watermark
        .get("assets")
        .and_then(Value::as_array)
        .ok_or_else(|| anyhow::anyhow!("final-sealed Enumeration handoff has no asset axis"))?
        .iter()
        .map(|asset| {
            asset
                .as_str()
                .and_then(golish_pentest_domain::canonical_web_origin)
                .map(|origin| origin.key)
                .ok_or_else(|| {
                    anyhow::anyhow!("final-sealed Enumeration handoff contains a non-origin asset")
                })
        })
        .collect()
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum StageHandoffStatus {
    Published,
    Invalidated,
}

impl StageHandoffRow {
    pub fn status(&self) -> StageHandoffStatus {
        if self.invalidated_at.is_some() {
            StageHandoffStatus::Invalidated
        } else {
            StageHandoffStatus::Published
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn runtime_memory_repo_contract_handoff_is_pass_sealed_and_org_scoped() {
        assert_eq!(TABLE_NAME, "stage_handoffs");
        assert!(SOURCE_UNIT_UNIQUE_SQL.contains("source_stage_run_unit_id"));
        assert!(DELIVERABLE_UNIQUE_SQL.contains("deliverable_submission_id"));
        assert!(SCOPE_MEMBERSHIP_FK_SQL.contains("scope_snapshot_id, organization_id"));
        assert!(SOURCE_UNIT_OWNER_FK_SQL.contains(
            "source_stage_run_unit_id, operation_id, stage_execution_id, organization_id"
        ));
        assert!(DELIVERABLE_OWNER_FK_SQL.contains("stage_deliverable_submissions"));
    }

    #[test]
    fn final_sealed_enumeration_watermark_yields_only_canonical_origins() {
        let organization_id = Uuid::new_v4();
        let watermark = serde_json::json!({
            "kind": "information_coverage_v1",
            "stage": "enumeration",
            "organization_id": organization_id,
            "assets": [
                "HTTPS://App.Example.com/login",
                "https://app.example.com:443/other",
                "http://app.example.com"
            ]
        });

        assert_eq!(
            enumeration_origins_from_final_sealed_watermark(&watermark, organization_id).unwrap(),
            BTreeSet::from([
                "http://app.example.com:80".to_string(),
                "https://app.example.com:443".to_string(),
            ])
        );
    }

    #[test]
    fn missing_final_sealed_enumeration_handoff_is_not_an_empty_surface() {
        let error = enumeration_origins_from_optional_final_sealed_watermark(None, Uuid::new_v4())
            .expect_err("a missing handoff must fail closed");

        assert!(error
            .to_string()
            .contains("missing final-sealed Enumeration handoff"));
    }

    #[test]
    fn final_sealed_enumeration_handoff_may_authorize_an_explicit_empty_surface() {
        let organization_id = Uuid::new_v4();
        let watermark = serde_json::json!({
            "kind": "information_coverage_v1",
            "stage": "enumeration",
            "organization_id": organization_id,
            "assets": []
        });

        assert!(enumeration_origins_from_optional_final_sealed_watermark(
            Some(&watermark),
            organization_id,
        )
        .expect("an explicit sealed empty axis is valid")
        .is_empty());
    }

    #[test]
    fn final_sealed_enumeration_watermark_rejects_foreign_or_non_origin_assets() {
        let organization_id = Uuid::new_v4();
        let foreign = serde_json::json!({
            "kind": "information_coverage_v1",
            "stage": "enumeration",
            "organization_id": Uuid::new_v4(),
            "assets": ["https://app.example.com:443"]
        });
        assert!(
            enumeration_origins_from_final_sealed_watermark(&foreign, organization_id).is_err()
        );

        let malformed = serde_json::json!({
            "kind": "information_coverage_v1",
            "stage": "enumeration",
            "organization_id": organization_id,
            "assets": ["app.example.com"]
        });
        assert!(
            enumeration_origins_from_final_sealed_watermark(&malformed, organization_id).is_err()
        );
    }
}
