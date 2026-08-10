//! Pure review bundle, cursor and verdict validation for Target Intel.

use std::collections::BTreeSet;

use serde::{Deserialize, Serialize};
use serde_json::Value;
use uuid::Uuid;

use super::intel_goal_contract::canonical_sha256;

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum IntelReviewSectionKind {
    DurableState,
    ObservableActions,
    FrozenContract,
    CompletionClaim,
}

impl IntelReviewSectionKind {
    pub const ORDER: [Self; 4] = [
        Self::DurableState,
        Self::ObservableActions,
        Self::FrozenContract,
        Self::CompletionClaim,
    ];

    pub const fn ordinal(self) -> u8 {
        match self {
            Self::DurableState => 1,
            Self::ObservableActions => 2,
            Self::FrozenContract => 3,
            Self::CompletionClaim => 4,
        }
    }

    pub const fn as_str(self) -> &'static str {
        match self {
            Self::DurableState => "durable_state",
            Self::ObservableActions => "observable_actions",
            Self::FrozenContract => "frozen_contract",
            Self::CompletionClaim => "completion_claim",
        }
    }

    pub const fn next(self) -> Option<Self> {
        match self {
            Self::DurableState => Some(Self::ObservableActions),
            Self::ObservableActions => Some(Self::FrozenContract),
            Self::FrozenContract => Some(Self::CompletionClaim),
            Self::CompletionClaim => None,
        }
    }
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct IntelReviewSection {
    pub kind: IntelReviewSectionKind,
    pub payload: Value,
    pub sha256: String,
}

