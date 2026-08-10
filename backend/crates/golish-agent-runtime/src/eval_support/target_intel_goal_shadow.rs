//! Explicit fixture/dev-only Target Intel Goal shadow authority.
//!
//! Production profile names, environment variables and historical operation
//! state are intentionally not selectors. The only enabling input is the
//! strongly typed fixture object held by [`EvalConfig`](super::EvalConfig).

use std::collections::BTreeMap;

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum GoalShadowMode {
    ObserveOnly,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum FixtureCapabilityStatus {
    EnabledStrictPassive,
    DisabledNotInPlanA,
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct CapabilityManifest {
    pub capabilities: BTreeMap<&'static str, FixtureCapabilityStatus>,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum FakeTransport {
    InMemoryFixture,
}

impl FakeTransport {
    pub const fn is_fake(self) -> bool {
        true
    }
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct TargetIntelGoalShadowFixture {
    pub mode: GoalShadowMode,
    pub contract_version: &'static str,
    pub review_schema: &'static str,
    pub browser_mode: &'static str,
    pub advisory_rework_enabled: bool,
    pub fixture_dev_only: bool,
    pub shadow_observe_only: bool,
    pub capability_manifest: CapabilityManifest,
    pub external_transport: FakeTransport,
}

impl TargetIntelGoalShadowFixture {
    pub fn strict_passive() -> Self {
        Self {
            mode: GoalShadowMode::ObserveOnly,
            contract_version: "target_intel_goal.fixture.v1",
            review_schema: "intel_review.v1",
            browser_mode: "strict_passive",
            advisory_rework_enabled: false,
            fixture_dev_only: true,
            shadow_observe_only: true,
            capability_manifest: CapabilityManifest {
                capabilities: [
                    (
                        "strict_passive",
                        FixtureCapabilityStatus::EnabledStrictPassive,
                    ),
                    (
                        "public_web_readonly",
                        FixtureCapabilityStatus::DisabledNotInPlanA,
                    ),
                    (
                        "server_side_web_search",
                        FixtureCapabilityStatus::DisabledNotInPlanA,
                    ),
                ]
                .into_iter()
                .collect(),
            },
            external_transport: FakeTransport::InMemoryFixture,
        }
    }

    pub const fn allow_provider_server_web_search(&self) -> bool {
        false
    }

    pub fn strict_passive_public_tools_enabled(&self) -> bool {
        self.fixture_dev_only
            && self.shadow_observe_only
            && self.external_transport.is_fake()
            && self.capability_manifest.capabilities.get("strict_passive")
                == Some(&FixtureCapabilityStatus::EnabledStrictPassive)
    }
}

pub const fn production_profile_enables_goal_shadow(_profile_id: &str) -> bool {
    false
}

pub fn select_goal_shadow(
    explicit_fixture: Option<&TargetIntelGoalShadowFixture>,
    existing_operation: bool,
) -> Option<&TargetIntelGoalShadowFixture> {
    if existing_operation {
        return None;
    }
    explicit_fixture.filter(|fixture| {
        fixture.fixture_dev_only
            && fixture.shadow_observe_only
            && fixture.external_transport.is_fake()
            && fixture.mode == GoalShadowMode::ObserveOnly
            && !fixture.advisory_rework_enabled
    })
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn production_profiles_do_not_enable_target_intel_goal_shadow() {
        assert!(!production_profile_enables_goal_shadow("red_team"));
        assert!(!production_profile_enables_goal_shadow("pentest"));
    }

    #[test]
    fn fixture_context_is_the_only_shadow_selector() {
        let fixture = TargetIntelGoalShadowFixture::strict_passive();
        assert_eq!(fixture.mode, GoalShadowMode::ObserveOnly);
        assert!(!fixture.advisory_rework_enabled);
        assert!(fixture.external_transport.is_fake());
        assert!(select_goal_shadow(Some(&fixture), false).is_some());
        assert!(select_goal_shadow(None, false).is_none());
    }

    #[test]
    fn existing_operation_context_cannot_be_reinterpreted() {
        let fixture = TargetIntelGoalShadowFixture::strict_passive();
        assert!(select_goal_shadow(Some(&fixture), true).is_none());
    }
}
