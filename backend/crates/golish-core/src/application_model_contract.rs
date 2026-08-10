//! Operation-frozen contract for the Application Understanding insertion.
//!
//! This type is intentionally a closed, dependency-free value. Database
//! defaults and runtime selection live in higher layers; once an operation is
//! created, those layers must persist one of these exact values and must not
//! reinterpret it from the current stage or deployment state.

use serde::{Deserialize, Serialize};

#[derive(Debug, Default, Clone, Copy, PartialEq, Eq, Hash, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum ApplicationModelContract {
    #[default]
    LegacyNoModel,
    ApplicationModelV1,
}

impl ApplicationModelContract {
    pub const ALL: [Self; 2] = [Self::LegacyNoModel, Self::ApplicationModelV1];

    pub const fn as_str(self) -> &'static str {
        match self {
            Self::LegacyNoModel => "legacy_no_model",
            Self::ApplicationModelV1 => "application_model_v1",
        }
    }
}

#[derive(Debug, Clone, PartialEq, Eq, thiserror::Error)]
#[error("unknown application-model operation contract: {0}")]
pub struct ApplicationModelContractParseError(String);

impl TryFrom<&str> for ApplicationModelContract {
    type Error = ApplicationModelContractParseError;

    fn try_from(value: &str) -> Result<Self, Self::Error> {
        match value {
            "legacy_no_model" => Ok(Self::LegacyNoModel),
            "application_model_v1" => Ok(Self::ApplicationModelV1),
            other => Err(ApplicationModelContractParseError(other.to_string())),
        }
    }
}

impl TryFrom<String> for ApplicationModelContract {
    type Error = ApplicationModelContractParseError;

    fn try_from(value: String) -> Result<Self, Self::Error> {
        Self::try_from(value.as_str())
    }
}

/// Compatibility name used by the stage-graph cutover API. The shorter core
/// name is the durable cross-layer contract; both names are the same type.
pub type ApplicationModelOperationContract = ApplicationModelContract;

#[cfg(test)]
mod tests {
    use super::ApplicationModelContract;

    #[test]
    fn application_model_contract_values_round_trip_and_reject_unknown_values() {
        for contract in ApplicationModelContract::ALL {
            assert_eq!(
                ApplicationModelContract::try_from(contract.as_str()),
                Ok(contract)
            );
        }
        assert!(ApplicationModelContract::try_from("latest_if_available").is_err());
    }
}
