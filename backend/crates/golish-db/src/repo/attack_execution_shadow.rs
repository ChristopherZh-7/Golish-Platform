//! Dedicated whole-record shadow reads for the Candidate execution rollout.
//!
//! The legacy semantic mirror lives in its own additive table. In particular,
//! a runtime-memory `v2_only` operation never writes `operation_state.state_blob`.
//! V2 semantics are rebuilt from authoritative Candidate rows for every read.

use serde::{Deserialize, Serialize};
use sha2::{Digest, Sha256};
use sqlx::{PgConnection, PgPool};
use uuid::Uuid;

use super::attack_candidates::AcceptCandidateBatch;

#[derive(Debug, Clone, Serialize, Deserialize, PartialEq, Eq)]
pub struct AttackShadowDecisionRow {
    pub work_item_key: String,
    pub kind: String,
    pub semantic_hash: String,
}

#[derive(Debug, Clone, Copy, Serialize, Deserialize, PartialEq, Eq)]
pub struct AttackShadowReviewCountsRow {
    pub wave_unit_count: u32,
    pub review_closed_unit_count: u32,
    pub candidate_decision_count: u32,
    pub no_candidate_decision_count: u32,
}

#[derive(Debug, Clone, Serialize, Deserialize, PartialEq, Eq)]
pub struct AttackShadowCompleteReadRow {
    pub decisions: Vec<AttackShadowDecisionRow>,
    pub review_counts: AttackShadowReviewCountsRow,
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub enum AttackShadowV2ReadRow {
    Complete(AttackShadowCompleteReadRow),
    Missing,
    Incomplete,
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct AttackExecutionShadowSampleRow {
    pub operation_id: Uuid,
    pub stage_run_unit_id: Uuid,
    pub organization_id: Option<Uuid>,
    pub contract: String,
    pub legacy_record: Option<AttackShadowCompleteReadRow>,
    pub v2_record: AttackShadowV2ReadRow,
    pub comparison: Option<String>,
    pub selected_source: Option<String>,
    pub selected_record_hash: Option<String>,
}

#[derive(Debug, sqlx::FromRow)]
struct StoredShadowRow {
    stage_run_unit_id: Uuid,
    operation_id: Uuid,
    organization_id: Uuid,
    attack_execution_contract: String,
    legacy_record: serde_json::Value,
    legacy_record_hash: String,
    comparison: Option<String>,
    selected_source: Option<String>,
    selected_record_hash: Option<String>,
}

#[derive(Debug, sqlx::FromRow)]
struct PersistedDecisionRow {
    work_item_id: Uuid,
    work_item_key: String,
    decision_kind: Option<String>,
    candidate_id: Option<Uuid>,
    no_candidate_reason_code: Option<String>,
    no_candidate_detail: Option<String>,
    hypothesis: Option<String>,
    technique: Option<String>,
    rationale: Option<String>,
    prior_refs: Option<serde_json::Value>,
    suggested_approach: Option<String>,
    priority: Option<String>,
    execution_plan: Option<serde_json::Value>,
    candidate_plan_hash: Option<String>,
    risk_class: Option<String>,
    candidate_evidence_ids: Vec<i64>,
    decision_evidence_ids: Vec<i64>,
}

fn shadow_error(message: impl Into<String>) -> crate::DbError {
    crate::DbError::Other(anyhow::anyhow!(message.into()))
}

fn canonicalize(value: &serde_json::Value) -> serde_json::Value {
    match value {
        serde_json::Value::Object(map) => serde_json::Value::Object(
            map.iter()
                .map(|(key, value)| (key.clone(), canonicalize(value)))
                .collect::<std::collections::BTreeMap<_, _>>()
                .into_iter()
                .collect(),
        ),
        serde_json::Value::Array(values) => {
            serde_json::Value::Array(values.iter().map(canonicalize).collect())
        }
        _ => value.clone(),
    }
}

fn sha256_json(value: &serde_json::Value) -> crate::Result<String> {
    let bytes = serde_json::to_vec(&canonicalize(value))?;
    Ok(Sha256::digest(bytes)
        .iter()
        .map(|byte| format!("{byte:02x}"))
        .collect())
}

fn complete_record_hash(record: &AttackShadowCompleteReadRow) -> crate::Result<String> {
    sha256_json(&serde_json::to_value(record)?)
}

#[derive(Debug)]
struct ExpectedShadowSelection {
    comparison: &'static str,
    selected_source: &'static str,
    selected_record_hash: String,
}

fn expected_shadow_selection(
    contract: &str,
    legacy_record: &AttackShadowCompleteReadRow,
    v2_record: &AttackShadowV2ReadRow,
) -> crate::Result<ExpectedShadowSelection> {
    let comparison = match v2_record {
        AttackShadowV2ReadRow::Complete(v2) if v2 == legacy_record => "match",
        AttackShadowV2ReadRow::Complete(_) => "mismatch",
        AttackShadowV2ReadRow::Missing | AttackShadowV2ReadRow::Incomplete => "v2_missing",
    };
    let selected_source = match contract {
        "dual_write_read_legacy" => "legacy",
        "dual_write_read_v2_fallback" => match v2_record {
            AttackShadowV2ReadRow::Complete(_) => "v2",
            AttackShadowV2ReadRow::Missing | AttackShadowV2ReadRow::Incomplete => "legacy_fallback",
        },
        _ => return Err(shadow_error("attack shadow sample contract is invalid")),
    };
    let selected_record = match selected_source {
        "legacy" | "legacy_fallback" => legacy_record,
        "v2" => match v2_record {
            AttackShadowV2ReadRow::Complete(record) => record,
            AttackShadowV2ReadRow::Missing | AttackShadowV2ReadRow::Incomplete => {
                return Err(shadow_error("selected V2 attack record is incomplete"));
            }
        },
        _ => unreachable!(),
    };
    Ok(ExpectedShadowSelection {
        comparison,
        selected_source,
        selected_record_hash: complete_record_hash(selected_record)?,
    })
}

fn normalize_record(
    mut decisions: Vec<AttackShadowDecisionRow>,
    review_counts: AttackShadowReviewCountsRow,
) -> crate::Result<AttackShadowCompleteReadRow> {
    decisions.sort_by(|left, right| {
        left.work_item_key
            .cmp(&right.work_item_key)
            .then_with(|| left.kind.cmp(&right.kind))
            .then_with(|| left.semantic_hash.cmp(&right.semantic_hash))
    });
    if decisions
        .windows(2)
        .any(|pair| pair[0].work_item_key == pair[1].work_item_key)
    {
        return Err(shadow_error("attack shadow decision key is duplicated"));
    }
    let candidate_count = decisions
        .iter()
        .filter(|decision| decision.kind == "candidate")
        .count();
    let no_candidate_count = decisions.len().saturating_sub(candidate_count);
    if usize::try_from(review_counts.candidate_decision_count).ok() != Some(candidate_count)
        || usize::try_from(review_counts.no_candidate_decision_count).ok()
            != Some(no_candidate_count)
        || review_counts.review_closed_unit_count > review_counts.wave_unit_count
    {
        return Err(shadow_error("attack shadow review counts are incomplete"));
    }
    Ok(AttackShadowCompleteReadRow {
        decisions,
        review_counts,
    })
}

fn sorted_evidence(values: &[i64]) -> Vec<i64> {
    let mut values = values.to_vec();
    values.sort_unstable();
    values
}

fn candidate_payload_from_command(
    draft: &super::attack_candidates::AcceptedCandidateDraft,
) -> serde_json::Value {
    serde_json::json!({
        "work_item_id": draft.work_item_id,
        "candidate_id": draft.candidate_id,
        "hypothesis": draft.hypothesis,
        "technique": draft.technique,
        "rationale": draft.rationale,
        "prior_refs": draft.prior_refs,
        "suggested_approach": draft.suggested_approach,
        "priority": draft.priority,
        "execution_plan": draft.execution_plan,
        "candidate_plan_hash": draft.candidate_plan_hash,
        "risk_class": draft.risk_class,
        "evidence_ids": sorted_evidence(&draft.evidence_ids),
    })
}

fn no_candidate_payload_from_command(
    decision: &super::attack_candidates::NoCandidateDecision,
) -> serde_json::Value {
    serde_json::json!({
        "work_item_id": decision.work_item_id,
        "reason_code": decision.reason_code,
        "detail": decision.detail,
        "evidence_ids": sorted_evidence(&decision.evidence_ids),
    })
}

async fn load_persisted_decisions(
    connection: &mut PgConnection,
    operation_id: Uuid,
    stage_run_unit_id: Uuid,
) -> crate::Result<(Option<(Uuid, Uuid, i32)>, Vec<PersistedDecisionRow>)> {
    let authority: Option<(Uuid, Uuid, i32)> = sqlx::query_as(
        r#"SELECT wave_unit.id,wave_unit.organization_id,
                  COALESCE(wave_unit.manifest_count,0)
             FROM stage_run_units stage_unit
             JOIN attack_wave_runs wave_run
               ON wave_run.operation_id=stage_unit.operation_id
              AND wave_run.scope_snapshot_id=stage_unit.scope_snapshot_id
              AND wave_run.generation=stage_unit.generation
             JOIN attack_wave_units wave_unit
               ON wave_unit.wave_run_id=wave_run.id
              AND wave_unit.operation_id=stage_unit.operation_id
              AND wave_unit.scope_snapshot_id=stage_unit.scope_snapshot_id
              AND wave_unit.organization_id=stage_unit.organization_id
             JOIN stage_handoffs handoff
               ON handoff.source_stage_run_unit_id=stage_unit.id
              AND handoff.operation_id=stage_unit.operation_id
              AND handoff.stage_execution_id=stage_unit.stage_execution_id
              AND handoff.organization_id=stage_unit.organization_id
              AND handoff.from_stage_kind=stage_unit.stage_kind
              AND handoff.scope_snapshot_id=stage_unit.scope_snapshot_id
              AND handoff.invalidated_at IS NULL
             JOIN stage_deliverable_submissions submission
               ON submission.id=handoff.deliverable_submission_id
              AND submission.operation_id=stage_unit.operation_id
              AND submission.stage_execution_id=stage_unit.stage_execution_id
              AND submission.stage_run_unit_id=stage_unit.id
              AND submission.organization_id=stage_unit.organization_id
              AND submission.stage_kind=stage_unit.stage_kind
            WHERE stage_unit.id=$2 AND stage_unit.operation_id=$1
              AND stage_unit.stage_kind='attack_candidate'
              AND stage_unit.status='passed' AND stage_unit.terminal_at IS NOT NULL
              AND wave_unit.manifest_hash IS NOT NULL
              AND wave_unit.manifest_count IS NOT NULL
              AND wave_unit.manifest_frozen_at IS NOT NULL
            FOR UPDATE OF wave_unit"#,
    )
    .bind(operation_id)
    .bind(stage_run_unit_id)
    .fetch_optional(&mut *connection)
    .await?;
    let Some((wave_unit_id, organization_id, manifest_count)) = authority else {
        return Ok((None, Vec::new()));
    };
    let decisions = sqlx::query_as::<_, PersistedDecisionRow>(
        r#"SELECT item.id AS work_item_id,item.work_item_key,item.decision_kind,
                  item.candidate_id,item.no_candidate_reason_code,item.no_candidate_detail,
                  candidate.hypothesis,candidate.technique,candidate.rationale,
                  candidate.prior_refs,candidate.suggested_approach,candidate.priority,
                  candidate.execution_plan,candidate.candidate_plan_hash,candidate.risk_class,
                  COALESCE((
                      SELECT array_agg(link.evidence_id ORDER BY link.evidence_id)
                        FROM attack_candidate_evidence link
                       WHERE link.candidate_id=item.candidate_id AND link.role='support'
                  ),ARRAY[]::BIGINT[]) AS candidate_evidence_ids,
                  COALESCE((
                      SELECT array_agg(link.evidence_id ORDER BY link.evidence_id)
                        FROM attack_candidate_work_item_evidence link
                       WHERE link.work_item_id=item.id AND link.role='decision'
                  ),ARRAY[]::BIGINT[]) AS decision_evidence_ids
             FROM attack_candidate_work_items item
        LEFT JOIN attack_candidates candidate
               ON candidate.candidate_id=item.candidate_id
              AND candidate.operation_uuid=item.operation_id
              AND candidate.source_work_item_id=item.id
            WHERE item.operation_id=$1 AND item.wave_unit_id=$2
              AND item.organization_id=$3
            ORDER BY item.work_item_key,item.id"#,
    )
    .bind(operation_id)
    .bind(wave_unit_id)
    .bind(organization_id)
    .fetch_all(&mut *connection)
    .await?;
    Ok((
        Some((wave_unit_id, organization_id, manifest_count)),
        decisions,
    ))
}

fn v2_record_from_rows(
    authority: Option<(Uuid, Uuid, i32)>,
    rows: &[PersistedDecisionRow],
) -> crate::Result<AttackShadowV2ReadRow> {
    let Some((_, _, manifest_count)) = authority else {
        return Ok(AttackShadowV2ReadRow::Missing);
    };
    if manifest_count <= 0 || usize::try_from(manifest_count).ok() != Some(rows.len()) {
        return Ok(AttackShadowV2ReadRow::Incomplete);
    }
    let mut decisions = Vec::with_capacity(rows.len());
    let mut candidate_count = 0_u32;
    let mut no_candidate_count = 0_u32;
    for row in rows {
        let (kind, payload) = match row.decision_kind.as_deref() {
            Some("candidate") => {
                if row.candidate_evidence_ids.is_empty() {
                    return Ok(AttackShadowV2ReadRow::Incomplete);
                }
                let (
                    Some(candidate_id),
                    Some(hypothesis),
                    Some(rationale),
                    Some(prior_refs),
                    Some(suggested_approach),
                    Some(priority),
                    Some(execution_plan),
                    Some(candidate_plan_hash),
                    Some(risk_class),
                ) = (
                    row.candidate_id,
                    row.hypothesis.as_ref(),
                    row.rationale.as_ref(),
                    row.prior_refs.as_ref(),
                    row.suggested_approach.as_ref(),
                    row.priority.as_ref(),
                    row.execution_plan.as_ref(),
                    row.candidate_plan_hash.as_ref(),
                    row.risk_class.as_ref(),
                )
                else {
                    return Ok(AttackShadowV2ReadRow::Incomplete);
                };
                candidate_count += 1;
                (
                    "candidate",
                    serde_json::json!({
                        "work_item_id": row.work_item_id,
                        "candidate_id": candidate_id,
                        "hypothesis": hypothesis,
                        "technique": row.technique,
                        "rationale": rationale,
                        "prior_refs": prior_refs,
                        "suggested_approach": suggested_approach,
                        "priority": priority,
                        "execution_plan": execution_plan,
                        "candidate_plan_hash": candidate_plan_hash,
                        "risk_class": risk_class,
                        "evidence_ids": row.candidate_evidence_ids,
                    }),
                )
            }
            Some("no_candidate") => {
                if row.decision_evidence_ids.is_empty() {
                    return Ok(AttackShadowV2ReadRow::Incomplete);
                }
                let (Some(reason_code), Some(detail)) = (
                    row.no_candidate_reason_code.as_ref(),
                    row.no_candidate_detail.as_ref(),
                ) else {
                    return Ok(AttackShadowV2ReadRow::Incomplete);
                };
                no_candidate_count += 1;
                (
                    "no_candidate",
                    serde_json::json!({
                        "work_item_id": row.work_item_id,
                        "reason_code": reason_code,
                        "detail": detail,
                        "evidence_ids": row.decision_evidence_ids,
                    }),
                )
            }
            _ => return Ok(AttackShadowV2ReadRow::Incomplete),
        };
        decisions.push(AttackShadowDecisionRow {
            work_item_key: row.work_item_key.clone(),
            kind: kind.to_string(),
            semantic_hash: sha256_json(&payload)?,
        });
    }
    Ok(AttackShadowV2ReadRow::Complete(normalize_record(
        decisions,
        AttackShadowReviewCountsRow {
            wave_unit_count: 1,
            // This read model is the immutable Candidate final-Gate snapshot,
            // before the separate human review barrier can close.
            review_closed_unit_count: 0,
            candidate_decision_count: candidate_count,
            no_candidate_decision_count: no_candidate_count,
        },
    )?))
}

/// Rebuild the current whole-record V2 projection without requiring a shadow
/// row. The database INSERT owner independently performs the same rebuild and
/// remains final authority for the durable comparison seal.
pub async fn load_v2_record_with_connection(
    connection: &mut PgConnection,
    operation_id: Uuid,
    stage_run_unit_id: Uuid,
) -> crate::Result<AttackShadowV2ReadRow> {
    let (authority, rows) =
        load_persisted_decisions(connection, operation_id, stage_run_unit_id).await?;
    v2_record_from_rows(authority, &rows)
}

fn legacy_record_from_command(
    command: &AcceptCandidateBatch,
    rows: &[PersistedDecisionRow],
) -> crate::Result<AttackShadowCompleteReadRow> {
    let by_id = rows
        .iter()
        .map(|row| (row.work_item_id, row))
        .collect::<std::collections::HashMap<_, _>>();
    let mut decisions = Vec::with_capacity(command.expected_work_item_ids.len());
    for draft in &command.candidates {
        let row = by_id
            .get(&draft.work_item_id)
            .ok_or_else(|| shadow_error("attack shadow Candidate is outside the manifest"))?;
        decisions.push(AttackShadowDecisionRow {
            work_item_key: row.work_item_key.clone(),
            kind: "candidate".to_string(),
            semantic_hash: sha256_json(&candidate_payload_from_command(draft))?,
        });
    }
    for decision in &command.no_candidate_decisions {
        let row = by_id
            .get(&decision.work_item_id)
            .ok_or_else(|| shadow_error("attack shadow no-candidate is outside the manifest"))?;
        decisions.push(AttackShadowDecisionRow {
            work_item_key: row.work_item_key.clone(),
            kind: "no_candidate".to_string(),
            semantic_hash: sha256_json(&no_candidate_payload_from_command(decision))?,
        });
    }
    normalize_record(
        decisions,
        AttackShadowReviewCountsRow {
            wave_unit_count: 1,
            review_closed_unit_count: 0,
            candidate_decision_count: u32::try_from(command.candidates.len())
                .map_err(|_| shadow_error("attack shadow Candidate count overflow"))?,
            no_candidate_decision_count: u32::try_from(command.no_candidate_decisions.len())
                .map_err(|_| shadow_error("attack shadow no-candidate count overflow"))?,
        },
    )
}

/// Persist one immutable legacy semantic mirror after V2 Candidate acceptance.
/// The caller holds the operation/final-seal transaction, so either both
/// sources commit or neither source does.
pub(super) async fn persist_candidate_legacy_mirror(
    connection: &mut PgConnection,
    contract: &str,
    command: &AcceptCandidateBatch,
) -> crate::Result<()> {
    if !matches!(
        contract,
        "dual_write_read_legacy" | "dual_write_read_v2_fallback"
    ) {
        return Ok(());
    }
    let (authority, rows) = load_persisted_decisions(
        connection,
        command.operation_id,
        command.decision_stage_run_unit_id,
    )
    .await?;
    let Some((_, organization_id, manifest_count)) = authority else {
        return Err(shadow_error("authoritative Candidate V2 record is missing"));
    };
    if usize::try_from(manifest_count).ok() != Some(rows.len()) {
        return Err(shadow_error(
            "authoritative Candidate V2 record is incomplete",
        ));
    }
    let legacy_record = legacy_record_from_command(command, &rows)?;
    // Do not require semantic equality here. Dual write must retain a complete
    // legacy mirror even when a V2 adapter bug diverges; the production kit
    // selector records that mismatch and the deployment promotion gate blocks.
    let legacy_record_hash = complete_record_hash(&legacy_record)?;
    sqlx::query(
        r#"INSERT INTO attack_execution_shadow_reads (
               stage_run_unit_id,operation_id,stage_execution_id,organization_id,
               attack_execution_contract,stage_kind,legacy_record,legacy_record_hash
           ) VALUES ($1,$2,$3,$4,$5,'attack_candidate',$6,$7)
           ON CONFLICT (stage_run_unit_id) DO NOTHING"#,
    )
    .bind(command.decision_stage_run_unit_id)
    .bind(command.operation_id)
    .bind(command.decision_stage_execution_id)
    .bind(organization_id)
    .bind(contract)
    .bind(serde_json::to_value(&legacy_record)?)
    .bind(&legacy_record_hash)
    .execute(&mut *connection)
    .await?;
    let persisted = load_stored_for_update(
        connection,
        command.operation_id,
        command.decision_stage_run_unit_id,
    )
    .await?
    .ok_or_else(|| shadow_error("attack shadow mirror insert disappeared"))?;
    if persisted.organization_id != organization_id
        || persisted.attack_execution_contract != contract
        || persisted.legacy_record != serde_json::to_value(&legacy_record)?
        || persisted.legacy_record_hash != legacy_record_hash
    {
        return Err(shadow_error("attack shadow mirror replay drift"));
    }
    Ok(())
}

async fn load_stored_for_update(
    connection: &mut PgConnection,
    operation_id: Uuid,
    stage_run_unit_id: Uuid,
) -> crate::Result<Option<StoredShadowRow>> {
    Ok(sqlx::query_as::<_, StoredShadowRow>(
        r#"SELECT stage_run_unit_id,operation_id,organization_id,
                  attack_execution_contract,legacy_record,legacy_record_hash,
                  comparison,selected_source,selected_record_hash
             FROM attack_execution_shadow_reads
            WHERE operation_id=$1 AND stage_run_unit_id=$2
            FOR UPDATE"#,
    )
    .bind(operation_id)
    .bind(stage_run_unit_id)
    .fetch_optional(&mut *connection)
    .await?)
}

pub async fn load_unit_sample_with_connection(
    connection: &mut PgConnection,
    operation_id: Uuid,
    stage_run_unit_id: Uuid,
) -> crate::Result<Option<AttackExecutionShadowSampleRow>> {
    let contract: Option<String> = sqlx::query_scalar(
        "SELECT attack_execution_contract FROM operation_state
          WHERE operation_id=$1 AND superseded_by IS NULL",
    )
    .bind(operation_id)
    .fetch_optional(&mut *connection)
    .await?;
    let Some(contract) = contract else {
        return Ok(None);
    };
    let stored = load_stored_for_update(connection, operation_id, stage_run_unit_id).await?;
    let (authority, rows) =
        load_persisted_decisions(connection, operation_id, stage_run_unit_id).await?;
    let v2_record = v2_record_from_rows(authority, &rows)?;
    match (contract.as_str(), stored) {
        ("dual_write_read_legacy" | "dual_write_read_v2_fallback", Some(stored)) => {
            let legacy_record: AttackShadowCompleteReadRow =
                serde_json::from_value(stored.legacy_record)?;
            if stored.operation_id != operation_id
                || stored.stage_run_unit_id != stage_run_unit_id
                || stored.attack_execution_contract != contract
                || complete_record_hash(&legacy_record)? != stored.legacy_record_hash
            {
                return Err(shadow_error("attack shadow persisted identity drift"));
            }
            Ok(Some(AttackExecutionShadowSampleRow {
                operation_id,
                stage_run_unit_id,
                organization_id: Some(stored.organization_id),
                contract,
                legacy_record: Some(legacy_record),
                v2_record,
                comparison: stored.comparison,
                selected_source: stored.selected_source,
                selected_record_hash: stored.selected_record_hash,
            }))
        }
        ("dual_write_read_legacy" | "dual_write_read_v2_fallback", None) => Err(shadow_error(
            "dual-write Candidate final seal is missing its legacy mirror",
        )),
        ("v2_only", None) => Ok(Some(AttackExecutionShadowSampleRow {
            operation_id,
            stage_run_unit_id,
            organization_id: authority.map(|(_, organization_id, _)| organization_id),
            contract,
            legacy_record: None,
            v2_record,
            comparison: None,
            selected_source: None,
            selected_record_hash: None,
        })),
        ("v2_only", Some(_)) => Err(shadow_error(
            "v2_only operation unexpectedly contains a legacy attack mirror",
        )),
        _ => Ok(None),
    }
}

pub async fn load_unit_sample(
    pool: &PgPool,
    operation_id: Uuid,
    stage_run_unit_id: Uuid,
) -> crate::Result<Option<AttackExecutionShadowSampleRow>> {
    let mut connection = pool.acquire().await?;
    load_unit_sample_with_connection(&mut connection, operation_id, stage_run_unit_id).await
}

/// Verify the result of the kit whole-record selector against the DB-owned seal.
/// The shadow INSERT trigger rebuilds V2, derives comparison/source/hash, and
/// timestamps the attestation atomically; this seam never owns that conclusion.
pub async fn record_unit_selection_with_connection(
    connection: &mut PgConnection,
    operation_id: Uuid,
    stage_run_unit_id: Uuid,
    comparison: &str,
    selected_source: &str,
) -> crate::Result<()> {
    if !matches!(comparison, "match" | "mismatch" | "v2_missing")
        || !matches!(selected_source, "legacy" | "v2" | "legacy_fallback")
    {
        return Err(shadow_error(
            "attack shadow selection vocabulary is invalid",
        ));
    }
    let stored = load_stored_for_update(connection, operation_id, stage_run_unit_id)
        .await?
        .ok_or_else(|| shadow_error("attack shadow selection has no legacy mirror"))?;
    let legacy_record: AttackShadowCompleteReadRow =
        serde_json::from_value(stored.legacy_record.clone())?;
    let (authority, rows) =
        load_persisted_decisions(connection, operation_id, stage_run_unit_id).await?;
    let v2_record = v2_record_from_rows(authority, &rows)?;
    let expected = expected_shadow_selection(
        &stored.attack_execution_contract,
        &legacy_record,
        &v2_record,
    )?;
    if comparison != expected.comparison {
        return Err(shadow_error(
            "attack shadow comparison does not match durable truth",
        ));
    }
    if selected_source != expected.selected_source {
        return Err(shadow_error(
            "attack shadow selected source violates the frozen contract",
        ));
    }
    let selected_record_hash = expected.selected_record_hash;
    if let (Some(existing_comparison), Some(existing_source), Some(existing_hash)) = (
        stored.comparison.as_deref(),
        stored.selected_source.as_deref(),
        stored.selected_record_hash.as_deref(),
    ) {
        if existing_comparison == comparison
            && existing_source == selected_source
            && existing_hash == selected_record_hash
        {
            return Ok(());
        }
        return Err(shadow_error("attack shadow selection replay drift"));
    }
    Err(shadow_error(
        "attack shadow DB-owned selection seal is incomplete",
    ))
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct AttackShadowAggregate {
    pub sample_count: u64,
    pub mismatch_count: u64,
    pub incomplete_count: u64,
}

/// Rebuild the exact Candidate-admission cohort selected by the database
/// promotion cutoff. Missing shadow rows remain visible as incomplete samples;
/// they are never dropped by an inner join over the shadow table.
pub(super) async fn aggregate_for_candidate_cohort(
    connection: &mut PgConnection,
    contract: &str,
    rollout_rank: i16,
    admission_cutoff: i64,
) -> crate::Result<AttackShadowAggregate> {
    let sample_ids = sqlx::query_as::<_, (Uuid, Uuid, bool)>(
        r#"SELECT stage_unit.operation_id,
                  stage_unit.id,
                  EXISTS (
                      SELECT 1
                        FROM attack_execution_shadow_reads AS shadow
                       WHERE shadow.operation_id=stage_unit.operation_id
                         AND shadow.stage_run_unit_id=stage_unit.id
                  ) AS shadow_exists
             FROM attack_execution_candidate_admissions AS admission
             JOIN attack_wave_runs AS wave
               ON wave.operation_id=admission.operation_id
              AND wave.scope_snapshot_id=admission.scope_snapshot_id
             JOIN attack_wave_units AS wave_unit
               ON wave_unit.wave_run_id=wave.id
              AND wave_unit.operation_id=wave.operation_id
              AND wave_unit.scope_snapshot_id=wave.scope_snapshot_id
             JOIN stage_run_units AS stage_unit
               ON stage_unit.operation_id=wave.operation_id
              AND stage_unit.scope_snapshot_id=wave.scope_snapshot_id
              AND stage_unit.organization_id=wave_unit.organization_id
              AND stage_unit.generation=wave.generation
              AND stage_unit.stage_kind='attack_candidate'
              AND stage_unit.status='passed'
              AND stage_unit.terminal_at IS NOT NULL
            WHERE admission.attack_execution_contract=$1
              AND admission.rollout_rank=$2
              AND admission.admission_seq <= $3
            ORDER BY stage_unit.operation_id,stage_unit.id
            FOR SHARE OF stage_unit"#,
    )
    .bind(contract)
    .bind(rollout_rank)
    .bind(admission_cutoff)
    .fetch_all(&mut *connection)
    .await?;
    let sample_count = u64::try_from(sample_ids.len())
        .map_err(|_| shadow_error("attack Candidate cohort sample count is invalid"))?;
    let mut mismatch_count = 0_u64;
    let mut incomplete_count = 0_u64;
    for (operation_id, stage_run_unit_id, shadow_exists) in sample_ids {
        if !shadow_exists {
            incomplete_count = incomplete_count.saturating_add(1);
            continue;
        }
        let sample = load_unit_sample_with_connection(connection, operation_id, stage_run_unit_id)
            .await?
            .ok_or_else(|| shadow_error("attack Candidate cohort sample disappeared"))?;
        let Some(legacy_record) = sample.legacy_record.as_ref() else {
            incomplete_count = incomplete_count.saturating_add(1);
            continue;
        };
        let expected =
            expected_shadow_selection(&sample.contract, legacy_record, &sample.v2_record)?;
        if expected.comparison == "mismatch" {
            mismatch_count = mismatch_count.saturating_add(1);
        }
        let attestation_matches = sample.comparison.as_deref() == Some(expected.comparison)
            && sample.selected_source.as_deref() == Some(expected.selected_source)
            && sample.selected_record_hash.as_deref()
                == Some(expected.selected_record_hash.as_str());
        if expected.comparison == "v2_missing" || !attestation_matches {
            incomplete_count = incomplete_count.saturating_add(1);
        }
    }
    Ok(AttackShadowAggregate {
        sample_count,
        mismatch_count,
        incomplete_count,
    })
}
