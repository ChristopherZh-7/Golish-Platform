//! Pure auto-promote decision logic for discovered subsidiary candidates.
//!
//! Decides which discovered organization candidates qualify for automatic
//! promotion to child organizations, based on the engagement's discovery
//! policy (ownership threshold, active status, dedupe keys). No DB / IO here:
//! the caller applies the decisions. Re-exported from the parent module.

use std::collections::HashSet;

use serde::{Deserialize, Serialize};
use serde_json::Value;

use crate::organizations::{OrganizationCandidate, OrganizationCandidates};
use golish_app_core::GolishError;

use super::filter_passes;

#[derive(Debug, Clone, Serialize, Deserialize, PartialEq, Eq)]
#[serde(rename_all = "snake_case")]
pub(crate) enum AutoPromoteSkipReason {
    EmptyName,
    MissingOwnership,
    OwnershipBelowThreshold,
    InactiveStatus,
    Duplicate,
    PolicyFilterFailed,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub(crate) struct AutoPromoteChildDecision {
    pub(crate) candidate: OrganizationCandidate,
    pub(crate) promote: bool,
    pub(crate) reason: Option<AutoPromoteSkipReason>,
    pub(crate) ownership_percent: Option<f64>,
}

fn parse_ownership_percent(raw: &str) -> Option<f64> {
    let cleaned = raw.trim().trim_end_matches('%').replace(',', "");
    if cleaned.is_empty() {
        return None;
    }
    cleaned.parse::<f64>().ok()
}

fn discovery_policy_threshold(
    policy: &golish_pentest::models::AssetIntelDiscoveryConfig,
) -> Option<f64> {
    use golish_pentest::models::AssetIntelNormalizeFilterOp as Op;
    policy
        .promote_when
        .iter()
        .find(|filter| {
            filter.field == policy.ownership_field && matches!(filter.op, Op::Gte | Op::Gt | Op::Eq)
        })
        .and_then(|filter| parse_ownership_percent(&filter.value))
}

fn candidate_raw_field<'a>(candidate: &'a OrganizationCandidate, field: &str) -> Option<&'a str> {
    candidate
        .evidence
        .get("raw")
        .and_then(|raw| raw.get(field))
        .and_then(Value::as_str)
        .map(str::trim)
        .filter(|value| !value.is_empty() && *value != "-")
}

pub(crate) fn auto_promote_child_decisions(
    candidates: &OrganizationCandidates,
    policy: &golish_pentest::models::AssetIntelDiscoveryConfig,
    existing_child_names: &HashSet<String>,
) -> Vec<AutoPromoteChildDecision> {
    let mut seen = existing_child_names.clone();
    let threshold = discovery_policy_threshold(policy);
    let mut decisions = Vec::new();
    for candidate in &candidates.organizations {
        let name = candidate.value.trim();
        if name.is_empty() {
            decisions.push(AutoPromoteChildDecision {
                candidate: candidate.clone(),
                promote: false,
                reason: Some(AutoPromoteSkipReason::EmptyName),
                ownership_percent: None,
            });
            continue;
        }

        let raw = candidate.evidence.get("raw").unwrap_or(&Value::Null);
        if !filter_passes(raw, &policy.promote_when) {
            let status = candidate_raw_field(candidate, "status");
            let ownership = candidate_raw_field(candidate, &policy.ownership_field)
                .and_then(parse_ownership_percent);
            let reason = if threshold.is_some_and(|min| ownership.is_some_and(|value| value < min))
            {
                AutoPromoteSkipReason::OwnershipBelowThreshold
            } else if status.is_some_and(|value| value != "开业" && value != "存续") {
                AutoPromoteSkipReason::InactiveStatus
            } else {
                AutoPromoteSkipReason::PolicyFilterFailed
            };
            decisions.push(AutoPromoteChildDecision {
                candidate: candidate.clone(),
                promote: false,
                reason: Some(reason),
                ownership_percent: ownership,
            });
            continue;
        }

        let ownership = candidate_raw_field(candidate, &policy.ownership_field)
            .and_then(parse_ownership_percent);
        if ownership.is_none() {
            decisions.push(AutoPromoteChildDecision {
                candidate: candidate.clone(),
                promote: false,
                reason: Some(AutoPromoteSkipReason::MissingOwnership),
                ownership_percent: None,
            });
            continue;
        };
        let percent = ownership.expect("checked is_some above");

        if existing_child_names.contains(&name.to_lowercase()) {
            decisions.push(AutoPromoteChildDecision {
                candidate: candidate.clone(),
                promote: false,
                reason: Some(AutoPromoteSkipReason::Duplicate),
                ownership_percent: Some(percent),
            });
            continue;
        }

        let dedupe_key = policy
            .dedupe_by
            .iter()
            .filter_map(|field| candidate_raw_field(candidate, field))
            .next()
            .unwrap_or(name)
            .to_lowercase();
        if !seen.insert(dedupe_key) {
            decisions.push(AutoPromoteChildDecision {
                candidate: candidate.clone(),
                promote: false,
                reason: Some(AutoPromoteSkipReason::Duplicate),
                ownership_percent: Some(percent),
            });
            continue;
        }

        decisions.push(AutoPromoteChildDecision {
            candidate: candidate.clone(),
            promote: true,
            reason: None,
            ownership_percent: Some(percent),
        });
    }
    decisions
}

/// 从候选 provider 的 discovery 配置里选出第一个开启 `auto_promote` 的；都没有则
/// 返回 default（`auto_promote=false`）。让 harness 子公司发现路径复用 GUI
/// `asset_intel_hydrate_subsidiaries` 的 policy 选择（promote 决策仍由
/// `auto_promote_child_decisions` 纯函数兜底，含 I8 的「跑了→筛掉」vs「没跑」区分）。
pub(crate) fn select_discovery_policy<'a>(
    discoveries: impl IntoIterator<Item = &'a golish_pentest::models::AssetIntelDiscoveryConfig>,
) -> golish_pentest::models::AssetIntelDiscoveryConfig {
    discoveries
        .into_iter()
        .find(|d| d.auto_promote)
        .cloned()
        .unwrap_or_default()
}

pub(crate) fn clear_engagement_candidates_from_intel(
    mut intel: Value,
) -> Result<Value, GolishError> {
    if !intel.is_object() {
        return Ok(Value::Object(serde_json::Map::new()));
    }
    let root = intel.as_object_mut().ok_or_else(|| {
        GolishError::Internal("organization intel must be a JSON object".to_string())
    })?;
    if let Some(engagement) = root.get_mut("engagement").and_then(Value::as_object_mut) {
        engagement.remove("candidates");
    }
    Ok(intel)
}

#[cfg(test)]
mod policy_tests {
    use super::select_discovery_policy;

    fn disc(auto: bool) -> golish_pentest::models::AssetIntelDiscoveryConfig {
        golish_pentest::models::AssetIntelDiscoveryConfig {
            auto_promote: auto,
            ..Default::default()
        }
    }

    #[test]
    fn picks_first_auto_promote_policy() {
        assert!(select_discovery_policy(&[disc(false), disc(true)]).auto_promote);
    }

    #[test]
    fn no_auto_promote_or_empty_yields_default_off() {
        assert!(!select_discovery_policy(&[disc(false)]).auto_promote);
        let empty: &[golish_pentest::models::AssetIntelDiscoveryConfig] = &[];
        assert!(!select_discovery_policy(empty).auto_promote);
    }
}
