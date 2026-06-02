//! Pre-action authorizer — profile authorization-ceiling check (Doc 3 §5.1).
//!
//! The per-stage tool boundary is enforced by the category whitelist
//! ([`super::stage_allows`] over `StageSpec.allowed_tool_types`); this module now
//! only enforces the orthogonal authorization ceiling: a scan's classified
//! [`IntentAxis`] must not exceed the operation profile's `max_authorization`.

use serde::{Deserialize, Serialize};
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
}
