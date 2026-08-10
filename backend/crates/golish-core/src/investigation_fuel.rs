//! Deterministic unified-Investigation fuel and semantic-cycle contracts.

use serde::{Deserialize, Serialize};
use sha2::{Digest, Sha256};
use std::collections::{BTreeMap, BTreeSet};
use uuid::Uuid;

#[derive(Debug, Clone, Copy, PartialEq, Eq, PartialOrd, Ord, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum InvestigationFuelAxisV1 {
    AnalysisGeneration,
    VerificationTask,
    Campaign,
    Subtask,
    NestedDelegation,
    ConsultOrToolCall,
    PreparedAction,
    WallClockMillis,
    ProviderToken,
    RiskMicros,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum InvestigationFuelReservationStateV1 {
    Reserved,
    Consumed,
    RefundedBeforeBegin,
    UnknownHeld,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize)]
pub struct InvestigationFuelHeadV1 {
    pub axis: InvestigationFuelAxisV1,
    pub limit: u64,
    pub reserved: u64,
    pub consumed: u64,
    pub unknown_held: u64,
    pub refunded_before_begin: u64,
    pub head_version: u64,
}

impl InvestigationFuelHeadV1 {
    pub fn remaining(&self) -> u64 {
        self.limit.saturating_sub(
            self.reserved
                .saturating_add(self.consumed)
                .saturating_add(self.unknown_held),
        )
    }

    fn validate(&self) -> Result<(), InvestigationFuelError> {
        if self.limit == 0 {
            return Err(InvestigationFuelError::InvalidLimit(self.axis));
        }
        if self
            .reserved
            .saturating_add(self.consumed)
            .saturating_add(self.unknown_held)
            > self.limit
        {
            return Err(InvestigationFuelError::InvariantViolation(self.axis));
        }
        Ok(())
    }
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize)]
pub struct InvestigationFuelReservationV1 {
    pub reservation_id: Uuid,
    pub axis: InvestigationFuelAxisV1,
    pub amount: u64,
    pub work_key_sha256: String,
    pub state: InvestigationFuelReservationStateV1,
    pub reservation_epoch: u64,
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct InvestigationFuelLedgerV1 {
    heads: BTreeMap<InvestigationFuelAxisV1, InvestigationFuelHeadV1>,
    reservations: BTreeMap<Uuid, InvestigationFuelReservationV1>,
}

impl InvestigationFuelLedgerV1 {
    pub fn new(
        limits: impl IntoIterator<Item = (InvestigationFuelAxisV1, u64)>,
    ) -> Result<Self, InvestigationFuelError> {
        let mut heads = BTreeMap::new();
        for (axis, limit) in limits {
            if limit == 0 || heads.contains_key(&axis) {
                return Err(InvestigationFuelError::InvalidLimit(axis));
            }
            heads.insert(
                axis,
                InvestigationFuelHeadV1 {
                    axis,
                    limit,
                    reserved: 0,
                    consumed: 0,
                    unknown_held: 0,
                    refunded_before_begin: 0,
                    head_version: 0,
                },
            );
        }
        if heads.is_empty() {
            return Err(InvestigationFuelError::EmptyBudget);
        }
        Ok(Self {
            heads,
            reservations: BTreeMap::new(),
        })
    }

    pub fn head(
        &self,
        axis: InvestigationFuelAxisV1,
    ) -> Result<&InvestigationFuelHeadV1, InvestigationFuelError> {
        self.heads
            .get(&axis)
            .ok_or(InvestigationFuelError::UnknownAxis(axis))
    }

    pub fn reservation(&self, id: Uuid) -> Option<&InvestigationFuelReservationV1> {
        self.reservations.get(&id)
    }

