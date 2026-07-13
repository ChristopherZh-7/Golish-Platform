use golish_cleanup_domain::CleanupAttemptStatus;

#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub enum ReconcileAction {
    AwaitLease,
    VerifyAbsence,
    Retry,
    Terminal,
}

pub const fn reconcile_action(
    status: CleanupAttemptStatus,
    lease_expired: bool,
) -> ReconcileAction {
    match status {
        CleanupAttemptStatus::Claimed | CleanupAttemptStatus::Executing if !lease_expired => {
            ReconcileAction::AwaitLease
        }
        CleanupAttemptStatus::Claimed | CleanupAttemptStatus::Executing => ReconcileAction::Retry,
        CleanupAttemptStatus::CleanedPendingVerification => ReconcileAction::VerifyAbsence,
        CleanupAttemptStatus::VerificationFailed | CleanupAttemptStatus::ExecutionFailed => {
            ReconcileAction::Retry
        }
        CleanupAttemptStatus::VerifiedAbsent => ReconcileAction::Terminal,
    }
}
