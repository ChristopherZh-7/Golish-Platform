//! Pre-action authorizer — profile authorization-ceiling check (Doc 3 §5.1).
//!
//! The per-stage tool boundary is enforced by the category whitelist
//! ([`super::stage_allows`] over `StageSpec.allowed_tool_types`); this module now
//! only enforces the orthogonal authorization ceiling: a scan's classified
//! [`IntentAxis`] must not exceed the operation profile's `max_authorization`.

use serde::{Deserialize, Serialize};
use serde_json::Value;
use thiserror::Error;

use super::profile::AuthorizationLevel;
use super::types::IntentAxis;

#[derive(Debug, Error, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub enum AuthorizationError {
    #[error("intent '{intent:?}' requires authz level above profile max_authorization ({max:?})")]
    IntentExceedsAuthorization {
        intent: IntentAxis,
        max: AuthorizationLevel,
    },
    #[error("Candidate verifier tool '{tool_name}' is not permitted")]
    CandidateToolNotAllowed { tool_name: String },
    #[error("Candidate verifier actions must execute in the foreground")]
    CandidateForegroundRequired,
    #[error("Candidate verifier arguments may not override trusted identity '{field}'")]
    CandidateIdentityOverride { field: String },
    #[error("verify_execute_candidate_action accepts only one integer action_ordinal")]
    CandidateActionOrdinalOnly,
}

impl AuthorizationError {
    pub const fn code(&self) -> &'static str {
        match self {
            Self::IntentExceedsAuthorization { .. } => "HARNESS_AUTHORIZATION_EXCEEDED",
            Self::CandidateToolNotAllowed { .. } => "ATTACK_VERIFIER_TOOL_NOT_ALLOWED",
            Self::CandidateForegroundRequired => "ATTACK_VERIFIER_FOREGROUND_REQUIRED",
            Self::CandidateIdentityOverride { .. } => "ATTACK_VERIFIER_IDENTITY_OVERRIDE",
            Self::CandidateActionOrdinalOnly => "ATTACK_VERIFIER_ACTION_ORDINAL_ONLY",
        }
    }
}

/// C3 · authorization context threaded to per-tool dispatch.
///
/// Bundles the operation's authorization ceiling (`profile.max_authorization`,
/// constant per operation) with the current subtask's classified [`IntentAxis`]
/// (per subtask). Carried in the agentic loop context so per-tool dispatch can
/// run [`PreActionAuthorizer::check_intent_ceiling`] without re-loading the
/// profile JSON on every tool call. `Copy` so it threads cheaply through the
/// bridge side-channel.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct HarnessAuthz {
    pub max_authorization: AuthorizationLevel,
    pub intent: IntentAxis,
}

pub struct PreActionAuthorizer;

impl PreActionAuthorizer {
    /// Candidate verification is a closed tool surface. The opaque context is
    /// trusted runtime state; model JSON may select only an action ordinal and
    /// can never select an identity, raw execution recipe, or background
    /// control path.
    pub fn check_candidate_tool_call(
        candidate: Option<&golish_core::CandidateAttemptContextRef>,
        tool_name: &str,
        args: &Value,
    ) -> Result<(), AuthorizationError> {
        if candidate.is_none() {
            return Ok(());
        }

        if matches!(
            tool_name,
            "wait_for_background_jobs" | "check_job" | "kill_job" | "pentest_run"
        ) {
            return Err(AuthorizationError::CandidateToolNotAllowed {
                tool_name: tool_name.to_string(),
            });
        }
        if !matches!(
            tool_name,
            "verify_execute_candidate_action" | "list_recent_evidence" | "submit_candidate_attempt"
        ) {
            return Err(AuthorizationError::CandidateToolNotAllowed {
                tool_name: tool_name.to_string(),
            });
        }

        if let Some(field) = find_forbidden_candidate_arg(args) {
            if field == "background" {
                return Err(AuthorizationError::CandidateForegroundRequired);
            }
            return Err(AuthorizationError::CandidateIdentityOverride {
                field: field.to_string(),
            });
        }

        if tool_name == "verify_execute_candidate_action" {
            let Some(object) = args.as_object() else {
                return Err(AuthorizationError::CandidateActionOrdinalOnly);
            };
            if object.len() != 1
                || object
                    .get("action_ordinal")
                    .and_then(Value::as_u64)
                    .is_none()
            {
                return Err(AuthorizationError::CandidateActionOrdinalOnly);
            }
        }

        Ok(())
    }

    /// Intent-vs-profile-ceiling check (no tool-list confinement).
    ///
    /// The per-stage tool boundary is enforced separately by the category
    /// whitelist ([`super::stage_allows`] over `allowed_tool_types`); here we only
    /// enforce the orthogonal authorization ceiling — a scan's classified
    /// [`IntentAxis`] must not exceed the profile's `max_authorization`.
    pub fn check_intent_ceiling(
        intent: IntentAxis,
        max_authorization: AuthorizationLevel,
    ) -> Result<(), AuthorizationError> {
        let required_level = match intent {
            IntentAxis::PassiveObserve => AuthorizationLevel::PassiveIntel,
            IntentAxis::ActiveProbe => AuthorizationLevel::ActiveRecon,
            IntentAxis::VulnValidation => AuthorizationLevel::VulnValidation,
            IntentAxis::ExploitValidation => AuthorizationLevel::ControlledExploit,
        };
        if required_level.rank() > max_authorization.rank() {
            return Err(AuthorizationError::IntentExceedsAuthorization {
                intent,
                max: max_authorization,
            });
        }
        Ok(())
    }
}

