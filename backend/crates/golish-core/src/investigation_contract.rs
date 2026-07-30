//! Operation-frozen contract for Candidate analysis and Hypothesis Registry rollout.
//!
//! This module is deliberately pure. Deployment defaults and operation-frozen
//! values are persisted by higher layers; callers may not reinterpret a mode
//! through environment variables or a second policy matrix.

use serde::{Deserialize, Serialize};

#[derive(Debug, Default, Clone, Copy, PartialEq, Eq, Hash, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum InvestigationContractVersion {
    #[default]
    LegacyCandidateV1,
    HypothesisRegistryV1,
}

#[derive(Debug, Default, Clone, Copy, PartialEq, Eq, Hash, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum InvestigationRolloutMode {
    #[default]
    LegacyOnly,
    ShadowRegistry,
    DualReadCompare,
    RegistryAuthoritativeLegacyProjection,
    NewOnly,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum InvestigationAuthority {
    Legacy,
    Registry,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum ComparePolicy {
    Off,
    PromotionBlocking,
    WholeRecordExact,
    AuditOnly,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum CampaignWritePolicy {
    Off,
    ShadowAudit,
    CompareOnly,
    Canonical,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum LegacyProjectionPolicy {
    Native,
    CanonicalDerivedFailClosed,
    HistoricalReadOnly,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum InvestigationErrorCode {
    Forbidden,
    InvalidId,
    InvalidArgument,
    CursorInvalid,
    ProjectionStale,
    AuthorityCorrupt,
    Database,
    LegacyProjectionDiverged,
}

impl InvestigationErrorCode {
    pub const ALL: [Self; 8] = [
        Self::Forbidden,
        Self::InvalidId,
        Self::InvalidArgument,
        Self::CursorInvalid,
        Self::ProjectionStale,
        Self::AuthorityCorrupt,
        Self::Database,
        Self::LegacyProjectionDiverged,
    ];

    pub const fn as_str(self) -> &'static str {
        match self {
            Self::Forbidden => "INVESTIGATION_FORBIDDEN",
            Self::InvalidId => "INVESTIGATION_INVALID_ID",
            Self::InvalidArgument => "INVESTIGATION_INVALID_ARGUMENT",
            Self::CursorInvalid => "INVESTIGATION_CURSOR_INVALID",
            Self::ProjectionStale => "INVESTIGATION_PROJECTION_STALE",
            Self::AuthorityCorrupt => "INVESTIGATION_AUTHORITY_CORRUPT",
            Self::Database => "INVESTIGATION_DATABASE",
            Self::LegacyProjectionDiverged => "INVESTIGATION_LEGACY_PROJECTION_DIVERGED",
        }
    }
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct InvestigationModePolicy {
    pub canonical_writer: InvestigationAuthority,
    pub gate_authority: InvestigationAuthority,
    pub allow_legacy_mutation: bool,
    pub write_registry_shadow: bool,
    pub campaign_write_policy: CampaignWritePolicy,
    pub allow_prepared_action_jit: bool,
    pub compare_policy: ComparePolicy,
    pub legacy_projection: LegacyProjectionPolicy,
}

#[derive(Debug, thiserror::Error, Clone, PartialEq, Eq)]
pub enum InvestigationContractParseError {
    #[error("unknown investigation contract version: {0}")]
    UnknownContractVersion(String),
    #[error("unknown investigation rollout mode: {0}")]
    UnknownRolloutMode(String),
}

impl InvestigationContractVersion {
    pub const ALL: [Self; 2] = [Self::LegacyCandidateV1, Self::HypothesisRegistryV1];

    pub const fn as_str(self) -> &'static str {
        match self {
            Self::LegacyCandidateV1 => "legacy_candidate_v1",
            Self::HypothesisRegistryV1 => "hypothesis_registry_v1",
        }
    }

    pub const fn allows(self, mode: InvestigationRolloutMode) -> bool {
        matches!(
            (self, mode),
            (
                Self::LegacyCandidateV1,
                InvestigationRolloutMode::LegacyOnly
            ) | (
                Self::HypothesisRegistryV1,
                InvestigationRolloutMode::ShadowRegistry
            ) | (
                Self::HypothesisRegistryV1,
                InvestigationRolloutMode::DualReadCompare
            ) | (
                Self::HypothesisRegistryV1,
                InvestigationRolloutMode::RegistryAuthoritativeLegacyProjection
            ) | (
                Self::HypothesisRegistryV1,
                InvestigationRolloutMode::NewOnly
            )
        )
    }
}

impl TryFrom<&str> for InvestigationContractVersion {
    type Error = InvestigationContractParseError;

    fn try_from(value: &str) -> Result<Self, Self::Error> {
        match value {
            "legacy_candidate_v1" => Ok(Self::LegacyCandidateV1),
            "hypothesis_registry_v1" => Ok(Self::HypothesisRegistryV1),
            value => Err(InvestigationContractParseError::UnknownContractVersion(
                value.to_owned(),
            )),
        }
    }
}

impl InvestigationRolloutMode {
    pub const ALL: [Self; 5] = [
        Self::LegacyOnly,
        Self::ShadowRegistry,
        Self::DualReadCompare,
        Self::RegistryAuthoritativeLegacyProjection,
        Self::NewOnly,
    ];

    pub const fn as_str(self) -> &'static str {
        match self {
            Self::LegacyOnly => "legacy_only",
            Self::ShadowRegistry => "shadow_registry",
            Self::DualReadCompare => "dual_read_compare",
            Self::RegistryAuthoritativeLegacyProjection => {
                "registry_authoritative_legacy_projection"
            }
            Self::NewOnly => "new_only",
        }
    }

    pub const fn mode_rank(self) -> i16 {
        match self {
            Self::LegacyOnly => 0,
            Self::ShadowRegistry => 1,
            Self::DualReadCompare => 2,
            Self::RegistryAuthoritativeLegacyProjection => 3,
            Self::NewOnly => 4,
        }
    }

    pub const fn policy(self) -> InvestigationModePolicy {
        use CampaignWritePolicy::{Canonical, CompareOnly, Off as CampaignOff, ShadowAudit};
        use ComparePolicy::{AuditOnly, Off, PromotionBlocking, WholeRecordExact};
        use InvestigationAuthority::{Legacy, Registry};
        use LegacyProjectionPolicy::{CanonicalDerivedFailClosed, HistoricalReadOnly, Native};

        match self {
            Self::LegacyOnly => InvestigationModePolicy {
                canonical_writer: Legacy,
                gate_authority: Legacy,
                allow_legacy_mutation: true,
                write_registry_shadow: false,
                campaign_write_policy: CampaignOff,
                allow_prepared_action_jit: false,
                compare_policy: Off,
                legacy_projection: Native,
            },
            Self::ShadowRegistry => InvestigationModePolicy {
                canonical_writer: Legacy,
                gate_authority: Legacy,
                allow_legacy_mutation: true,
                write_registry_shadow: true,
                campaign_write_policy: ShadowAudit,
                allow_prepared_action_jit: false,
                compare_policy: PromotionBlocking,
                legacy_projection: Native,
            },
            Self::DualReadCompare => InvestigationModePolicy {
                canonical_writer: Legacy,
                gate_authority: Legacy,
                allow_legacy_mutation: true,
                write_registry_shadow: true,
                campaign_write_policy: CompareOnly,
                allow_prepared_action_jit: false,
                compare_policy: WholeRecordExact,
                legacy_projection: Native,
            },
            Self::RegistryAuthoritativeLegacyProjection => InvestigationModePolicy {
                canonical_writer: Registry,
                gate_authority: Registry,
                allow_legacy_mutation: false,
                write_registry_shadow: false,
                campaign_write_policy: Canonical,
                allow_prepared_action_jit: true,
                compare_policy: AuditOnly,
                legacy_projection: CanonicalDerivedFailClosed,
            },
            Self::NewOnly => InvestigationModePolicy {
                canonical_writer: Registry,
                gate_authority: Registry,
                allow_legacy_mutation: false,
                write_registry_shadow: false,
                campaign_write_policy: Canonical,
                allow_prepared_action_jit: true,
                compare_policy: Off,
                legacy_projection: HistoricalReadOnly,
            },
        }
    }
}

impl TryFrom<&str> for InvestigationRolloutMode {
    type Error = InvestigationContractParseError;

    fn try_from(value: &str) -> Result<Self, Self::Error> {
        match value {
            "legacy_only" => Ok(Self::LegacyOnly),
            "shadow_registry" => Ok(Self::ShadowRegistry),
            "dual_read_compare" => Ok(Self::DualReadCompare),
            "registry_authoritative_legacy_projection" => {
                Ok(Self::RegistryAuthoritativeLegacyProjection)
            }
            "new_only" => Ok(Self::NewOnly),
            value => Err(InvestigationContractParseError::UnknownRolloutMode(
                value.to_owned(),
            )),
        }
    }
}

#[cfg(test)]
mod tests {
    use super::{
        CampaignWritePolicy, ComparePolicy, InvestigationAuthority, InvestigationContractVersion,
        InvestigationErrorCode, InvestigationRolloutMode, LegacyProjectionPolicy,
    };

    #[test]
    fn investigation_rollout_matrix_is_the_single_final_policy() {
        use CampaignWritePolicy::{Canonical, CompareOnly, Off, ShadowAudit};
        use ComparePolicy::{AuditOnly, Off as CompareOff, PromotionBlocking, WholeRecordExact};
        use InvestigationAuthority::{Legacy, Registry};
        use LegacyProjectionPolicy::{CanonicalDerivedFailClosed, HistoricalReadOnly, Native};
        let expected = [
            (
                InvestigationRolloutMode::LegacyOnly,
                Legacy,
                true,
                false,
                Off,
                false,
                CompareOff,
                Native,
            ),
            (
                InvestigationRolloutMode::ShadowRegistry,
                Legacy,
                true,
                true,
                ShadowAudit,
                false,
                PromotionBlocking,
                Native,
            ),
            (
                InvestigationRolloutMode::DualReadCompare,
                Legacy,
                true,
                true,
                CompareOnly,
                false,
                WholeRecordExact,
                Native,
            ),
            (
                InvestigationRolloutMode::RegistryAuthoritativeLegacyProjection,
                Registry,
                false,
                false,
                Canonical,
                true,
                AuditOnly,
                CanonicalDerivedFailClosed,
            ),
            (
                InvestigationRolloutMode::NewOnly,
                Registry,
                false,
                false,
                Canonical,
                true,
                CompareOff,
                HistoricalReadOnly,
            ),
        ];
        for (mode, authority, legacy_mutation, shadow, campaign, jit, compare, projection) in
            expected
        {
            let policy = mode.policy();
            assert_eq!(policy.canonical_writer, authority);
            assert_eq!(policy.gate_authority, authority);
            assert_eq!(policy.allow_legacy_mutation, legacy_mutation);
            assert_eq!(policy.write_registry_shadow, shadow);
            assert_eq!(policy.campaign_write_policy, campaign);
            assert_eq!(policy.allow_prepared_action_jit, jit);
            assert_eq!(policy.compare_policy, compare);
            assert_eq!(policy.legacy_projection, projection);
        }
    }

    #[test]
    fn legal_contract_mode_pairs_are_closed() {
        assert!(InvestigationContractVersion::LegacyCandidateV1
            .allows(InvestigationRolloutMode::LegacyOnly));
        for mode in [
            InvestigationRolloutMode::ShadowRegistry,
            InvestigationRolloutMode::DualReadCompare,
            InvestigationRolloutMode::RegistryAuthoritativeLegacyProjection,
            InvestigationRolloutMode::NewOnly,
        ] {
            assert!(InvestigationContractVersion::HypothesisRegistryV1.allows(mode));
            assert!(!InvestigationContractVersion::LegacyCandidateV1.allows(mode));
        }
        assert!(!InvestigationContractVersion::HypothesisRegistryV1
            .allows(InvestigationRolloutMode::LegacyOnly));
    }

    #[test]
    fn investigation_error_codes_are_stable_and_closed() {
        assert_eq!(
            InvestigationErrorCode::ALL.map(InvestigationErrorCode::as_str),
            [
                "INVESTIGATION_FORBIDDEN",
                "INVESTIGATION_INVALID_ID",
                "INVESTIGATION_INVALID_ARGUMENT",
                "INVESTIGATION_CURSOR_INVALID",
                "INVESTIGATION_PROJECTION_STALE",
                "INVESTIGATION_AUTHORITY_CORRUPT",
                "INVESTIGATION_DATABASE",
                "INVESTIGATION_LEGACY_PROJECTION_DIVERGED",
            ],
        );
    }

    #[test]
    fn investigation_unknown_contract_and_mode_values_fail_closed() {
        assert!(InvestigationContractVersion::try_from("future_contract").is_err());
        assert!(InvestigationRolloutMode::try_from("future_mode").is_err());
    }
}
