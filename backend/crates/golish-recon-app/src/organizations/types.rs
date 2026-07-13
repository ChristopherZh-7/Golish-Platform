//! Wire types for the organizations commands: the read `Organization`
//! struct, candidate DTOs, and the profile-patch input (+ its conversion
//! into the DB-layer [`ProfilePatch`]).

use golish_db::repo::organizations::ProfilePatch;
use serde::{Deserialize, Serialize};
use serde_json::Value;

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct Organization {
    pub id: String,
    pub project_path: String,
    pub name: String,
    pub parent_id: Option<String>,
    pub description: String,
    pub owner: String,
    pub sort_order: i32,
    // profile 字段（与 5-tab UI 对应）
    pub aliases: Vec<String>,
    pub industry: String,
    pub tier: String,
    pub credit_code: String,
    pub domains: serde_json::Value,
    pub ip_ranges: serde_json::Value,
    pub asns: serde_json::Value,
    pub email_domains: serde_json::Value,
    pub scope_rules: serde_json::Value,
    pub intel: serde_json::Value,
    pub notes: String,
    // 二期字段（schema 已就位，UI 后续 PR）
    pub certificates: serde_json::Value,
    pub subsidiaries: serde_json::Value,
    pub business_systems: serde_json::Value,
    pub cloud_assets: serde_json::Value,
    pub github_orgs: serde_json::Value,
    pub social_accounts: serde_json::Value,
    pub historical_vulns: serde_json::Value,
    pub contacts: serde_json::Value,
    pub created_at: u64,
    pub updated_at: u64,
}

#[derive(Debug, Clone, Serialize, Deserialize, PartialEq)]
#[serde(rename_all = "snake_case")]
pub enum OrganizationCandidateKind {
    Organization,
    Target,
}

#[derive(Debug, Clone, Serialize, Deserialize, ts_rs::TS)]
#[serde(rename_all = "camelCase")]
#[ts(export, export_to = "../../../../frontend/lib/generated/")]
pub struct OrganizationCandidate {
    pub id: String,
    #[ts(type = "\"organization\" | \"target\"")]
    pub kind: OrganizationCandidateKind,
    pub label: String,
    pub value: String,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    #[ts(optional)]
    pub organization_id: Option<String>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    #[ts(optional)]
    pub ownership_percent: Option<String>,
    #[serde(default)]
    pub source: String,
    #[serde(default)]
    pub confidence: f64,
    #[serde(default)]
    pub status: String,
    #[serde(default)]
    #[ts(type = "unknown")]
    pub evidence: Value,
    #[serde(default)]
    #[ts(type = "number")]
    pub created_at: u64,
}

#[derive(Debug, Clone, Serialize, Deserialize, Default, ts_rs::TS)]
#[serde(rename_all = "camelCase")]
#[ts(export, export_to = "../../../../frontend/lib/generated/")]
pub struct OrganizationCandidates {
    pub organizations: Vec<OrganizationCandidate>,
    pub targets: Vec<OrganizationCandidate>,
}

/// One stable human-review row for a candidate organization. Text fields remain
/// editable, while the identity fields stay immutable throughout review.
#[derive(Debug, Clone, Serialize, Deserialize, ts_rs::TS)]
#[serde(rename_all = "camelCase")]
#[ts(export, export_to = "../../../../frontend/lib/generated/")]
pub struct UnitReviewDecisionRow {
    pub review_row_id: String,
    pub candidate_id: String,
    pub organization_id: Option<String>,
    pub name: String,
    pub aliases: Vec<String>,
    pub domains: Vec<String>,
    pub ownership_percent: Option<String>,
    pub included: bool,
}

#[derive(Debug, Clone, Serialize, Deserialize, ts_rs::TS)]
#[serde(rename_all = "camelCase")]
#[ts(export, export_to = "../../../../frontend/lib/generated/")]
pub struct UnitReviewSubmission {
    pub rows: Vec<UnitReviewDecisionRow>,
}

/// 前端 PATCH 入参；每个字段 `Option` 表示「不传 = 不修改」。
///
/// 校验在 `validate_profile_patch` 一遍走完，发现任一字段不合法立刻 400，
/// 不写库——避免一半字段进去、一半 reject 的半成品状态。
// NOTE: keys are snake_case (serde default), matching both the `Organization`
// read struct and the frontend `lib/api/organizations.ts` patch payload. An
// earlier `rename_all = "camelCase"` here silently dropped every multi-word
// field (credit_code / ip_ranges / scope_rules / …) sent by the UI.
#[derive(Debug, Clone, Default, Deserialize)]
pub struct OrganizationProfilePatch {
    pub aliases: Option<Vec<String>>,
    pub industry: Option<String>,
    pub tier: Option<String>,
    pub credit_code: Option<String>,
    pub domains: Option<serde_json::Value>,
    pub ip_ranges: Option<serde_json::Value>,
    pub asns: Option<serde_json::Value>,
    pub email_domains: Option<serde_json::Value>,
    pub scope_rules: Option<serde_json::Value>,
    pub intel: Option<serde_json::Value>,
    pub notes: Option<String>,
    pub certificates: Option<serde_json::Value>,
    pub subsidiaries: Option<serde_json::Value>,
    pub business_systems: Option<serde_json::Value>,
    pub cloud_assets: Option<serde_json::Value>,
    pub github_orgs: Option<serde_json::Value>,
    pub social_accounts: Option<serde_json::Value>,
    pub historical_vulns: Option<serde_json::Value>,
    pub contacts: Option<serde_json::Value>,
}

impl From<OrganizationProfilePatch> for ProfilePatch {
    fn from(p: OrganizationProfilePatch) -> Self {
        ProfilePatch {
            aliases: p.aliases,
            industry: p.industry,
            tier: p.tier,
            credit_code: p.credit_code,
            domains: p.domains,
            ip_ranges: p.ip_ranges,
            asns: p.asns,
            email_domains: p.email_domains,
            scope_rules: p.scope_rules,
            intel: p.intel,
            notes: p.notes,
            certificates: p.certificates,
            subsidiaries: p.subsidiaries,
            business_systems: p.business_systems,
            cloud_assets: p.cloud_assets,
            github_orgs: p.github_orgs,
            social_accounts: p.social_accounts,
            historical_vulns: p.historical_vulns,
            contacts: p.contacts,
        }
    }
}
