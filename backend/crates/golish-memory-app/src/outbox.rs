use serde::{Deserialize, Serialize};

#[derive(Clone, Copy, Debug, Eq, Hash, PartialEq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum DeliveryStatus {
    BlockedDependency,
    Pending,
    Leased,
    Succeeded,
    SucceededSuppressed,
    RetryableFailed,
    Stale,
    DeadLetter,
}

impl DeliveryStatus {
    pub const fn as_str(self) -> &'static str {
        match self {
            Self::BlockedDependency => "blocked_dependency",
            Self::Pending => "pending",
            Self::Leased => "leased",
            Self::Succeeded => "succeeded",
            Self::SucceededSuppressed => "succeeded_suppressed",
            Self::RetryableFailed => "retryable_failed",
            Self::Stale => "stale",
            Self::DeadLetter => "dead_letter",
        }
    }

    pub const fn satisfies_dependency(self) -> bool {
        matches!(self, Self::Succeeded | Self::SucceededSuppressed)
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn succeeded_and_suppressed_both_satisfy_dependencies() {
        assert!(DeliveryStatus::Succeeded.satisfies_dependency());
        assert!(DeliveryStatus::SucceededSuppressed.satisfies_dependency());
        assert!(!DeliveryStatus::Pending.satisfies_dependency());
    }
}
