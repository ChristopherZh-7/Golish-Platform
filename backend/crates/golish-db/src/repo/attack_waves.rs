//! Operation/scope/org-scoped Wave repository for Candidate V2.

use chrono::{DateTime, Utc};
use sha2::{Digest, Sha256};
use sqlx::{PgPool, Postgres, Transaction};
use uuid::Uuid;

#[derive(Debug, Clone, sqlx::FromRow)]
pub struct AttackWaveRunRow {
    pub id: Uuid,
    pub operation_id: Uuid,
    pub scope_snapshot_id: Uuid,
    pub generation: i32,
    pub status: String,
    pub policy_snapshot: serde_json::Value,
    pub policy_hash: String,
    pub max_waves: i32,
    pub max_candidates_total: i32,
    pub max_chain_depth: i32,
    pub max_attempts_total: i32,
    pub row_version: i64,
    pub created_at: DateTime<Utc>,
    pub updated_at: DateTime<Utc>,
    pub terminal_at: Option<DateTime<Utc>>,
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub enum AttackWaveEntry {
    VulnTriageHandoff {
        stage_execution_id: Uuid,
        stage_run_unit_id: Uuid,
        deliverable_submission_id: Uuid,
    },
    FactDeltaConsolidation {
        consolidation_id: Uuid,
    },
}

#[derive(Debug, Clone)]
pub struct AttackWaveUnitRow {
    pub id: Uuid,
    pub wave_run_id: Uuid,
    pub operation_id: Uuid,
    pub scope_snapshot_id: Uuid,
    pub organization_id: Uuid,
    pub entry: AttackWaveEntry,
    pub ordinal: i32,
    pub status: String,
    pub review_closed: bool,
    pub verification_closed: bool,
    pub consolidation_status: String,
    pub manifest_hash: Option<String>,
    pub manifest_count: Option<i32>,
    pub manifest_frozen_at: Option<DateTime<Utc>>,
    pub row_version: i64,
    pub created_at: DateTime<Utc>,
    pub updated_at: DateTime<Utc>,
    pub terminal_at: Option<DateTime<Utc>>,
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct InitialAttackWaveUnitAuthority {
    pub operation_id: Uuid,
    pub scope_snapshot_id: Uuid,
    pub organization_id: Uuid,
    pub ordinal: i32,
    pub entry: AttackWaveEntry,
    pub handoff_id: Uuid,
    pub evidence_ids: Vec<i64>,
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct InitialAttackWaveAuthority {
    pub operation_id: Uuid,
    pub scope_snapshot_id: Uuid,
    pub generation: i32,
    pub predecessor_stage_execution_id: Uuid,
    pub predecessor_generation: i32,
    pub units: Vec<InitialAttackWaveUnitAuthority>,
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct FrozenAttackWaveManifestAuthority {
    pub manifest_hash: String,
    pub manifest_count: i32,
    pub manifest_frozen_at: DateTime<Utc>,
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub enum CurrentAttackWaveUnitState {
    /// Generation zero has been opened from exact predecessor handoffs, but its
    /// deterministic work-item manifest has not yet been frozen.
    AwaitingManifest,
    /// The unit has a frozen manifest. The unit status remains authoritative
    /// for deciding whether reasoning, review, or verification should run.
    Runnable {
        manifest: FrozenAttackWaveManifestAuthority,
    },
    /// A follow-on Wave retains this frozen organization even though no
    /// accepted FactDelta produced work for it.
    TerminalNoInput,
}

#[derive(Debug, Clone)]
pub struct CurrentAttackWaveUnitAuthority {
    pub unit: AttackWaveUnitRow,
    pub state: CurrentAttackWaveUnitState,
}

#[derive(Debug, Clone)]
pub struct CurrentAttackWaveAuthority {
    pub wave: AttackWaveRunRow,
    pub units: Vec<CurrentAttackWaveUnitAuthority>,
}

#[derive(Debug, Clone)]
pub struct TerminalAttackWaveAuthority {
    pub last_wave: AttackWaveRunRow,
}

#[derive(Debug, Clone)]
pub enum AttackWaveAuthority {
    Initial(InitialAttackWaveAuthority),
    Current(CurrentAttackWaveAuthority),
    Terminal(TerminalAttackWaveAuthority),
}

#[derive(Debug, Clone, sqlx::FromRow)]
struct AttackWaveUnitDbRow {
    id: Uuid,
    wave_run_id: Uuid,
    operation_id: Uuid,
    scope_snapshot_id: Uuid,
    organization_id: Uuid,
    entry_stage_execution_id: Option<Uuid>,
    entry_stage_run_unit_id: Option<Uuid>,
    entry_deliverable_submission_id: Option<Uuid>,
    entry_stage_kind: Option<String>,
    entry_consolidation_id: Option<Uuid>,
    ordinal: i32,
    status: String,
    review_closed: bool,
    verification_closed: bool,
    consolidation_status: String,
    manifest_hash: Option<String>,
    manifest_count: Option<i32>,
    manifest_frozen_at: Option<DateTime<Utc>>,
    row_version: i64,
    created_at: DateTime<Utc>,
    updated_at: DateTime<Utc>,
    terminal_at: Option<DateTime<Utc>>,
}

#[derive(Debug, sqlx::FromRow)]
struct OperationAuthorityRow {
    runtime_memory_contract: String,
    attack_execution_contract: String,
    project_scope_id: Option<Uuid>,
}

#[derive(Debug, Clone, Copy, sqlx::FromRow)]
struct FrozenScopeUnitRow {
    organization_id: Uuid,
    ordinal: i32,
}

#[derive(Debug, sqlx::FromRow)]
struct InitialEntryAuthorityRow {
    organization_id: Uuid,
    ordinal: i32,
    generation: i32,
    stage_execution_id: Uuid,
    stage_run_unit_id: Uuid,
    deliverable_submission_id: Uuid,
    handoff_id: Uuid,
    evidence_ids: Vec<i64>,
}

fn authority_conflict(code: &'static str) -> crate::DbError {
    crate::DbError::Other(anyhow::anyhow!(code))
}

pub(super) fn deterministic_initial_wave_run_id(operation_id: Uuid, generation: i32) -> Uuid {
    Uuid::new_v5(
        &Uuid::NAMESPACE_OID,
        format!("{operation_id}:candidate-wave:{generation}").as_bytes(),
    )
}

pub(super) fn deterministic_initial_wave_unit_id(wave_run_id: Uuid, organization_id: Uuid) -> Uuid {
    Uuid::new_v5(
        &Uuid::NAMESPACE_OID,
        format!("{wave_run_id}:{organization_id}").as_bytes(),
    )
}

pub(super) fn deterministic_initial_policy() -> crate::Result<(serde_json::Value, String)> {
    let policy_snapshot = serde_json::json!({
        "max_attempts_total": 200,
        "max_candidates_total": 100,
        "max_chain_depth": 3,
        "max_waves": 3,
    });
    let policy_bytes = serde_json::to_vec(&policy_snapshot)?;
    let policy_hash = Sha256::digest(policy_bytes)
        .iter()
        .map(|byte| format!("{byte:02x}"))
        .collect::<String>();
    Ok((policy_snapshot, format!("sha256:{policy_hash}")))
}

fn decode_wave_unit(row: AttackWaveUnitDbRow) -> crate::Result<AttackWaveUnitRow> {
    let entry = match (
        row.entry_stage_execution_id,
        row.entry_stage_run_unit_id,
        row.entry_deliverable_submission_id,
        row.entry_stage_kind.as_deref(),
        row.entry_consolidation_id,
    ) {
        (
            Some(stage_execution_id),
            Some(stage_run_unit_id),
            Some(deliverable_submission_id),
            Some("vuln_triage"),
            None,
        ) => AttackWaveEntry::VulnTriageHandoff {
            stage_execution_id,
            stage_run_unit_id,
            deliverable_submission_id,
        },
        (None, None, None, None, Some(consolidation_id)) => {
            AttackWaveEntry::FactDeltaConsolidation { consolidation_id }
        }
        _ => {
            return Err(crate::DbError::Other(anyhow::anyhow!(
                "invalid attack WaveUnit entry shape"
            )))
        }
    };
    Ok(AttackWaveUnitRow {
        id: row.id,
        wave_run_id: row.wave_run_id,
        operation_id: row.operation_id,
        scope_snapshot_id: row.scope_snapshot_id,
        organization_id: row.organization_id,
        entry,
        ordinal: row.ordinal,
        status: row.status,
        review_closed: row.review_closed,
        verification_closed: row.verification_closed,
        consolidation_status: row.consolidation_status,
        manifest_hash: row.manifest_hash,
        manifest_count: row.manifest_count,
        manifest_frozen_at: row.manifest_frozen_at,
        row_version: row.row_version,
        created_at: row.created_at,
        updated_at: row.updated_at,
        terminal_at: row.terminal_at,
    })
}

#[derive(Debug, Clone)]
pub struct OpenAttackWaveUnit {
    pub wave_run_id: Uuid,
    pub wave_unit_id: Uuid,
    pub operation_id: Uuid,
    pub scope_snapshot_id: Uuid,
    pub organization_id: Uuid,
    pub entry_stage_execution_id: Uuid,
    pub entry_stage_run_unit_id: Uuid,
    pub entry_deliverable_submission_id: Uuid,
    pub generation: i32,
    pub ordinal: i32,
    pub policy_snapshot: serde_json::Value,
    pub policy_hash: String,
    pub max_waves: i32,
    pub max_candidates_total: i32,
    pub max_chain_depth: i32,
    pub max_attempts_total: i32,
}

const WAVE_COLUMNS: &str = "id,operation_id,scope_snapshot_id,generation,status,policy_snapshot,\
    policy_hash,max_waves,max_candidates_total,max_chain_depth,max_attempts_total,row_version,\
    created_at,updated_at,terminal_at";
const UNIT_COLUMNS: &str = "id,wave_run_id,operation_id,scope_snapshot_id,organization_id,\
    entry_stage_execution_id,entry_stage_run_unit_id,entry_deliverable_submission_id,\
    entry_stage_kind,entry_consolidation_id,ordinal,status,review_closed,verification_closed,\
    consolidation_status,manifest_hash,manifest_count,manifest_frozen_at,row_version,\
    created_at,updated_at,terminal_at";

/// Load the only DB-authoritative Candidate attack cursor for an operation.
///
/// The read is serialized behind the operation row so a concurrent Wave
/// consolidation cannot expose a source/target half-state. Both frozen
/// contracts must be `v2_only`, the organization scope must be sealed, and a
/// current Wave must cover that scope exactly. Dual attack contracts may write
/// Candidate V2 while Verification remains disabled; `v2_only` additionally
/// requires runtime-memory `v2_only`. When no Wave has ever existed,
/// the initial generation is derived only from a complete set of exact,
/// generation-zero `vuln_triage` final handoffs. A terminal history is returned
/// explicitly and is never mistaken for a fresh initial Wave.
pub async fn load_current_authority(
    pool: &PgPool,
    operation_id: Uuid,
) -> crate::Result<AttackWaveAuthority> {
    let mut tx = pool.begin().await?;
    let authority = load_current_authority_in_transaction(&mut tx, operation_id).await?;
    tx.commit().await?;
    Ok(authority)
}

async fn load_current_authority_in_transaction(
    tx: &mut Transaction<'_, Postgres>,
    operation_id: Uuid,
) -> crate::Result<AttackWaveAuthority> {
    let operation = sqlx::query_as::<_, OperationAuthorityRow>(
        r#"SELECT runtime_memory_contract,attack_execution_contract,project_scope_id
             FROM operation_state
            WHERE operation_id=$1
            FOR SHARE"#,
    )
    .bind(operation_id)
    .fetch_optional(&mut **tx)
    .await?
    .ok_or_else(|| crate::DbError::NotFound("operation_state".to_string()))?;
    if operation.runtime_memory_contract == "legacy_v1"
        || operation.attack_execution_contract == "legacy"
        || (operation.attack_execution_contract == "v2_only"
            && operation.runtime_memory_contract != "v2_only")
    {
        return Err(authority_conflict(
            "attack_wave_authority_requires_v2_writing_contracts",
        ));
    }
    let project_scope_id = operation
        .project_scope_id
        .ok_or_else(|| authority_conflict("attack_wave_authority_project_scope_missing"))?;
    let scope_snapshot_ids: Vec<Uuid> = sqlx::query_scalar(
        r#"SELECT id
             FROM operation_org_scope_snapshots
            WHERE operation_id=$1 AND project_scope_id=$2 AND sealed_at IS NOT NULL
            FOR SHARE"#,
    )
    .bind(operation_id)
    .bind(project_scope_id)
    .fetch_all(&mut **tx)
    .await?;
    let [scope_snapshot_id] = scope_snapshot_ids.as_slice() else {
        return Err(authority_conflict(
            "attack_wave_authority_sealed_scope_mismatch",
        ));
    };
    let scope_snapshot_id = *scope_snapshot_id;
    let scope_units = sqlx::query_as::<_, FrozenScopeUnitRow>(
        r#"SELECT organization_id,ordinal
             FROM operation_org_scope_units
            WHERE snapshot_id=$1
            ORDER BY ordinal,organization_id
            FOR SHARE"#,
    )
    .bind(scope_snapshot_id)
    .fetch_all(&mut **tx)
    .await?;
    if scope_units.is_empty() {
        return Err(authority_conflict("attack_wave_authority_scope_empty"));
    }

    let wave_sql = format!(
        "SELECT {WAVE_COLUMNS} FROM attack_wave_runs
          WHERE operation_id=$1 ORDER BY generation,id FOR SHARE"
    );
    let waves = sqlx::query_as::<_, AttackWaveRunRow>(&wave_sql)
        .bind(operation_id)
        .fetch_all(&mut **tx)
        .await?;
    for (index, wave) in waves.iter().enumerate() {
        let expected_generation = i32::try_from(index)
            .map_err(|_| authority_conflict("attack_wave_generation_overflow"))?;
        if wave.scope_snapshot_id != scope_snapshot_id || wave.generation != expected_generation {
            return Err(authority_conflict(
                "attack_wave_generation_history_mismatch",
            ));
        }
    }
    let active_wave_indexes = waves
        .iter()
        .enumerate()
        .filter_map(|(index, wave)| {
            matches!(wave.status.as_str(), "open" | "review" | "verification").then_some(index)
        })
        .collect::<Vec<_>>();
    if active_wave_indexes.len() > 1 {
        return Err(authority_conflict("attack_wave_multiple_current_rows"));
    }
    let Some(active_wave_index) = active_wave_indexes.first().copied() else {
        return if let Some(last_wave) = waves.last() {
            validate_terminal_wave(tx, operation_id, scope_snapshot_id, last_wave, &scope_units)
                .await?;
            Ok(AttackWaveAuthority::Terminal(TerminalAttackWaveAuthority {
                last_wave: last_wave.clone(),
            }))
        } else {
            load_initial_authority(tx, operation_id, scope_snapshot_id, &scope_units).await
        };
    };
    if active_wave_index + 1 != waves.len()
        || waves[..active_wave_index]
            .iter()
            .any(|wave| wave.status != "terminal")
    {
        return Err(authority_conflict("attack_wave_current_cursor_mismatch"));
    }
    let wave = waves[active_wave_index].clone();
    let unit_sql = format!(
        "SELECT {UNIT_COLUMNS} FROM attack_wave_units
          WHERE wave_run_id=$1 AND operation_id=$2 AND scope_snapshot_id=$3
          ORDER BY ordinal,organization_id FOR SHARE"
    );
    let unit_rows = sqlx::query_as::<_, AttackWaveUnitDbRow>(&unit_sql)
        .bind(wave.id)
        .bind(operation_id)
        .bind(scope_snapshot_id)
        .fetch_all(&mut **tx)
        .await?;
    if unit_rows.len() != scope_units.len() {
        if waves.len() == 1 {
            if let Some(initial) = try_load_partial_initial_authority(
                tx,
                operation_id,
                scope_snapshot_id,
                &wave,
                &scope_units,
                &unit_rows,
            )
            .await?
            {
                return Ok(AttackWaveAuthority::Initial(initial));
            }
        }
        return Err(authority_conflict("attack_wave_unit_scope_mismatch"));
    }
    let mut units = Vec::with_capacity(unit_rows.len());
    for (scope_unit, unit_row) in scope_units.iter().zip(unit_rows) {
        let unit = decode_wave_unit(unit_row)?;
        if unit.wave_run_id != wave.id
            || unit.operation_id != operation_id
            || unit.scope_snapshot_id != scope_snapshot_id
            || unit.organization_id != scope_unit.organization_id
            || unit.ordinal != scope_unit.ordinal
        {
            return Err(authority_conflict("attack_wave_unit_scope_mismatch"));
        }
        let state = current_unit_state(&unit)?;
        units.push(CurrentAttackWaveUnitAuthority { unit, state });
    }
    Ok(AttackWaveAuthority::Current(CurrentAttackWaveAuthority {
        wave,
        units,
    }))
}

async fn validate_terminal_wave(
    tx: &mut Transaction<'_, Postgres>,
    operation_id: Uuid,
    scope_snapshot_id: Uuid,
    wave: &AttackWaveRunRow,
    scope_units: &[FrozenScopeUnitRow],
) -> crate::Result<()> {
    if wave.status != "terminal" || wave.terminal_at.is_none() {
        return Err(authority_conflict("attack_wave_terminal_cursor_mismatch"));
    }
    let unit_sql = format!(
        "SELECT {UNIT_COLUMNS} FROM attack_wave_units
          WHERE wave_run_id=$1 AND operation_id=$2 AND scope_snapshot_id=$3
          ORDER BY ordinal,organization_id FOR SHARE"
    );
    let unit_rows = sqlx::query_as::<_, AttackWaveUnitDbRow>(&unit_sql)
        .bind(wave.id)
        .bind(operation_id)
        .bind(scope_snapshot_id)
        .fetch_all(&mut **tx)
        .await?;
    if unit_rows.len() != scope_units.len() {
        return Err(authority_conflict("attack_wave_unit_scope_mismatch"));
    }
    for (scope_unit, unit_row) in scope_units.iter().zip(unit_rows) {
        let unit = decode_wave_unit(unit_row)?;
        if unit.organization_id != scope_unit.organization_id
            || unit.ordinal != scope_unit.ordinal
            || unit.status != "terminal"
            || !unit.review_closed
            || !unit.verification_closed
            || unit.consolidation_status != "terminal"
            || unit.terminal_at.is_none()
        {
            return Err(authority_conflict("attack_wave_terminal_unit_mismatch"));
        }
    }
    Ok(())
}

async fn load_initial_authority(
    tx: &mut Transaction<'_, Postgres>,
    operation_id: Uuid,
    scope_snapshot_id: Uuid,
    scope_units: &[FrozenScopeUnitRow],
) -> crate::Result<AttackWaveAuthority> {
    let entries = sqlx::query_as::<_, InitialEntryAuthorityRow>(
        r#"SELECT scope_unit.organization_id,scope_unit.ordinal,unit.generation,
                  unit.stage_execution_id,unit.id AS stage_run_unit_id,
                  handoff.deliverable_submission_id,handoff.id AS handoff_id,
                  handoff.evidence_ids
             FROM operation_org_scope_units AS scope_unit
             JOIN stage_run_units AS unit
               ON unit.operation_id=$1
              AND unit.scope_snapshot_id=scope_unit.snapshot_id
              AND unit.organization_id=scope_unit.organization_id
              AND unit.stage_kind='vuln_triage'
              AND unit.status='passed'
              AND unit.terminal_at IS NOT NULL
             JOIN stage_handoffs AS handoff
               ON handoff.operation_id=unit.operation_id
              AND handoff.scope_snapshot_id=unit.scope_snapshot_id
              AND handoff.organization_id=unit.organization_id
              AND handoff.from_stage_kind=unit.stage_kind
              AND handoff.stage_execution_id=unit.stage_execution_id
              AND handoff.source_stage_run_unit_id=unit.id
              AND handoff.invalidated_at IS NULL
            WHERE scope_unit.snapshot_id=$2
            ORDER BY scope_unit.ordinal,scope_unit.organization_id,unit.id,handoff.id
            FOR SHARE OF unit,handoff"#,
    )
    .bind(operation_id)
    .bind(scope_snapshot_id)
    .fetch_all(&mut **tx)
    .await?;
    if entries.len() != scope_units.len() {
        return Err(authority_conflict(
            "attack_wave_initial_predecessor_scope_mismatch",
        ));
    }
    let predecessor_stage_execution_id = entries
        .first()
        .map(|entry| entry.stage_execution_id)
        .ok_or_else(|| authority_conflict("attack_wave_initial_predecessor_missing"))?;
    let predecessor_generation = entries
        .first()
        .map(|entry| entry.generation)
        .ok_or_else(|| authority_conflict("attack_wave_initial_predecessor_missing"))?;
    let mut units = Vec::with_capacity(entries.len());
    for (scope_unit, entry) in scope_units.iter().zip(entries) {
        if entry.organization_id != scope_unit.organization_id
            || entry.ordinal != scope_unit.ordinal
            || entry.generation != predecessor_generation
            || entry.stage_execution_id != predecessor_stage_execution_id
            || entry.evidence_ids.is_empty()
        {
            return Err(authority_conflict(
                "attack_wave_initial_predecessor_mismatch",
            ));
        }
        units.push(InitialAttackWaveUnitAuthority {
            operation_id,
            scope_snapshot_id,
            organization_id: entry.organization_id,
            ordinal: entry.ordinal,
            entry: AttackWaveEntry::VulnTriageHandoff {
                stage_execution_id: entry.stage_execution_id,
                stage_run_unit_id: entry.stage_run_unit_id,
                deliverable_submission_id: entry.deliverable_submission_id,
            },
            handoff_id: entry.handoff_id,
            evidence_ids: entry.evidence_ids,
        });
    }
    Ok(AttackWaveAuthority::Initial(InitialAttackWaveAuthority {
        operation_id,
        scope_snapshot_id,
        generation: 0,
        predecessor_stage_execution_id,
        predecessor_generation,
        units,
    }))
}

/// Recover only the crash shape produced when generation-zero organization seed transactions
/// commit independently. Every persisted subset member must already be a complete, deterministic
/// open+frozen-manifest write. Any looser partial Wave remains an ordinary scope mismatch.
async fn try_load_partial_initial_authority(
    tx: &mut Transaction<'_, Postgres>,
    operation_id: Uuid,
    scope_snapshot_id: Uuid,
    wave: &AttackWaveRunRow,
    scope_units: &[FrozenScopeUnitRow],
    unit_rows: &[AttackWaveUnitDbRow],
) -> crate::Result<Option<InitialAttackWaveAuthority>> {
    if wave.generation != 0
        || wave.status != "open"
        || wave.terminal_at.is_some()
        || wave.row_version != 0
        || wave.updated_at != wave.created_at
        || unit_rows.is_empty()
        || unit_rows.len() >= scope_units.len()
    {
        return Ok(None);
    }
    let expected_wave_run_id = deterministic_initial_wave_run_id(operation_id, 0);
    let (expected_policy_snapshot, expected_policy_hash) = deterministic_initial_policy()?;
    if wave.id != expected_wave_run_id
        || wave.scope_snapshot_id != scope_snapshot_id
        || wave.policy_snapshot != expected_policy_snapshot
        || wave.policy_hash != expected_policy_hash
        || wave.max_waves != 3
        || wave.max_candidates_total != 100
        || wave.max_chain_depth != 3
        || wave.max_attempts_total != 200
    {
        return Ok(None);
    }

    let initial =
        match load_initial_authority(tx, operation_id, scope_snapshot_id, scope_units).await? {
            AttackWaveAuthority::Initial(initial) => initial,
            _ => return Ok(None),
        };
    for unit_row in unit_rows.iter().cloned() {
        let unit = decode_wave_unit(unit_row)?;
        let Some(expected_scope_unit) = scope_units
            .iter()
            .find(|scope_unit| scope_unit.organization_id == unit.organization_id)
        else {
            return Ok(None);
        };
        let Some(expected_initial_unit) = initial
            .units
            .iter()
            .find(|expected| expected.organization_id == unit.organization_id)
        else {
            return Ok(None);
        };
        let expected_wave_unit_id =
            deterministic_initial_wave_unit_id(wave.id, unit.organization_id);
        if unit.id != expected_wave_unit_id
            || unit.wave_run_id != wave.id
            || unit.operation_id != operation_id
            || unit.scope_snapshot_id != scope_snapshot_id
            || unit.ordinal != expected_scope_unit.ordinal
            || unit.ordinal != expected_initial_unit.ordinal
            || unit.entry != expected_initial_unit.entry
            || unit.status != "open"
            || unit.review_closed
            || unit.verification_closed
            || unit.consolidation_status != "pending"
            || unit.terminal_at.is_some()
            || unit.row_version != 1
            || unit
                .manifest_frozen_at
                .is_some_and(|at| at < unit.created_at)
            || !matches!(
                current_unit_state(&unit),
                Ok(CurrentAttackWaveUnitState::Runnable { .. })
            )
        {
            return Ok(None);
        }
        let has_reasoning_decision: bool = sqlx::query_scalar(
            r#"SELECT EXISTS (
                   SELECT 1 FROM attack_candidate_work_items
                    WHERE wave_unit_id=$1
                      AND (
                          decision_kind IS NOT NULL
                          OR candidate_id IS NOT NULL
                          OR no_candidate_reason_code IS NOT NULL
                          OR no_candidate_detail IS NOT NULL
                          OR decided_at IS NOT NULL
                      )
               )"#,
        )
        .bind(unit.id)
        .fetch_one(&mut **tx)
        .await?;
        if has_reasoning_decision {
            return Ok(None);
        }
        super::attack_candidate_work_items::load_frozen_entry_evidence_ids_with_connection(
            tx,
            operation_id,
            scope_snapshot_id,
            wave.id,
            unit.id,
            unit.organization_id,
        )
        .await
        .map_err(|_| authority_conflict("attack_wave_partial_initial_manifest_mismatch"))?;
    }
    Ok(Some(initial))
}

fn current_unit_state(unit: &AttackWaveUnitRow) -> crate::Result<CurrentAttackWaveUnitState> {
    match (
        unit.status.as_str(),
        unit.review_closed,
        unit.verification_closed,
        unit.consolidation_status.as_str(),
        unit.manifest_hash.as_ref(),
        unit.manifest_count,
        unit.manifest_frozen_at,
        unit.terminal_at,
        &unit.entry,
    ) {
        (
            "open",
            false,
            false,
            "pending",
            None,
            None,
            None,
            None,
            AttackWaveEntry::VulnTriageHandoff { .. },
        ) => Ok(CurrentAttackWaveUnitState::AwaitingManifest),
        (
            "open" | "reasoning" | "review" | "verification",
            _,
            _,
            "pending" | "ready",
            Some(manifest_hash),
            Some(manifest_count),
            Some(manifest_frozen_at),
            None,
            _,
        ) if !manifest_hash.trim().is_empty() && manifest_count > 0 => {
            let lifecycle_is_exact = match unit.status.as_str() {
                "open" | "reasoning" | "review" => {
                    !unit.review_closed
                        && !unit.verification_closed
                        && unit.consolidation_status == "pending"
                }
                "verification" => {
                    unit.review_closed
                        && ((!unit.verification_closed && unit.consolidation_status == "pending")
                            || (unit.verification_closed && unit.consolidation_status == "ready"))
                }
                _ => false,
            };
            if !lifecycle_is_exact {
                return Err(authority_conflict("attack_wave_unit_state_mismatch"));
            }
            Ok(CurrentAttackWaveUnitState::Runnable {
                manifest: FrozenAttackWaveManifestAuthority {
                    manifest_hash: manifest_hash.clone(),
                    manifest_count,
                    manifest_frozen_at,
                },
            })
        }
        (
            "terminal",
            true,
            true,
            "terminal",
            None,
            None,
            None,
            Some(_),
            AttackWaveEntry::FactDeltaConsolidation { .. },
        ) => Ok(CurrentAttackWaveUnitState::TerminalNoInput),
        _ => Err(authority_conflict("attack_wave_unit_state_mismatch")),
    }
}

/// Open/replay one WaveUnit from an exact upstream vuln_triage final handoff.
/// Natural-key replay compares every frozen policy and entry identity field;
/// drift fails closed rather than silently reusing a different wave.
pub async fn open_from_vuln_triage_handoff(
    tx: &mut Transaction<'_, Postgres>,
    input: &OpenAttackWaveUnit,
) -> crate::Result<(AttackWaveRunRow, AttackWaveUnitRow)> {
    if input.generation < 0
        || input.ordinal < 0
        || input.policy_hash.trim().is_empty()
        || !input.policy_snapshot.is_object()
    {
        return Err(crate::DbError::Other(anyhow::anyhow!(
            "invalid attack wave entry request"
        )));
    }
    let wave_insert = format!(
        "INSERT INTO attack_wave_runs(
             id,operation_id,scope_snapshot_id,generation,status,policy_snapshot,policy_hash,
             max_waves,max_candidates_total,max_chain_depth,max_attempts_total)
         VALUES($1,$2,$3,$4,'open',$5,$6,$7,$8,$9,$10)
         ON CONFLICT(operation_id,generation) DO NOTHING RETURNING {WAVE_COLUMNS}"
    );
    let inserted_wave = sqlx::query_as::<_, AttackWaveRunRow>(&wave_insert)
        .bind(input.wave_run_id)
        .bind(input.operation_id)
        .bind(input.scope_snapshot_id)
        .bind(input.generation)
        .bind(&input.policy_snapshot)
        .bind(&input.policy_hash)
        .bind(input.max_waves)
        .bind(input.max_candidates_total)
        .bind(input.max_chain_depth)
        .bind(input.max_attempts_total)
        .fetch_optional(&mut **tx)
        .await?;
    let wave = match inserted_wave {
        Some(row) => row,
        None => {
            let sql = format!(
                "SELECT {WAVE_COLUMNS} FROM attack_wave_runs
                 WHERE operation_id=$1 AND generation=$2 FOR UPDATE"
            );
            sqlx::query_as(&sql)
                .bind(input.operation_id)
                .bind(input.generation)
                .fetch_one(&mut **tx)
                .await?
        }
    };
    if wave.id != input.wave_run_id
        || wave.scope_snapshot_id != input.scope_snapshot_id
        || wave.policy_snapshot != input.policy_snapshot
        || wave.policy_hash != input.policy_hash
        || wave.max_waves != input.max_waves
        || wave.max_candidates_total != input.max_candidates_total
        || wave.max_chain_depth != input.max_chain_depth
        || wave.max_attempts_total != input.max_attempts_total
    {
        return Err(crate::DbError::Other(anyhow::anyhow!(
            "attack wave replay drift"
        )));
    }

    let unit_insert = format!(
        "INSERT INTO attack_wave_units(
             id,wave_run_id,operation_id,scope_snapshot_id,organization_id,
             entry_stage_execution_id,entry_stage_run_unit_id,
             entry_deliverable_submission_id,entry_stage_kind,ordinal,status)
         VALUES($1,$2,$3,$4,$5,$6,$7,$8,'vuln_triage',$9,'open')
         ON CONFLICT(wave_run_id,organization_id) DO NOTHING RETURNING {UNIT_COLUMNS}"
    );
    let inserted_unit = sqlx::query_as::<_, AttackWaveUnitDbRow>(&unit_insert)
        .bind(input.wave_unit_id)
        .bind(input.wave_run_id)
        .bind(input.operation_id)
        .bind(input.scope_snapshot_id)
        .bind(input.organization_id)
        .bind(input.entry_stage_execution_id)
        .bind(input.entry_stage_run_unit_id)
        .bind(input.entry_deliverable_submission_id)
        .bind(input.ordinal)
        .fetch_optional(&mut **tx)
        .await?;
    let unit = match inserted_unit {
        Some(row) => decode_wave_unit(row)?,
        None => {
            let sql = format!(
                "SELECT {UNIT_COLUMNS} FROM attack_wave_units
                 WHERE wave_run_id=$1 AND organization_id=$2 FOR UPDATE"
            );
            let row = sqlx::query_as::<_, AttackWaveUnitDbRow>(&sql)
                .bind(input.wave_run_id)
                .bind(input.organization_id)
                .fetch_one(&mut **tx)
                .await?;
            decode_wave_unit(row)?
        }
    };
    let expected_entry = AttackWaveEntry::VulnTriageHandoff {
        stage_execution_id: input.entry_stage_execution_id,
        stage_run_unit_id: input.entry_stage_run_unit_id,
        deliverable_submission_id: input.entry_deliverable_submission_id,
    };
    if unit.id != input.wave_unit_id
        || unit.operation_id != input.operation_id
        || unit.scope_snapshot_id != input.scope_snapshot_id
        || unit.entry != expected_entry
        || unit.ordinal != input.ordinal
    {
        return Err(crate::DbError::Other(anyhow::anyhow!(
            "attack wave-unit replay drift"
        )));
    }
    Ok((wave, unit))
}

pub async fn lock_wave(
    tx: &mut Transaction<'_, Postgres>,
    operation_id: Uuid,
    scope_snapshot_id: Uuid,
    wave_run_id: Uuid,
) -> crate::Result<AttackWaveRunRow> {
    let sql = format!(
        "SELECT {WAVE_COLUMNS} FROM attack_wave_runs
         WHERE id=$1 AND operation_id=$2 AND scope_snapshot_id=$3 FOR UPDATE"
    );
    sqlx::query_as(&sql)
        .bind(wave_run_id)
        .bind(operation_id)
        .bind(scope_snapshot_id)
        .fetch_optional(&mut **tx)
        .await?
        .ok_or_else(|| crate::DbError::NotFound("attack_wave_run".to_string()))
}

pub async fn lock_wave_unit(
    tx: &mut Transaction<'_, Postgres>,
    operation_id: Uuid,
    scope_snapshot_id: Uuid,
    wave_run_id: Uuid,
    wave_unit_id: Uuid,
    organization_id: Uuid,
) -> crate::Result<AttackWaveUnitRow> {
    let sql = format!(
        "SELECT {UNIT_COLUMNS} FROM attack_wave_units
         WHERE id=$1 AND wave_run_id=$2 AND operation_id=$3
           AND scope_snapshot_id=$4 AND organization_id=$5 FOR UPDATE"
    );
    let row = sqlx::query_as::<_, AttackWaveUnitDbRow>(&sql)
        .bind(wave_unit_id)
        .bind(wave_run_id)
        .bind(operation_id)
        .bind(scope_snapshot_id)
        .bind(organization_id)
        .fetch_optional(&mut **tx)
        .await?
        .ok_or_else(|| crate::DbError::NotFound("attack_wave_unit".to_string()))?;
    decode_wave_unit(row)
}

pub async fn set_review_closed(
    tx: &mut Transaction<'_, Postgres>,
    wave_unit: &AttackWaveUnitRow,
    closed: bool,
) -> crate::Result<AttackWaveUnitRow> {
    let sql = format!(
        "UPDATE attack_wave_units SET review_closed=$2,
             status=CASE WHEN $2 THEN 'verification' ELSE 'review' END,
             row_version=row_version+1,updated_at=NOW()
         WHERE id=$1 AND row_version=$3 RETURNING {UNIT_COLUMNS}"
    );
    let row = sqlx::query_as::<_, AttackWaveUnitDbRow>(&sql)
        .bind(wave_unit.id)
        .bind(closed)
        .bind(wave_unit.row_version)
        .fetch_optional(&mut **tx)
        .await?
        .ok_or_else(|| crate::DbError::Other(anyhow::anyhow!("stale attack wave unit")))?;
    decode_wave_unit(row)
}
