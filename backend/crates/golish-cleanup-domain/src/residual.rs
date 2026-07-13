use serde::{Deserialize, Serialize};

#[derive(Clone, Debug, Eq, PartialEq, Serialize, Deserialize)]
pub struct ResidualRisk {
    pub summary: String,
    pub severity: String,
}

impl ResidualRisk {
    pub fn validate(&self) -> bool {
        !self.summary.trim().is_empty()
            && self.summary.len() <= 4_096
            && matches!(
                self.severity.as_str(),
                "low" | "medium" | "high" | "critical"
            )
    }
}
