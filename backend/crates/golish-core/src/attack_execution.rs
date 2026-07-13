//! Operation-frozen rollout contract for the Candidate verification pipeline.
//!
//! This foundation type intentionally contains no database rows, capability
//! recipes, or runtime implementation details. Deployment defaults are persisted
//! later; an existing operation must never be switched by an environment value.

use serde::{Deserialize, Serialize};
use uuid::Uuid;

/// Opaque, server-owned identity for one approved Candidate verification
/// attempt.  It intentionally carries no execution recipe, capability, action
/// constraints, budget, or scope material: every action reloads those values
/// from the database under the trusted runtime/lease fence.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct CandidateAttemptContextRef {
    pub candidate_id: Uuid,
    pub approval_id: Uuid,
    pub attempt_id: Uuid,
    pub candidate_plan_hash: String,
}

#[derive(Debug, thiserror::Error, Clone, PartialEq, Eq)]
pub enum CandidateToolBoundaryError {
    #[error("Candidate verifier tool '{0}' is not permitted")]
    ToolNotAllowed(String),
    #[error("Candidate verifier actions must execute in the foreground")]
    ForegroundRequired,
    #[error("Candidate verifier arguments may not override trusted identity '{0}'")]
    IdentityOverride(String),
    #[error("verify_execute_candidate_action accepts only one integer action_ordinal")]
    ActionOrdinalOnly,
}

impl CandidateToolBoundaryError {
    pub const fn code(&self) -> &'static str {
        match self {
            Self::ToolNotAllowed(_) => "ATTACK_VERIFIER_TOOL_NOT_ALLOWED",
            Self::ForegroundRequired => "ATTACK_VERIFIER_FOREGROUND_REQUIRED",
            Self::IdentityOverride(_) => "ATTACK_VERIFIER_IDENTITY_OVERRIDE",
            Self::ActionOrdinalOnly => "ATTACK_VERIFIER_ACTION_ORDINAL_ONLY",
        }
    }
}

/// Dependency-floor enforcement used by the sub-agent executor, which cannot
/// depend upward on golish-agent-kit. Higher layers additionally perform their
/// intent/profile checks and DB action authorization.
pub fn check_candidate_tool_boundary(
    candidate: Option<&CandidateAttemptContextRef>,
    tool_name: &str,
    args: &serde_json::Value,
) -> Result<(), CandidateToolBoundaryError> {
    if candidate.is_none() {
        return Ok(());
    }
    if !matches!(
        tool_name,
        "verify_execute_candidate_action" | "list_recent_evidence" | "submit_candidate_attempt"
    ) {
        return Err(CandidateToolBoundaryError::ToolNotAllowed(
            tool_name.to_string(),
        ));
    }
    if let Some(field) = find_forbidden_candidate_arg(args) {
        return if field == "background" {
            Err(CandidateToolBoundaryError::ForegroundRequired)
        } else {
            Err(CandidateToolBoundaryError::IdentityOverride(
                field.to_string(),
            ))
        };
    }
    if tool_name == "verify_execute_candidate_action" {
        let Some(object) = args.as_object() else {
            return Err(CandidateToolBoundaryError::ActionOrdinalOnly);
        };
        if object.len() != 1
            || object
                .get("action_ordinal")
                .and_then(serde_json::Value::as_u64)
                .is_none()
        {
            return Err(CandidateToolBoundaryError::ActionOrdinalOnly);
        }
    }
    Ok(())
}

fn find_forbidden_candidate_arg(value: &serde_json::Value) -> Option<&str> {
    match value {
        serde_json::Value::Object(object) => object.iter().find_map(|(key, nested)| {
            if matches!(
                key.as_str(),
                "candidate_id"
                    | "approval_id"
                    | "attempt_id"
                    | "candidate_plan_hash"
                    | "background"
            ) {
                Some(key.as_str())
            } else {
                find_forbidden_candidate_arg(nested)
            }
        }),
        serde_json::Value::Array(values) => values.iter().find_map(find_forbidden_candidate_arg),
        _ => None,
    }
}

#[derive(Debug, Default, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum AttackExecutionContract {
    #[default]
    Legacy,
    DualWriteReadLegacy,
    DualWriteReadV2Fallback,
    V2Only,
}

impl AttackExecutionContract {
    pub const ALL: [Self; 4] = [
        Self::Legacy,
        Self::DualWriteReadLegacy,
        Self::DualWriteReadV2Fallback,
        Self::V2Only,
    ];

    pub const fn as_str(self) -> &'static str {
        match self {
            Self::Legacy => "legacy",
            Self::DualWriteReadLegacy => "dual_write_read_legacy",
            Self::DualWriteReadV2Fallback => "dual_write_read_v2_fallback",
            Self::V2Only => "v2_only",
        }
    }

    pub const fn writes_v2(self) -> bool {
        !matches!(self, Self::Legacy)
    }

    pub const fn executes_v2_verifier(self) -> bool {
        matches!(self, Self::V2Only)
    }
}

#[cfg(test)]
mod tests {
    use super::{AttackExecutionContract, CandidateAttemptContextRef};

    #[test]
    fn core_attempt_context_contains_only_opaque_identity() {
        let candidate_id = uuid::Uuid::new_v4();
        let approval_id = uuid::Uuid::new_v4();
        let attempt_id = uuid::Uuid::new_v4();
        let ctx = CandidateAttemptContextRef {
            candidate_id,
            approval_id,
            attempt_id,
            candidate_plan_hash: "sha256:abc".into(),
        };

        assert_eq!(ctx.candidate_id, candidate_id);
        assert_eq!(ctx.approval_id, approval_id);
        assert_eq!(ctx.attempt_id, attempt_id);
        assert_eq!(ctx.candidate_plan_hash, "sha256:abc");
    }

    #[test]
    fn attack_execution_contract_has_stable_persisted_values() {
        assert_eq!(
            AttackExecutionContract::ALL.map(AttackExecutionContract::as_str),
            [
                "legacy",
                "dual_write_read_legacy",
                "dual_write_read_v2_fallback",
                "v2_only"
            ]
        );
        for contract in AttackExecutionContract::ALL {
            let encoded = serde_json::to_string(&contract).unwrap();
            assert_eq!(encoded, format!("\"{}\"", contract.as_str()));
        }
    }

    #[test]
    fn only_v2_only_executes_the_v2_verifier() {
        assert!(!AttackExecutionContract::Legacy.writes_v2());
        assert!(AttackExecutionContract::DualWriteReadLegacy.writes_v2());
        assert!(AttackExecutionContract::DualWriteReadV2Fallback.writes_v2());
        assert!(AttackExecutionContract::V2Only.writes_v2());
        assert!(AttackExecutionContract::V2Only.executes_v2_verifier());
        assert!(!AttackExecutionContract::DualWriteReadV2Fallback.executes_v2_verifier());
    }
}
