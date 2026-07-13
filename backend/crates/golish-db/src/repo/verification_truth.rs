//! Exact persisted Verification Gate snapshot projection.

use std::collections::{BTreeMap, BTreeSet};

use sqlx::{PgPool, Postgres, Transaction};
use uuid::Uuid;

fn unavailable(message: &str) -> crate::DbError {
    crate::DbError::Other(anyhow::anyhow!(message.to_string()))
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct AttemptTerminalTruthRow {
    pub candidate_id: Uuid,
    pub attempt_id: Uuid,
    pub candidate_plan_hash: String,
    pub status: String,
    pub proof_evidence_ids: Vec<i64>,
    pub refutation_evidence_ids: Vec<i64>,
    pub blocker_evidence_ids: Vec<i64>,
    pub blocker_reason_code: Option<String>,
    pub finding_id: Option<Uuid>,
    pub finding_lineage_exact: bool,
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct ResidualRiskTruthRow {
    pub residual_risk_id: Uuid,
    pub reason_code: String,
    pub disclosure_status: String,
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct VerificationTruthRow {
    pub operation_id: Uuid,
    pub scope_snapshot_id: Uuid,
    pub wave_run_id: Uuid,
    pub wave_unit_id: Uuid,
    pub organization_id: Uuid,
    pub review_closed: bool,
    pub pending_work_items: u32,
    pub approved_ever: u32,
    pub attempts: Vec<AttemptTerminalTruthRow>,
    pub residual_risks: Vec<ResidualRiskTruthRow>,
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct VerificationUnitAuthorityRow {
    pub wave_unit_id: Uuid,
    pub organization_id: Uuid,
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct VerificationTruthSetRow {
    pub operation_id: Uuid,
    pub scope_snapshot_id: Uuid,
    pub wave_run_id: Uuid,
    pub expected_units: Vec<VerificationUnitAuthorityRow>,
    pub snapshots: Vec<VerificationTruthRow>,
}

#[derive(Debug, Clone, Copy)]
pub struct CloseVerificationUnit {
    pub operation_id: Uuid,
    pub scope_snapshot_id: Uuid,
    pub wave_run_id: Uuid,
    pub wave_unit_id: Uuid,
    pub organization_id: Uuid,
    pub verification_stage_execution_id: Uuid,
    pub verification_stage_run_unit_id: Uuid,
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct ClosedVerificationUnit {
    pub wave_unit_id: Uuid,
    pub row_version: i64,
    pub verification_closed: bool,
    pub consolidation_status: String,
    pub verification_stage_run_unit_id: Uuid,
    pub verification_stage_run_unit_status: String,
    pub verification_primary_worker_run_id: Uuid,
    pub verification_primary_worker_status: String,
    pub verification_handoff_id: Uuid,
    pub verification_handoff_payload_sha256: String,
    pub replayed: bool,
}

#[derive(Debug, sqlx::FromRow)]
struct VerificationWaveAuthority {
    operation_id: Uuid,
    scope_snapshot_id: Uuid,
    wave_run_id: Uuid,
    response_loss_replay: bool,
}

#[derive(Debug, sqlx::FromRow)]
struct VerificationAuthority {
    operation_id: Uuid,
    scope_snapshot_id: Uuid,
    wave_run_id: Uuid,
    wave_unit_id: Uuid,
    organization_id: Uuid,
    review_closed: bool,
    verification_closed: bool,
    consolidation_status: String,
    status: String,
    terminal_at: Option<chrono::DateTime<chrono::Utc>>,
    manifest_hash: Option<String>,
    manifest_count: Option<i32>,
    manifest_frozen_at: Option<chrono::DateTime<chrono::Utc>>,
}

#[derive(Debug, sqlx::FromRow)]
struct CloseVerificationAuthority {
    generation: i32,
    wave_status: String,
    wave_terminal_at: Option<chrono::DateTime<chrono::Utc>>,
    unit_status: String,
    review_closed: bool,
    verification_closed: bool,
    consolidation_status: String,
    unit_terminal_at: Option<chrono::DateTime<chrono::Utc>>,
    unit_row_version: i64,
    verification_generation: i32,
    verification_unit_status: String,
    verification_unit_terminal_at: Option<chrono::DateTime<chrono::Utc>>,
    verification_unit_row_version: i64,
}

#[derive(Debug, sqlx::FromRow)]
struct VerificationPrimaryWorkerAuthority {
    id: Uuid,
    status: String,
    terminal_at: Option<chrono::DateTime<chrono::Utc>>,
    checkpoint_version: i64,
    attempt_epoch: i64,
    lease_token: Option<Uuid>,
    lease_owner: Option<String>,
    lease_acquired_at: Option<chrono::DateTime<chrono::Utc>>,
    lease_expires_at: Option<chrono::DateTime<chrono::Utc>>,
    heartbeat_at: Option<chrono::DateTime<chrono::Utc>>,
    active_tool_call_id: Option<Uuid>,
    active_tool_started_at: Option<chrono::DateTime<chrono::Utc>>,
}

#[derive(Debug, sqlx::FromRow)]
struct AttemptTruthRow {
    candidate_id: Uuid,
    attempt_id: Uuid,
    candidate_plan_hash: String,
    status: String,
    blocker_reason_code: Option<String>,
    finding_id: Option<Uuid>,
    finding_lineage_exact: bool,
}

#[derive(Debug, sqlx::FromRow)]
struct ResidualRiskRow {
    residual_risk_id: Uuid,
    reason_code: String,
    disclosure_status: String,
}

#[derive(Debug, sqlx::FromRow)]
struct VerificationFactDeltaRefRow {
    fact_delta_id: Uuid,
    source_attempt_id: Uuid,
    candidate_id: Uuid,
    canonical_ref_kind: String,
    canonical_ref_id: Uuid,
    canonical_ref_version: i64,
    canonical_ref_hash: String,
    delta_kind: String,
    status: String,
    evidence_ids: Vec<i64>,
    evidence_subset_exact: bool,
}

#[derive(Debug, sqlx::FromRow)]
struct VerificationFindingRefRow {
    finding_id: Uuid,
    candidate_attempt_id: Uuid,
    candidate_id: Uuid,
    source_row_version: i64,
    observed_at: chrono::DateTime<chrono::Utc>,
    canonical_content: serde_json::Value,
    evidence_ids: Vec<i64>,
}

#[derive(Debug, sqlx::FromRow)]
struct VerificationNoCandidateRefRow {
    work_item_id: Uuid,
    work_item_key: String,
    reason_code: String,
    detail: String,
    decided_at: chrono::DateTime<chrono::Utc>,
    evidence_ids: Vec<i64>,
}

#[derive(Debug)]
struct VerificationHandoffMaterial {
    id: Uuid,
    payload: serde_json::Value,
    payload_sha256: String,
    evidence_ids: Vec<i64>,
    coverage_watermark: serde_json::Value,
    verification_truth_hash: String,
}

/// Load every exact active Verification unit for a V2-only operation. When a
/// consolidation response was lost, fall back only to the latest terminal
/// source Wave backed by its exact immutable consolidation row. Optional org
/// narrowing is server-owned routing context, never a model-supplied key.
pub async fn load_for_operation(
    pool: &PgPool,
    operation_id: Uuid,
    organization_id: Option<Uuid>,
) -> crate::Result<VerificationTruthSetRow> {
    let mut tx = pool.begin().await?;
    sqlx::query("SET TRANSACTION ISOLATION LEVEL REPEATABLE READ READ ONLY")
        .execute(&mut *tx)
        .await?;
    let wave = sqlx::query_as::<_, VerificationWaveAuthority>(
        r#"SELECT operation.operation_id,wave.scope_snapshot_id,wave.id AS wave_run_id,
                  wave.status='terminal' AS response_loss_replay
             FROM operation_state operation
             JOIN attack_wave_runs wave ON wave.operation_id=operation.operation_id
             LEFT JOIN attack_wave_consolidations consolidation
               ON consolidation.source_wave_run_id=wave.id
              AND consolidation.operation_id=wave.operation_id
              AND consolidation.scope_snapshot_id=wave.scope_snapshot_id
              AND consolidation.source_generation=wave.generation
              AND consolidation.source_wave_version_after=wave.row_version
              AND consolidation.policy_hash=wave.policy_hash
            WHERE operation.operation_id=$1
              AND operation.runtime_memory_contract='v2_only'
              AND operation.attack_execution_contract='v2_only'
              AND (
                    (wave.status='verification' AND wave.terminal_at IS NULL)
                    OR (
                        wave.status='terminal' AND wave.terminal_at IS NOT NULL
                        AND consolidation.id IS NOT NULL
                        AND consolidation.decision_kind IN (
                            'opened_next_wave','closed_no_delta','exhausted'
                        )
                    )
                  )
            ORDER BY
                  CASE WHEN wave.status='verification' THEN 0 ELSE 1 END,
                  wave.generation DESC,wave.id
            LIMIT 1"#,
    )
    .bind(operation_id)
    .fetch_optional(&mut *tx)
    .await?
    .ok_or_else(|| unavailable("exact Verification wave authority is missing"))?;
    let authorities = sqlx::query_as::<_, VerificationAuthority>(
        r#"SELECT $1::UUID AS operation_id,$2::UUID AS scope_snapshot_id,
                  $3::UUID AS wave_run_id,wave_unit.id AS wave_unit_id,
                  wave_unit.organization_id,wave_unit.review_closed,
                  wave_unit.verification_closed,wave_unit.consolidation_status,
                  wave_unit.status,wave_unit.terminal_at,wave_unit.manifest_hash,
                  wave_unit.manifest_count,wave_unit.manifest_frozen_at
             FROM attack_wave_units wave_unit
            WHERE wave_unit.wave_run_id=$3
              AND wave_unit.operation_id=$1
              AND wave_unit.scope_snapshot_id=$2
              AND ($4::UUID IS NULL OR wave_unit.organization_id=$4)
            ORDER BY wave_unit.ordinal,wave_unit.organization_id"#,
    )
    .bind(wave.operation_id)
    .bind(wave.scope_snapshot_id)
    .bind(wave.wave_run_id)
    .bind(organization_id)
    .fetch_all(&mut *tx)
    .await?;
    if authorities.is_empty() {
        return Err(unavailable(
            "exact Verification wave unit authority is missing",
        ));
    }
    if authorities.iter().any(|unit| {
        let active_verification = unit.status == "verification" && unit.terminal_at.is_none();
        let terminal_no_input = unit.status == "terminal"
            && unit.terminal_at.is_some()
            && unit.review_closed
            && unit.verification_closed
            && unit.consolidation_status == "terminal"
            && unit.manifest_hash.is_none()
            && unit.manifest_count.is_none()
            && unit.manifest_frozen_at.is_none();
        let terminal_manifest = unit.status == "terminal"
            && unit.terminal_at.is_some()
            && unit.review_closed
            && unit.verification_closed
            && unit.consolidation_status == "terminal"
            && unit
                .manifest_hash
                .as_deref()
                .is_some_and(|hash| !hash.trim().is_empty())
            && unit.manifest_count.is_some_and(|count| count > 0)
            && unit.manifest_frozen_at.is_some();
        if wave.response_loss_replay {
            !terminal_no_input && !terminal_manifest
        } else {
            !active_verification && !terminal_no_input
        }
    }) {
        return Err(unavailable(
            "Verification wave unit is not verification-ready",
        ));
    }
    let expected_units = authorities
        .iter()
        .map(|authority| VerificationUnitAuthorityRow {
            wave_unit_id: authority.wave_unit_id,
            organization_id: authority.organization_id,
        })
        .collect();
    let mut snapshots = Vec::with_capacity(authorities.len());
    for authority in authorities {
        snapshots.push(load_exact(&mut tx, authority).await?);
    }
    let truth = VerificationTruthSetRow {
        operation_id: wave.operation_id,
        scope_snapshot_id: wave.scope_snapshot_id,
        wave_run_id: wave.wave_run_id,
        expected_units,
        snapshots,
    };
    tx.commit().await?;
    Ok(truth)
}

fn attempt_has_terminal_authority(attempt: &AttemptTerminalTruthRow) -> bool {
    match attempt.status.as_str() {
        "verified" => {
            !attempt.proof_evidence_ids.is_empty()
                && attempt.refutation_evidence_ids.is_empty()
                && attempt.blocker_evidence_ids.is_empty()
                && attempt.blocker_reason_code.is_none()
                && attempt.finding_id.is_some()
                && attempt.finding_lineage_exact
        }
        "refuted" => {
            attempt.proof_evidence_ids.is_empty()
                && !attempt.refutation_evidence_ids.is_empty()
                && attempt.blocker_evidence_ids.is_empty()
                && attempt.blocker_reason_code.is_none()
                && attempt.finding_id.is_none()
                && !attempt.finding_lineage_exact
        }
        "blocked" => {
            attempt.proof_evidence_ids.is_empty()
                && attempt.refutation_evidence_ids.is_empty()
                && (!attempt.blocker_evidence_ids.is_empty()
                    || attempt.blocker_reason_code.is_some())
                && attempt.finding_id.is_none()
                && !attempt.finding_lineage_exact
        }
        _ => false,
    }
}

async fn build_verification_handoff_material(
    tx: &mut Transaction<'_, Postgres>,
    truth: &VerificationTruthRow,
) -> crate::Result<VerificationHandoffMaterial> {
    let fact_deltas = sqlx::query_as::<_, VerificationFactDeltaRefRow>(
        r#"SELECT delta.id AS fact_delta_id,delta.source_attempt_id,
                  delta.candidate_id,delta.canonical_ref_kind,
                  delta.canonical_ref_id,delta.canonical_ref_version,
                  delta.canonical_ref_hash,delta.delta_kind,delta.status,
                  COALESCE(
                    ARRAY(
                      SELECT evidence.evidence_id
                        FROM attack_fact_delta_evidence AS evidence
                       WHERE evidence.fact_delta_id=delta.id
                       ORDER BY evidence.evidence_id
                    ),
                    ARRAY[]::BIGINT[]
                  ) AS evidence_ids,
                  NOT EXISTS(
                    SELECT 1
                      FROM attack_fact_delta_evidence AS delta_evidence
                     WHERE delta_evidence.fact_delta_id=delta.id
                       AND NOT EXISTS(
                         SELECT 1
                           FROM candidate_attempt_evidence AS attempt_evidence
                          WHERE attempt_evidence.attempt_id=delta.source_attempt_id
                            AND attempt_evidence.evidence_id=delta_evidence.evidence_id
                            AND attempt_evidence.role='fact_delta'
                       )
                  ) AS evidence_subset_exact
             FROM attack_fact_deltas AS delta
            WHERE delta.operation_id=$1 AND delta.scope_snapshot_id=$2
              AND delta.wave_run_id=$3 AND delta.wave_unit_id=$4
              AND delta.organization_id=$5
              AND verification_attempt_terminal_bundle_exact(
                    delta.source_attempt_id,$1,$2,$3,$4,$5
                  )
            ORDER BY delta.id"#,
    )
    .bind(truth.operation_id)
    .bind(truth.scope_snapshot_id)
    .bind(truth.wave_run_id)
    .bind(truth.wave_unit_id)
    .bind(truth.organization_id)
    .fetch_all(&mut **tx)
    .await?;
    if fact_deltas.iter().any(|delta| {
        delta.status != "proposed" || delta.evidence_ids.is_empty() || !delta.evidence_subset_exact
    }) {
        return Err(unavailable(
            "Verification FactDelta proposal is not evidence-backed by its exact source Attempt",
        ));
    }

    let findings = sqlx::query_as::<_, VerificationFindingRefRow>(
        r#"SELECT finding.id AS finding_id,
                  lineage.candidate_attempt_id,lineage.candidate_id,
                  finding.row_version AS source_row_version,
                  finding.updated_at AS observed_at,
                  (to_jsonb(finding) - 'target_id') || jsonb_build_object(
                      'finding_lineage_id',lineage.id,
                      'finding_lineage_row_version',lineage.row_version,
                      'canonical_target_snapshot',lineage.canonical_target_snapshot
                  ) AS canonical_content,
                  COALESCE(
                    ARRAY(
                      SELECT evidence.evidence_id
                        FROM candidate_attempt_evidence AS evidence
                       WHERE evidence.attempt_id=lineage.candidate_attempt_id
                         AND evidence.role='proof'
                       ORDER BY evidence.evidence_id
                    ),
                    ARRAY[]::BIGINT[]
                  ) AS evidence_ids
             FROM finding_lineage AS lineage
             JOIN findings AS finding ON finding.id=lineage.finding_id
            WHERE lineage.operation_id=$1 AND lineage.scope_snapshot_id=$2
              AND lineage.wave_run_id=$3 AND lineage.wave_unit_id=$4
              AND lineage.organization_id=$5
            ORDER BY finding.id"#,
    )
    .bind(truth.operation_id)
    .bind(truth.scope_snapshot_id)
    .bind(truth.wave_run_id)
    .bind(truth.wave_unit_id)
    .bind(truth.organization_id)
    .fetch_all(&mut **tx)
    .await?;
    let verified_attempts = truth
        .attempts
        .iter()
        .filter(|attempt| attempt.status == "verified")
        .collect::<Vec<_>>();
    if findings.len() != verified_attempts.len()
        || findings.iter().any(|finding| {
            verified_attempts.iter().all(|attempt| {
                attempt.attempt_id != finding.candidate_attempt_id
                    || attempt.candidate_id != finding.candidate_id
                    || attempt.finding_id != Some(finding.finding_id)
                    || attempt.proof_evidence_ids != finding.evidence_ids
            })
        })
    {
        return Err(unavailable(
            "Verification canonical Finding projection does not match exact verified Attempt truth",
        ));
    }
    let canonical_finding_refs = findings
        .iter()
        .map(|finding| {
            serde_json::json!({
                "key": {
                    "kind": "finding",
                    "finding_id": finding.finding_id,
                },
                "organization_id": truth.organization_id,
                "source_table": "findings",
                "source_row_version": finding.source_row_version,
                "observed_at_unix_micros": finding.observed_at.timestamp_micros(),
                "content_sha256": super::operation_scope_decisions::sha256_json(
                    &finding.canonical_content,
                ),
                "evidence_ids": finding.evidence_ids,
            })
        })
        .collect::<Vec<_>>();
    let finding_ref_by_id = findings
        .iter()
        .zip(&canonical_finding_refs)
        .map(|(finding, canonical_ref)| (finding.finding_id, canonical_ref.clone()))
        .collect::<BTreeMap<_, _>>();

    let no_candidate_decisions = sqlx::query_as::<_, VerificationNoCandidateRefRow>(
        r#"SELECT work.id AS work_item_id,work.work_item_key,
                  work.no_candidate_reason_code AS reason_code,
                  work.no_candidate_detail AS detail,work.decided_at,
                  COALESCE(
                    ARRAY(
                      SELECT evidence.evidence_id
                        FROM attack_candidate_work_item_evidence AS evidence
                       WHERE evidence.work_item_id=work.id
                         AND evidence.role='decision'
                       ORDER BY evidence.evidence_id
                    ),
                    ARRAY[]::BIGINT[]
                  ) AS evidence_ids
             FROM attack_candidate_work_items AS work
            WHERE work.operation_id=$1 AND work.scope_snapshot_id=$2
              AND work.wave_unit_id=$3 AND work.organization_id=$4
              AND work.decision_kind='no_candidate'
            ORDER BY work.id"#,
    )
    .bind(truth.operation_id)
    .bind(truth.scope_snapshot_id)
    .bind(truth.wave_unit_id)
    .bind(truth.organization_id)
    .fetch_all(&mut **tx)
    .await?;
    if no_candidate_decisions
        .iter()
        .any(|decision| decision.evidence_ids.is_empty())
    {
        return Err(unavailable(
            "Verification no-candidate decision is missing exact decision evidence",
        ));
    }

    let mut typed_claims = Vec::new();
    for attempt in &truth.attempts {
        let evidence_ids = attempt
            .proof_evidence_ids
            .iter()
            .chain(&attempt.refutation_evidence_ids)
            .chain(&attempt.blocker_evidence_ids)
            .copied()
            .collect::<BTreeSet<_>>()
            .into_iter()
            .collect::<Vec<_>>();
        let payload = serde_json::json!({
            "attempt_id": attempt.attempt_id,
            "candidate_id": attempt.candidate_id,
            "candidate_plan_hash": attempt.candidate_plan_hash,
            "disposition": attempt.status,
            "finding_id": attempt.finding_id,
            "blocker_reason_code": attempt.blocker_reason_code,
            "finding_ref": attempt.finding_id.and_then(|finding_id| {
                finding_ref_by_id.get(&finding_id).cloned()
            }),
            "evidence_ids": evidence_ids,
        });
        typed_claims.push(serde_json::json!({
            "kind": "candidate_attempt_terminal",
            "payload": payload,
        }));
        if attempt.status == "verified" {
            typed_claims.push(serde_json::json!({
                "kind": "verified_candidate_attempt",
                "payload": payload,
            }));
        }
    }
    for decision in &no_candidate_decisions {
        typed_claims.push(serde_json::json!({
            "kind": "attack_no_candidate_decision",
            "payload": {
                "work_item_id": decision.work_item_id,
                "work_item_key": decision.work_item_key,
                "reason_code": decision.reason_code,
                "detail": decision.detail,
                "decided_at_unix_micros": decision.decided_at.timestamp_micros(),
                "evidence_ids": decision.evidence_ids,
            },
        }));
    }
    for delta in &fact_deltas {
        typed_claims.push(serde_json::json!({
            "kind": "attack_fact_delta_proposal",
            "payload": {
                "fact_delta_id": delta.fact_delta_id,
                "source_attempt_id": delta.source_attempt_id,
                "candidate_id": delta.candidate_id,
                "canonical_ref_kind": delta.canonical_ref_kind,
                "canonical_ref_id": delta.canonical_ref_id,
                "canonical_ref_version": delta.canonical_ref_version,
                "canonical_ref_hash": delta.canonical_ref_hash,
                "delta_kind": delta.delta_kind,
                "status": delta.status,
                "evidence_ids": delta.evidence_ids,
            },
        }));
    }
    let evidence_ids = truth
        .attempts
        .iter()
        .flat_map(|attempt| {
            attempt
                .proof_evidence_ids
                .iter()
                .chain(&attempt.refutation_evidence_ids)
                .chain(&attempt.blocker_evidence_ids)
        })
        .chain(
            no_candidate_decisions
                .iter()
                .flat_map(|decision| decision.evidence_ids.iter()),
        )
        .chain(
            fact_deltas
                .iter()
                .flat_map(|delta| delta.evidence_ids.iter()),
        )
        .copied()
        .collect::<BTreeSet<_>>()
        .into_iter()
        .collect::<Vec<_>>();
    let coverage_watermark = serde_json::json!({
        "approved_candidate_count": truth.approved_ever,
        "terminal_attempt_count": truth.attempts.len(),
        "verified_finding_count": truth
            .attempts
            .iter()
            .filter(|attempt| attempt.finding_id.is_some())
            .count(),
        "no_candidate_decision_count": no_candidate_decisions.len(),
        "fact_delta_proposal_count": fact_deltas.len(),
    });
    let truth_material = serde_json::json!({
        "schema_version": 1,
        "operation_id": truth.operation_id,
        "scope_snapshot_id": truth.scope_snapshot_id,
        "wave_run_id": truth.wave_run_id,
        "wave_unit_id": truth.wave_unit_id,
        "organization_id": truth.organization_id,
        "canonical_fact_refs": canonical_finding_refs,
        "typed_claims": typed_claims,
        "coverage_watermark": coverage_watermark,
        "evidence_ids": evidence_ids,
    });
    let verification_truth_hash = super::operation_scope_decisions::sha256_json(&truth_material);
    let payload = serde_json::json!({
        "schema_version": 1,
        "canonical_fact_refs": canonical_finding_refs,
        "typed_claims": typed_claims,
        "coverage_watermark": coverage_watermark,
        "evidence_ids": evidence_ids,
        "verification_truth_hash": verification_truth_hash,
    });
    let payload_sha256 = super::operation_scope_decisions::sha256_json(&payload);
    Ok(VerificationHandoffMaterial {
        id: Uuid::new_v5(&truth.wave_unit_id, b"verification-stage-handoff:v1"),
        payload,
        payload_sha256,
        evidence_ids,
        coverage_watermark,
        verification_truth_hash,
    })
}

fn verification_authority_for_close(
    command: CloseVerificationUnit,
    authority: &CloseVerificationAuthority,
) -> VerificationAuthority {
    VerificationAuthority {
        operation_id: command.operation_id,
        scope_snapshot_id: command.scope_snapshot_id,
        wave_run_id: command.wave_run_id,
        wave_unit_id: command.wave_unit_id,
        organization_id: command.organization_id,
        review_closed: authority.review_closed,
        verification_closed: authority.verification_closed,
        consolidation_status: authority.consolidation_status.clone(),
        status: authority.unit_status.clone(),
        terminal_at: authority.unit_terminal_at,
        manifest_hash: None,
        manifest_count: None,
        manifest_frozen_at: None,
    }
}

fn verification_handoff_matches(
    handoff: &super::stage_handoffs::VerificationStageHandoffRow,
    command: CloseVerificationUnit,
    authority: &CloseVerificationAuthority,
    primary_worker_id: Uuid,
    material: &VerificationHandoffMaterial,
) -> bool {
    handoff.id == material.id
        && handoff.operation_id == command.operation_id
        && handoff.scope_snapshot_id == command.scope_snapshot_id
        && handoff.wave_run_id == command.wave_run_id
        && handoff.wave_unit_id == command.wave_unit_id
        && handoff.organization_id == command.organization_id
        && handoff.stage_execution_id == command.verification_stage_execution_id
        && handoff.source_stage_run_unit_id == command.verification_stage_run_unit_id
        && handoff.primary_worker_run_id == primary_worker_id
        && handoff.wave_generation == authority.generation
        && handoff.wave_unit_row_version_after_close == authority.unit_row_version
        && handoff.from_stage_kind == "verification"
        && handoff.authority_kind == "verification_wave_close"
        && handoff.payload == material.payload
        && handoff.payload_sha256 == material.payload_sha256
        && handoff.evidence_ids == material.evidence_ids
        && handoff.coverage_watermark == material.coverage_watermark
        && handoff.verification_truth_hash == material.verification_truth_hash
        && handoff.schema_version == 1
}

/// Close one active Verification WaveUnit from DB truth. Candidate, Attempt
/// and evidence authority is reloaded under the same transaction that seals
/// the logical primary WorkerRun and StageRunUnit before the durable global
/// cursor can observe `consolidation_status='ready'`.
pub async fn close_verification_unit(
    tx: &mut Transaction<'_, Postgres>,
    command: CloseVerificationUnit,
) -> crate::Result<ClosedVerificationUnit> {
    let authority = sqlx::query_as::<_, CloseVerificationAuthority>(
        r#"SELECT wave.generation,wave.status AS wave_status,
                  wave.terminal_at AS wave_terminal_at,
                  unit.status AS unit_status,unit.review_closed,
                  unit.verification_closed,unit.consolidation_status,
                  unit.terminal_at AS unit_terminal_at,
                  unit.row_version AS unit_row_version,
                  verification_unit.generation AS verification_generation,
                  verification_unit.status AS verification_unit_status,
                  verification_unit.terminal_at AS verification_unit_terminal_at,
                  verification_unit.row_version AS verification_unit_row_version
             FROM operation_state AS operation
             JOIN attack_wave_runs AS wave
               ON wave.id=$3 AND wave.operation_id=operation.operation_id
              AND wave.scope_snapshot_id=$2
             JOIN attack_wave_units AS unit
               ON unit.id=$4 AND unit.wave_run_id=wave.id
              AND unit.operation_id=wave.operation_id
              AND unit.scope_snapshot_id=wave.scope_snapshot_id
              AND unit.organization_id=$5
             JOIN stage_run_units AS verification_unit
               ON verification_unit.id=$7
              AND verification_unit.operation_id=operation.operation_id
              AND verification_unit.stage_execution_id=$6
              AND verification_unit.scope_snapshot_id=wave.scope_snapshot_id
              AND verification_unit.organization_id=unit.organization_id
              AND verification_unit.stage_kind='verification'
              AND verification_unit.specialist='candidate_verifier'
            WHERE operation.operation_id=$1
              AND operation.runtime_memory_contract='v2_only'
              AND operation.attack_execution_contract='v2_only'
            FOR UPDATE OF wave,unit,verification_unit"#,
    )
    .bind(command.operation_id)
    .bind(command.scope_snapshot_id)
    .bind(command.wave_run_id)
    .bind(command.wave_unit_id)
    .bind(command.organization_id)
    .bind(command.verification_stage_execution_id)
    .bind(command.verification_stage_run_unit_id)
    .fetch_optional(&mut **tx)
    .await?
    .ok_or_else(|| unavailable("exact VerificationUnit close authority is missing"))?;
    if authority.generation != authority.verification_generation {
        return Err(unavailable(
            "Verification StageRunUnit generation does not match the durable Wave",
        ));
    }
    let primary_worker = sqlx::query_as::<_, VerificationPrimaryWorkerAuthority>(
        r#"SELECT worker.id,worker.status,worker.terminal_at,
                  worker.checkpoint_version,worker.attempt_epoch,
                  worker.lease_token,worker.lease_owner,worker.lease_acquired_at,
                  worker.lease_expires_at,worker.heartbeat_at,
                  worker.active_tool_call_id,worker.active_tool_started_at
             FROM stage_worker_runs AS worker
            WHERE worker.operation_id=$1 AND worker.stage_execution_id=$2
              AND worker.stage_run_unit_id=$3 AND worker.organization_id=$4
              AND worker.worker_generation=$5
              AND worker.specialist='candidate_verifier'
              AND worker.work_item_kind='organization'
              AND worker.work_item_key='verification'
            FOR UPDATE"#,
    )
    .bind(command.operation_id)
    .bind(command.verification_stage_execution_id)
    .bind(command.verification_stage_run_unit_id)
    .bind(command.organization_id)
    .bind(authority.generation)
    .fetch_optional(&mut **tx)
    .await?
    .ok_or_else(|| unavailable("Verification logical primary WorkerRun is missing"))?;
    if authority.wave_status == "verification"
        && authority.wave_terminal_at.is_none()
        && authority.verification_closed
        && authority.consolidation_status == "ready"
        && authority.unit_status == "verification"
        && authority.unit_terminal_at.is_none()
        && authority.verification_unit_status == "passed"
        && authority.verification_unit_terminal_at.is_some()
        && primary_worker.status == "passed"
        && primary_worker.terminal_at.is_some()
        && primary_worker.lease_token.is_none()
        && primary_worker.lease_owner.is_none()
        && primary_worker.lease_acquired_at.is_none()
        && primary_worker.lease_expires_at.is_none()
        && primary_worker.heartbeat_at.is_none()
        && primary_worker.active_tool_call_id.is_none()
        && primary_worker.active_tool_started_at.is_none()
    {
        let truth = load_exact(tx, verification_authority_for_close(command, &authority)).await?;
        if truth.pending_work_items != 0
            || truth.approved_ever as usize != truth.attempts.len()
            || truth.attempts.iter().any(|attempt| {
                attempt.candidate_plan_hash.trim().is_empty()
                    || !attempt_has_terminal_authority(attempt)
            })
        {
            return Err(unavailable(
                "VerificationUnit replay terminal Candidate truth is invalid",
            ));
        }
        let material = build_verification_handoff_material(tx, &truth).await?;
        let handoff = super::stage_handoffs::get_verification_with_executor(
            &mut **tx,
            command.verification_stage_run_unit_id,
        )
        .await
        .map_err(|error| unavailable(&error.to_string()))?
        .ok_or_else(|| unavailable("Verification typed handoff is missing"))?;
        if !verification_handoff_matches(
            &handoff,
            command,
            &authority,
            primary_worker.id,
            &material,
        ) {
            return Err(unavailable("Verification typed handoff replay mismatch"));
        }
        return Ok(ClosedVerificationUnit {
            wave_unit_id: command.wave_unit_id,
            row_version: authority.unit_row_version,
            verification_closed: true,
            consolidation_status: "ready".to_string(),
            verification_stage_run_unit_id: command.verification_stage_run_unit_id,
            verification_stage_run_unit_status: authority.verification_unit_status,
            verification_primary_worker_run_id: primary_worker.id,
            verification_primary_worker_status: primary_worker.status,
            verification_handoff_id: handoff.id,
            verification_handoff_payload_sha256: handoff.payload_sha256,
            replayed: true,
        });
    }
    if authority.wave_status != "verification"
        || authority.wave_terminal_at.is_some()
        || authority.unit_status != "verification"
        || authority.unit_terminal_at.is_some()
        || !authority.review_closed
        || authority.verification_closed
        || authority.consolidation_status != "pending"
        || authority.verification_unit_status != "queued"
        || authority.verification_unit_terminal_at.is_some()
    {
        return Err(unavailable("VerificationUnit is not close-ready"));
    }
    if primary_worker.status != "queued"
        || primary_worker.terminal_at.is_some()
        || primary_worker.lease_token.is_some()
        || primary_worker.lease_owner.is_some()
        || primary_worker.lease_acquired_at.is_some()
        || primary_worker.lease_expires_at.is_some()
        || primary_worker.heartbeat_at.is_some()
        || primary_worker.active_tool_call_id.is_some()
        || primary_worker.active_tool_started_at.is_some()
    {
        return Err(unavailable(
            "Verification logical primary WorkerRun is not close-ready",
        ));
    }
    let truth = load_exact(tx, verification_authority_for_close(command, &authority)).await?;
    if truth.pending_work_items != 0
        || truth.approved_ever as usize != truth.attempts.len()
        || truth.attempts.iter().any(|attempt| {
            attempt.candidate_plan_hash.trim().is_empty()
                || !attempt_has_terminal_authority(attempt)
        })
    {
        return Err(unavailable(
            "VerificationUnit still has pending or invalid terminal Candidate truth",
        ));
    }
    let handoff_material = build_verification_handoff_material(tx, &truth).await?;
    let updated: Option<i64> = sqlx::query_scalar(
        r#"UPDATE attack_wave_units
              SET verification_closed=TRUE,consolidation_status='ready',
                  row_version=row_version+1,updated_at=NOW()
            WHERE id=$1 AND wave_run_id=$2 AND operation_id=$3
              AND scope_snapshot_id=$4 AND organization_id=$5
              AND row_version=$6 AND status='verification'
              AND review_closed AND NOT verification_closed
              AND consolidation_status='pending' AND terminal_at IS NULL
            RETURNING row_version"#,
    )
    .bind(command.wave_unit_id)
    .bind(command.wave_run_id)
    .bind(command.operation_id)
    .bind(command.scope_snapshot_id)
    .bind(command.organization_id)
    .bind(authority.unit_row_version)
    .fetch_optional(&mut **tx)
    .await?;
    let row_version = updated.ok_or_else(|| unavailable("VerificationUnit close CAS was lost"))?;
    let worker_status: Option<String> = sqlx::query_scalar(
        r#"UPDATE stage_worker_runs
              SET status='passed',terminal_at=NOW(),updated_at=NOW()
            WHERE id=$1 AND operation_id=$2 AND stage_execution_id=$3
              AND stage_run_unit_id=$4 AND organization_id=$5
              AND worker_generation=$6 AND specialist='candidate_verifier'
              AND work_item_kind='organization' AND work_item_key='verification'
              AND checkpoint_version=$7 AND attempt_epoch=$8
              AND status='queued' AND terminal_at IS NULL
              AND lease_token IS NULL AND lease_owner IS NULL
              AND lease_acquired_at IS NULL AND lease_expires_at IS NULL
              AND heartbeat_at IS NULL AND active_tool_call_id IS NULL
              AND active_tool_started_at IS NULL
            RETURNING status"#,
    )
    .bind(primary_worker.id)
    .bind(command.operation_id)
    .bind(command.verification_stage_execution_id)
    .bind(command.verification_stage_run_unit_id)
    .bind(command.organization_id)
    .bind(authority.generation)
    .bind(primary_worker.checkpoint_version)
    .bind(primary_worker.attempt_epoch)
    .fetch_optional(&mut **tx)
    .await?;
    let worker_status = worker_status
        .ok_or_else(|| unavailable("Verification primary WorkerRun close CAS was lost"))?;
    let unit_status: Option<String> = sqlx::query_scalar(
        r#"UPDATE stage_run_units
              SET status='passed',
                  pass_watermark=jsonb_build_object(
                      'schema_version',1,
                      'source','candidate_v2_verification_close',
                      'attack_wave_unit_id',$8::TEXT,
                      'consolidation_status','ready',
                      'typed_handoff_id',$9::TEXT,
                      'verification_truth_hash',$10::TEXT,
                      'handoff_payload_sha256',$11::TEXT
                  ),
                  row_version=row_version+1,terminal_at=NOW(),updated_at=NOW()
            WHERE id=$1 AND operation_id=$2 AND stage_execution_id=$3
              AND scope_snapshot_id=$4 AND organization_id=$5
              AND stage_kind='verification' AND generation=$6
              AND specialist='candidate_verifier' AND row_version=$7
              AND status='queued' AND terminal_at IS NULL
            RETURNING status"#,
    )
    .bind(command.verification_stage_run_unit_id)
    .bind(command.operation_id)
    .bind(command.verification_stage_execution_id)
    .bind(command.scope_snapshot_id)
    .bind(command.organization_id)
    .bind(authority.generation)
    .bind(authority.verification_unit_row_version)
    .bind(command.wave_unit_id.hyphenated().to_string())
    .bind(handoff_material.id.hyphenated().to_string())
    .bind(&handoff_material.verification_truth_hash)
    .bind(&handoff_material.payload_sha256)
    .fetch_optional(&mut **tx)
    .await?;
    let unit_status =
        unit_status.ok_or_else(|| unavailable("Verification StageRunUnit close CAS was lost"))?;
    let handoff = super::stage_handoffs::insert_verification_with_executor(
        &mut **tx,
        &super::stage_handoffs::NewVerificationStageHandoffRow {
            id: handoff_material.id,
            operation_id: command.operation_id,
            scope_snapshot_id: command.scope_snapshot_id,
            wave_run_id: command.wave_run_id,
            wave_unit_id: command.wave_unit_id,
            organization_id: command.organization_id,
            stage_execution_id: command.verification_stage_execution_id,
            source_stage_run_unit_id: command.verification_stage_run_unit_id,
            primary_worker_run_id: primary_worker.id,
            wave_generation: authority.generation,
            wave_unit_row_version_after_close: row_version,
            payload: handoff_material.payload,
            payload_sha256: handoff_material.payload_sha256,
            evidence_ids: handoff_material.evidence_ids,
            coverage_watermark: handoff_material.coverage_watermark,
            verification_truth_hash: handoff_material.verification_truth_hash,
        },
    )
    .await
    .map_err(|error| unavailable(&error.to_string()))?;
    Ok(ClosedVerificationUnit {
        wave_unit_id: command.wave_unit_id,
        row_version,
        verification_closed: true,
        consolidation_status: "ready".to_string(),
        verification_stage_run_unit_id: command.verification_stage_run_unit_id,
        verification_stage_run_unit_status: unit_status,
        verification_primary_worker_run_id: primary_worker.id,
        verification_primary_worker_status: worker_status,
        verification_handoff_id: handoff.id,
        verification_handoff_payload_sha256: handoff.payload_sha256,
        replayed: false,
    })
}

async fn load_exact(
    tx: &mut Transaction<'_, Postgres>,
    authority: VerificationAuthority,
) -> crate::Result<VerificationTruthRow> {
    let pending_work_items: i64 = sqlx::query_scalar(
        r#"SELECT COUNT(*)
             FROM attack_candidate_work_items work
            WHERE work.operation_id=$1 AND work.scope_snapshot_id=$2
              AND work.wave_unit_id=$3
              AND work.organization_id=$4
              AND (
                work.decision_kind IS NULL
                OR (
                  work.decision_kind='no_candidate'
                  AND NOT EXISTS(
                    SELECT 1 FROM attack_candidate_work_item_evidence evidence
                     WHERE evidence.work_item_id=work.id AND evidence.role='decision'
                  )
                )
                OR (
                  work.decision_kind='candidate'
                  AND NOT EXISTS(
                    SELECT 1
                      FROM attack_candidates candidate
                      JOIN attack_candidate_approvals approval
                        ON approval.candidate_id=candidate.candidate_id
                       AND approval.operation_id=candidate.operation_uuid
                       AND approval.scope_snapshot_id=candidate.scope_snapshot_id
                       AND approval.wave_run_id=candidate.wave_run_id
                       AND approval.wave_unit_id=candidate.wave_unit_id
                       AND approval.organization_id=candidate.organization_id
                       AND approval.status<>'rejected'
                     WHERE candidate.source_work_item_id=work.id
                       AND candidate.terminal_attempt_id IS NOT NULL
                       AND candidate.disposition IN ('verified','refuted','blocked')
                  )
                )
              )"#,
    )
    .bind(authority.operation_id)
    .bind(authority.scope_snapshot_id)
    .bind(authority.wave_unit_id)
    .bind(authority.organization_id)
    .fetch_one(&mut **tx)
    .await?;
    let approved_ever: i64 = sqlx::query_scalar(
        r#"SELECT COUNT(DISTINCT approval.candidate_id)
             FROM attack_candidate_approvals approval
            WHERE approval.operation_id=$1 AND approval.scope_snapshot_id=$2
              AND approval.wave_run_id=$3 AND approval.wave_unit_id=$4
              AND approval.organization_id=$5 AND approval.status<>'rejected'"#,
    )
    .bind(authority.operation_id)
    .bind(authority.scope_snapshot_id)
    .bind(authority.wave_run_id)
    .bind(authority.wave_unit_id)
    .bind(authority.organization_id)
    .fetch_one(&mut **tx)
    .await?;
    let attempt_rows = sqlx::query_as::<_, AttemptTruthRow>(
        r#"SELECT candidate.candidate_id,attempt.id AS attempt_id,
                  attempt.candidate_plan_hash,attempt.status,
                  NULLIF(BTRIM(attempt.result_json->>'blocker_reason_code'),'')
                    AS blocker_reason_code,
                  candidate.terminal_finding_id AS finding_id,
                  EXISTS(
                    SELECT 1 FROM finding_lineage lineage
                     WHERE lineage.finding_id=candidate.terminal_finding_id
                       AND lineage.candidate_attempt_id=attempt.id
                       AND lineage.candidate_id=candidate.candidate_id
                       AND lineage.operation_id=candidate.operation_uuid
                       AND lineage.scope_snapshot_id=candidate.scope_snapshot_id
                       AND lineage.wave_run_id=candidate.wave_run_id
                       AND lineage.wave_unit_id=candidate.wave_unit_id
                       AND lineage.organization_id=candidate.organization_id
                       AND lineage.candidate_plan_hash=candidate.candidate_plan_hash
                  ) AS finding_lineage_exact
             FROM attack_candidates candidate
             JOIN candidate_attempts attempt
               ON attempt.id=candidate.terminal_attempt_id
              AND attempt.candidate_id=candidate.candidate_id
              AND attempt.operation_id=candidate.operation_uuid
              AND attempt.scope_snapshot_id=candidate.scope_snapshot_id
              AND attempt.wave_run_id=candidate.wave_run_id
              AND attempt.wave_unit_id=candidate.wave_unit_id
              AND attempt.organization_id=candidate.organization_id
              AND attempt.candidate_plan_hash=candidate.candidate_plan_hash
            WHERE candidate.operation_uuid=$1 AND candidate.scope_snapshot_id=$2
              AND candidate.wave_run_id=$3 AND candidate.wave_unit_id=$4
              AND candidate.organization_id=$5
              AND candidate.disposition IN ('verified','refuted','blocked')
              AND verification_attempt_terminal_bundle_exact(
                    attempt.id,$1,$2,$3,$4,$5
                  )
              AND EXISTS(
                SELECT 1 FROM attack_candidate_approvals approval
                 WHERE approval.candidate_id=candidate.candidate_id
                   AND approval.operation_id=candidate.operation_uuid
                   AND approval.scope_snapshot_id=candidate.scope_snapshot_id
                   AND approval.wave_run_id=candidate.wave_run_id
                   AND approval.wave_unit_id=candidate.wave_unit_id
                   AND approval.organization_id=candidate.organization_id
                   AND approval.status<>'rejected'
              )
            ORDER BY candidate.candidate_id"#,
    )
    .bind(authority.operation_id)
    .bind(authority.scope_snapshot_id)
    .bind(authority.wave_run_id)
    .bind(authority.wave_unit_id)
    .bind(authority.organization_id)
    .fetch_all(&mut **tx)
    .await?;
    let mut attempts = Vec::with_capacity(attempt_rows.len());
    for row in attempt_rows {
        let links: Vec<(i64, String)> = sqlx::query_as(
            "SELECT evidence_id,role FROM candidate_attempt_evidence
             WHERE attempt_id=$1 ORDER BY evidence_id,role",
        )
        .bind(row.attempt_id)
        .fetch_all(&mut **tx)
        .await?;
        let ids_for = |role: &str| {
            links
                .iter()
                .filter(|(_, actual_role)| actual_role == role)
                .map(|(id, _)| *id)
                .collect::<Vec<_>>()
        };
        attempts.push(AttemptTerminalTruthRow {
            candidate_id: row.candidate_id,
            attempt_id: row.attempt_id,
            candidate_plan_hash: row.candidate_plan_hash,
            status: row.status,
            proof_evidence_ids: ids_for("proof"),
            refutation_evidence_ids: ids_for("refutation"),
            blocker_evidence_ids: ids_for("blocker"),
            blocker_reason_code: row.blocker_reason_code,
            finding_id: row.finding_id,
            finding_lineage_exact: row.finding_lineage_exact,
        });
    }
    let residual_risks = sqlx::query_as::<_, ResidualRiskRow>(
        r#"SELECT id AS residual_risk_id,reason_code,disclosure_status
             FROM attack_residual_risks
            WHERE operation_id=$1 AND scope_snapshot_id=$2 AND wave_run_id=$3
              AND wave_unit_id=$4 AND organization_id=$5
            ORDER BY id"#,
    )
    .bind(authority.operation_id)
    .bind(authority.scope_snapshot_id)
    .bind(authority.wave_run_id)
    .bind(authority.wave_unit_id)
    .bind(authority.organization_id)
    .fetch_all(&mut **tx)
    .await?
    .into_iter()
    .map(|row| ResidualRiskTruthRow {
        residual_risk_id: row.residual_risk_id,
        reason_code: row.reason_code,
        disclosure_status: row.disclosure_status,
    })
    .collect();
    Ok(VerificationTruthRow {
        operation_id: authority.operation_id,
        scope_snapshot_id: authority.scope_snapshot_id,
        wave_run_id: authority.wave_run_id,
        wave_unit_id: authority.wave_unit_id,
        organization_id: authority.organization_id,
        review_closed: authority.review_closed,
        pending_work_items: u32::try_from(pending_work_items).map_err(|_| {
            crate::DbError::Other(anyhow::anyhow!("pending work-item count overflow"))
        })?,
        approved_ever: u32::try_from(approved_ever).map_err(|_| {
            crate::DbError::Other(anyhow::anyhow!("approved Candidate count overflow"))
        })?,
        attempts,
        residual_risks,
    })
}
