//! Atomic verified Attempt -> Finding + immutable lineage terminalization.

use chrono::{DateTime, Utc};
use golish_memory_domain::{
    CanonicalRowId, CanonicalSourceKind, KnowledgeEventEnvelopeV1, KnowledgeEventNameV1,
    KnowledgeEventPayloadV1, ProjectScopeId, SourceRef,
};
use sqlx::{Postgres, Transaction};
use uuid::Uuid;

use super::findings::{self, CandidateVerifiedFindingWrite};
use super::{attack_execution_lanes, attack_fact_deltas, attack_waves};

#[derive(Debug, Clone)]
pub struct TerminalizeCandidateAttempt {
    pub operation_id: Uuid,
    pub scope_snapshot_id: Uuid,
    pub wave_run_id: Uuid,
    pub wave_unit_id: Uuid,
    pub organization_id: Uuid,
    pub candidate_id: Uuid,
    pub approval_id: Uuid,
    pub attempt_id: Uuid,
    pub candidate_plan_hash: String,
    pub expected_result_hash: String,
    pub worker_run_id: Uuid,
    pub stage_execution_id: Uuid,
    pub stage_run_unit_id: Uuid,
    pub lease_token: Uuid,
    pub lease_owner: String,
    pub attempt_epoch: i64,
    pub expected_checkpoint_version: i64,
}

#[derive(Debug, Clone)]
pub struct TerminalizeVerifiedFinding {
    pub operation_id: Uuid,
    pub scope_snapshot_id: Uuid,
    pub wave_run_id: Uuid,
    pub wave_unit_id: Uuid,
    pub organization_id: Uuid,
    pub candidate_id: Uuid,
    pub approval_id: Uuid,
    pub attempt_id: Uuid,
    pub candidate_plan_hash: String,
    pub expected_result_hash: String,
    pub proof_evidence_ids: Vec<i64>,
    pub worker_run_id: Uuid,
    pub stage_execution_id: Uuid,
    pub stage_run_unit_id: Uuid,
    pub lease_token: Uuid,
    pub lease_owner: String,
    pub attempt_epoch: i64,
    pub expected_checkpoint_version: i64,
}