    pub fn reserve(
        &mut self,
        axis: InvestigationFuelAxisV1,
        amount: u64,
        work_key_sha256: String,
        reservation_id: Uuid,
        expected_head_version: u64,
    ) -> Result<InvestigationFuelReservationV1, InvestigationFuelError> {
        if reservation_id.is_nil() || amount == 0 {
            return Err(InvestigationFuelError::InvalidReservation);
        }
        validate_sha256(&work_key_sha256)?;
        if let Some(existing) = self.reservations.get(&reservation_id) {
            if existing.axis == axis
                && existing.amount == amount
                && existing.work_key_sha256 == work_key_sha256
            {
                return Ok(existing.clone());
            }
            return Err(InvestigationFuelError::ReservationIdentityCollision);
        }
        let head = self
            .heads
            .get_mut(&axis)
            .ok_or(InvestigationFuelError::UnknownAxis(axis))?;
        if head.head_version != expected_head_version {
            return Err(InvestigationFuelError::StaleHead {
                expected: head.head_version,
                actual: expected_head_version,
            });
        }
        if head.remaining() < amount {
            return Err(InvestigationFuelError::Exhausted {
                axis,
                requested: amount,
                remaining: head.remaining(),
            });
        }
        head.reserved = head.reserved.saturating_add(amount);
        head.head_version = head.head_version.saturating_add(1);
        head.validate()?;
        let reservation = InvestigationFuelReservationV1 {
            reservation_id,
            axis,
            amount,
            work_key_sha256,
            state: InvestigationFuelReservationStateV1::Reserved,
            reservation_epoch: head.head_version,
        };
        self.reservations
            .insert(reservation_id, reservation.clone());
        Ok(reservation)
    }

    pub fn consume(
        &mut self,
        reservation_id: Uuid,
        expected_head_version: u64,
    ) -> Result<(), InvestigationFuelError> {
        self.transition_reservation(
            reservation_id,
            expected_head_version,
            InvestigationFuelReservationStateV1::Consumed,
        )
    }

    pub fn mark_unknown_held(
        &mut self,
        reservation_id: Uuid,
        expected_head_version: u64,
    ) -> Result<(), InvestigationFuelError> {
        self.transition_reservation(
            reservation_id,
            expected_head_version,
            InvestigationFuelReservationStateV1::UnknownHeld,
        )
    }

    pub fn refund_before_begin(
        &mut self,
        reservation_id: Uuid,
        expected_head_version: u64,
    ) -> Result<(), InvestigationFuelError> {
        self.transition_reservation(
            reservation_id,
            expected_head_version,
            InvestigationFuelReservationStateV1::RefundedBeforeBegin,
        )
    }

    fn transition_reservation(
        &mut self,
        reservation_id: Uuid,
        expected_head_version: u64,
        next: InvestigationFuelReservationStateV1,
    ) -> Result<(), InvestigationFuelError> {
        let reservation = self
            .reservations
            .get(&reservation_id)
            .ok_or(InvestigationFuelError::UnknownReservation)?;
        if reservation.state == next {
            return Ok(());
        }
        if reservation.state != InvestigationFuelReservationStateV1::Reserved {
            return Err(InvestigationFuelError::IllegalReservationTransition {
                current: reservation.state,
                next,
            });
        }
        let axis = reservation.axis;
        let amount = reservation.amount;
        let head = self
            .heads
            .get_mut(&axis)
            .ok_or(InvestigationFuelError::UnknownAxis(axis))?;
        if head.head_version != expected_head_version {
            return Err(InvestigationFuelError::StaleHead {
                expected: head.head_version,
                actual: expected_head_version,
            });
        }
        head.reserved = head
            .reserved
            .checked_sub(amount)
            .ok_or(InvestigationFuelError::InvariantViolation(axis))?;
        match next {
            InvestigationFuelReservationStateV1::Consumed => {
                head.consumed = head.consumed.saturating_add(amount);
            }
            InvestigationFuelReservationStateV1::UnknownHeld => {
                head.unknown_held = head.unknown_held.saturating_add(amount);
            }
            InvestigationFuelReservationStateV1::RefundedBeforeBegin => {
                head.refunded_before_begin = head.refunded_before_begin.saturating_add(amount);
            }
            InvestigationFuelReservationStateV1::Reserved => {
                return Err(InvestigationFuelError::IllegalReservationTransition {
                    current: InvestigationFuelReservationStateV1::Reserved,
                    next,
                })
            }
        }
        head.head_version = head.head_version.saturating_add(1);
        head.validate()?;
        self.reservations
            .get_mut(&reservation_id)
            .expect("reservation exists")
            .state = next;
        Ok(())
    }
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize)]
pub struct InvestigationSemanticCycleReceiptV1 {
    pub cycle_fingerprint_sha256: String,
    pub hypothesis_revision_sha256: String,
    pub verification_plan_sha256: String,
    pub semantic_evidence_set_sha256: String,
    pub open_obligation_set_sha256: String,
    pub remaining_work_set_sha256: String,
}

