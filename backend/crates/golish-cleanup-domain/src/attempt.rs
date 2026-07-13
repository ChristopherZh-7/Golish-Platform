use serde::{Deserialize, Serialize};

use crate::{CleanupError, CleanupObligationStatus};

#[derive(Clone, Copy, Debug, Eq, PartialEq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum CleanupAttemptStatus {
    Claimed,
    Executing,
    CleanedPendingVerification,
    VerifiedAbsent,
    VerificationFailed,
    ExecutionFailed,
}

impl CleanupAttemptStatus {
    pub const fn is_live(self) -> bool {
        matches!(
            self,
            Self::Claimed | Self::Executing | Self::CleanedPendingVerification
        )
    }

    pub const fn is_terminal(self) -> bool {
        !self.is_live()
    }
}

#[derive(Clone, Copy, Debug, Eq, PartialEq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum AbsenceResult {
    Absent,
    StillPresent,
    Inconclusive,
}

#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub struct CleanupTransition {
    pub attempt: CleanupAttemptStatus,
    pub obligation: CleanupObligationStatus,
    pub may_create_next_attempt: bool,
}

pub fn begin_execution(status: CleanupAttemptStatus) -> Result<CleanupAttemptStatus, CleanupError> {
    match status {
        CleanupAttemptStatus::Claimed => Ok(CleanupAttemptStatus::Executing),
        _ => Err(CleanupError::InvalidAttemptTransition),
    }
}

pub fn mark_cleaned_pending_verification(
    status: CleanupAttemptStatus,
) -> Result<CleanupAttemptStatus, CleanupError> {
    match status {
        CleanupAttemptStatus::Executing => Ok(CleanupAttemptStatus::CleanedPendingVerification),
        _ => Err(CleanupError::InvalidAttemptTransition),
    }
}

pub fn apply_absence_result(
    status: CleanupAttemptStatus,
    result: AbsenceResult,
) -> Result<CleanupTransition, CleanupError> {
    if status != CleanupAttemptStatus::CleanedPendingVerification {
        return Err(CleanupError::InvalidAttemptTransition);
    }
    Ok(match result {
        AbsenceResult::Absent => CleanupTransition {
            attempt: CleanupAttemptStatus::VerifiedAbsent,
            obligation: CleanupObligationStatus::VerifiedAbsent,
            may_create_next_attempt: false,
        },
        AbsenceResult::StillPresent | AbsenceResult::Inconclusive => CleanupTransition {
            attempt: CleanupAttemptStatus::VerificationFailed,
            obligation: CleanupObligationStatus::Open,
            may_create_next_attempt: true,
        },
    })
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn inconclusive_absence_closes_attempt_but_keeps_obligation_retryable() {
        let next = apply_absence_result(
            CleanupAttemptStatus::CleanedPendingVerification,
            AbsenceResult::Inconclusive,
        )
        .unwrap();
        assert_eq!(next.attempt, CleanupAttemptStatus::VerificationFailed);
        assert_eq!(next.obligation, CleanupObligationStatus::Open);
        assert!(next.may_create_next_attempt);
    }

    #[test]
    fn only_the_three_nonterminal_attempt_states_hold_the_live_slot() {
        assert!(CleanupAttemptStatus::Claimed.is_live());
        assert!(CleanupAttemptStatus::Executing.is_live());
        assert!(CleanupAttemptStatus::CleanedPendingVerification.is_live());
        assert!(CleanupAttemptStatus::VerifiedAbsent.is_terminal());
        assert!(CleanupAttemptStatus::VerificationFailed.is_terminal());
        assert!(CleanupAttemptStatus::ExecutionFailed.is_terminal());
    }

    #[test]
    fn absence_cannot_skip_cleanup_execution() {
        assert_eq!(
            apply_absence_result(CleanupAttemptStatus::Executing, AbsenceResult::Absent),
            Err(CleanupError::InvalidAttemptTransition)
        );
    }
}
