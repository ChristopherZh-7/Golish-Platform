//! Deterministic closure contract for one unified Investigation stage run.
//!
//! The model never constructs or validates this authority. The host derives
//! every census from durable rows bound to one operation, stage execution and
//! owning `stage_run` request, then persists the validated closure. A residual
//! is terminal/reportable work, not evidence that the subject was checked.

use serde::{Deserialize, Serialize};
use uuid::Uuid;

pub const INVESTIGATION_RUN_CLOSURE_CONTRACT_V1: &str = "investigation_run_closure.v1";

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum InvestigationRunClosureDispositionV1 {
    Pass,
    PassWithGaps,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct InvestigationExactSetCensusV1 {
    pub member_count: u32,
    pub member_set_sha256: String,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct InvestigationTerminalWorkCensusV1 {
    pub total_count: u32,
    pub terminal_count: u32,
    pub cancelled_before_start_count: u32,
    pub recovery_required_count: u32,
    pub member_set_sha256: String,
}

impl InvestigationTerminalWorkCensusV1 {
    pub const fn is_fully_accounted(&self) -> bool {
        self.recovery_required_count == 0
            && self.total_count
                == self
                    .terminal_count
                    .saturating_add(self.cancelled_before_start_count)
    }
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct InvestigationDelegationCensusV1 {
    pub task_count: u32,
    pub primary_count: u32,
    pub runnable_subtask_count: u32,
    pub independently_dispatched_subtask_count: u32,
    pub logical_dispatch_count: u32,
    pub unique_logical_dispatch_count: u32,
    pub sealed_task_census_count: u32,
    pub member_set_sha256: String,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct InvestigationFuelClosureV1 {
    pub reservation_count: u32,
    pub consumed_count: u32,
    pub refunded_count: u32,
    pub unknown_held_count: u32,
    pub open_count: u32,
    pub semantic_cycle_count: u32,
    pub reservation_set_sha256: String,
    pub semantic_cycle_set_sha256: String,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct InvestigationRunClosureV1 {
    pub closure_id: Uuid,
    pub operation_id: Uuid,
    pub stage_execution_id: Uuid,
    pub owning_stage_run_request_id: String,
    pub scope_snapshot_id: Uuid,
    pub run_state_head_version: u64,
    pub stop_epoch: u64,
    pub snapshot_set: InvestigationExactSetCensusV1,
    pub main_read_session_set: InvestigationExactSetCensusV1,
    pub generation_set: InvestigationExactSetCensusV1,
    pub admission_set: InvestigationExactSetCensusV1,
    pub verification_task_set: InvestigationExactSetCensusV1,
    pub objective_assignment_set: InvestigationExactSetCensusV1,
    pub objective_outcome_set: InvestigationExactSetCensusV1,
    pub work: InvestigationTerminalWorkCensusV1,
    pub campaigns: InvestigationTerminalWorkCensusV1,
    pub prepared_actions: InvestigationTerminalWorkCensusV1,
    pub fact_deltas: InvestigationTerminalWorkCensusV1,
    pub delegation: InvestigationDelegationCensusV1,
    pub fuel: InvestigationFuelClosureV1,
    pub fixed_point_receipt_id: Uuid,
    pub fixed_point_receipt_sha256: String,
    pub residual_set: InvestigationExactSetCensusV1,
    pub disposition: InvestigationRunClosureDispositionV1,
    pub contract_version: String,
}

impl InvestigationRunClosureV1 {
    pub fn validate(&self) -> Result<(), InvestigationRunClosureError> {
        if [
            self.closure_id,
            self.operation_id,
            self.stage_execution_id,
            self.scope_snapshot_id,
            self.fixed_point_receipt_id,
        ]
        .into_iter()
        .any(|id| id.is_nil())
        {
            return Err(InvestigationRunClosureError::InvalidIdentity);
        }
        if self.owning_stage_run_request_id.trim().is_empty()
            || self.owning_stage_run_request_id.len() > 512
            || self.contract_version != INVESTIGATION_RUN_CLOSURE_CONTRACT_V1
        {
            return Err(InvestigationRunClosureError::InvalidContract);
        }
        for hash in self.all_hashes() {
            validate_sha256(hash)?;
        }
        if self.snapshot_set.member_count == 0
            || self.main_read_session_set.member_count != self.snapshot_set.member_count
            || self.generation_set.member_count == 0
            || self.admission_set.member_count != self.generation_set.member_count
        {
            return Err(InvestigationRunClosureError::AuthoritySetIncomplete);
        }
        if !self.work.is_fully_accounted()
            || !self.campaigns.is_fully_accounted()
            || !self.prepared_actions.is_fully_accounted()
            || !self.fact_deltas.is_fully_accounted()
        {
            return Err(InvestigationRunClosureError::OpenWork);
        }
        if self.delegation.task_count != self.delegation.primary_count
            || self.delegation.task_count != self.delegation.sealed_task_census_count
            || self.delegation.runnable_subtask_count
                != self.delegation.independently_dispatched_subtask_count
            || self.delegation.logical_dispatch_count
                != self.delegation.unique_logical_dispatch_count
        {
            return Err(InvestigationRunClosureError::DelegationCensusInvalid);
        }
        if self.fuel.open_count > 0 || self.fuel.unknown_held_count > 0 {
            return Err(InvestigationRunClosureError::FuelNotClosed);
        }
        let settled_fuel = self
            .fuel
            .consumed_count
            .checked_add(self.fuel.refunded_count)
            .ok_or(InvestigationRunClosureError::FuelNotClosed)?;
        if settled_fuel != self.fuel.reservation_count {
            return Err(InvestigationRunClosureError::FuelNotClosed);
        }
        match (self.residual_set.member_count, self.disposition) {
            (0, InvestigationRunClosureDispositionV1::Pass)
            | (1.., InvestigationRunClosureDispositionV1::PassWithGaps) => Ok(()),
            _ => Err(InvestigationRunClosureError::DispositionMismatch),
        }
    }

    fn all_hashes(&self) -> [&str; 16] {
        [
            &self.snapshot_set.member_set_sha256,
            &self.main_read_session_set.member_set_sha256,
            &self.generation_set.member_set_sha256,
            &self.admission_set.member_set_sha256,
            &self.verification_task_set.member_set_sha256,
            &self.objective_assignment_set.member_set_sha256,
            &self.objective_outcome_set.member_set_sha256,
            &self.work.member_set_sha256,
            &self.campaigns.member_set_sha256,
            &self.prepared_actions.member_set_sha256,
            &self.fact_deltas.member_set_sha256,
            &self.delegation.member_set_sha256,
            &self.fuel.reservation_set_sha256,
            &self.fuel.semantic_cycle_set_sha256,
            &self.fixed_point_receipt_sha256,
            &self.residual_set.member_set_sha256,
        ]
    }
}

#[derive(Debug, thiserror::Error, Clone, PartialEq, Eq)]
pub enum InvestigationRunClosureError {
    #[error("invalid unified Investigation closure identity")]
    InvalidIdentity,
    #[error("invalid unified Investigation closure contract")]
    InvalidContract,
    #[error("invalid unified Investigation closure hash")]
    InvalidHash,
    #[error("unified Investigation authority exact set is incomplete")]
    AuthoritySetIncomplete,
    #[error("unified Investigation still owns nonterminal work")]
    OpenWork,
    #[error("unified Investigation delegation census is invalid")]
    DelegationCensusInvalid,
    #[error("unified Investigation fuel ledger is not closed")]
    FuelNotClosed,
    #[error("unified Investigation closure disposition does not match residuals")]
    DispositionMismatch,
}

fn validate_sha256(value: &str) -> Result<(), InvestigationRunClosureError> {
    let Some(hex) = value.strip_prefix("sha256:") else {
        return Err(InvestigationRunClosureError::InvalidHash);
    };
    if hex.len() != 64
        || !hex
            .bytes()
            .all(|byte| byte.is_ascii_hexdigit() && !byte.is_ascii_uppercase())
    {
        return Err(InvestigationRunClosureError::InvalidHash);
    }
    Ok(())
}

#[cfg(test)]
mod tests {
    use super::*;

    fn hash(nibble: char) -> String {
        format!("sha256:{}", nibble.to_string().repeat(64))
    }

    fn set(count: u32, nibble: char) -> InvestigationExactSetCensusV1 {
        InvestigationExactSetCensusV1 {
            member_count: count,
            member_set_sha256: hash(nibble),
        }
    }

    fn terminal(count: u32, nibble: char) -> InvestigationTerminalWorkCensusV1 {
        InvestigationTerminalWorkCensusV1 {
            total_count: count,
            terminal_count: count,
            cancelled_before_start_count: 0,
            recovery_required_count: 0,
            member_set_sha256: hash(nibble),
        }
    }

    fn valid_closure() -> InvestigationRunClosureV1 {
        InvestigationRunClosureV1 {
            closure_id: Uuid::new_v4(),
            operation_id: Uuid::new_v4(),
            stage_execution_id: Uuid::new_v4(),
            owning_stage_run_request_id: "stage-run-request".into(),
            scope_snapshot_id: Uuid::new_v4(),
            run_state_head_version: 7,
            stop_epoch: 0,
            snapshot_set: set(2, '1'),
            main_read_session_set: set(2, '2'),
            generation_set: set(2, '3'),
            admission_set: set(2, '4'),
            verification_task_set: set(1, '5'),
            objective_assignment_set: set(1, '6'),
            objective_outcome_set: set(1, '7'),
            work: terminal(6, '8'),
            campaigns: terminal(1, '9'),
            prepared_actions: terminal(1, 'a'),
            fact_deltas: terminal(1, 'b'),
            delegation: InvestigationDelegationCensusV1 {
                task_count: 2,
                primary_count: 2,
                runnable_subtask_count: 4,
                independently_dispatched_subtask_count: 4,
                logical_dispatch_count: 4,
                unique_logical_dispatch_count: 4,
                sealed_task_census_count: 2,
                member_set_sha256: hash('c'),
            },
            fuel: InvestigationFuelClosureV1 {
                reservation_count: 4,
                consumed_count: 3,
                refunded_count: 1,
                unknown_held_count: 0,
                open_count: 0,
                semantic_cycle_count: 1,
                reservation_set_sha256: hash('d'),
                semantic_cycle_set_sha256: hash('e'),
            },
            fixed_point_receipt_id: Uuid::new_v4(),
            fixed_point_receipt_sha256: hash('f'),
            residual_set: set(0, '0'),
            disposition: InvestigationRunClosureDispositionV1::Pass,
            contract_version: INVESTIGATION_RUN_CLOSURE_CONTRACT_V1.into(),
        }
    }

    #[test]
    fn closure_requires_exact_sessions_terminal_work_delegation_and_fuel() {
        valid_closure().validate().unwrap();

        let mut missing_session = valid_closure();
        missing_session.main_read_session_set.member_count = 1;
        assert_eq!(
            missing_session.validate(),
            Err(InvestigationRunClosureError::AuthoritySetIncomplete)
        );

        let mut open_action = valid_closure();
        open_action.prepared_actions.terminal_count = 0;
        assert_eq!(
            open_action.validate(),
            Err(InvestigationRunClosureError::OpenWork)
        );

        let mut primary_only = valid_closure();
        primary_only
            .delegation
            .independently_dispatched_subtask_count = 0;
        assert_eq!(
            primary_only.validate(),
            Err(InvestigationRunClosureError::DelegationCensusInvalid)
        );

        let mut held = valid_closure();
        held.fuel.unknown_held_count = 1;
        assert_eq!(
            held.validate(),
            Err(InvestigationRunClosureError::FuelNotClosed)
        );
    }

    #[test]
    fn residuals_are_pass_with_gaps_and_never_checked_empty() {
        let mut closure = valid_closure();
        closure.residual_set = set(2, '1');
        assert_eq!(
            closure.validate(),
            Err(InvestigationRunClosureError::DispositionMismatch)
        );
        closure.disposition = InvestigationRunClosureDispositionV1::PassWithGaps;
        closure.validate().unwrap();
    }

    #[test]
    fn closure_rejects_a_malformed_residual_exact_set_hash() {
        let mut closure = valid_closure();
        closure.residual_set.member_set_sha256 = "not-a-sha256".to_string();

        assert_eq!(
            closure.validate(),
            Err(InvestigationRunClosureError::InvalidHash)
        );
    }
}