/// Server-only recovery terminalization. Every identity and evidence link is
/// reloaded from the immutable `CandidateRecoveryCase`; callers can select
/// neither a target nor a result disposition. Unknown external side effects
/// always converge to `blocked`, never to an inferred verified/refuted fact.
#[derive(Debug, Clone, Copy)]
pub struct TerminalizeBlockedCandidateRecovery {
    pub recovery_case_id: Uuid,
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct TerminalizedFinding {
    pub finding_id: Uuid,
    pub attempt_id: Uuid,
    pub replayed: bool,
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct TerminalizedCandidateAttempt {
    pub scope_snapshot_id: Uuid,
    pub wave_run_id: Uuid,
    pub wave_unit_id: Uuid,
    pub organization_id: Uuid,
    pub candidate_id: Uuid,
    pub attempt_id: Uuid,
    pub status: String,
    /// Compatibility name retained for existing scheduler logging. It is
    /// always identical to `status`.
    pub disposition: String,
    pub finding_id: Option<Uuid>,
    pub evidence_count: u32,
    pub fact_delta_count: u32,
    pub replayed: bool,
}

#[derive(Debug, sqlx::FromRow)]
struct TerminalStateRow {
    candidate_disposition: String,
    terminal_attempt_id: Option<Uuid>,
    terminal_finding_id: Option<Uuid>,
    target_live_id: Option<Uuid>,
    target_type_at_time: String,
    target_value_at_time: String,
    target_identity_hash: String,
    execution_plan: serde_json::Value,
    attempt_status: String,
    stage_worker_run_id: Uuid,
    result_json: Option<serde_json::Value>,
    result_hash: Option<String>,
    attempt_row_version: i64,
    terminal_at: Option<DateTime<Utc>>,
    approval_status: String,
}

#[derive(Debug, sqlx::FromRow)]
struct TerminalAttemptUpdate {
    row_version: i64,
    terminal_at: DateTime<Utc>,
}

async fn append_legacy_attempt_terminal_shadow(
    tx: &mut Transaction<'_, Postgres>,
    mode: golish_core::InvestigationRolloutMode,
    command: &TerminalizeCandidateAttempt,
    row_version: i64,
    terminal_at: DateTime<Utc>,
    disposition: &str,
    result_hash: &str,
) -> crate::Result<()> {
    super::hypothesis_legacy_projection::append_legacy_attempt_shadow_with_connection(
        tx,
        mode,
        command.operation_id,
        terminal_at,
        super::hypothesis_legacy_projection::LegacyAttemptShadowSourceV1 {
            attempt_id: command.attempt_id,
            entity_version: row_version,
            organization_id: command.organization_id,
            candidate_id: command.candidate_id,
            candidate_plan_hash: command.candidate_plan_hash.clone(),
            result_hash: result_hash.to_owned(),
            disposition: disposition.to_owned(),
        },
    )
    .await?;
    Ok(())
}

#[derive(Debug, sqlx::FromRow)]
struct RecoveryTerminalStateRow {
    operation_id: Uuid,
    scope_snapshot_id: Uuid,
    wave_run_id: Uuid,
    wave_unit_id: Uuid,
    organization_id: Uuid,
    candidate_id: Uuid,
    approval_id: Uuid,
    attempt_id: Uuid,
    candidate_plan_hash: String,
    recovery_status: String,
    resolution_kind: Option<String>,
    candidate_disposition: String,
    terminal_attempt_id: Option<Uuid>,
    terminal_finding_id: Option<Uuid>,
    target_live_id: Option<Uuid>,
    target_type_at_time: String,
    target_value_at_time: String,
    target_identity_hash: String,
    execution_plan: serde_json::Value,
    attempt_status: String,
    attempt_row_version: i64,
    worker_run_id: Uuid,
    worker_status: String,
    stage_execution_id: Uuid,
    stage_run_unit_id: Uuid,
    project_scope_id: Uuid,
}

#[derive(Debug, sqlx::FromRow)]
struct FindingReplayRow {
    id: Uuid,
    title: String,
    severity: String,
    cvss: Option<f64>,
    target_id: Option<Uuid>,
    target: String,
    description: String,
    steps: String,
    remediation: String,
    evidence: serde_json::Value,
    project_path: Option<String>,
    source: String,
}

#[derive(Debug)]
struct VerifiedFindingPayload {
    title: String,
    severity: String,
    cvss: Option<f64>,
    affected_target: String,
    description: String,
    steps: String,
    remediation: String,
}

#[derive(Debug)]
struct FactDeltaPayload {
    fact_kind: attack_fact_deltas::AttackFactDeltaKind,
    canonical_ref_kind: String,
    canonical_ref_id: Uuid,
    canonical_ref_version: i64,
    canonical_ref_hash: String,
    _summary: String,
    evidence_ids: Vec<i64>,
}

#[derive(Debug)]
struct TerminalPayload {
    disposition: String,
    proof_evidence_ids: Vec<i64>,
    refutation_evidence_ids: Vec<i64>,
    blocker_evidence_ids: Vec<i64>,
    blocker_reason_code: Option<String>,
    finding: Option<VerifiedFindingPayload>,
    fact_deltas: Vec<FactDeltaPayload>,
}

#[derive(Debug, Clone)]
pub(super) struct FuelExhaustedCandidateTerminalEvent {
    pub project_scope_id: Uuid,
    pub operation_id: Uuid,
    pub organization_id: Uuid,
    pub attempt_id: Uuid,
    pub candidate_id: Uuid,
    pub approval_id: Uuid,
    pub candidate_plan_hash: String,
    pub result_hash: String,
    pub target_type_at_time: String,
    pub target_value_at_time: String,
    pub target_identity_hash: String,
    pub source_version: i64,
    pub occurred_at: DateTime<Utc>,
    pub blocker_evidence_ids: Vec<i64>,
}

pub(super) async fn append_fuel_exhausted_candidate_terminal_event(
    tx: &mut Transaction<'_, Postgres>,
    terminal: &FuelExhaustedCandidateTerminalEvent,
) -> crate::Result<()> {
    let mut evidence_ids = terminal.blocker_evidence_ids.clone();
    evidence_ids.sort_unstable();
    evidence_ids.dedup();
    if evidence_ids.is_empty() || evidence_ids.iter().any(|evidence_id| *evidence_id <= 0) {
        return Err(conflict(
            "fuel-exhausted Candidate terminal event requires exact evidence",
        ));
    }
    let source_stream_key = format!("candidate-attempt:{}", terminal.attempt_id);
    let source = SourceRef {
        source_kind: CanonicalSourceKind::CandidateAttempt,
        row_id: CanonicalRowId::Uuid(terminal.attempt_id),
        source_stream_key: source_stream_key.clone(),
        version: terminal.source_version,
    };
    let event = KnowledgeEventEnvelopeV1 {
        event_id: Uuid::new_v5(
            &terminal.attempt_id,
            KnowledgeEventNameV1::CandidateAttemptTerminal
                .as_str()
                .as_bytes(),
        ),
        project_scope_id: Some(ProjectScopeId(terminal.project_scope_id)),
        organization_id_at_time: Some(terminal.organization_id),
        source_operation_id: terminal.operation_id,
        event_name: KnowledgeEventNameV1::CandidateAttemptTerminal,
        schema_version: 1,
        payload: KnowledgeEventPayloadV1 {
            source,
            source_stream_key,
            source_version: terminal.source_version,
            structured_payload: serde_json::json!({
                "attempt_id": terminal.attempt_id,
                "candidate_id": terminal.candidate_id,
                "approval_id": terminal.approval_id,
                "disposition": "blocked",
                "candidate_plan_hash": terminal.candidate_plan_hash,
                "result_hash": terminal.result_hash,
                "finding_id": serde_json::Value::Null,
                "target_type_at_time": terminal.target_type_at_time,
                "target_value_at_time": terminal.target_value_at_time,
                "target_identity_hash": terminal.target_identity_hash,
                "evidence_ids": evidence_ids,
                "proof_evidence_ids": [],
                "refutation_evidence_ids": [],
                "blocker_evidence_ids": evidence_ids,
                "blocker_reason_code": "max_attempts_total",
                "fact_delta_count": 0,
            }),
        },
        occurred_at: terminal.occurred_at,
    };
    super::knowledge_outbox::append_event_with_catalog_deliveries(tx, &event)
        .await
        .map_err(|error| {
            crate::DbError::Other(anyhow::anyhow!("candidate_terminal_outbox_failed: {error}"))
        })?;
    Ok(())
}

async fn append_candidate_terminal_event(
    tx: &mut Transaction<'_, Postgres>,
    command: &TerminalizeCandidateAttempt,
    project_scope_id: Uuid,
    state: &TerminalStateRow,
    payload: &TerminalPayload,
    finding_id: Option<Uuid>,
    source_version: i64,
    occurred_at: DateTime<Utc>,
    relational_evidence: &[(i64, String)],
    result_hash: &str,
) -> crate::Result<()> {
    let mut evidence_ids = relational_evidence
        .iter()
        .map(|(evidence_id, _)| *evidence_id)
        .collect::<Vec<_>>();
    evidence_ids.sort_unstable();
    evidence_ids.dedup();
    let source_stream_key = format!("candidate-attempt:{}", command.attempt_id);
    let source = SourceRef {
        source_kind: CanonicalSourceKind::CandidateAttempt,
        row_id: CanonicalRowId::Uuid(command.attempt_id),
        source_stream_key: source_stream_key.clone(),
        version: source_version,
    };
    let event = KnowledgeEventEnvelopeV1 {
        event_id: Uuid::new_v5(
            &command.attempt_id,
            KnowledgeEventNameV1::CandidateAttemptTerminal
                .as_str()
                .as_bytes(),
        ),
        project_scope_id: Some(ProjectScopeId(project_scope_id)),
        organization_id_at_time: Some(command.organization_id),
        source_operation_id: command.operation_id,
        event_name: KnowledgeEventNameV1::CandidateAttemptTerminal,
        schema_version: 1,
        payload: KnowledgeEventPayloadV1 {
            source,
            source_stream_key,
            source_version,
            structured_payload: serde_json::json!({
                "attempt_id": command.attempt_id,
                "candidate_id": command.candidate_id,
                "approval_id": command.approval_id,
                "disposition": payload.disposition,
                "candidate_plan_hash": command.candidate_plan_hash,
                "result_hash": result_hash,
                "finding_id": finding_id,
                "target_type_at_time": state.target_type_at_time,
                "target_value_at_time": state.target_value_at_time,
                "target_identity_hash": state.target_identity_hash,
                "evidence_ids": evidence_ids,
                "proof_evidence_ids": payload.proof_evidence_ids,
                "refutation_evidence_ids": payload.refutation_evidence_ids,
                "blocker_evidence_ids": payload.blocker_evidence_ids,
                "blocker_reason_code": payload.blocker_reason_code,
                "fact_delta_count": payload.fact_deltas.len(),
            }),
        },
        occurred_at,
    };
    super::knowledge_outbox::append_event_with_catalog_deliveries(tx, &event)
        .await
        .map_err(|error| {
            crate::DbError::Other(anyhow::anyhow!("candidate_terminal_outbox_failed: {error}"))
        })?;
    Ok(())
}

fn conflict(message: &str) -> crate::DbError {
    crate::DbError::Other(anyhow::anyhow!(message.to_string()))
}

async fn durable_terminal_counts(
    tx: &mut Transaction<'_, Postgres>,
    attempt_id: Uuid,
) -> crate::Result<(u32, u32)> {
    let (evidence_count, fact_delta_count): (i64, i64) = sqlx::query_as(
        r#"SELECT
               (SELECT COUNT(DISTINCT evidence_id)
                  FROM candidate_attempt_evidence
                 WHERE attempt_id=$1),
               (SELECT COUNT(*)
                  FROM attack_fact_deltas
                 WHERE source_attempt_id=$1)"#,
    )
    .bind(attempt_id)
    .fetch_one(&mut **tx)
    .await?;
    let evidence_count = u32::try_from(evidence_count)
        .map_err(|_| conflict("Candidate Attempt evidence count exceeds trace capacity"))?;
    let fact_delta_count = u32::try_from(fact_delta_count)
        .map_err(|_| conflict("Candidate Attempt FactDelta count exceeds trace capacity"))?;
    Ok((evidence_count, fact_delta_count))
}

fn sorted_unique_positive(ids: &[i64]) -> Option<Vec<i64>> {
    let mut ids = ids.to_vec();
    ids.sort_unstable();
    let len = ids.len();
    ids.dedup();
    (ids.len() == len && !ids.is_empty() && ids.iter().all(|id| *id > 0)).then_some(ids)
}

fn required_string(
    object: &serde_json::Map<String, serde_json::Value>,
    key: &str,
) -> Option<String> {
    object
        .get(key)
        .and_then(serde_json::Value::as_str)
        .map(str::trim)
        .filter(|value| !value.is_empty())
        .map(ToOwned::to_owned)
}

fn parse_verified_payload(result: &serde_json::Value) -> crate::Result<VerifiedFindingPayload> {
    let object = result
        .as_object()
        .ok_or_else(|| conflict("Attempt result is not an object"))?;
    if object
        .get("disposition")
        .and_then(serde_json::Value::as_str)
        != Some("verified")
    {
        return Err(conflict("Attempt result is not verified"));
    }
    let finding = object
        .get("finding")
        .and_then(serde_json::Value::as_object)
        .ok_or_else(|| conflict("verified Attempt has no Finding projection"))?;
    let severity = required_string(finding, "severity")
        .filter(|severity| {
            matches!(
                severity.as_str(),
                "critical" | "high" | "medium" | "low" | "info"
            )
        })
        .ok_or_else(|| conflict("verified Finding severity is invalid"))?;
    let reproduction_steps = finding
        .get("reproduction_steps")
        .and_then(serde_json::Value::as_array)
        .and_then(|steps| {
            steps
                .iter()
                .map(serde_json::Value::as_str)
                .map(|step| step.map(str::trim).filter(|step| !step.is_empty()))
                .collect::<Option<Vec<_>>>()
        })
        .filter(|steps| !steps.is_empty())
        .map(|steps| steps.join("\n"))
        .or_else(|| required_string(finding, "steps"))
        .ok_or_else(|| conflict("verified Finding reproduction_steps are missing"))?;
    let cvss = match finding.get("cvss") {
        None | Some(serde_json::Value::Null) => None,
        Some(value) => Some(
            value
                .as_f64()
                .filter(|score| score.is_finite() && (0.0..=10.0).contains(score))
                .ok_or_else(|| conflict("verified Finding cvss is invalid"))?,
        ),
    };
    Ok(VerifiedFindingPayload {
        title: required_string(finding, "title")
            .ok_or_else(|| conflict("verified Finding title is missing"))?,
        severity,
        cvss,
        affected_target: required_string(finding, "affected_target")
            .ok_or_else(|| conflict("verified Finding affected_target is missing"))?,
        description: required_string(finding, "description")
            .ok_or_else(|| conflict("verified Finding description is missing"))?,
        steps: reproduction_steps,
        remediation: required_string(finding, "remediation")
            .ok_or_else(|| conflict("verified Finding remediation is missing"))?,
    })
}

fn parse_evidence_ids(
    object: &serde_json::Map<String, serde_json::Value>,
    key: &str,
) -> crate::Result<Vec<i64>> {
    let Some(items) = object.get(key).and_then(serde_json::Value::as_array) else {
        return Ok(Vec::new());
    };
    let ids = items
        .iter()
        .map(serde_json::Value::as_i64)
        .collect::<Option<Vec<_>>>()
        .ok_or_else(|| conflict(&format!("invalid {key} evidence ids")))?;
    if ids.is_empty() {
        return Ok(ids);
    }
    sorted_unique_positive(&ids).ok_or_else(|| conflict(&format!("invalid {key} evidence ids")))
}

fn parse_terminal_payload(result: &serde_json::Value) -> crate::Result<TerminalPayload> {
    let object = result
        .as_object()
        .ok_or_else(|| conflict("Attempt result is not an object"))?;
    let disposition = required_string(object, "disposition")
        .filter(|value| matches!(value.as_str(), "verified" | "refuted" | "blocked"))
        .ok_or_else(|| conflict("Attempt result disposition is invalid"))?;
    let proof_evidence_ids = parse_evidence_ids(object, "proof_evidence_ids")?;
    let refutation_evidence_ids = parse_evidence_ids(object, "refutation_evidence_ids")?;
    let blocker_evidence_ids = parse_evidence_ids(object, "blocker_evidence_ids")?;
    let blocker_reason_code = object
        .get("blocker_reason_code")
        .and_then(serde_json::Value::as_str)
        .map(str::trim)
        .filter(|value| !value.is_empty())
        .map(ToOwned::to_owned);
    let finding = if disposition == "verified" {
        Some(parse_verified_payload(result)?)
    } else {
        if object.get("finding").is_some_and(|value| !value.is_null()) {
            return Err(conflict("non-verified Attempt cannot carry a Finding"));
        }
        None
    };
    match disposition.as_str() {
        "verified"
            if proof_evidence_ids.is_empty()
                || !refutation_evidence_ids.is_empty()
                || !blocker_evidence_ids.is_empty()
                || blocker_reason_code.is_some() =>
        {
            return Err(conflict("verified Attempt evidence roles are invalid"));
        }
        "refuted"
            if refutation_evidence_ids.is_empty()
                || !proof_evidence_ids.is_empty()
                || !blocker_evidence_ids.is_empty()
                || blocker_reason_code.is_some() =>
        {
            return Err(conflict("refuted Attempt evidence roles are invalid"));
        }
        "blocked"
            if !proof_evidence_ids.is_empty()
                || !refutation_evidence_ids.is_empty()
                || (blocker_evidence_ids.is_empty() && blocker_reason_code.is_none()) =>
        {
            return Err(conflict("blocked Attempt evidence roles are invalid"));
        }
        _ => {}
    }
    let fact_deltas = object
        .get("fact_deltas")
        .and_then(serde_json::Value::as_array)
        .map(|deltas| {
            deltas
                .iter()
                .map(|delta| {
                    let delta = delta
                        .as_object()
                        .ok_or_else(|| conflict("FactDelta is not an object"))?;
                    let canonical_ref_id = required_string(delta, "canonical_ref_id")
                        .and_then(|value| Uuid::parse_str(&value).ok())
                        .filter(|value| !value.is_nil())
                        .ok_or_else(|| conflict("FactDelta canonical_ref_id is invalid"))?;
                    let evidence_ids = parse_evidence_ids(delta, "evidence_ids")?;
                    if evidence_ids.is_empty() {
                        return Err(conflict("FactDelta evidence_ids are required"));
                    }
                    Ok(FactDeltaPayload {
                        fact_kind: delta
                            .get("fact_kind")
                            .and_then(serde_json::Value::as_str)
                            .and_then(attack_fact_deltas::AttackFactDeltaKind::parse)
                            .ok_or_else(|| conflict("FactDelta fact_kind is invalid"))?,
                        canonical_ref_kind: required_string(delta, "canonical_ref_kind")
                            .ok_or_else(|| conflict("FactDelta canonical_ref_kind is required"))?,
                        canonical_ref_id,
                        canonical_ref_version: delta
                            .get("canonical_ref_version")
                            .and_then(serde_json::Value::as_i64)
                            .filter(|version| *version > 0)
                            .ok_or_else(|| {
                                conflict("FactDelta canonical_ref_version is invalid")
                            })?,
                        canonical_ref_hash: required_string(delta, "canonical_ref_hash")
                            .ok_or_else(|| conflict("FactDelta canonical_ref_hash is required"))?,
                        _summary: required_string(delta, "summary")
                            .ok_or_else(|| conflict("FactDelta summary is required"))?,
                        evidence_ids,
                    })
                })
                .collect::<crate::Result<Vec<_>>>()
        })
        .transpose()?
        .unwrap_or_default();
    Ok(TerminalPayload {
        disposition,
        proof_evidence_ids,
        refutation_evidence_ids,
        blocker_evidence_ids,
        blocker_reason_code,
        finding,
        fact_deltas,
    })
}

/// Terminalize one submitted Attempt from its immutable persisted result. The
/// result, approval, action journal and relational evidence links are reloaded
/// under lock; no caller payload can introduce Finding or evidence truth here.
pub async fn terminalize_candidate_attempt(
    tx: &mut Transaction<'_, Postgres>,
    command: TerminalizeCandidateAttempt,
) -> crate::Result<TerminalizedCandidateAttempt> {
    terminalize_candidate_attempt_inner(tx, command, None).await
}

/// Recovery-only entrypoint. Identity, result and checkpoint are loaded from
/// an immutable intent/barrier by `candidate_recovery`. The original executor
/// lease may be expired or already cleared by a generic reaper; no executor
/// authority is resurrected and no external action is replayed.
pub(super) async fn terminalize_candidate_attempt_from_intent(
    tx: &mut Transaction<'_, Postgres>,
    command: TerminalizeCandidateAttempt,
    submitted_result: serde_json::Value,
) -> crate::Result<TerminalizedCandidateAttempt> {
    terminalize_candidate_attempt_inner(tx, command, Some(submitted_result)).await
}

async fn terminalize_candidate_attempt_inner(
    tx: &mut Transaction<'_, Postgres>,
    command: TerminalizeCandidateAttempt,
    intent_result: Option<serde_json::Value>,
) -> crate::Result<TerminalizedCandidateAttempt> {
    let server_authority = intent_result.is_some();
    let investigation_mode =
        super::candidate_attempts::lock_v2_operation(tx, command.operation_id).await?;
    let contracts: Option<(String, String, Uuid)> = sqlx::query_as(
        "SELECT runtime_memory_contract,attack_execution_contract,project_scope_id
         FROM operation_state WHERE operation_id=$1 FOR UPDATE",
    )
    .bind(command.operation_id)
    .fetch_optional(&mut **tx)
    .await?;
    let (runtime_contract, attack_contract, project_scope_id) =
        contracts.ok_or_else(|| crate::DbError::NotFound("operation_state".to_string()))?;
    if runtime_contract != "v2_only" || attack_contract != "v2_only" {
        return Err(conflict(
            "operation contracts do not authorize V2 terminalization",
        ));
    }
    attack_waves::lock_wave(
        tx,
        command.operation_id,
        command.scope_snapshot_id,
        command.wave_run_id,
    )
    .await?;
    attack_waves::lock_wave_unit(
        tx,
        command.operation_id,
        command.scope_snapshot_id,
        command.wave_run_id,
        command.wave_unit_id,
        command.organization_id,
    )
    .await?;
    let lane = attack_execution_lanes::lock_global(tx).await?;
    let state = sqlx::query_as::<_, TerminalStateRow>(
        r#"SELECT candidate.disposition AS candidate_disposition,
                  candidate.terminal_attempt_id,candidate.terminal_finding_id,
                  candidate.target_live_id,candidate.target_type_at_time,
                  candidate.target_value_at_time,candidate.target_identity_hash,
                  candidate.execution_plan,
                  attempt.status AS attempt_status,attempt.stage_worker_run_id,
                  attempt.result_json,attempt.result_hash,
                  attempt.row_version AS attempt_row_version,attempt.terminal_at,
                  approval.status AS approval_status
             FROM attack_candidates candidate
             JOIN attack_candidate_approvals approval
               ON approval.id=$2 AND approval.candidate_id=candidate.candidate_id
              AND approval.operation_id=candidate.operation_uuid
              AND approval.scope_snapshot_id=candidate.scope_snapshot_id
              AND approval.wave_run_id=candidate.wave_run_id
              AND approval.wave_unit_id=candidate.wave_unit_id
              AND approval.organization_id=candidate.organization_id
              AND approval.candidate_plan_hash=candidate.candidate_plan_hash
             JOIN candidate_attempts attempt
               ON attempt.id=$3 AND attempt.candidate_id=candidate.candidate_id
              AND attempt.approval_id=approval.id
              AND attempt.operation_id=candidate.operation_uuid
              AND attempt.scope_snapshot_id=candidate.scope_snapshot_id
              AND attempt.wave_run_id=candidate.wave_run_id
              AND attempt.wave_unit_id=candidate.wave_unit_id
              AND attempt.organization_id=candidate.organization_id
              AND attempt.candidate_plan_hash=candidate.candidate_plan_hash
            WHERE candidate.candidate_id=$1 AND candidate.operation_uuid=$4
              AND candidate.scope_snapshot_id=$5 AND candidate.wave_run_id=$6
              AND candidate.wave_unit_id=$7 AND candidate.organization_id=$8
              AND candidate.candidate_plan_hash=$9
            FOR UPDATE OF candidate,approval,attempt"#,
    )
    .bind(command.candidate_id)
    .bind(command.approval_id)
    .bind(command.attempt_id)
    .bind(command.operation_id)
    .bind(command.scope_snapshot_id)
    .bind(command.wave_run_id)
    .bind(command.wave_unit_id)
    .bind(command.organization_id)
    .bind(&command.candidate_plan_hash)
    .fetch_optional(&mut **tx)
    .await?
    .ok_or_else(|| crate::DbError::NotFound("verified_candidate_attempt".to_string()))?;
    if state
        .result_hash
        .as_deref()
        .is_some_and(|hash| hash != command.expected_result_hash)
        || (!server_authority
            && state.result_hash.as_deref() != Some(&command.expected_result_hash))
    {
        return Err(conflict("terminalization result hash drift"));
    }
    let result_json = match (intent_result.as_ref(), state.result_json.as_ref()) {
        (Some(intent), Some(persisted)) if intent != persisted => {
            return Err(conflict("terminalization persisted result drift"));
        }
        (Some(intent), _) => intent,
        (None, Some(persisted)) => persisted,
        (None, None) => return Err(conflict("terminalization result is missing")),
    };
    let payload = parse_terminal_payload(result_json)?;
    if payload
        .finding
        .as_ref()
        .is_some_and(|finding| finding.affected_target != state.target_value_at_time)
    {
        return Err(conflict(
            "verified Finding affected_target does not match frozen Candidate target",
        ));
    }
    let actual_evidence: Vec<(i64, String)> = sqlx::query_as(
        "SELECT evidence_id,role FROM candidate_attempt_evidence
         WHERE attempt_id=$1 ORDER BY evidence_id,role",
    )
    .bind(command.attempt_id)
    .fetch_all(&mut **tx)
    .await?;
    let mut expected_evidence = payload
        .proof_evidence_ids
        .iter()
        .map(|id| (*id, "proof".to_string()))
        .chain(
            payload
                .refutation_evidence_ids
                .iter()
                .map(|id| (*id, "refutation".to_string())),
        )
        .chain(
            payload
                .blocker_evidence_ids
                .iter()
                .map(|id| (*id, "blocker".to_string())),
        )
        .chain(payload.fact_deltas.iter().flat_map(|delta| {
            delta
                .evidence_ids
                .iter()
                .map(|id| (*id, "fact_delta".to_string()))
        }))
        .collect::<Vec<_>>();
    expected_evidence.sort();
    expected_evidence.dedup();
    if expected_evidence != actual_evidence {
        return Err(conflict(
            "terminalization result does not match relational evidence roles",
        ));
    }
    let project_path: String = sqlx::query_scalar(
        "SELECT canonical_project_path FROM project_scopes
         WHERE project_scope_id=$1 AND retired_at IS NULL FOR SHARE",
    )
    .bind(project_scope_id)
    .fetch_one(&mut **tx)
    .await?;
    let evidence_json = serde_json::Value::Array(
        payload
            .proof_evidence_ids
            .iter()
            .copied()
            .map(serde_json::Value::from)
            .collect(),
    );

    if matches!(
        state.attempt_status.as_str(),
        "verified" | "refuted" | "blocked"
    ) || state.terminal_attempt_id.is_some()
        || state.terminal_finding_id.is_some()
    {
        if state.attempt_status != payload.disposition
            || state.terminal_at.is_none()
            || state.candidate_disposition != payload.disposition
            || state.terminal_attempt_id != Some(command.attempt_id)
        {
            return Err(conflict("partial Candidate terminal state"));
        }
        if payload.disposition != "verified" {
            if state.terminal_finding_id.is_some() {
                return Err(conflict("non-verified Candidate has a terminal Finding"));
            }
            append_candidate_terminal_event(
                tx,
                &command,
                project_scope_id,
                &state,
                &payload,
                None,
                state.attempt_row_version,
                state
                    .terminal_at
                    .ok_or_else(|| conflict("terminal Attempt timestamp missing"))?,
                &actual_evidence,
                &command.expected_result_hash,
            )
            .await?;
            append_legacy_attempt_terminal_shadow(
                tx,
                investigation_mode,
                &command,
                state.attempt_row_version,
                state
                    .terminal_at
                    .ok_or_else(|| conflict("terminal Attempt timestamp missing"))?,
                &payload.disposition,
                &command.expected_result_hash,
            )
            .await?;
            let (evidence_count, fact_delta_count) =
                durable_terminal_counts(tx, command.attempt_id).await?;
            return Ok(TerminalizedCandidateAttempt {
                scope_snapshot_id: command.scope_snapshot_id,
                wave_run_id: command.wave_run_id,
                wave_unit_id: command.wave_unit_id,
                organization_id: command.organization_id,
                candidate_id: command.candidate_id,
                attempt_id: command.attempt_id,
                status: payload.disposition.clone(),
                disposition: payload.disposition,
                finding_id: None,
                evidence_count,
                fact_delta_count,
                replayed: true,
            });
        }
        let finding_id = state
            .terminal_finding_id
            .ok_or_else(|| conflict("partial Candidate terminal state"))?;
        let finding_payload = payload
            .finding
            .as_ref()
            .ok_or_else(|| conflict("verified Candidate has no Finding payload"))?;
        let persisted = sqlx::query_as::<_, FindingReplayRow>(
            r#"SELECT finding.id,finding.title,finding.sev::TEXT AS severity,
                      finding.cvss,finding.target_id,finding.target,finding.description,finding.steps,
                      finding.remediation,finding.evidence,finding.project_path,finding.source
                 FROM findings finding
                 JOIN finding_lineage lineage ON lineage.finding_id=finding.id
                WHERE finding.id=$1 AND lineage.candidate_attempt_id=$2
                  AND lineage.candidate_id=$3 AND lineage.operation_id=$4
                  AND lineage.scope_snapshot_id=$5 AND lineage.wave_run_id=$6
                  AND lineage.wave_unit_id=$7 AND lineage.organization_id=$8
                  AND lineage.candidate_plan_hash=$9
                FOR UPDATE OF finding,lineage"#,
        )
        .bind(finding_id)
        .bind(command.attempt_id)
        .bind(command.candidate_id)
        .bind(command.operation_id)
        .bind(command.scope_snapshot_id)
        .bind(command.wave_run_id)
        .bind(command.wave_unit_id)
        .bind(command.organization_id)
        .bind(&command.candidate_plan_hash)
        .fetch_optional(&mut **tx)
        .await?
        .ok_or_else(|| conflict("terminal Finding lineage missing"))?;
        if persisted.id != finding_id
            || persisted.title != finding_payload.title
            || persisted.severity != finding_payload.severity
            || persisted.cvss != finding_payload.cvss
            || persisted.target_id != state.target_live_id
            || persisted.target != state.target_value_at_time
            || persisted.description != finding_payload.description
            || persisted.steps != finding_payload.steps
            || persisted.remediation != finding_payload.remediation
            || persisted.evidence != evidence_json
            || persisted.project_path.as_deref() != Some(project_path.as_str())
            || persisted.source != "candidate_v2"
        {
            return Err(conflict("terminal Finding replay payload drift"));
        }
        append_candidate_terminal_event(
            tx,
            &command,
            project_scope_id,
            &state,
            &payload,
            Some(finding_id),
            state.attempt_row_version,
            state
                .terminal_at
                .ok_or_else(|| conflict("terminal Attempt timestamp missing"))?,
            &actual_evidence,
            &command.expected_result_hash,
        )
        .await?;
        append_legacy_attempt_terminal_shadow(
            tx,
            investigation_mode,
            &command,
            state.attempt_row_version,
            state
                .terminal_at
                .ok_or_else(|| conflict("terminal Attempt timestamp missing"))?,
            &payload.disposition,
            &command.expected_result_hash,
        )
        .await?;
        let (evidence_count, fact_delta_count) =
            durable_terminal_counts(tx, command.attempt_id).await?;
        return Ok(TerminalizedCandidateAttempt {
            scope_snapshot_id: command.scope_snapshot_id,
            wave_run_id: command.wave_run_id,
            wave_unit_id: command.wave_unit_id,
            organization_id: command.organization_id,
            candidate_id: command.candidate_id,
            attempt_id: command.attempt_id,
            status: payload.disposition.clone(),
            disposition: payload.disposition,
            finding_id: Some(finding_id),
            evidence_count,
            fact_delta_count,
            replayed: true,
        });
    }

    let expected_attempt_status = if server_authority {
        "terminalization_pending"
    } else {
        "submitted"
    };
    let lane_owned_by_original_executor = lane.stage_worker_run_id == Some(command.worker_run_id)
        && lane.lease_token == Some(command.lease_token)
        && lane.lease_owner.as_deref() == Some(command.lease_owner.as_str());
    let server_lane_identity_drift = server_authority
        && lane.stage_worker_run_id == Some(command.worker_run_id)
        && !lane_owned_by_original_executor;
    if state.attempt_status != expected_attempt_status
        || state.terminal_at.is_some()
        || state.candidate_disposition != "approved"
        || !matches!(state.approval_status.as_str(), "approved" | "expired")
        || state.stage_worker_run_id != command.worker_run_id
        || (!server_authority && !lane_owned_by_original_executor)
        || server_lane_identity_drift
        || (!server_authority
            && lane
                .lease_expires_at
                .is_none_or(|expires| expires <= Utc::now()))
        || (server_authority && (state.result_json.is_some() || state.result_hash.is_some()))
    {
        return Err(conflict("Candidate terminalization authority fence lost"));
    }
    let worker: Option<Uuid> = sqlx::query_scalar(
        "SELECT id FROM stage_worker_runs
         WHERE id=$1 AND operation_id=$2 AND stage_execution_id=$3
           AND stage_run_unit_id=$4 AND organization_id=$5
           AND attempt_epoch=$8 AND checkpoint_version=$9
           AND active_tool_call_id IS NULL
           AND (
               (NOT $10 AND status='running' AND lease_token=$6
                        AND lease_owner=$7 AND lease_expires_at>NOW())
               OR
               ($10 AND status IN ('running','queued','recovery_required') AND (
                    (lease_token=$6 AND lease_owner=$7)
                    OR
                    (lease_token IS NULL AND lease_owner IS NULL
                     AND lease_acquired_at IS NULL AND lease_expires_at IS NULL
                     AND heartbeat_at IS NULL)
               ))
           )
         FOR UPDATE",
    )
    .bind(command.worker_run_id)
    .bind(command.operation_id)
    .bind(command.stage_execution_id)
    .bind(command.stage_run_unit_id)
    .bind(command.organization_id)
    .bind(command.lease_token)
    .bind(&command.lease_owner)
    .bind(command.attempt_epoch)
    .bind(command.expected_checkpoint_version)
    .bind(server_authority)
    .fetch_optional(&mut **tx)
    .await?;
    if worker.is_none() {
        return Err(conflict("Candidate terminalization WorkerRun fence lost"));
    }

    super::candidate_attempts::validate_terminal_action_journal(
        tx,
        command.attempt_id,
        &state.execution_plan,
    )
    .await?;

    let attempt_updated = if server_authority {
        sqlx::query_as::<_, TerminalAttemptUpdate>(
            "UPDATE candidate_attempts
             SET status=$3,result_json=$4,result_hash=$5,terminal_at=NOW(),
                 row_version=row_version+1,updated_at=NOW()
             WHERE id=$1 AND status='terminalization_pending'
               AND stage_worker_run_id=$2
               AND result_json IS NULL AND result_hash IS NULL
             RETURNING row_version,terminal_at",
        )
        .bind(command.attempt_id)
        .bind(command.worker_run_id)
        .bind(&payload.disposition)
        .bind(result_json)
        .bind(&command.expected_result_hash)
        .fetch_optional(&mut **tx)
        .await?
    } else {
        sqlx::query_as::<_, TerminalAttemptUpdate>(
            "UPDATE candidate_attempts SET status=$3,terminal_at=NOW(),
                 row_version=row_version+1,updated_at=NOW()
             WHERE id=$1 AND status='submitted' AND stage_worker_run_id=$2
             RETURNING row_version,terminal_at",
        )
        .bind(command.attempt_id)
        .bind(command.worker_run_id)
        .bind(&payload.disposition)
        .fetch_optional(&mut **tx)
        .await?
    };
    let attempt_updated =
        attempt_updated.ok_or_else(|| conflict("Candidate terminalization Attempt CAS lost"))?;
    let finding_id = if let Some(finding_payload) = payload.finding.as_ref() {
        let finding_id = Uuid::new_v4();
        findings::insert_verified_candidate_with_executor(
            &mut **tx,
            golish_pentest_domain::FindingWriteContext::VerificationTerminalizer {
                attempt_id: command.attempt_id,
            },
            &CandidateVerifiedFindingWrite {
                id: finding_id,
                title: finding_payload.title.clone(),
                severity: finding_payload.severity.clone(),
                cvss: finding_payload.cvss,
                target_live_id: state.target_live_id,
                target_value_at_time: state.target_value_at_time.clone(),
                description: finding_payload.description.clone(),
                steps: finding_payload.steps.clone(),
                remediation: finding_payload.remediation.clone(),
                evidence: evidence_json,
                project_path,
            },
        )
        .await?;
        sqlx::query(
            r#"INSERT INTO finding_lineage(
               id,finding_id,candidate_attempt_id,candidate_id,operation_id,
               scope_snapshot_id,wave_run_id,wave_unit_id,organization_id,target_live_id,
               target_type_at_time,target_value_at_time,target_identity_hash,candidate_plan_hash)
           VALUES($1,$2,$3,$4,$5,$6,$7,$8,$9,$10,$11,$12,$13,$14)"#,
        )
        .bind(Uuid::new_v4())
        .bind(finding_id)
        .bind(command.attempt_id)
        .bind(command.candidate_id)
        .bind(command.operation_id)
        .bind(command.scope_snapshot_id)
        .bind(command.wave_run_id)
        .bind(command.wave_unit_id)
        .bind(command.organization_id)
        .bind(state.target_live_id)
        .bind(&state.target_type_at_time)
        .bind(&state.target_value_at_time)
        .bind(&state.target_identity_hash)
        .bind(&command.candidate_plan_hash)
        .execute(&mut **tx)
        .await?;
        Some(finding_id)
    } else {
        None
    };

    for delta in &payload.fact_deltas {
        let dedupe_hash = attack_fact_deltas::semantic_dedupe_hash(
            &state.target_identity_hash,
            &delta.canonical_ref_kind,
            delta.canonical_ref_id,
            delta.canonical_ref_version,
            &delta.canonical_ref_hash,
            delta.fact_kind.as_str(),
        )?;
        attack_fact_deltas::propose_fact_delta(
            tx,
            attack_fact_deltas::ProposeAttackFactDelta {
                operation_id: command.operation_id,
                scope_snapshot_id: command.scope_snapshot_id,
                wave_run_id: command.wave_run_id,
                wave_unit_id: command.wave_unit_id,
                organization_id: command.organization_id,
                source_attempt_id: command.attempt_id,
                candidate_id: command.candidate_id,
                candidate_plan_hash: command.candidate_plan_hash.clone(),
                canonical_ref_kind: delta.canonical_ref_kind.clone(),
                canonical_ref_id: delta.canonical_ref_id,
                canonical_ref_version: delta.canonical_ref_version,
                canonical_ref_hash: delta.canonical_ref_hash.clone(),
                delta_kind: delta.fact_kind.as_str().to_string(),
                dedupe_hash,
                evidence_ids: delta.evidence_ids.clone(),
            },
        )
        .await?;
    }
    let candidate_updated = sqlx::query(
        "UPDATE attack_candidates SET disposition=$4,terminal_attempt_id=$2,
             terminal_finding_id=$3,row_version=row_version+1,updated_at=NOW()
         WHERE candidate_id=$1 AND disposition='approved'
           AND terminal_attempt_id IS NULL AND terminal_finding_id IS NULL",
    )
    .bind(command.candidate_id)
    .bind(command.attempt_id)
    .bind(finding_id)
    .bind(&payload.disposition)
    .execute(&mut **tx)
    .await?;
    if candidate_updated.rows_affected() != 1 {
        return Err(conflict("Candidate terminalization Candidate CAS lost"));
    }
    if lane.stage_worker_run_id == Some(command.worker_run_id) {
        // A retained expired lane can be released only under its exact original
        // identity. If a generic reaper already cleared it (or another Worker
        // currently owns the global lane), terminal truth must not mutate that
        // unrelated current owner.
        attack_execution_lanes::release_global(
            tx,
            command.worker_run_id,
            command.lease_token,
            &command.lease_owner,
        )
        .await?;
    }
    let worker_updated = sqlx::query(
        "UPDATE stage_worker_runs
         SET status='passed',lease_token=NULL,lease_owner=NULL,lease_acquired_at=NULL,
             lease_expires_at=NULL,heartbeat_at=NULL,terminal_at=NOW(),updated_at=NOW(),
             checkpoint_version=checkpoint_version+1
         WHERE id=$1 AND operation_id=$2 AND stage_execution_id=$3
           AND stage_run_unit_id=$4 AND organization_id=$5
           AND attempt_epoch=$7 AND checkpoint_version=$8
           AND active_tool_call_id IS NULL
           AND (
               (NOT $9 AND status='running' AND lease_token=$6 AND lease_owner=$10)
               OR
               ($9 AND status IN ('running','queued','recovery_required') AND (
                    (lease_token=$6 AND lease_owner=$10)
                    OR
                    (lease_token IS NULL AND lease_owner IS NULL
                     AND lease_acquired_at IS NULL AND lease_expires_at IS NULL
                     AND heartbeat_at IS NULL)
               ))
           )",
    )
    .bind(command.worker_run_id)
    .bind(command.operation_id)
    .bind(command.stage_execution_id)
    .bind(command.stage_run_unit_id)
    .bind(command.organization_id)
    .bind(command.lease_token)
    .bind(command.attempt_epoch)
    .bind(command.expected_checkpoint_version)
    .bind(server_authority)
    .bind(&command.lease_owner)
    .execute(&mut **tx)
    .await?;
    if worker_updated.rows_affected() != 1 {
        return Err(conflict("Candidate terminalization WorkerRun CAS lost"));
    }
    append_candidate_terminal_event(
        tx,
        &command,
        project_scope_id,
        &state,
        &payload,
        finding_id,
        attempt_updated.row_version,
        attempt_updated.terminal_at,
        &actual_evidence,
        &command.expected_result_hash,
    )
    .await?;
    append_legacy_attempt_terminal_shadow(
        tx,
        investigation_mode,
        &command,
        attempt_updated.row_version,
        attempt_updated.terminal_at,
        &payload.disposition,
        &command.expected_result_hash,
    )
    .await?;
    let (evidence_count, fact_delta_count) =
        durable_terminal_counts(tx, command.attempt_id).await?;
    Ok(TerminalizedCandidateAttempt {
        scope_snapshot_id: command.scope_snapshot_id,
        wave_run_id: command.wave_run_id,
        wave_unit_id: command.wave_unit_id,
        organization_id: command.organization_id,
        candidate_id: command.candidate_id,
        attempt_id: command.attempt_id,
        status: payload.disposition.clone(),
        disposition: payload.disposition,
        finding_id,
        evidence_count,
        fact_delta_count,
        replayed: false,
    })
}

/// Converge an operator-approved outcome-unknown recovery case without ever
/// replaying the external action. The recovery case itself is the immutable
/// authority/receipt. Exact external evidence may be attached as blocker
/// evidence, but it cannot be promoted to proof/refutation without a typed
/// action result and comparator.
pub(super) async fn terminalize_blocked_candidate_recovery(
    tx: &mut Transaction<'_, Postgres>,
    command: TerminalizeBlockedCandidateRecovery,
) -> crate::Result<TerminalizedCandidateAttempt> {
    if command.recovery_case_id.is_nil() {
        return Err(conflict("invalid Candidate recovery terminalization"));
    }
    let state = sqlx::query_as::<_, RecoveryTerminalStateRow>(
        r#"SELECT recovery.operation_id,recovery.scope_snapshot_id,
                  recovery.wave_run_id,recovery.wave_unit_id,recovery.organization_id,
                  recovery.candidate_id,recovery.approval_id,recovery.attempt_id,
                  recovery.candidate_plan_hash,recovery.status AS recovery_status,
                  recovery.resolution_kind,
                  candidate.disposition AS candidate_disposition,
                  candidate.terminal_attempt_id,candidate.terminal_finding_id,
                  candidate.target_live_id,candidate.target_type_at_time,
                  candidate.target_value_at_time,candidate.target_identity_hash,
                  candidate.execution_plan,attempt.status AS attempt_status,
                  attempt.row_version AS attempt_row_version,
                  worker.id AS worker_run_id,worker.status AS worker_status,
                  worker.stage_execution_id,worker.stage_run_unit_id,
                  operation.project_scope_id
             FROM candidate_recovery_cases recovery
             JOIN operation_state operation
               ON operation.operation_id=recovery.operation_id
              AND operation.runtime_memory_contract='v2_only'
              AND operation.attack_execution_contract='v2_only'
             JOIN attack_candidates candidate
               ON candidate.candidate_id=recovery.candidate_id
              AND candidate.operation_uuid=recovery.operation_id
              AND candidate.scope_snapshot_id=recovery.scope_snapshot_id
              AND candidate.wave_run_id=recovery.wave_run_id
              AND candidate.wave_unit_id=recovery.wave_unit_id
              AND candidate.organization_id=recovery.organization_id
              AND candidate.candidate_plan_hash=recovery.candidate_plan_hash
             JOIN candidate_attempts attempt
               ON attempt.id=recovery.attempt_id
              AND attempt.candidate_id=recovery.candidate_id
              AND attempt.approval_id=recovery.approval_id
              AND attempt.stage_worker_run_id=recovery.worker_run_id
             JOIN stage_worker_runs worker
               ON worker.id=recovery.worker_run_id
              AND worker.operation_id=recovery.operation_id
              AND worker.organization_id=recovery.organization_id
            WHERE recovery.id=$1
            FOR UPDATE OF recovery,candidate,attempt,worker,operation"#,
    )
    .bind(command.recovery_case_id)
    .fetch_optional(&mut **tx)
    .await?
    .ok_or_else(|| crate::DbError::NotFound("candidate_recovery_case".to_string()))?;
    let investigation_mode =
        super::candidate_attempts::lock_v2_operation(tx, state.operation_id).await?;
    if state.recovery_status != "decision_recorded"
        || !matches!(
            state.resolution_kind.as_deref(),
            Some("terminalize_blocked_outcome_unknown")
                | Some("accept_external_result_with_exact_evidence")
        )
        || state.candidate_disposition != "approved"
        || state.terminal_attempt_id.is_some()
        || state.terminal_finding_id.is_some()
        || state.attempt_status != "running"
        || state.worker_status != "recovery_required"
    {
        return Err(conflict(
            "Candidate recovery is not eligible for blocked terminalization",
        ));
    }
    attack_waves::lock_wave(
        tx,
        state.operation_id,
        state.scope_snapshot_id,
        state.wave_run_id,
    )
    .await?;
    attack_waves::lock_wave_unit(
        tx,
        state.operation_id,
        state.scope_snapshot_id,
        state.wave_run_id,
        state.wave_unit_id,
        state.organization_id,
    )
    .await?;
    let lane = attack_execution_lanes::lock_global(tx).await?;
    if lane.stage_worker_run_id == Some(state.worker_run_id) {
        return Err(conflict(
            "Candidate recovery terminalization requires the expired lane to be released",
        ));
    }

    let mut blocker_evidence_ids: Vec<i64> = sqlx::query_scalar(
        "SELECT evidence_id FROM candidate_recovery_evidence
          WHERE recovery_case_id=$1 AND role='external_result'
          ORDER BY evidence_id",
    )
    .bind(command.recovery_case_id)
    .fetch_all(&mut **tx)
    .await?;
    blocker_evidence_ids.sort_unstable();
    blocker_evidence_ids.dedup();
    if state.resolution_kind.as_deref() == Some("accept_external_result_with_exact_evidence")
        && blocker_evidence_ids.is_empty()
    {
        return Err(conflict(
            "external Candidate recovery has no exact evidence",
        ));
    }
    for evidence_id in &blocker_evidence_ids {
        sqlx::query(
            "INSERT INTO candidate_attempt_evidence(attempt_id,evidence_id,role)
             VALUES($1,$2,'blocker') ON CONFLICT DO NOTHING",
        )
        .bind(state.attempt_id)
        .bind(evidence_id)
        .execute(&mut **tx)
        .await?;
    }
    let blocker_reason_code = match state.resolution_kind.as_deref() {
        Some("accept_external_result_with_exact_evidence") => {
            "operator_external_result_recorded_without_typed_comparator"
        }
        _ => "operator_outcome_unknown",
    };
    let result_json = serde_json::json!({
        "blocker_evidence_ids": blocker_evidence_ids,
        "blocker_reason_code": blocker_reason_code,
        "disposition": "blocked",
        "fact_deltas": [],
        "finding": null,
        "proof_evidence_ids": [],
        "recovery_case_id": command.recovery_case_id,
        "refutation_evidence_ids": [],
        "schema_version": 1,
    });
    let result_hash = format!(
        "sha256:{}",
        super::operation_scope_decisions::sha256_json(&result_json)
    );
    let attempt_updated = sqlx::query_as::<_, TerminalAttemptUpdate>(
        "UPDATE candidate_attempts
         SET status='blocked',result_json=$3,result_hash=$4,
             terminal_at=NOW(),row_version=row_version+1,updated_at=NOW()
         WHERE id=$1 AND stage_worker_run_id=$2 AND status='running'
           AND row_version=$5 AND result_json IS NULL AND result_hash IS NULL
         RETURNING row_version,terminal_at",
    )
    .bind(state.attempt_id)
    .bind(state.worker_run_id)
    .bind(&result_json)
    .bind(&result_hash)
    .bind(state.attempt_row_version)
    .fetch_optional(&mut **tx)
    .await?
    .ok_or_else(|| conflict("Candidate recovery Attempt terminalization CAS lost"))?;
    let candidate_updated = sqlx::query(
        "UPDATE attack_candidates
         SET disposition='blocked',terminal_attempt_id=$2,terminal_finding_id=NULL,
             row_version=row_version+1,updated_at=NOW()
         WHERE candidate_id=$1 AND operation_uuid=$3 AND disposition='approved'
           AND terminal_attempt_id IS NULL AND terminal_finding_id IS NULL",
    )
    .bind(state.candidate_id)
    .bind(state.attempt_id)
    .bind(state.operation_id)
    .execute(&mut **tx)
    .await?;
    if candidate_updated.rows_affected() != 1 {
        return Err(conflict(
            "Candidate recovery Candidate terminalization CAS lost",
        ));
    }
    let worker_updated = sqlx::query(
        "UPDATE stage_worker_runs
         SET status='exhausted',terminal_at=NOW(),updated_at=NOW()
         WHERE id=$1 AND status='recovery_required'
           AND lease_token IS NULL AND active_tool_call_id IS NULL",
    )
    .bind(state.worker_run_id)
    .execute(&mut **tx)
    .await?;
    if worker_updated.rows_affected() != 1 {
        return Err(conflict(
            "Candidate recovery Worker terminalization CAS lost",
        ));
    }
    let terminal_state = TerminalStateRow {
        candidate_disposition: "blocked".to_string(),
        terminal_attempt_id: Some(state.attempt_id),
        terminal_finding_id: None,
        target_live_id: state.target_live_id,
        target_type_at_time: state.target_type_at_time,
        target_value_at_time: state.target_value_at_time,
        target_identity_hash: state.target_identity_hash,
        execution_plan: state.execution_plan,
        attempt_status: "blocked".to_string(),
        stage_worker_run_id: state.worker_run_id,
        result_json: Some(result_json.clone()),
        result_hash: Some(result_hash.clone()),
        attempt_row_version: attempt_updated.row_version,
        terminal_at: Some(attempt_updated.terminal_at),
        approval_status: "approved".to_string(),
    };
    let payload = TerminalPayload {
        disposition: "blocked".to_string(),
        proof_evidence_ids: Vec::new(),
        refutation_evidence_ids: Vec::new(),
        blocker_evidence_ids: blocker_evidence_ids.clone(),
        blocker_reason_code: Some(blocker_reason_code.to_string()),
        finding: None,
        fact_deltas: Vec::new(),
    };
    let event_command = TerminalizeCandidateAttempt {
        operation_id: state.operation_id,
        scope_snapshot_id: state.scope_snapshot_id,
        wave_run_id: state.wave_run_id,
        wave_unit_id: state.wave_unit_id,
        organization_id: state.organization_id,
        candidate_id: state.candidate_id,
        approval_id: state.approval_id,
        attempt_id: state.attempt_id,
        candidate_plan_hash: state.candidate_plan_hash,
        expected_result_hash: result_hash.clone(),
        worker_run_id: state.worker_run_id,
        stage_execution_id: state.stage_execution_id,
        stage_run_unit_id: state.stage_run_unit_id,
        lease_token: Uuid::nil(),
        lease_owner: "server:candidate-recovery".to_string(),
        attempt_epoch: 0,
        expected_checkpoint_version: 0,
    };
    let relational_evidence = blocker_evidence_ids
        .iter()
        .map(|evidence_id| (*evidence_id, "blocker".to_string()))
        .collect::<Vec<_>>();
    append_candidate_terminal_event(
        tx,
        &event_command,
        state.project_scope_id,
        &terminal_state,
        &payload,
        None,
        attempt_updated.row_version,
        attempt_updated.terminal_at,
        &relational_evidence,
        &result_hash,
    )
    .await?;
    append_legacy_attempt_terminal_shadow(
        tx,
        investigation_mode,
        &event_command,
        attempt_updated.row_version,
        attempt_updated.terminal_at,
        "blocked",
        &result_hash,
    )
    .await?;
    Ok(TerminalizedCandidateAttempt {
        scope_snapshot_id: state.scope_snapshot_id,
        wave_run_id: state.wave_run_id,
        wave_unit_id: state.wave_unit_id,
        organization_id: state.organization_id,
        candidate_id: state.candidate_id,
        attempt_id: state.attempt_id,
        status: "blocked".to_string(),
        disposition: "blocked".to_string(),
        finding_id: None,
        evidence_count: u32::try_from(blocker_evidence_ids.len())
            .map_err(|_| conflict("Candidate recovery evidence count overflow"))?,
        fact_delta_count: 0,
        replayed: false,
    })
}

/// Compatibility adapter for the persisted verified-only contract used before
/// Task 8 generalized the terminalizer. It retains the caller proof assertion,
/// while the generic terminalizer remains authoritative for all three terminal
/// dispositions.
pub async fn terminalize_verified_finding(
    tx: &mut Transaction<'_, Postgres>,
    command: TerminalizeVerifiedFinding,
) -> crate::Result<TerminalizedFinding> {
    let expected_proof = sorted_unique_positive(&command.proof_evidence_ids)
        .ok_or_else(|| conflict("terminalization proof assertion is invalid"))?;
    let actual_proof: Vec<i64> = sqlx::query_scalar(
        "SELECT evidence_id FROM candidate_attempt_evidence
         WHERE attempt_id=$1 AND role='proof' ORDER BY evidence_id",
    )
    .bind(command.attempt_id)
    .fetch_all(&mut **tx)
    .await?;
    if expected_proof != actual_proof {
        return Err(conflict(
            "terminalization proof does not match relational evidence",
        ));
    }
    let terminal = terminalize_candidate_attempt(
        tx,
        TerminalizeCandidateAttempt {
            operation_id: command.operation_id,
            scope_snapshot_id: command.scope_snapshot_id,
            wave_run_id: command.wave_run_id,
            wave_unit_id: command.wave_unit_id,
            organization_id: command.organization_id,
            candidate_id: command.candidate_id,
            approval_id: command.approval_id,
            attempt_id: command.attempt_id,
            candidate_plan_hash: command.candidate_plan_hash,
            expected_result_hash: command.expected_result_hash,
            worker_run_id: command.worker_run_id,
            stage_execution_id: command.stage_execution_id,
            stage_run_unit_id: command.stage_run_unit_id,
            lease_token: command.lease_token,
            lease_owner: command.lease_owner,
            attempt_epoch: command.attempt_epoch,
            expected_checkpoint_version: command.expected_checkpoint_version,
        },
    )
    .await?;
    if terminal.disposition != "verified" {
        return Err(conflict(
            "verified terminalizer received non-verified result",
        ));
    }
    Ok(TerminalizedFinding {
        finding_id: terminal
            .finding_id
            .ok_or_else(|| conflict("verified terminalization produced no Finding"))?,
        attempt_id: terminal.attempt_id,
        replayed: terminal.replayed,
    })
}

#[cfg(test)]
mod tests {
    use super::parse_terminal_payload;

    #[test]
    fn raw_terminal_payload_rejects_unknown_fact_delta_kind() {
        let error = parse_terminal_payload(&serde_json::json!({
            "disposition": "blocked",
            "blocker_reason_code": "follow_on_only",
            "fact_deltas": [{
                "fact_kind": "model_invented_prose",
                "canonical_ref_kind": "attack_candidate_work_item",
                "canonical_ref_id": uuid::Uuid::from_u128(99),
                "canonical_ref_version": 1,
                "canonical_ref_hash": "sha256:canonical",
                "summary": "untrusted prose must not become a new delta kind",
                "evidence_ids": [17]
            }]
        }))
        .expect_err("raw Candidate result must fail closed on an unknown delta kind");

        assert!(error.to_string().contains("FactDelta fact_kind"));
    }
}