impl InvestigationSemanticCycleReceiptV1 {
    pub fn host_create(
        hypothesis_revision_sha256: String,
        verification_plan_sha256: String,
        semantic_evidence_set_sha256: String,
        open_obligation_set_sha256: String,
        remaining_work_set_sha256: String,
    ) -> Result<Self, InvestigationFuelError> {
        for hash in [
            &hypothesis_revision_sha256,
            &verification_plan_sha256,
            &semantic_evidence_set_sha256,
            &open_obligation_set_sha256,
            &remaining_work_set_sha256,
        ] {
            validate_sha256(hash)?;
        }
        let cycle_fingerprint_sha256 = sha256_json(&(
            &hypothesis_revision_sha256,
            &verification_plan_sha256,
            &semantic_evidence_set_sha256,
            &open_obligation_set_sha256,
            &remaining_work_set_sha256,
        ));
        Ok(Self {
            cycle_fingerprint_sha256,
            hypothesis_revision_sha256,
            verification_plan_sha256,
            semantic_evidence_set_sha256,
            open_obligation_set_sha256,
            remaining_work_set_sha256,
        })
    }
}

pub fn append_semantic_cycle_once(
    seen: &mut BTreeSet<String>,
    receipt: &InvestigationSemanticCycleReceiptV1,
) -> Result<(), InvestigationFuelError> {
    if !seen.insert(receipt.cycle_fingerprint_sha256.clone()) {
        return Err(InvestigationFuelError::SemanticCycleRepeated);
    }
    Ok(())
}

#[derive(Debug, thiserror::Error, Clone, PartialEq, Eq)]
pub enum InvestigationFuelError {
    #[error("investigation fuel budget is empty")]
    EmptyBudget,
    #[error("invalid limit for fuel axis {0:?}")]
    InvalidLimit(InvestigationFuelAxisV1),
    #[error("unknown fuel axis {0:?}")]
    UnknownAxis(InvestigationFuelAxisV1),
    #[error("invalid fuel reservation")]
    InvalidReservation,
    #[error("fuel reservation identity collision")]
    ReservationIdentityCollision,
    #[error("unknown fuel reservation")]
    UnknownReservation,
    #[error("stale fuel head: expected {expected}, got {actual}")]
    StaleHead { expected: u64, actual: u64 },
    #[error("fuel exhausted for {axis:?}: requested {requested}, remaining {remaining}")]
    Exhausted {
        axis: InvestigationFuelAxisV1,
        requested: u64,
        remaining: u64,
    },
    #[error("invalid fuel work-key hash")]
    InvalidHash,
    #[error("fuel invariant violated for {0:?}")]
    InvariantViolation(InvestigationFuelAxisV1),
    #[error("illegal fuel reservation transition {current:?} -> {next:?}")]
    IllegalReservationTransition {
        current: InvestigationFuelReservationStateV1,
        next: InvestigationFuelReservationStateV1,
    },
    #[error("investigation semantic cycle repeated")]
    SemanticCycleRepeated,
}

fn validate_sha256(value: &str) -> Result<(), InvestigationFuelError> {
    let Some(hex) = value.strip_prefix("sha256:") else {
        return Err(InvestigationFuelError::InvalidHash);
    };
    if hex.len() != 64
        || !hex
            .bytes()
            .all(|byte| byte.is_ascii_hexdigit() && !byte.is_ascii_uppercase())
    {
        return Err(InvestigationFuelError::InvalidHash);
    }
    Ok(())
}

