use chrono::{DateTime, Utc};
use golish_cleanup_domain::CleanupAttemptStatus;

pub fn may_reclaim(
    status: CleanupAttemptStatus,
    lease_expires_at: DateTime<Utc>,
    now: DateTime<Utc>,
) -> bool {
    matches!(
        status,
        CleanupAttemptStatus::Claimed | CleanupAttemptStatus::Executing
    ) && lease_expires_at <= now
}
