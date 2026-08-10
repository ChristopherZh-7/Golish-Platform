//! Pure, operation-frozen Target Intel Goal authority contracts.

use serde::{Deserialize, Serialize};
use serde_json::Value;
use sha2::{Digest, Sha256};
use uuid::Uuid;

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum IntelGoalRuntimeMode {
    Legacy,
    ObserveShadow,
    AdvisoryRework,
    IntelGoalV1,
}

impl IntelGoalRuntimeMode {
    pub const fn for_missing_row() -> Self {
        Self::Legacy
    }

    pub fn parse(value: &str) -> Result<Self, IntelGoalContractError> {
        match value {
            "legacy" | "legacy_six_axis_v1" => Ok(Self::Legacy),
            "observe_shadow" => Ok(Self::ObserveShadow),
            "advisory_rework" => Ok(Self::AdvisoryRework),
            "intel_goal_v1" => Ok(Self::IntelGoalV1),
            _ => Err(IntelGoalContractError::UnknownRuntimeMode),
        }
    }
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum IntelGoalCompletionAuthority {
    LegacySixAxisV1,
    IntelGoalV1,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize)]
#[serde(rename_all = "snake_case")]
pub enum StageAgentExecutionProfile {
    Worker,
    ReadOnlyReviewer,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize)]
#[serde(rename_all = "snake_case")]
pub enum StageAgentTerminalContract {
    WorkerOutputV1,
    IntelReviewV1,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct IntelGoalOperationContract {
    pub operation_id: Uuid,
    pub profile_id: String,
    pub runtime_mode: IntelGoalRuntimeMode,
    pub completion_authority: IntelGoalCompletionAuthority,
    pub goal_contract_version: String,
    pub canonical_goal_contract: Value,
    pub goal_contract_sha256: String,
    pub methodology_payload: Value,
    pub methodology_sha256: String,
    pub tool_manifest: Value,
    pub tool_manifest_sha256: String,
    pub provider_capability_manifest: Value,
    pub provider_capability_sha256: String,
    pub browser_policy: Value,
    pub budget_policy: Value,
    pub max_review_rounds: u32,
    pub reviewer_retry_fuel: u32,
}

impl IntelGoalOperationContract {
    pub fn validate(&self) -> Result<(), IntelGoalContractError> {
        if self.operation_id.is_nil()
            || self.profile_id.trim().is_empty()
            || self.goal_contract_version != "target_intel_goal.v1"
            || self.max_review_rounds == 0
        {
            return Err(IntelGoalContractError::InvalidIdentity);
        }
        for (payload, expected) in [
            (&self.canonical_goal_contract, &self.goal_contract_sha256),
            (&self.methodology_payload, &self.methodology_sha256),
            (&self.tool_manifest, &self.tool_manifest_sha256),
            (
                &self.provider_capability_manifest,
                &self.provider_capability_sha256,
            ),
        ] {
            if canonical_sha256(payload) != *expected {
                return Err(IntelGoalContractError::HashMismatch);
            }
        }
        match (self.runtime_mode, self.completion_authority) {
            (IntelGoalRuntimeMode::IntelGoalV1, IntelGoalCompletionAuthority::IntelGoalV1)
            | (
                IntelGoalRuntimeMode::Legacy
                | IntelGoalRuntimeMode::ObserveShadow
                | IntelGoalRuntimeMode::AdvisoryRework,
                IntelGoalCompletionAuthority::LegacySixAxisV1,
            ) => Ok(()),
            _ => Err(IntelGoalContractError::AuthorityModeMismatch),
        }
    }
}

#[derive(Debug, Clone, PartialEq, Eq, thiserror::Error)]
pub enum IntelGoalContractError {
    #[error("INTEL_GOAL_UNKNOWN_RUNTIME_MODE")]
    UnknownRuntimeMode,
    #[error("INTEL_GOAL_OPERATION_CONTRACT_IDENTITY_INVALID")]
    InvalidIdentity,
    #[error("INTEL_GOAL_OPERATION_CONTRACT_HASH_MISMATCH")]
    HashMismatch,
    #[error("INTEL_GOAL_OPERATION_CONTRACT_AUTHORITY_MODE_MISMATCH")]
    AuthorityModeMismatch,
}

pub fn canonical_sha256(value: &Value) -> String {
    let mut bytes = Vec::new();
    write_canonical_json(value, &mut bytes);
    format!(
        "sha256:{}",
        Sha256::digest(bytes)
            .iter()
            .map(|byte| format!("{byte:02x}"))
            .collect::<String>()
    )
}

fn write_canonical_json(value: &Value, output: &mut Vec<u8>) {
    match value {
        Value::Null | Value::Bool(_) | Value::Number(_) | Value::String(_) => {
            output.extend(serde_json::to_vec(value).unwrap_or_default());
        }
        Value::Array(values) => {
            output.push(b'[');
            for (index, value) in values.iter().enumerate() {
                if index > 0 {
                    output.push(b',');
                }
                write_canonical_json(value, output);
            }
            output.push(b']');
        }
        Value::Object(map) => {
            output.push(b'{');
            let mut keys = map.keys().collect::<Vec<_>>();
            keys.sort();
            for (index, key) in keys.into_iter().enumerate() {
                if index > 0 {
                    output.push(b',');
                }
                output.extend(serde_json::to_vec(key).unwrap_or_default());
                output.push(b':');
                write_canonical_json(&map[key], output);
            }
            output.push(b'}');
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn canonicalization_sorts_object_keys_but_preserves_array_order() {
        assert_eq!(
            canonical_sha256(&serde_json::json!({"b": 2, "a": 1})),
            canonical_sha256(&serde_json::json!({"a": 1, "b": 2}))
        );
        assert_ne!(
            canonical_sha256(&serde_json::json!({"actions": ["a", "b"]})),
            canonical_sha256(&serde_json::json!({"actions": ["b", "a"]}))
        );
    }

    #[test]
    fn missing_operation_contract_is_legacy_and_unknown_mode_fails_closed() {
        assert_eq!(
            IntelGoalRuntimeMode::for_missing_row(),
            IntelGoalRuntimeMode::Legacy
        );
        assert!(IntelGoalRuntimeMode::parse("future_mode").is_err());
    }
}