fn sha256_json(value: &impl Serialize) -> String {
    let digest = Sha256::digest(
        serde_json::to_vec(value).expect("investigation fuel identity material is serializable"),
    )
    .iter()
    .map(|byte| format!("{byte:02x}"))
    .collect::<String>();
    format!("sha256:{digest}")
}

#[cfg(test)]
mod tests {
    use super::*;

    fn hash(byte: char) -> String {
        format!("sha256:{}", byte.to_string().repeat(64))
    }

    #[test]
    fn investigation_fuel_atomic_reservation_cannot_oversell_or_accept_stale_head() {
        let axis = InvestigationFuelAxisV1::Campaign;
        let mut ledger = InvestigationFuelLedgerV1::new([(axis, 1)]).unwrap();
        ledger
            .reserve(axis, 1, hash('a'), Uuid::from_u128(1), 0)
            .unwrap();
        assert_eq!(
            ledger.reserve(axis, 1, hash('b'), Uuid::from_u128(2), 0),
            Err(InvestigationFuelError::StaleHead {
                expected: 1,
                actual: 0
            })
        );
        assert!(matches!(
            ledger.reserve(axis, 1, hash('b'), Uuid::from_u128(2), 1),
            Err(InvestigationFuelError::Exhausted { .. })
        ));
    }

    #[test]
    fn investigation_fuel_durable_begin_consumes_and_unknown_execution_stays_held() {
        let axis = InvestigationFuelAxisV1::PreparedAction;
        let mut ledger = InvestigationFuelLedgerV1::new([(axis, 2)]).unwrap();
        ledger
            .reserve(axis, 1, hash('a'), Uuid::from_u128(1), 0)
            .unwrap();
        ledger.consume(Uuid::from_u128(1), 1).unwrap();
        ledger
            .reserve(axis, 1, hash('b'), Uuid::from_u128(2), 2)
            .unwrap();
        ledger.mark_unknown_held(Uuid::from_u128(2), 3).unwrap();
        let head = ledger.head(axis).unwrap();
        assert_eq!(head.consumed, 1);
        assert_eq!(head.unknown_held, 1);
        assert_eq!(head.remaining(), 0);
        assert_eq!(
            ledger.refund_before_begin(Uuid::from_u128(2), 4),
            Err(InvestigationFuelError::IllegalReservationTransition {
                current: InvestigationFuelReservationStateV1::UnknownHeld,
                next: InvestigationFuelReservationStateV1::RefundedBeforeBegin,
            })
        );
    }

    #[test]
    fn investigation_fuel_refund_is_allowed_only_before_begin() {
        let axis = InvestigationFuelAxisV1::Subtask;
        let mut ledger = InvestigationFuelLedgerV1::new([(axis, 1)]).unwrap();
        ledger
            .reserve(axis, 1, hash('a'), Uuid::from_u128(1), 0)
            .unwrap();
        ledger.refund_before_begin(Uuid::from_u128(1), 1).unwrap();
        assert_eq!(ledger.head(axis).unwrap().remaining(), 1);
        assert_eq!(
            ledger.consume(Uuid::from_u128(1), 2),
            Err(InvestigationFuelError::IllegalReservationTransition {
                current: InvestigationFuelReservationStateV1::RefundedBeforeBegin,
                next: InvestigationFuelReservationStateV1::Consumed,
            })
        );
    }

    #[test]
    fn investigation_fuel_semantic_cycle_guard_is_timestamp_independent() {
        let first = InvestigationSemanticCycleReceiptV1::host_create(
            hash('a'),
            hash('b'),
            hash('c'),
            hash('d'),
            hash('e'),
        )
        .unwrap();
        let replay = first.clone();
        let mut seen = BTreeSet::new();
        append_semantic_cycle_once(&mut seen, &first).unwrap();
        assert_eq!(
            append_semantic_cycle_once(&mut seen, &replay),
            Err(InvestigationFuelError::SemanticCycleRepeated)
        );
    }
}
