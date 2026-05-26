//! Pre-action authorizer (Doc 3 §5.1 inner loop step "pre-action authorizer").
//!
//! 每个 tool call 在 dispatch 前过一次本检查; 命中 forbidden_tools 或 越 max_authorization
//! 时 deny.

use serde::{Deserialize, Serialize};
use thiserror::Error;

use super::profile::{AuthorizationLevel, Profile};
use super::stage_spec::StageSpec;
use super::types::IntentAxis;

#[derive(Debug, Error, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub enum AuthorizationError {
    #[error("tool '{tool}' is in stage forbidden_tools list")]
    ToolForbidden { tool: String },
    #[error("tool '{tool}' is not in stage allowed_tools list")]
    ToolNotAllowed { tool: String },
    #[error(
        "intent '{intent:?}' requires authz level above profile max_authorization ({max:?})"
    )]
    IntentExceedsAuthorization {
        intent: IntentAxis,
        max: AuthorizationLevel,
    },
}

pub struct PreActionAuthorizer;

impl PreActionAuthorizer {
    /// 一条 tool call 是否被授权 · gate 前置守门.
    pub fn check(
        tool: &str,
        spec: &StageSpec,
        profile: &Profile,
        intent: IntentAxis,
    ) -> Result<(), AuthorizationError> {
        if spec.forbidden_tools.iter().any(|t| t == tool) {
            return Err(AuthorizationError::ToolForbidden {
                tool: tool.to_string(),
            });
        }
        if !spec.allowed_tools.iter().any(|t| t == tool) {
            return Err(AuthorizationError::ToolNotAllowed {
                tool: tool.to_string(),
            });
        }
        let required_level = match intent {
            IntentAxis::PassiveObserve => AuthorizationLevel::PassiveIntel,
            IntentAxis::ActiveProbe => AuthorizationLevel::ActiveRecon,
            IntentAxis::VulnValidation => AuthorizationLevel::VulnValidation,
            IntentAxis::ExploitValidation => AuthorizationLevel::ControlledExploit,
        };
        if required_level.rank() > profile.max_authorization.rank() {
            return Err(AuthorizationError::IntentExceedsAuthorization {
                intent,
                max: profile.max_authorization,
            });
        }
        Ok(())
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use super::super::stage_spec::load_stage_spec_from_json;
    use super::super::profile::load_profile_from_json;

    const ASSESSMENT_JSON: &str =
        include_str!("../../../../../resources/harness/profiles/assessment.json");
    const STAGE_JSON: &str = include_str!(
        "../../../../../resources/harness/stages/external_attack_surface.json"
    );

    fn fixtures() -> (Profile, StageSpec) {
        (
            load_profile_from_json(ASSESSMENT_JSON).expect("profile"),
            load_stage_spec_from_json(STAGE_JSON).expect("stage"),
        )
    }

    #[test]
    fn allowed_tool_passes() {
        let (p, s) = fixtures();
        let r = PreActionAuthorizer::check("dns_resolve", &s, &p, IntentAxis::ActiveProbe);
        assert!(r.is_ok());
    }

    #[test]
    fn forbidden_tool_rejected() {
        let (p, s) = fixtures();
        let r = PreActionAuthorizer::check("metasploit", &s, &p, IntentAxis::ExploitValidation);
        assert!(matches!(r, Err(AuthorizationError::ToolForbidden { .. })));
    }

    #[test]
    fn unknown_tool_rejected() {
        let (p, s) = fixtures();
        let r = PreActionAuthorizer::check("random_tool_not_listed", &s, &p, IntentAxis::PassiveObserve);
        assert!(matches!(r, Err(AuthorizationError::ToolNotAllowed { .. })));
    }

    #[test]
    fn intent_exploit_exceeds_assessment_authz() {
        let (p, s) = fixtures();
        // dns_resolve is in allowed_tools, but ExploitValidation intent exceeds L2 active_recon
        let r = PreActionAuthorizer::check("dns_resolve", &s, &p, IntentAxis::ExploitValidation);
        assert!(matches!(
            r,
            Err(AuthorizationError::IntentExceedsAuthorization { .. })
        ));
    }

    #[test]
    fn passive_observe_within_assessment_authz() {
        let (p, s) = fixtures();
        let r = PreActionAuthorizer::check("dns_resolve", &s, &p, IntentAxis::PassiveObserve);
        assert!(r.is_ok());
    }
}