impl IntelReviewSection {
    pub fn new(kind: IntelReviewSectionKind, payload: Value) -> Self {
        let sha256 = canonical_sha256(&payload);
        Self {
            kind,
            payload,
            sha256,
        }
    }
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct IntelReviewBundleIdentity {
    pub review_id: Uuid,
    pub operation_id: Uuid,
    pub stage_execution_id: Uuid,
    pub stage_run_unit_id: Uuid,
    pub organization_id: Uuid,
    pub team_plan_id: Uuid,
    pub controller_work_item_id: Uuid,
    pub controller_worker_run_id: Uuid,
    pub controller_message_chain_id: Uuid,
    pub goal_epoch: i64,
    pub review_generation: i64,
    pub round: u32,
    pub state_revision: i64,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct IntelReviewBundle {
    pub identity: IntelReviewBundleIdentity,
    pub sections: Vec<IntelReviewSection>,
    pub bundle_sha256: String,
}

impl IntelReviewBundle {
    pub fn freeze(
        identity: IntelReviewBundleIdentity,
        payloads: [Value; 4],
    ) -> Result<Self, IntelReviewError> {
        if identity.review_id.is_nil()
            || identity.operation_id.is_nil()
            || identity.organization_id.is_nil()
            || identity.goal_epoch < 0
            || identity.review_generation < 0
            || identity.round == 0
        {
            return Err(IntelReviewError::IdentityInvalid);
        }
        let sections = IntelReviewSectionKind::ORDER
            .into_iter()
            .zip(payloads)
            .map(|(kind, payload)| IntelReviewSection::new(kind, payload))
            .collect::<Vec<_>>();
        let bundle_sha256 = canonical_sha256(
            &serde_json::to_value((&identity, &sections))
                .map_err(|_| IntelReviewError::CanonicalizationFailed)?,
        );
        Ok(Self {
            identity,
            sections,
            bundle_sha256,
        })
    }

    pub fn validate(&self) -> Result<(), IntelReviewError> {
        if self.sections.len() != 4
            || !self.sections.iter().zip(IntelReviewSectionKind::ORDER).all(
                |(section, expected)| {
                    section.kind == expected && section.sha256 == canonical_sha256(&section.payload)
                },
            )
        {
            return Err(IntelReviewError::SectionOrderOrHashInvalid);
        }
        let expected = canonical_sha256(
            &serde_json::to_value((&self.identity, &self.sections))
                .map_err(|_| IntelReviewError::CanonicalizationFailed)?,
        );
        (expected == self.bundle_sha256)
            .then_some(())
            .ok_or(IntelReviewError::BundleHashMismatch)
    }
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct IntelReviewReadCursor {
    pub review_id: Uuid,
    pub reviewer_worker_run_id: Uuid,
    pub bundle_sha256: String,
    pub next_ordinal: u8,
}

impl IntelReviewReadCursor {
    pub fn read(
        &mut self,
        bundle: &IntelReviewBundle,
        reviewer_worker_run_id: Uuid,
        requested: IntelReviewSectionKind,
    ) -> Result<IntelReviewSection, IntelReviewError> {
        if self.review_id != bundle.identity.review_id
            || self.reviewer_worker_run_id != reviewer_worker_run_id
            || self.bundle_sha256 != bundle.bundle_sha256
        {
            return Err(IntelReviewError::ForeignOrStaleReader);
        }
        if requested.ordinal() != self.next_ordinal {
            return Err(IntelReviewError::SectionOutOfOrder);
        }
        let section = bundle.sections[(self.next_ordinal - 1) as usize].clone();
        self.next_ordinal += 1;
        Ok(section)
    }

    pub const fn completion_claim_read(&self) -> bool {
        self.next_ordinal == 5
    }
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "SCREAMING_SNAKE_CASE")]
pub enum IntelReviewDecision {
    Pass,
    Rework,
    NeedsHuman,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum IntelReviewFindingMateriality {
    Critical,
    Major,
    Minor,
    Advisory,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct IntelReviewFinding {
    pub finding_id: Uuid,
    pub fingerprint: String,
    pub materiality: IntelReviewFindingMateriality,
    pub subject_refs: Vec<String>,
    pub reason: String,
    pub evidence_refs: Vec<String>,
    pub action_kind: Option<String>,
    pub capability_ref: Option<String>,
    pub close_condition: Option<String>,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum IntelInheritedFindingDispositionKind {
    Resolved,
    StillOpen,
    NeedsHuman,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct IntelInheritedFindingDisposition {
    pub finding_id: Uuid,
    pub disposition: IntelInheritedFindingDispositionKind,
    pub resolution_refs: Vec<String>,
    pub reason: String,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct IntelReviewVerdict {
    pub schema: String,
    pub decision: IntelReviewDecision,
    pub findings: Vec<IntelReviewFinding>,
    pub inherited_dispositions: Vec<IntelInheritedFindingDisposition>,
    pub residuals: Vec<String>,
    pub human_requirement: Option<String>,
}

impl IntelReviewVerdict {
    /// Replace reviewer-provided finding fingerprints with the canonical
    /// semantic identity used by the durable repository. Invalid shapes still
    /// fail [`Self::validate`]; this method never turns prose into authority.
    pub fn stamp_finding_fingerprints(&mut self) {
        for finding in &mut self.findings {
            let mut subject_refs = finding
                .subject_refs
                .iter()
                .map(|value| value.trim().to_string())
                .collect::<Vec<_>>();
            subject_refs.sort();
            subject_refs.dedup();
            let optional = |value: &Option<String>| {
                value
                    .as_deref()
                    .map(str::trim)
                    .filter(|value| !value.is_empty())
                    .map(str::to_string)
            };
            finding.fingerprint = canonical_sha256(&serde_json::json!({
                "materiality": finding.materiality,
                "subject_refs": subject_refs,
                "reason": finding.reason.trim(),
                "action_kind": optional(&finding.action_kind),
                "capability_ref": optional(&finding.capability_ref),
                "close_condition": optional(&finding.close_condition),
            }));
        }
    }

    pub fn validate(&self, inherited_material_findings: &[Uuid]) -> Result<(), IntelReviewError> {
        if self.schema != "intel_review.v1" {
            return Err(IntelReviewError::VerdictSchemaInvalid);
        }
        if self.findings.len() > 128
            || self.inherited_dispositions.len() > 128
            || self.residuals.len() > 128
            || self
                .residuals
                .iter()
                .any(|value| !bounded_nonempty(value, 4_000))
        {
            return Err(IntelReviewError::VerdictShapeInvalid);
        }
        let expected_inherited = inherited_material_findings
            .iter()
            .copied()
            .collect::<BTreeSet<_>>();
        let actual_inherited = self
            .inherited_dispositions
            .iter()
            .map(|item| item.finding_id)
            .collect::<BTreeSet<_>>();
        if expected_inherited.len() != inherited_material_findings.len()
            || actual_inherited.len() != self.inherited_dispositions.len()
            || actual_inherited != expected_inherited
            || self.inherited_dispositions.iter().any(|item| {
                item.finding_id.is_nil()
                    || !bounded_nonempty(&item.reason, 4_000)
                    || item.resolution_refs.len() > 128
                    || item
                        .resolution_refs
                        .iter()
                        .any(|reference| !bounded_nonempty(reference, 512))
                    || (item.disposition == IntelInheritedFindingDispositionKind::Resolved
                        && item.resolution_refs.is_empty())
            })
        {
            return Err(IntelReviewError::InheritedFindingNotDisposed);
        }
        let finding_ids = self
            .findings
            .iter()
            .map(|finding| finding.finding_id)
            .collect::<BTreeSet<_>>();
        let fingerprints = self
            .findings
            .iter()
            .map(|finding| finding.fingerprint.as_str())
            .collect::<BTreeSet<_>>();
        if finding_ids.len() != self.findings.len()
            || fingerprints.len() != self.findings.len()
            || self.findings.iter().any(|finding| {
                finding.finding_id.is_nil()
                    || !is_sha256(&finding.fingerprint)
                    || !bounded_nonempty(&finding.reason, 4_000)
                    || finding.subject_refs.len() > 128
                    || finding.evidence_refs.len() > 128
                    || finding
                        .subject_refs
                        .iter()
                        .chain(&finding.evidence_refs)
                        .any(|reference| !bounded_nonempty(reference, 512))
                    || finding
                        .action_kind
                        .as_deref()
                        .is_some_and(|value| !bounded_nonempty(value, 128))
                    || finding
                        .capability_ref
                        .as_deref()
                        .is_some_and(|value| !bounded_nonempty(value, 512))
                    || finding
                        .close_condition
                        .as_deref()
                        .is_some_and(|value| !bounded_nonempty(value, 4_000))
            })
        {
            return Err(IntelReviewError::VerdictShapeInvalid);
        }
        let material = self.findings.iter().filter(|finding| {
            matches!(
                finding.materiality,
                IntelReviewFindingMateriality::Critical | IntelReviewFindingMateriality::Major
            )
        });
        match self.decision {
            IntelReviewDecision::Pass => {
                if material.count() > 0
                    || self.human_requirement.is_some()
                    || self.inherited_dispositions.iter().any(|item| {
                        item.disposition != IntelInheritedFindingDispositionKind::Resolved
                    })
                {
                    return Err(IntelReviewError::PassHasOpenMaterialFinding);
                }
            }
            IntelReviewDecision::Rework => {
                if self.human_requirement.is_some()
                    || !material.into_iter().any(|finding| {
                        !finding.evidence_refs.is_empty()
                            && finding
                                .action_kind
                                .as_deref()
                                .is_some_and(|value| !value.trim().is_empty())
                            && finding
                                .close_condition
                                .as_deref()
                                .is_some_and(|value| !value.trim().is_empty())
                    })
                {
                    return Err(IntelReviewError::ReworkNotActionable);
                }
            }
            IntelReviewDecision::NeedsHuman => {
                if !matches!(
                    self.human_requirement.as_deref(),
                    Some(
                        "credential"
                            | "scope_confirmation"
                            | "subject_confirmation"
                            | "provider_recovery"
                            | "review_fixed_point"
                    )
                ) {
                    return Err(IntelReviewError::HumanRequirementInvalid);
                }
            }
        }
        Ok(())
    }
}

fn bounded_nonempty(value: &str, max_chars: usize) -> bool {
    !value.trim().is_empty()
        && value.chars().count() <= max_chars
        && !value.chars().any(char::is_control)
}

fn is_sha256(value: &str) -> bool {
    value.strip_prefix("sha256:").is_some_and(|hex| {
        hex.len() == 64
            && hex
                .bytes()
                .all(|byte| byte.is_ascii_hexdigit() && !byte.is_ascii_uppercase())
    })
}

#[derive(Debug, Clone, PartialEq, Eq, thiserror::Error)]
pub enum IntelReviewError {
    #[error("INTEL_REVIEW_IDENTITY_INVALID")]
    IdentityInvalid,
    #[error("INTEL_REVIEW_CANONICALIZATION_FAILED")]
    CanonicalizationFailed,
    #[error("INTEL_REVIEW_SECTION_ORDER_OR_HASH_INVALID")]
    SectionOrderOrHashInvalid,
    #[error("INTEL_REVIEW_BUNDLE_HASH_MISMATCH")]
    BundleHashMismatch,
    #[error("INTEL_REVIEW_FOREIGN_OR_STALE_READER")]
    ForeignOrStaleReader,
    #[error("INTEL_REVIEW_SECTION_OUT_OF_ORDER")]
    SectionOutOfOrder,
    #[error("INTEL_REVIEW_VERDICT_SCHEMA_INVALID")]
    VerdictSchemaInvalid,
    #[error("INTEL_REVIEW_VERDICT_SHAPE_INVALID")]
    VerdictShapeInvalid,
    #[error("INTEL_REVIEW_INHERITED_FINDING_NOT_DISPOSED")]
    InheritedFindingNotDisposed,
    #[error("INTEL_REVIEW_PASS_HAS_OPEN_MATERIAL_FINDING")]
    PassHasOpenMaterialFinding,
    #[error("INTEL_REVIEW_REWORK_NOT_ACTIONABLE")]
    ReworkNotActionable,
    #[error("INTEL_REVIEW_HUMAN_REQUIREMENT_INVALID")]
    HumanRequirementInvalid,
}

#[cfg(test)]
mod tests {
    use super::*;

    fn identity() -> IntelReviewBundleIdentity {
        IntelReviewBundleIdentity {
            review_id: Uuid::from_u128(1),
            operation_id: Uuid::from_u128(2),
            stage_execution_id: Uuid::from_u128(3),
            stage_run_unit_id: Uuid::from_u128(4),
            organization_id: Uuid::from_u128(5),
            team_plan_id: Uuid::from_u128(6),
            controller_work_item_id: Uuid::from_u128(7),
            controller_worker_run_id: Uuid::from_u128(8),
            controller_message_chain_id: Uuid::from_u128(9),
            goal_epoch: 1,
            review_generation: 1,
            round: 1,
            state_revision: 1,
        }
    }

    #[test]
    fn review_bundle_hash_is_stable_and_section_order_is_fixed() {
        let payloads = [
            serde_json::json!({"facts": []}),
            serde_json::json!({"actions": ["a", "b"]}),
            serde_json::json!({"goal": "v1"}),
            serde_json::json!({"claim": "done"}),
        ];
        let first = IntelReviewBundle::freeze(identity(), payloads.clone()).unwrap();
        let second = IntelReviewBundle::freeze(identity(), payloads).unwrap();
        assert_eq!(first.bundle_sha256, second.bundle_sha256);
        assert_eq!(
            first
                .sections
                .iter()
                .map(|item| item.kind)
                .collect::<Vec<_>>(),
            IntelReviewSectionKind::ORDER
        );
    }

    #[test]
    fn section_cursor_rejects_completion_claim_before_prior_sections() {
        let bundle = IntelReviewBundle::freeze(
            identity(),
            [Value::Null, Value::Null, Value::Null, Value::Null],
        )
        .unwrap();
        let reviewer = Uuid::from_u128(10);
        let mut cursor = IntelReviewReadCursor {
            review_id: bundle.identity.review_id,
            reviewer_worker_run_id: reviewer,
            bundle_sha256: bundle.bundle_sha256.clone(),
            next_ordinal: 1,
        };
        assert!(cursor
            .read(&bundle, reviewer, IntelReviewSectionKind::CompletionClaim)
            .is_err());
    }

    #[test]
    fn trusted_host_stamps_semantic_finding_fingerprint_before_validation() {
        let mut verdict = IntelReviewVerdict {
            schema: "intel_review.v1".to_string(),
            decision: IntelReviewDecision::Rework,
            findings: vec![IntelReviewFinding {
                finding_id: Uuid::from_u128(11),
                fingerprint: format!("sha256:{}", "0".repeat(64)),
                materiality: IntelReviewFindingMateriality::Major,
                subject_refs: vec!["subject:b".to_string(), "subject:a".to_string()],
                reason: " current query receipts do not close the plan ".to_string(),
                evidence_refs: vec!["audit:7".to_string()],
                action_kind: Some("continue_semantic_search".to_string()),
                capability_ref: None,
                close_condition: Some("land a terminal query receipt".to_string()),
            }],
            inherited_dispositions: Vec::new(),
            residuals: Vec::new(),
            human_requirement: None,
        };

        verdict.stamp_finding_fingerprints();
        let first = verdict.findings[0].fingerprint.clone();
        assert!(is_sha256(&first));
        assert_ne!(first, format!("sha256:{}", "0".repeat(64)));
        verdict.stamp_finding_fingerprints();
        assert_eq!(verdict.findings[0].fingerprint, first);
        verdict
            .validate(&[])
            .expect("host-stamped verdict validates");
    }
}
