//! Pure Tool Truth coverage compiler and reducers.
//!
//! The database bridge owns source locking and persistence. This module owns
//! the deterministic interpretation shared by the sealer and shadow Gate, so
//! neither side can invent a smaller technique/applicability matrix.

use std::collections::BTreeSet;
use std::fmt::Write;

use sha2::{Digest, Sha256};
use thiserror::Error;
use uuid::Uuid;

use super::stage_capability::{capabilities_for_stage, capabilities_for_technique};
use super::stage_spec::StageSpec;
use super::technique_resolver::{
    classify_stage_asset, resolve_expected_techniques, technique_applies_web_aware,
};
use super::{load_embedded_stage_spec, StageKind};

const RECEIPT_STAGES: [StageKind; 4] = [
    StageKind::TargetIntel,
    StageKind::ExternalAttackSurface,
    StageKind::Enumeration,
    StageKind::VulnTriage,
];

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct DenominatorAsset {
    pub target_id: Uuid,
    pub exact_asset: String,
    pub asset_type: String,
    pub web_capable: bool,
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct ServerDerivedDenominatorItem {
    pub input_key: String,
    pub target_id: Uuid,
    pub exact_asset: String,
    pub technique: String,
    pub expected_capability: String,
    pub item_hash: String,
}

#[derive(Debug, Error, Clone, PartialEq, Eq)]
pub enum ToolTruthDenominatorError {
    #[error("stage spec could not be loaded: {0}")]
    StageSpec(String),
    #[error("stage spec kind does not match the requested stage")]
    StageSpecKindDrift,
    #[error("resolver returned technique outside the embedded stage spec: {0}")]
    ResolverOutsideStageSpec(String),
    #[error("authoritative denominator input is empty")]
    EmptyAuthoritativeInput,
    #[error("no registered capability covers technique {0}")]
    CapabilityMappingMissing(String),
    #[error("capability registry contains technique outside the embedded stage spec: {0}")]
    CapabilityOutsideStageSpec(String),
}

pub fn build_denominator_items(
    stage: StageKind,
    assets: &[DenominatorAsset],
) -> Result<Vec<ServerDerivedDenominatorItem>, ToolTruthDenominatorError> {
    let spec = load_embedded_stage_spec(stage)
        .map_err(|error| ToolTruthDenominatorError::StageSpec(error.to_string()))?;
    build_denominator_items_from_spec(stage, &spec, assets)
}

pub fn build_denominator_items_from_spec(
    stage: StageKind,
    spec: &StageSpec,
    assets: &[DenominatorAsset],
) -> Result<Vec<ServerDerivedDenominatorItem>, ToolTruthDenominatorError> {
    if spec.kind != stage {
        return Err(ToolTruthDenominatorError::StageSpecKindDrift);
    }
    if assets.is_empty() {
        return Err(ToolTruthDenominatorError::EmptyAuthoritativeInput);
    }

    let classes = assets
        .iter()
        .map(|asset| classify_stage_asset(stage, Some(&asset.asset_type), &asset.exact_asset))
        .collect::<Vec<_>>();
    let resolved = resolve_expected_techniques(stage, &classes);
    let techniques = if resolved.is_empty() {
        spec.expected_techniques.clone()
    } else {
        for technique in &resolved {
            if !spec.expected_techniques.contains(technique) {
                return Err(ToolTruthDenominatorError::ResolverOutsideStageSpec(
                    technique.clone(),
                ));
            }
        }
        resolved
    };
    if techniques.is_empty() {
        return Err(ToolTruthDenominatorError::EmptyAuthoritativeInput);
    }

    // Validate the full declared catalog before applying per-asset pruning. A
    // newly declared but currently inapplicable technique must not disappear
    // silently just because today's fixture happens not to exercise it.
    for technique in &spec.expected_techniques {
        registered_capability_for(stage, technique)?;
    }

    let mut items = Vec::new();
    for asset in assets {
        let class = classify_stage_asset(stage, Some(&asset.asset_type), &asset.exact_asset);
        for technique in &techniques {
            if !technique_applies_web_aware(
                stage,
                class,
                &asset.exact_asset,
                technique,
                asset.web_capable,
            ) {
                continue;
            }
            let capability = registered_capability_for(stage, technique)?;
            let input_key = format!(
                "{}\u{1f}{}\u{1f}{}",
                asset.target_id, asset.exact_asset, technique
            );
            let item_hash = sha256_prefixed(
                format!(
                    "{}\u{1f}{}\u{1f}{}\u{1f}{}\u{1f}{}",
                    asset.target_id, asset.exact_asset, asset.asset_type, technique, capability
                )
                .as_bytes(),
            );
            items.push(ServerDerivedDenominatorItem {
                input_key,
                target_id: asset.target_id,
                exact_asset: asset.exact_asset.clone(),
                technique: technique.clone(),
                expected_capability: capability,
                item_hash,
            });
        }
    }
    if items.is_empty() {
        return Err(ToolTruthDenominatorError::EmptyAuthoritativeInput);
    }
    items.sort_by(|left, right| left.input_key.cmp(&right.input_key));
    Ok(items)
}

fn registered_capability_for(
    stage: StageKind,
    technique: &str,
) -> Result<String, ToolTruthDenominatorError> {
    let mut capabilities = capabilities_for_technique(stage, technique);
    capabilities.sort_by_key(|capability| capability.id);
    capabilities
        .first()
        .map(|capability| capability.id.to_string())
        .ok_or_else(|| ToolTruthDenominatorError::CapabilityMappingMissing(technique.to_string()))
}

pub fn validate_denominator_catalog() -> Result<(), ToolTruthDenominatorError> {
    for stage in RECEIPT_STAGES {
        let spec = load_embedded_stage_spec(stage)
            .map_err(|error| ToolTruthDenominatorError::StageSpec(error.to_string()))?;
        let declared = spec
            .expected_techniques
            .iter()
            .map(String::as_str)
            .collect::<BTreeSet<_>>();
        for technique in &spec.expected_techniques {
            registered_capability_for(stage, technique)?;
        }
        for technique in capabilities_for_stage(stage)
            .into_iter()
            .flat_map(|capability| capability.techniques)
        {
            if !declared.contains(technique) {
                return Err(ToolTruthDenominatorError::CapabilityOutsideStageSpec(
                    technique.to_string(),
                ));
            }
        }
    }
    Ok(())
}

fn sha256_prefixed(bytes: &[u8]) -> String {
    let digest = Sha256::digest(bytes);
    let mut encoded = String::with_capacity(71);
    encoded.push_str("sha256:");
    for byte in digest {
        write!(&mut encoded, "{byte:02x}").expect("writing to String cannot fail");
    }
    encoded
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum ToolTruthControlDecision {
    Allow,
    Hold,
}

impl ToolTruthControlDecision {
    pub const fn as_str(self) -> &'static str {
        match self {
            Self::Allow => "allow",
            Self::Hold => "hold",
        }
    }
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum ToolTruthCoverageGrade {
    Complete,
    Degraded,
    Incomplete,
}

impl ToolTruthCoverageGrade {
    pub const fn as_str(self) -> &'static str {
        match self {
            Self::Complete => "complete",
            Self::Degraded => "degraded",
            Self::Incomplete => "incomplete",
        }
    }
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct ToolTruthReceiptCoverage {
    pub expected: usize,
    pub terminal: usize,
    pub degraded: usize,
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct ToolTruthShadowAssessment {
    pub legacy_allowed: bool,
    pub control_decision: ToolTruthControlDecision,
    pub coverage_grade: ToolTruthCoverageGrade,
    pub divergence: bool,
    pub missing_dynamic_child_keys: Vec<String>,
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct DynamicChildGraph {
    pub manifest_kind: String,
    pub manifest_present: bool,
    pub sealed_empty: bool,
    pub declared_member_count: usize,
    pub declared_manifest_hash: String,
    pub member_keys: Vec<String>,
    pub terminal_closure_keys: Vec<String>,
}

impl DynamicChildGraph {
    pub fn from_keys(kind: &str, members: &[&str], closed: &[&str]) -> Self {
        Self::from_owned_keys(
            kind,
            members.iter().map(|key| (*key).to_string()).collect(),
            closed.iter().map(|key| (*key).to_string()).collect(),
        )
    }

    pub fn from_owned_keys(kind: &str, mut members: Vec<String>, mut closed: Vec<String>) -> Self {
        members.sort();
        members.dedup();
        closed.sort();
        closed.dedup();
        Self {
            manifest_kind: kind.to_string(),
            manifest_present: true,
            sealed_empty: false,
            declared_member_count: members.len(),
            declared_manifest_hash: child_manifest_hash(kind, &members),
            member_keys: members,
            terminal_closure_keys: closed,
        }
    }

    pub fn sealed_empty(kind: &str) -> Self {
        Self {
            manifest_kind: kind.to_string(),
            manifest_present: true,
            sealed_empty: true,
            declared_member_count: 0,
            declared_manifest_hash: child_manifest_hash(kind, &[]),
            member_keys: Vec::new(),
            terminal_closure_keys: Vec::new(),
        }
    }
}

fn child_manifest_hash(kind: &str, members: &[String]) -> String {
    let mut bytes = kind.as_bytes().to_vec();
    for member in members {
        bytes.push(0x1f);
        bytes.extend_from_slice(member.as_bytes());
    }
    sha256_prefixed(&bytes)
}

pub fn evaluate_dynamic_child_closure(graphs: &[DynamicChildGraph]) -> ToolTruthShadowAssessment {
    let mut missing = BTreeSet::new();
    for graph in graphs {
        let members = graph.member_keys.iter().cloned().collect::<BTreeSet<_>>();
        let closures = graph
            .terminal_closure_keys
            .iter()
            .cloned()
            .collect::<BTreeSet<_>>();
        let manifest_valid = graph.manifest_present
            && graph.declared_member_count == members.len()
            && graph.declared_manifest_hash
                == child_manifest_hash(
                    &graph.manifest_kind,
                    &members.iter().cloned().collect::<Vec<_>>(),
                )
            && ((members.is_empty() && graph.sealed_empty)
                || (!members.is_empty() && !graph.sealed_empty));
        if !manifest_valid {
            missing.insert(format!("manifest:{}", graph.manifest_kind));
            continue;
        }
        for key in members.difference(&closures) {
            missing.insert((*key).clone());
        }
        for key in closures.difference(&members) {
            missing.insert(format!("unexpected:{key}"));
        }
    }
    let missing_dynamic_child_keys = missing.into_iter().collect::<Vec<_>>();
    let complete = missing_dynamic_child_keys.is_empty();
    ToolTruthShadowAssessment {
        legacy_allowed: false,
        control_decision: if complete {
            ToolTruthControlDecision::Allow
        } else {
            ToolTruthControlDecision::Hold
        },
        coverage_grade: if complete {
            ToolTruthCoverageGrade::Complete
        } else {
            ToolTruthCoverageGrade::Incomplete
        },
        divergence: false,
        missing_dynamic_child_keys,
    }
}

pub fn evaluate_shadow_tool_truth(
    legacy_allowed: bool,
    coverage: Option<ToolTruthReceiptCoverage>,
    dynamic_children: &[DynamicChildGraph],
) -> ToolTruthShadowAssessment {
    let dynamic = evaluate_dynamic_child_closure(dynamic_children);
    let coverage_grade = match coverage {
        None => ToolTruthCoverageGrade::Incomplete,
        Some(coverage) if coverage.expected == 0 || coverage.terminal < coverage.expected => {
            ToolTruthCoverageGrade::Incomplete
        }
        Some(coverage) if coverage.degraded > 0 => ToolTruthCoverageGrade::Degraded,
        Some(_) => ToolTruthCoverageGrade::Complete,
    };
    let coverage_grade = if dynamic.coverage_grade == ToolTruthCoverageGrade::Incomplete {
        ToolTruthCoverageGrade::Incomplete
    } else {
        coverage_grade
    };
    let control_decision = if coverage_grade == ToolTruthCoverageGrade::Incomplete {
        ToolTruthControlDecision::Hold
    } else {
        ToolTruthControlDecision::Allow
    };
    ToolTruthShadowAssessment {
        legacy_allowed,
        control_decision,
        coverage_grade,
        divergence: legacy_allowed != (control_decision == ToolTruthControlDecision::Allow),
        missing_dynamic_child_keys: dynamic.missing_dynamic_child_keys,
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::harness::types::StageKind;
    use uuid::Uuid;

    fn asset(value: &str, asset_type: &str, web_capable: bool) -> DenominatorAsset {
        DenominatorAsset {
            target_id: Uuid::new_v4(),
            exact_asset: value.to_string(),
            asset_type: asset_type.to_string(),
            web_capable,
        }
    }

    #[test]
    fn denominator_items_are_asset_times_expected_technique() {
        let assets = vec![
            asset("example.test", "domain", true),
            asset("198.51.100.10", "ip", false),
        ];
        let items = build_denominator_items(StageKind::TargetIntel, &assets).unwrap();
        assert_eq!(items.len(), 9);
        assert!(items
            .windows(2)
            .all(|pair| pair[0].input_key < pair[1].input_key));
    }

    #[test]
    fn denominator_catalog_is_exactly_the_embedded_spec_and_shared_applicability() {
        validate_denominator_catalog().unwrap();
    }

    #[test]
    fn new_spec_technique_without_registered_capability_fails_closed() {
        let mut spec = crate::harness::load_embedded_stage_spec(StageKind::TargetIntel).unwrap();
        spec.expected_techniques
            .push("GOLISH-INTEL-UNKNOWN".to_string());
        let error = build_denominator_items_from_spec(
            StageKind::TargetIntel,
            &spec,
            &[asset("example.test", "domain", true)],
        )
        .unwrap_err();
        assert!(matches!(
            error,
            ToolTruthDenominatorError::CapabilityMappingMissing(ref technique)
                if technique == "GOLISH-INTEL-UNKNOWN"
        ));
    }

    #[test]
    fn three_discovered_ports_with_only_two_fingerprinted_hold_gate() {
        let graph = DynamicChildGraph::from_keys(
            "open_tcp_port",
            &["tcp:80", "tcp:443", "tcp:8443"],
            &["tcp:80", "tcp:443"],
        );
        let assessment = evaluate_dynamic_child_closure(&[graph]);
        assert_eq!(assessment.control_decision, ToolTruthControlDecision::Hold);
        assert_eq!(
            assessment.coverage_grade,
            ToolTruthCoverageGrade::Incomplete
        );
        assert_eq!(assessment.missing_dynamic_child_keys, vec!["tcp:8443"]);
    }

    #[test]
    fn five_discovered_scripts_with_four_parsed_hold_gate() {
        let expected = (0..5)
            .map(|index| format!("script:{index}"))
            .collect::<Vec<_>>();
        let closed = expected[..4].to_vec();
        let graph = DynamicChildGraph::from_owned_keys("script", expected, closed);
        assert_eq!(
            evaluate_dynamic_child_closure(&[graph]).control_decision,
            ToolTruthControlDecision::Hold
        );
    }

    #[test]
    fn explicit_sealed_empty_child_manifest_is_not_missing() {
        let graph = DynamicChildGraph::sealed_empty("open_tcp_port");
        let assessment = evaluate_dynamic_child_closure(&[graph]);
        assert!(assessment.missing_dynamic_child_keys.is_empty());
        assert_eq!(assessment.coverage_grade, ToolTruthCoverageGrade::Complete);
    }

    #[test]
    fn missing_denominator_is_shadow_incomplete() {
        let assessment = evaluate_shadow_tool_truth(true, None, &[]);
        assert_eq!(assessment.control_decision, ToolTruthControlDecision::Hold);
        assert_eq!(
            assessment.coverage_grade,
            ToolTruthCoverageGrade::Incomplete
        );
        assert!(assessment.divergence);
    }

    #[test]
    fn tool_truth_shadow_grade_does_not_change_legacy_gate_result() {
        let assessment = evaluate_shadow_tool_truth(
            true,
            Some(ToolTruthReceiptCoverage {
                expected: 2,
                terminal: 1,
                degraded: 0,
            }),
            &[],
        );
        assert!(assessment.legacy_allowed);
        assert_eq!(assessment.control_decision, ToolTruthControlDecision::Hold);
        assert!(assessment.divergence);
    }
}
