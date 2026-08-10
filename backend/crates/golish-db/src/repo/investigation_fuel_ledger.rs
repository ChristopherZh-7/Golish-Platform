//! Durable CAS fuel ledger and semantic-cycle receipts for unified Investigation.
//!
//! Reserving fuel and appending its first event happen in one transaction. A
//! reservation can leave `reserved` exactly once: consume, refund before any
//! durable begin, or remain conservatively `unknown_held` after an ambiguous
//! execution. Database deferred constraints reconcile every mutable head with
//! the immutable reservation/event census at commit.

use golish_core::investigation_fuel::{
    InvestigationFuelAxisV1, InvestigationFuelReservationStateV1,
    InvestigationSemanticCycleReceiptV1,
};
use serde::Serialize;
use sha2::{Digest, Sha256};
use sqlx::{PgPool, Postgres, Transaction};
use uuid::Uuid;

#[derive(Debug, thiserror::Error)]
pub enum InvestigationFuelStoreError {
    #[error(transparent)]
    Sqlx(#[from] sqlx::Error),
    #[error("invalid Investigation fuel input: {0}")]
    InvalidInput(&'static str),
    #[error("Investigation fuel identity conflict: {0}")]
    IdentityConflict(&'static str),
    #[error("Investigation fuel CAS conflict: {0}")]
    CasConflict(&'static str),
    #[error("Investigation fuel exhausted")]
    Exhausted,
    #[error("illegal Investigation fuel reservation transition")]
    IllegalTransition,
}

pub type InvestigationFuelStoreResult<T> = Result<T, InvestigationFuelStoreError>;

#[derive(Debug, Clone, PartialEq, Eq)]
pub enum FuelBudgetScope {
    Operation,
    Unit {
        stage_run_unit_id: Uuid,
        scope_snapshot_id: Uuid,
        organization_id: Uuid,
    },
    Task {
        stage_run_unit_id: Uuid,
        scope_snapshot_id: Uuid,
        organization_id: Uuid,
        task_id: Uuid,
    },
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct CreateFuelBudgetInput {
    pub budget_id: Uuid,
    pub stable_request_id: Uuid,
    pub authority_id: Uuid,
    pub operation_id: Uuid,
    pub stage_execution_id: Uuid,
    pub owning_stage_run_request_id: String,
    pub scope: FuelBudgetScope,
    pub limits: Vec<(InvestigationFuelAxisV1, u64)>,
}

#[derive(Debug, Clone, sqlx::FromRow, PartialEq, Eq)]
pub struct FuelBudgetRow {
    pub budget_id: Uuid,
    pub stable_request_id: Uuid,
    pub authority_id: Uuid,
    pub operation_id: Uuid,
    pub stage_execution_id: Uuid,
    pub owning_stage_run_request_id: String,
    pub scope_kind: String,
    pub owner_id: Uuid,
    pub stage_run_unit_id: Option<Uuid>,
    pub scope_snapshot_id: Option<Uuid>,
    pub organization_id: Option<Uuid>,
    pub task_id: Option<Uuid>,
    pub budget_contract_version: String,
}

#[derive(Debug, Clone, sqlx::FromRow, PartialEq, Eq)]
pub struct FuelHeadRow {
    pub budget_id: Uuid,
    pub axis: String,
    pub limit_amount: i64,
    pub reserved_amount: i64,
    pub consumed_amount: i64,
    pub unknown_held_amount: i64,
    pub refunded_before_begin_amount: i64,
    pub head_version: i64,
}

#[derive(Debug, Clone, sqlx::FromRow, PartialEq, Eq)]
pub struct FuelReservationRow {
    pub reservation_id: Uuid,
    pub budget_id: Uuid,
    pub axis: String,
    pub amount: i64,
    pub work_key_sha256: String,
    pub state: String,
    pub reservation_epoch: i64,
    pub row_version: i64,
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct ReserveFuelInput {
    pub reservation_id: Uuid,
    pub event_id: Uuid,
    pub stable_request_id: Uuid,
    pub budget_id: Uuid,
    pub axis: InvestigationFuelAxisV1,
    pub amount: u64,
    pub work_key_sha256: String,
    pub expected_head_version: u64,
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct TransitionFuelReservationInput {
    pub reservation_id: Uuid,
    pub event_id: Uuid,
    pub stable_request_id: Uuid,
    pub next_state: InvestigationFuelReservationStateV1,
    pub expected_head_version: u64,
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct FuelMutationReceipt {
    pub reservation: FuelReservationRow,
    pub head: FuelHeadRow,
    pub replayed: bool,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum SemanticCycleDisposition {
    Advanced,
    FixedPoint,
    Residual,
    Stopped,
}

impl SemanticCycleDisposition {
    const fn as_str(self) -> &'static str {
        match self {
            Self::Advanced => "advanced",
            Self::FixedPoint => "fixed_point",
            Self::Residual => "residual",
            Self::Stopped => "stopped",
        }
    }
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct RecordSemanticCycleInput {
    pub semantic_cycle_receipt_id: Uuid,
    pub stable_request_id: Uuid,
    pub task_id: Uuid,
    pub receipt: InvestigationSemanticCycleReceiptV1,
    pub disposition: SemanticCycleDisposition,
    pub residual_reason_code: Option<String>,
    pub stop_receipt_id: Option<Uuid>,
}

#[derive(Debug, Clone, sqlx::FromRow, PartialEq, Eq)]
pub struct SemanticCycleRow {
    pub semantic_cycle_receipt_id: Uuid,
    pub stable_request_id: Uuid,
    pub task_id: Uuid,
    pub operation_id: Uuid,
    pub stage_execution_id: Uuid,
    pub stage_run_unit_id: Uuid,
    pub scope_snapshot_id: Uuid,
    pub organization_id: Uuid,
    pub cycle_fingerprint_sha256: String,
    pub hypothesis_revision_sha256: String,
    pub verification_plan_sha256: String,
    pub semantic_evidence_set_sha256: String,
    pub open_obligation_set_sha256: String,
    pub remaining_work_set_sha256: String,
    pub disposition: String,
    pub residual_reason_code: Option<String>,
    pub stop_receipt_id: Option<Uuid>,
    pub receipt_sha256: String,
}

#[derive(Debug, Serialize)]
struct FuelEventHashMaterial<'a> {
    reservation_id: Uuid,
    budget_id: Uuid,
    axis: &'a str,
    event_ordinal: i64,
    from_state: Option<&'a str>,
    to_state: &'a str,
    amount: i64,
}

#[derive(Debug, Serialize)]
struct SemanticCycleHashMaterial<'a> {
    task_id: Uuid,
    operation_id: Uuid,
    stage_execution_id: Uuid,
    stage_run_unit_id: Uuid,
    scope_snapshot_id: Uuid,
    organization_id: Uuid,
    cycle_fingerprint_sha256: &'a str,
    hypothesis_revision_sha256: &'a str,
    verification_plan_sha256: &'a str,
    semantic_evidence_set_sha256: &'a str,
    open_obligation_set_sha256: &'a str,
    remaining_work_set_sha256: &'a str,
    disposition: &'a str,
    residual_reason_code: Option<&'a str>,
    stop_receipt_id: Option<Uuid>,
}

pub async fn create_budget(
    pool: &PgPool,
    input: &CreateFuelBudgetInput,
) -> InvestigationFuelStoreResult<FuelBudgetRow> {
    validate_ids(&[
        input.budget_id,
        input.stable_request_id,
        input.authority_id,
        input.operation_id,
        input.stage_execution_id,
    ])?;
    if input.owning_stage_run_request_id.trim().is_empty() || input.limits.is_empty() {
        return Err(InvestigationFuelStoreError::InvalidInput("budget"));
    }
    let (scope_kind, owner_id, unit_id, snapshot_id, organization_id, task_id) =
        scope_columns(input.operation_id, &input.scope)?;
    let mut limits = input
        .limits
        .iter()
        .map(|(axis, limit)| {
            if *limit == 0 || *limit > i64::MAX as u64 {
                return Err(InvestigationFuelStoreError::InvalidInput("fuel_limit"));
            }
            Ok((axis_name(*axis).to_owned(), *limit as i64))
        })
        .collect::<InvestigationFuelStoreResult<Vec<_>>>()?;
    limits.sort_unstable();
    if limits.windows(2).any(|pair| pair[0].0 == pair[1].0) {
        return Err(InvestigationFuelStoreError::InvalidInput(
            "duplicate_fuel_axis",
        ));
    }

    let mut tx = pool.begin().await?;
    sqlx::query(
        r#"INSERT INTO investigation_fuel_budgets(
               budget_id,stable_request_id,authority_id,operation_id,
               stage_execution_id,owning_stage_run_request_id,scope_kind,
               owner_id,stage_run_unit_id,scope_snapshot_id,organization_id,task_id
           ) VALUES($1,$2,$3,$4,$5,$6,$7,$8,$9,$10,$11,$12)
           ON CONFLICT(stable_request_id) DO NOTHING"#,
    )
    .bind(input.budget_id)
    .bind(input.stable_request_id)
    .bind(input.authority_id)
    .bind(input.operation_id)
    .bind(input.stage_execution_id)
    .bind(&input.owning_stage_run_request_id)
    .bind(scope_kind)
    .bind(owner_id)
    .bind(unit_id)
    .bind(snapshot_id)
    .bind(organization_id)
    .bind(task_id)
    .execute(&mut *tx)
    .await?;
    let budget = load_budget_by_request_on(&mut tx, input.stable_request_id).await?;
    if budget.budget_id != input.budget_id
        || budget.authority_id != input.authority_id
        || budget.operation_id != input.operation_id
        || budget.stage_execution_id != input.stage_execution_id
        || budget.owning_stage_run_request_id != input.owning_stage_run_request_id
        || budget.scope_kind != scope_kind
        || budget.owner_id != owner_id
        || budget.stage_run_unit_id != unit_id
        || budget.scope_snapshot_id != snapshot_id
        || budget.organization_id != organization_id
        || budget.task_id != task_id
    {
        return Err(InvestigationFuelStoreError::IdentityConflict(
            "budget_replay_mismatch",
        ));
    }
    for (axis, limit) in &limits {
        sqlx::query(
            r#"INSERT INTO investigation_fuel_budget_heads(budget_id,axis,limit_amount)
               VALUES($1,$2,$3) ON CONFLICT(budget_id,axis) DO NOTHING"#,
        )
        .bind(budget.budget_id)
        .bind(axis)
        .bind(limit)
        .execute(&mut *tx)
        .await?;
    }
    let stored_limits = sqlx::query_as::<_, (String, i64)>(
        "SELECT axis,limit_amount FROM investigation_fuel_budget_heads WHERE budget_id=$1 ORDER BY axis",
    )
    .bind(budget.budget_id)
    .fetch_all(&mut *tx)
    .await?;
    if stored_limits != limits {
        return Err(InvestigationFuelStoreError::IdentityConflict(
            "budget_limit_replay_mismatch",
        ));
    }
    tx.commit().await?;
    Ok(budget)
}

pub async fn reserve_fuel(
    pool: &PgPool,
    input: &ReserveFuelInput,
) -> InvestigationFuelStoreResult<FuelMutationReceipt> {
    validate_ids(&[
        input.reservation_id,
        input.event_id,
        input.stable_request_id,
        input.budget_id,
    ])?;
    validate_hash(&input.work_key_sha256)?;
    if input.amount == 0
        || input.amount > i64::MAX as u64
        || input.expected_head_version > i64::MAX as u64
    {
        return Err(InvestigationFuelStoreError::InvalidInput("reservation"));
    }
    let axis = axis_name(input.axis);
    let amount = input.amount as i64;
    let expected = input.expected_head_version as i64;
    let mut tx = pool.begin().await?;
    if let Some(existing) = load_reservation_optional_on(&mut tx, input.reservation_id).await? {
        validate_reservation_replay(
            &existing,
            input.budget_id,
            axis,
            amount,
            &input.work_key_sha256,
        )?;
        validate_event_replay(
            &mut tx,
            input.event_id,
            input.stable_request_id,
            &existing,
            0,
            None,
            "reserved",
        )
        .await?;
        let head = load_head_on(&mut tx, existing.budget_id, &existing.axis).await?;
        tx.commit().await?;
        return Ok(FuelMutationReceipt {
            reservation: existing,
            head,
            replayed: true,
        });
    }

    let head = sqlx::query_as::<_, FuelHeadRow>(
        r#"UPDATE investigation_fuel_budget_heads
              SET reserved_amount=reserved_amount+$4,
                  head_version=head_version+1,
                  updated_at=statement_timestamp()
            WHERE budget_id=$1 AND axis=$2 AND head_version=$3
              AND limit_amount-reserved_amount-consumed_amount-unknown_held_amount >= $4
        RETURNING budget_id,axis,limit_amount,reserved_amount,consumed_amount,
                  unknown_held_amount,refunded_before_begin_amount,head_version"#,
    )
    .bind(input.budget_id)
    .bind(axis)
    .bind(expected)
    .bind(amount)
    .fetch_optional(&mut *tx)
    .await?;
    let Some(head) = head else {
        let current = load_head_on(&mut tx, input.budget_id, axis).await?;
        if current.head_version != expected {
            return Err(InvestigationFuelStoreError::CasConflict(
                "fuel_head_version",
            ));
        }
        return Err(InvestigationFuelStoreError::Exhausted);
    };
    let reservation_epoch = head.head_version;
    sqlx::query(
        r#"INSERT INTO investigation_fuel_reservations(
               reservation_id,budget_id,axis,amount,work_key_sha256,state,reservation_epoch
           ) VALUES($1,$2,$3,$4,$5,'reserved',$6)"#,
    )
    .bind(input.reservation_id)
    .bind(input.budget_id)
    .bind(axis)
    .bind(amount)
    .bind(&input.work_key_sha256)
    .bind(reservation_epoch)
    .execute(&mut *tx)
    .await?;
    let reservation = load_reservation_on(&mut tx, input.reservation_id).await?;
    insert_event(
        &mut tx,
        input.event_id,
        input.stable_request_id,
        &reservation,
        0,
        None,
        "reserved",
    )
    .await?;
    tx.commit().await?;
    Ok(FuelMutationReceipt {
        reservation,
        head,
        replayed: false,
    })
}

pub async fn transition_reservation(
    pool: &PgPool,
    input: &TransitionFuelReservationInput,
) -> InvestigationFuelStoreResult<FuelMutationReceipt> {
    validate_ids(&[
        input.reservation_id,
        input.event_id,
        input.stable_request_id,
    ])?;
    if input.expected_head_version > i64::MAX as u64 {
        return Err(InvestigationFuelStoreError::InvalidInput("head_version"));
    }
    let next = state_name(input.next_state);
    if next == "reserved" {
        return Err(InvestigationFuelStoreError::IllegalTransition);
    }
    let mut tx = pool.begin().await?;
    let current = sqlx::query_as::<_, FuelReservationRow>(
        r#"SELECT reservation_id,budget_id,axis,amount,work_key_sha256,state,
                  reservation_epoch,row_version
             FROM investigation_fuel_reservations
            WHERE reservation_id=$1 FOR UPDATE"#,
    )
    .bind(input.reservation_id)
    .fetch_one(&mut *tx)
    .await?;
    if current.state == next {
        validate_event_replay(
            &mut tx,
            input.event_id,
            input.stable_request_id,
            &current,
            current.row_version,
            Some("reserved"),
            next,
        )
        .await?;
        let head = load_head_on(&mut tx, current.budget_id, &current.axis).await?;
        tx.commit().await?;
        return Ok(FuelMutationReceipt {
            reservation: current,
            head,
            replayed: true,
        });
    }
    if current.state != "reserved" || current.row_version != 0 {
        return Err(InvestigationFuelStoreError::IllegalTransition);
    }
    let expected = input.expected_head_version as i64;
    let head = match input.next_state {
        InvestigationFuelReservationStateV1::Consumed => {
            update_head_for_transition(
                &mut tx,
                &current,
                expected,
                "consumed_amount=consumed_amount+$4",
            )
            .await?
        }
        InvestigationFuelReservationStateV1::RefundedBeforeBegin => {
            update_head_for_transition(
                &mut tx,
                &current,
                expected,
                "refunded_before_begin_amount=refunded_before_begin_amount+$4",
            )
            .await?
        }
        InvestigationFuelReservationStateV1::UnknownHeld => {
            update_head_for_transition(
                &mut tx,
                &current,
                expected,
                "unknown_held_amount=unknown_held_amount+$4",
            )
            .await?
        }
        InvestigationFuelReservationStateV1::Reserved => {
            return Err(InvestigationFuelStoreError::IllegalTransition)
        }
    };
    let updated = sqlx::query_as::<_, FuelReservationRow>(
        r#"UPDATE investigation_fuel_reservations
              SET state=$2,row_version=row_version+1,updated_at=statement_timestamp()
            WHERE reservation_id=$1 AND state='reserved' AND row_version=0
        RETURNING reservation_id,budget_id,axis,amount,work_key_sha256,state,
                  reservation_epoch,row_version"#,
    )
    .bind(input.reservation_id)
    .bind(next)
    .fetch_optional(&mut *tx)
    .await?
    .ok_or(InvestigationFuelStoreError::CasConflict(
        "reservation_row_version",
    ))?;
    insert_event(
        &mut tx,
        input.event_id,
        input.stable_request_id,
        &updated,
        1,
        Some("reserved"),
        next,
    )
    .await?;
    tx.commit().await?;
    Ok(FuelMutationReceipt {
        reservation: updated,
        head,
        replayed: false,
    })
}

pub async fn record_semantic_cycle(
    pool: &PgPool,
    input: &RecordSemanticCycleInput,
) -> InvestigationFuelStoreResult<SemanticCycleRow> {
    validate_ids(&[
        input.semantic_cycle_receipt_id,
        input.stable_request_id,
        input.task_id,
    ])?;
    match input.disposition {
        SemanticCycleDisposition::Advanced | SemanticCycleDisposition::FixedPoint => {
            if input.residual_reason_code.is_some() || input.stop_receipt_id.is_some() {
                return Err(InvestigationFuelStoreError::InvalidInput(
                    "semantic_cycle_disposition",
                ));
            }
        }
        SemanticCycleDisposition::Residual => {
            if input
                .residual_reason_code
                .as_deref()
                .is_none_or(|reason| reason.trim().is_empty())
                || input.stop_receipt_id.is_some()
            {
                return Err(InvestigationFuelStoreError::InvalidInput(
                    "semantic_cycle_residual",
                ));
            }
        }
        SemanticCycleDisposition::Stopped => {
            if input.residual_reason_code.is_some()
                || input.stop_receipt_id.is_none_or(|id| id.is_nil())
            {
                return Err(InvestigationFuelStoreError::InvalidInput(
                    "semantic_cycle_stop",
                ));
            }
        }
    }
    for hash in [
        &input.receipt.cycle_fingerprint_sha256,
        &input.receipt.hypothesis_revision_sha256,
        &input.receipt.verification_plan_sha256,
        &input.receipt.semantic_evidence_set_sha256,
        &input.receipt.open_obligation_set_sha256,
        &input.receipt.remaining_work_set_sha256,
    ] {
        validate_hash(hash)?;
    }
    let task_identity: (Uuid, Uuid, Uuid, Uuid, Uuid) = sqlx::query_as(
        r#"SELECT operation_id,stage_execution_id,stage_run_unit_id,
                  scope_snapshot_id,organization_id
             FROM hypothesis_verification_tasks WHERE task_id=$1"#,
    )
    .bind(input.task_id)
    .fetch_one(pool)
    .await?;
    let disposition = input.disposition.as_str();
    let receipt_sha256 = sha256_json(&SemanticCycleHashMaterial {
        task_id: input.task_id,
        operation_id: task_identity.0,
        stage_execution_id: task_identity.1,
        stage_run_unit_id: task_identity.2,
        scope_snapshot_id: task_identity.3,
        organization_id: task_identity.4,
        cycle_fingerprint_sha256: &input.receipt.cycle_fingerprint_sha256,
        hypothesis_revision_sha256: &input.receipt.hypothesis_revision_sha256,
        verification_plan_sha256: &input.receipt.verification_plan_sha256,
        semantic_evidence_set_sha256: &input.receipt.semantic_evidence_set_sha256,
        open_obligation_set_sha256: &input.receipt.open_obligation_set_sha256,
        remaining_work_set_sha256: &input.receipt.remaining_work_set_sha256,
        disposition,
        residual_reason_code: input.residual_reason_code.as_deref(),
        stop_receipt_id: input.stop_receipt_id,
    });
    sqlx::query(
        r#"INSERT INTO investigation_semantic_cycle_receipts(
               semantic_cycle_receipt_id,stable_request_id,task_id,operation_id,
               stage_execution_id,stage_run_unit_id,scope_snapshot_id,organization_id,
               cycle_fingerprint_sha256,hypothesis_revision_sha256,
               verification_plan_sha256,semantic_evidence_set_sha256,
               open_obligation_set_sha256,remaining_work_set_sha256,disposition,
               residual_reason_code,stop_receipt_id,receipt_sha256
           ) VALUES($1,$2,$3,$4,$5,$6,$7,$8,$9,$10,$11,$12,$13,$14,$15,$16,$17,$18)
           ON CONFLICT(stable_request_id) DO NOTHING"#,
    )
    .bind(input.semantic_cycle_receipt_id)
    .bind(input.stable_request_id)
    .bind(input.task_id)
    .bind(task_identity.0)
    .bind(task_identity.1)
    .bind(task_identity.2)
    .bind(task_identity.3)
    .bind(task_identity.4)
    .bind(&input.receipt.cycle_fingerprint_sha256)
    .bind(&input.receipt.hypothesis_revision_sha256)
    .bind(&input.receipt.verification_plan_sha256)
    .bind(&input.receipt.semantic_evidence_set_sha256)
    .bind(&input.receipt.open_obligation_set_sha256)
    .bind(&input.receipt.remaining_work_set_sha256)
    .bind(disposition)
    .bind(&input.residual_reason_code)
    .bind(input.stop_receipt_id)
    .bind(&receipt_sha256)
    .execute(pool)
    .await?;
    let row = load_semantic_cycle_by_request(pool, input.stable_request_id).await?;
    if row.semantic_cycle_receipt_id != input.semantic_cycle_receipt_id
        || row.task_id != input.task_id
        || row.operation_id != task_identity.0
        || row.stage_execution_id != task_identity.1
        || row.stage_run_unit_id != task_identity.2
        || row.scope_snapshot_id != task_identity.3
        || row.organization_id != task_identity.4
        || row.cycle_fingerprint_sha256 != input.receipt.cycle_fingerprint_sha256
        || row.hypothesis_revision_sha256 != input.receipt.hypothesis_revision_sha256
        || row.verification_plan_sha256 != input.receipt.verification_plan_sha256
        || row.semantic_evidence_set_sha256 != input.receipt.semantic_evidence_set_sha256
        || row.open_obligation_set_sha256 != input.receipt.open_obligation_set_sha256
        || row.remaining_work_set_sha256 != input.receipt.remaining_work_set_sha256
        || row.disposition != disposition
        || row.residual_reason_code != input.residual_reason_code
        || row.stop_receipt_id != input.stop_receipt_id
        || row.receipt_sha256 != receipt_sha256
    {
        return Err(InvestigationFuelStoreError::IdentityConflict(
            "semantic_cycle_replay_mismatch",
        ));
    }
    Ok(row)
}

async fn update_head_for_transition(
    tx: &mut Transaction<'_, Postgres>,
    reservation: &FuelReservationRow,
    expected_head_version: i64,
    target_increment: &'static str,
) -> InvestigationFuelStoreResult<FuelHeadRow> {
    let query = format!(
        r#"UPDATE investigation_fuel_budget_heads
              SET reserved_amount=reserved_amount-$4,
                  {target_increment},
                  head_version=head_version+1,
                  updated_at=statement_timestamp()
            WHERE budget_id=$1 AND axis=$2 AND head_version=$3
              AND reserved_amount >= $4
        RETURNING budget_id,axis,limit_amount,reserved_amount,consumed_amount,
                  unknown_held_amount,refunded_before_begin_amount,head_version"#
    );
    sqlx::query_as::<_, FuelHeadRow>(&query)
        .bind(reservation.budget_id)
        .bind(&reservation.axis)
        .bind(expected_head_version)
        .bind(reservation.amount)
        .fetch_optional(&mut **tx)
        .await?
        .ok_or(InvestigationFuelStoreError::CasConflict(
            "fuel_head_version",
        ))
}

async fn insert_event(
    tx: &mut Transaction<'_, Postgres>,
    event_id: Uuid,
    stable_request_id: Uuid,
    reservation: &FuelReservationRow,
    event_ordinal: i64,
    from_state: Option<&str>,
    to_state: &str,
) -> InvestigationFuelStoreResult<()> {
    let event_sha256 = fuel_event_hash(reservation, event_ordinal, from_state, to_state);
    sqlx::query(
        r#"INSERT INTO investigation_fuel_reservation_events(
               event_id,stable_request_id,reservation_id,budget_id,axis,
               event_ordinal,from_state,to_state,amount,event_sha256
           ) VALUES($1,$2,$3,$4,$5,$6,$7,$8,$9,$10)"#,
    )
    .bind(event_id)
    .bind(stable_request_id)
    .bind(reservation.reservation_id)
    .bind(reservation.budget_id)
    .bind(&reservation.axis)
    .bind(event_ordinal)
    .bind(from_state)
    .bind(to_state)
    .bind(reservation.amount)
    .bind(event_sha256)
    .execute(&mut **tx)
    .await?;
    Ok(())
}

async fn validate_event_replay(
    tx: &mut Transaction<'_, Postgres>,
    event_id: Uuid,
    stable_request_id: Uuid,
    reservation: &FuelReservationRow,
    event_ordinal: i64,
    from_state: Option<&str>,
    to_state: &str,
) -> InvestigationFuelStoreResult<()> {
    let stored: Option<(Uuid, Uuid, Option<String>, String, i64, String)> = sqlx::query_as(
        r#"SELECT event_id,stable_request_id,from_state,to_state,amount,event_sha256
             FROM investigation_fuel_reservation_events
            WHERE reservation_id=$1 AND event_ordinal=$2"#,
    )
    .bind(reservation.reservation_id)
    .bind(event_ordinal)
    .fetch_optional(&mut **tx)
    .await?;
    let expected_hash = fuel_event_hash(reservation, event_ordinal, from_state, to_state);
    if stored
        != Some((
            event_id,
            stable_request_id,
            from_state.map(str::to_owned),
            to_state.to_owned(),
            reservation.amount,
            expected_hash,
        ))
    {
        return Err(InvestigationFuelStoreError::IdentityConflict(
            "fuel_event_replay_mismatch",
        ));
    }
    Ok(())
}

fn fuel_event_hash(
    reservation: &FuelReservationRow,
    event_ordinal: i64,
    from_state: Option<&str>,
    to_state: &str,
) -> String {
    sha256_json(&FuelEventHashMaterial {
        reservation_id: reservation.reservation_id,
        budget_id: reservation.budget_id,
        axis: &reservation.axis,
        event_ordinal,
        from_state,
        to_state,
        amount: reservation.amount,
    })
}

fn validate_reservation_replay(
    row: &FuelReservationRow,
    budget_id: Uuid,
    axis: &str,
    amount: i64,
    work_key_sha256: &str,
) -> InvestigationFuelStoreResult<()> {
    if row.budget_id != budget_id
        || row.axis != axis
        || row.amount != amount
        || row.work_key_sha256 != work_key_sha256
    {
        return Err(InvestigationFuelStoreError::IdentityConflict(
            "reservation_replay_mismatch",
        ));
    }
    Ok(())
}

type FuelScopeColumns = (
    &'static str,
    Uuid,
    Option<Uuid>,
    Option<Uuid>,
    Option<Uuid>,
    Option<Uuid>,
);

fn scope_columns(
    operation_id: Uuid,
    scope: &FuelBudgetScope,
) -> InvestigationFuelStoreResult<FuelScopeColumns> {
    match scope {
        FuelBudgetScope::Operation => Ok(("operation", operation_id, None, None, None, None)),
        FuelBudgetScope::Unit {
            stage_run_unit_id,
            scope_snapshot_id,
            organization_id,
        } => {
            validate_ids(&[*stage_run_unit_id, *scope_snapshot_id, *organization_id])?;
            Ok((
                "unit",
                *stage_run_unit_id,
                Some(*stage_run_unit_id),
                Some(*scope_snapshot_id),
                Some(*organization_id),
                None,
            ))
        }
        FuelBudgetScope::Task {
            stage_run_unit_id,
            scope_snapshot_id,
            organization_id,
            task_id,
        } => {
            validate_ids(&[
                *stage_run_unit_id,
                *scope_snapshot_id,
                *organization_id,
                *task_id,
            ])?;
            Ok((
                "task",
                *task_id,
                Some(*stage_run_unit_id),
                Some(*scope_snapshot_id),
                Some(*organization_id),
                Some(*task_id),
            ))
        }
    }
}

async fn load_budget_by_request_on(
    tx: &mut Transaction<'_, Postgres>,
    stable_request_id: Uuid,
) -> InvestigationFuelStoreResult<FuelBudgetRow> {
    Ok(sqlx::query_as::<_, FuelBudgetRow>(
        r#"SELECT budget_id,stable_request_id,authority_id,operation_id,
                  stage_execution_id,owning_stage_run_request_id,scope_kind,
                  owner_id,stage_run_unit_id,scope_snapshot_id,organization_id,
                  task_id,budget_contract_version
             FROM investigation_fuel_budgets WHERE stable_request_id=$1"#,
    )
    .bind(stable_request_id)
    .fetch_one(&mut **tx)
    .await?)
}

async fn load_head_on(
    tx: &mut Transaction<'_, Postgres>,
    budget_id: Uuid,
    axis: &str,
) -> InvestigationFuelStoreResult<FuelHeadRow> {
    Ok(sqlx::query_as::<_, FuelHeadRow>(
        r#"SELECT budget_id,axis,limit_amount,reserved_amount,consumed_amount,
                  unknown_held_amount,refunded_before_begin_amount,head_version
             FROM investigation_fuel_budget_heads
            WHERE budget_id=$1 AND axis=$2"#,
    )
    .bind(budget_id)
    .bind(axis)
    .fetch_one(&mut **tx)
    .await?)
}

async fn load_reservation_optional_on(
    tx: &mut Transaction<'_, Postgres>,
    reservation_id: Uuid,
) -> InvestigationFuelStoreResult<Option<FuelReservationRow>> {
    Ok(sqlx::query_as::<_, FuelReservationRow>(
        r#"SELECT reservation_id,budget_id,axis,amount,work_key_sha256,state,
                  reservation_epoch,row_version
             FROM investigation_fuel_reservations WHERE reservation_id=$1"#,
    )
    .bind(reservation_id)
    .fetch_optional(&mut **tx)
    .await?)
}

async fn load_reservation_on(
    tx: &mut Transaction<'_, Postgres>,
    reservation_id: Uuid,
) -> InvestigationFuelStoreResult<FuelReservationRow> {
    load_reservation_optional_on(tx, reservation_id)
        .await?
        .ok_or_else(|| sqlx::Error::RowNotFound.into())
}

async fn load_semantic_cycle_by_request(
    pool: &PgPool,
    stable_request_id: Uuid,
) -> InvestigationFuelStoreResult<SemanticCycleRow> {
    Ok(sqlx::query_as::<_, SemanticCycleRow>(
        r#"SELECT semantic_cycle_receipt_id,stable_request_id,task_id,operation_id,
                  stage_execution_id,stage_run_unit_id,scope_snapshot_id,organization_id,
                  cycle_fingerprint_sha256,hypothesis_revision_sha256,
                  verification_plan_sha256,semantic_evidence_set_sha256,
                  open_obligation_set_sha256,remaining_work_set_sha256,disposition,
                  residual_reason_code,stop_receipt_id,receipt_sha256
             FROM investigation_semantic_cycle_receipts WHERE stable_request_id=$1"#,
    )
    .bind(stable_request_id)
    .fetch_one(pool)
    .await?)
}

fn validate_ids(ids: &[Uuid]) -> InvestigationFuelStoreResult<()> {
    if ids.iter().any(Uuid::is_nil) {
        return Err(InvestigationFuelStoreError::InvalidInput("uuid"));
    }
    Ok(())
}

fn validate_hash(value: &str) -> InvestigationFuelStoreResult<()> {
    let valid = value.strip_prefix("sha256:").is_some_and(|hex| {
        hex.len() == 64
            && hex
                .bytes()
                .all(|byte| byte.is_ascii_hexdigit() && !byte.is_ascii_uppercase())
    });
    if !valid {
        return Err(InvestigationFuelStoreError::InvalidInput("sha256"));
    }
    Ok(())
}

const fn axis_name(axis: InvestigationFuelAxisV1) -> &'static str {
    match axis {
        InvestigationFuelAxisV1::AnalysisGeneration => "analysis_generation",
        InvestigationFuelAxisV1::VerificationTask => "verification_task",
        InvestigationFuelAxisV1::Campaign => "campaign",
        InvestigationFuelAxisV1::Subtask => "subtask",
        InvestigationFuelAxisV1::NestedDelegation => "nested_delegation",
        InvestigationFuelAxisV1::ConsultOrToolCall => "consult_or_tool_call",
        InvestigationFuelAxisV1::PreparedAction => "prepared_action",
        InvestigationFuelAxisV1::WallClockMillis => "wall_clock_millis",
        InvestigationFuelAxisV1::ProviderToken => "provider_token",
        InvestigationFuelAxisV1::RiskMicros => "risk_micros",
    }
}

const fn state_name(state: InvestigationFuelReservationStateV1) -> &'static str {
    match state {
        InvestigationFuelReservationStateV1::Reserved => "reserved",
        InvestigationFuelReservationStateV1::Consumed => "consumed",
        InvestigationFuelReservationStateV1::RefundedBeforeBegin => "refunded_before_begin",
        InvestigationFuelReservationStateV1::UnknownHeld => "unknown_held",
    }
}

fn sha256_json(value: &impl Serialize) -> String {
    let digest = Sha256::digest(
        serde_json::to_vec(value).expect("Investigation persistence identity is serializable"),
    )
    .iter()
    .map(|byte| format!("{byte:02x}"))
    .collect::<String>();
    format!("sha256:{digest}")
}