fn find_forbidden_candidate_arg(value: &Value) -> Option<&str> {
    match value {
        Value::Object(object) => {
            for (key, nested) in object {
                if matches!(
                    key.as_str(),
                    "candidate_id"
                        | "approval_id"
                        | "attempt_id"
                        | "candidate_plan_hash"
                        | "background"
                ) {
                    return Some(key.as_str());
                }
                if let Some(field) = find_forbidden_candidate_arg(nested) {
                    return Some(field);
                }
            }
            None
        }
        Value::Array(values) => values.iter().find_map(find_forbidden_candidate_arg),
        _ => None,
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn passive_within_active_recon_ceiling() {
        // assessment ceiling = ActiveRecon; passive observe is below it.
        assert!(PreActionAuthorizer::check_intent_ceiling(
            IntentAxis::PassiveObserve,
            AuthorizationLevel::ActiveRecon,
        )
        .is_ok());
    }

    #[test]
    fn active_probe_within_active_recon_ceiling() {
        assert!(PreActionAuthorizer::check_intent_ceiling(
            IntentAxis::ActiveProbe,
            AuthorizationLevel::ActiveRecon,
        )
        .is_ok());
    }

    #[test]
    fn exploit_exceeds_assessment_ceiling() {
        let r = PreActionAuthorizer::check_intent_ceiling(
            IntentAxis::ExploitValidation,
            AuthorizationLevel::ActiveRecon,
        );
        assert!(matches!(
            r,
            Err(AuthorizationError::IntentExceedsAuthorization { .. })
        ));
    }

    #[test]
    fn exploit_within_controlled_exploit_ceiling() {
        assert!(PreActionAuthorizer::check_intent_ceiling(
            IntentAxis::ExploitValidation,
            AuthorizationLevel::ControlledExploit,
        )
        .is_ok());
    }

    #[test]
    fn candidate_context_rejects_background_control_tools_and_identity_override() {
        let candidate = golish_core::CandidateAttemptContextRef {
            candidate_id: uuid::Uuid::new_v4(),
            approval_id: uuid::Uuid::new_v4(),
            attempt_id: uuid::Uuid::new_v4(),
            candidate_plan_hash: "sha256:plan".to_string(),
        };

        for tool_name in [
            "wait_for_background_jobs",
            "check_job",
            "kill_job",
            "pentest_run",
            "record_finding",
        ] {
            let error = PreActionAuthorizer::check_candidate_tool_call(
                Some(&candidate),
                tool_name,
                &serde_json::json!({}),
            )
            .unwrap_err();
            assert_eq!(error.code(), "ATTACK_VERIFIER_TOOL_NOT_ALLOWED");
        }

        let error = PreActionAuthorizer::check_candidate_tool_call(
            Some(&candidate),
            "list_recent_evidence",
            &serde_json::json!({"filters": {"attempt_id": candidate.attempt_id}}),
        )
        .unwrap_err();
        assert_eq!(error.code(), "ATTACK_VERIFIER_IDENTITY_OVERRIDE");
    }

    #[test]
    fn candidate_context_rejects_background_execution() {
        let candidate = golish_core::CandidateAttemptContextRef {
            candidate_id: uuid::Uuid::new_v4(),
            approval_id: uuid::Uuid::new_v4(),
            attempt_id: uuid::Uuid::new_v4(),
            candidate_plan_hash: "sha256:plan".to_string(),
        };
        let error = PreActionAuthorizer::check_candidate_tool_call(
            Some(&candidate),
            "submit_candidate_attempt",
            &serde_json::json!({"background": false}),
        )
        .unwrap_err();
        assert_eq!(error.code(), "ATTACK_VERIFIER_FOREGROUND_REQUIRED");
    }

    #[test]
    fn candidate_wrapper_accepts_only_action_ordinal() {
        let candidate = golish_core::CandidateAttemptContextRef {
            candidate_id: uuid::Uuid::new_v4(),
            approval_id: uuid::Uuid::new_v4(),
            attempt_id: uuid::Uuid::new_v4(),
            candidate_plan_hash: "sha256:plan".to_string(),
        };
        assert!(PreActionAuthorizer::check_candidate_tool_call(
            Some(&candidate),
            "verify_execute_candidate_action",
            &serde_json::json!({"action_ordinal": 0}),
        )
        .is_ok());
        for invalid in [
            serde_json::json!({}),
            serde_json::json!({"action_ordinal": -1}),
            serde_json::json!({"action_ordinal": 0, "target": "override"}),
        ] {
            assert_eq!(
                PreActionAuthorizer::check_candidate_tool_call(
                    Some(&candidate),
                    "verify_execute_candidate_action",
                    &invalid,
                )
                .unwrap_err()
                .code(),
                "ATTACK_VERIFIER_ACTION_ORDINAL_ONLY"
            );
        }
    }
}
