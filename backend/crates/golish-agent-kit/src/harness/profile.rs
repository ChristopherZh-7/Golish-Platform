//! Profile DTO + JSON loader (Doc 3 §2).
//!
//! Phase 1c MVP: 仅 assessment profile · 加载 resources/harness/profiles/assessment.json.

use std::collections::HashSet;

use serde::{Deserialize, Serialize};
use thiserror::Error;

use super::stage_topology_contract::StageTopologyContract;
use super::types::StageKind;

/// Doc 3 §2.2 Authorization Level 六档 (L0-L5).
#[derive(Debug, Clone, Copy, PartialEq, Eq, PartialOrd, Ord, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum AuthorizationLevel {
    /// L0 · 仅查现有数据, 无任何探测.
    ObserveOnly,
    /// L1 · 仅被动收集 (公开数据库 / passive DNS / CT log).
    PassiveIntel,
    /// L2 · 低风险探测 (HTTP probe / DNS query / 主动子域枚举). assessment MAX.
    ActiveRecon,
    /// L3 · 非破坏性漏洞验证 (pentest).
    VulnValidation,
    /// L4 · 受控 exploit 验证 (pentest).
    ControlledExploit,
    /// L5 · 横移 / 后渗透 (red_team).
    PostExploitRedTeam,
}

impl AuthorizationLevel {
    /// L0=0 .. L5=5 数值, gate 比较 `tool_required_level <= authz_level`.
    pub const fn rank(self) -> u8 {
        match self {
            Self::ObserveOnly => 0,
            Self::PassiveIntel => 1,
            Self::ActiveRecon => 2,
            Self::VulnValidation => 3,
            Self::ControlledExploit => 4,
            Self::PostExploitRedTeam => 5,
        }
    }
}

/// Doc 3 §2.1 approval_policy.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct ApprovalPolicy {
    #[serde(default)]
    pub before_active_scan: bool,
    #[serde(default)]
    pub before_scope_expansion: bool,
}

/// scoping 阶段的 per-profile 行为策略 (设计 2026-06-06-scoping-per-mode-gate-hitl §3.2).
///
/// 容器级 `serde(default)`: 旧 profile JSON 无此块时整体取 [`ScopingPolicy::default`]
/// (保守安全默认: 要求人工确认 scope); `deny_unknown_fields` 拦住块内字段拼写错误.
#[derive(Debug, Clone, Serialize, Deserialize, PartialEq, Eq)]
#[serde(default, deny_unknown_fields)]
pub struct ScopingPolicy {
    /// 是否必须确认主体.
    pub require_subject: bool,
    /// 主体形态.
    pub subject_kind: SubjectKind,
    /// 红队专用: 先产出「单位名称候选」交人判断 (复用 organization_candidates).
    pub require_unit_candidates: bool,
    /// 资产确认方式.
    pub asset_confirmation: AssetConfirmation,
    /// 硬门禁开关: true 时 scoping 通过前必须有 `scope_human_approved` claim.
    pub require_human_scope_approval: bool,
    /// scoping 是否落组织 (pentest / red_team true).
    pub write_organizations: bool,
}

/// scoping 主体形态.
#[derive(Debug, Clone, Copy, Serialize, Deserialize, PartialEq, Eq, Default)]
#[serde(rename_all = "snake_case")]
pub enum SubjectKind {
    /// 不要求主体 (smoke).
    #[default]
    None,
    /// 自由文本主体, 记入 claim.subject (assessment / bug_bounty).
    Freetext,
    /// 必须建/选 organization (pentest / red_team).
    Organization,
    /// 组织或自由文本 (保留枚举值, 当前无 profile 使用).
    OrganizationOrFreetext,
    /// 云租户/账号 (cloud_assessment).
    CloudTenant,
}

/// scoping 资产确认方式.
#[derive(Debug, Clone, Copy, Serialize, Deserialize, PartialEq, Eq, Default)]
#[serde(rename_all = "snake_case")]
pub enum AssetConfirmation {
    /// 不交互 (smoke).
    None,
    /// AI 直接写, 仅记录, 不停下来确认.
    Auto,
    /// 列表给人增删改确认 (默认).
    #[default]
    Interactive,
}

impl Default for ScopingPolicy {
    /// 保守安全默认 (无 scoping_policy 的旧 profile): 要求人工确认 scope.
    fn default() -> Self {
        Self {
            require_subject: false,
            subject_kind: SubjectKind::Freetext,
            require_unit_candidates: false,
            asset_confirmation: AssetConfirmation::Interactive,
            require_human_scope_approval: true,
            write_organizations: false,
        }
    }
}

/// target_intel 阶段的 per-profile 行为策略
/// (设计 2026-06-06-intel-stage-ai-driven-per-mode §3.2).
///
/// 容器级 `serde(default)`: 旧 profile JSON 无此块时整体取 [`IntelPolicy::default`]
/// (保守默认: 跑被动情报); `deny_unknown_fields` 拦住块内字段拼写错误.
#[derive(Debug, Clone, Serialize, Deserialize, PartialEq, Eq)]
#[serde(default, deny_unknown_fields)]
pub struct IntelPolicy {
    /// 跑被动情报 (run) 还是跳过 (skip, 渗透: 资产明确直奔主动).
    pub passive_intel: PassiveIntelMode,
    /// 红队专用: 先做 ENScan 子公司发现.
    pub discover_subsidiaries: bool,
    /// 字段富化 (0.zone/quake/fofa…).
    pub enrich_assets: bool,
}

/// 被动情报模式.
#[derive(Debug, Clone, Copy, Serialize, Deserialize, PartialEq, Eq, Default)]
#[serde(rename_all = "snake_case")]
pub enum PassiveIntelMode {
    /// 跑被动收集 (默认).
    #[default]
    Run,
    /// 跳过被动 (渗透: 资产已在 scoping 确认).
    Skip,
}

impl Default for IntelPolicy {
    /// 保守默认 (旧 profile): 跑被动、不主动发现子公司、做富化.
    fn default() -> Self {
        Self {
            passive_intel: PassiveIntelMode::Run,
            discover_subsidiaries: false,
            enrich_assets: true,
        }
    }
}

/// Doc 3 §2.1 Profile · resources/harness/profiles/*.json 映射.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct Profile {
    pub id: String,
    pub display_name: String,
    pub max_authorization: AuthorizationLevel,
    pub allowed_stage_kinds: Vec<StageKind>,
    pub forbidden_stage_kinds: Vec<StageKind>,
    #[serde(default)]
    pub approval_policy: Option<ApprovalPolicy>,
    /// scoping 阶段 per-profile 策略 (设计 2026-06-06). 缺省 = `ScopingPolicy::default()`.
    #[serde(default)]
    pub scoping_policy: ScopingPolicy,
    /// target_intel 阶段 per-profile 策略 (设计 2026-06-06). 缺省 = `IntelPolicy::default()`.
    #[serde(default)]
    pub intel_policy: IntelPolicy,
    #[serde(default)]
    pub cleanup_required: bool,
    #[serde(default)]
    pub evidence_required: bool,
}

impl Profile {
    /// Doc 3 §3.3 DAG 投影 (helper) · allowed_stage_kinds 决定 profile 可走的 stage.
    pub fn allowed_stage_set(&self) -> HashSet<StageKind> {
        self.allowed_stage_kinds.iter().copied().collect()
    }

    /// Project the profile's historical attack-analysis slot through one
    /// operation-frozen topology.
    ///
    /// Profile JSON remains the stable capability/policy catalog. Legacy
    /// operations consume its Candidate+Verification pair byte-for-byte;
    /// unified operations replace that *complete pair* with AU+Investigation.
    /// A partial or mixed pair is configuration corruption and fails closed
    /// instead of silently producing a graph with a missing authority stage.
    pub fn allowed_stage_set_for_topology(
        &self,
        topology: StageTopologyContract,
    ) -> Result<HashSet<StageKind>, ProfileTopologyError> {
        let mut allowed = self.allowed_stage_set();
        let legacy_count = [StageKind::AttackCandidate, StageKind::Verification]
            .into_iter()
            .filter(|stage| allowed.contains(stage))
            .count();
        let unified_count = [
            StageKind::ApplicationUnderstanding,
            StageKind::Investigation,
        ]
        .into_iter()
        .filter(|stage| allowed.contains(stage))
        .count();

        if !matches!(legacy_count, 0 | 2) || !matches!(unified_count, 0 | 2) {
            return Err(ProfileTopologyError::PartialAttackAnalysisPair);
        }
        if legacy_count > 0 && unified_count > 0 {
            return Err(ProfileTopologyError::MixedAttackAnalysisTopology);
        }

        match topology {
            StageTopologyContract::LegacyCandidateVerificationV1 => {
                if unified_count > 0 {
                    return Err(ProfileTopologyError::UnifiedPairInLegacyTopology);
                }
            }
            StageTopologyContract::UnifiedInvestigationV1 => {
                if legacy_count == 2 {
                    allowed.remove(&StageKind::AttackCandidate);
                    allowed.remove(&StageKind::Verification);
                    allowed.insert(StageKind::ApplicationUnderstanding);
                    allowed.insert(StageKind::Investigation);
                }
            }
        }
        Ok(allowed)
    }
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Error)]
pub enum ProfileTopologyError {
    #[error("profile contains only one member of an attack-analysis stage pair")]
    PartialAttackAnalysisPair,
    #[error("profile mixes legacy and unified attack-analysis stage pairs")]
    MixedAttackAnalysisTopology,
    #[error("profile selects unified attack-analysis stages for a legacy topology")]
    UnifiedPairInLegacyTopology,
}

/// Profile JSON 加载错误.
#[derive(Debug, Error)]
pub enum ProfileLoadError {
    #[error("invalid JSON: {0}")]
    Json(#[from] serde_json::Error),
    #[error("profile id mismatch: file says {found}, requested {requested}")]
    IdMismatch { found: String, requested: String },
}

/// 静态 JSON 字符串 → Profile.
///
/// Phase 1c MVP 仅暴露这条; 真正从 disk 读由调用方做 (`std::fs::read_to_string`).
/// 这样单测可以无 IO 直接 fixture 字符串验证.
///
/// serde 默认会忽略未知字段, 所以 $schema / $comment 注释字段不会被当作错误.
pub fn load_profile_from_json(raw: &str) -> Result<Profile, ProfileLoadError> {
    let profile: Profile = serde_json::from_str(raw)?;
    Ok(profile)
}

#[cfg(test)]
mod tests {
    use super::*;

    /// assessment.json 内联 (与 resources/harness/profiles/assessment.json 一致).
    const ASSESSMENT_JSON: &str =
        include_str!("../../../../../resources/harness/profiles/assessment.json");

    #[test]
    fn load_assessment_profile_basic_shape() {
        let p = load_profile_from_json(ASSESSMENT_JSON).expect("parse");
        assert_eq!(p.id, "assessment");
        assert_eq!(p.display_name, "Security Assessment");
        assert_eq!(p.max_authorization, AuthorizationLevel::ActiveRecon);
        assert!(p.evidence_required);
        assert!(!p.cleanup_required);
    }

    #[test]
    fn assessment_profile_allowed_stages_match_doc3() {
        let p = load_profile_from_json(ASSESSMENT_JSON).expect("parse");
        let allowed = p.allowed_stage_set();
        for required in [
            StageKind::Scoping,
            StageKind::TargetIntel,
            StageKind::ExternalAttackSurface,
            StageKind::Enumeration,
            StageKind::Reporting,
        ] {
            assert!(allowed.contains(&required), "missing stage: {:?}", required);
        }
    }

    #[test]
    fn assessment_profile_forbids_high_authz_stages() {
        let p = load_profile_from_json(ASSESSMENT_JSON).expect("parse");
        let forbidden: HashSet<_> = p.forbidden_stage_kinds.into_iter().collect();
        for fb in [
            StageKind::VulnTriage,
            StageKind::Verification,
            StageKind::AccessValidation,
            StageKind::Cleanup,
        ] {
            assert!(forbidden.contains(&fb), "should forbid: {:?}", fb);
        }
    }

    #[test]
    fn assessment_profile_approval_policy() {
        let p = load_profile_from_json(ASSESSMENT_JSON).expect("parse");
        let policy = p.approval_policy.expect("policy");
        assert!(policy.before_active_scan);
        assert!(policy.before_scope_expansion);
    }

    #[test]
    fn authorization_level_rank_strictly_increasing() {
        let levels = [
            AuthorizationLevel::ObserveOnly,
            AuthorizationLevel::PassiveIntel,
            AuthorizationLevel::ActiveRecon,
            AuthorizationLevel::VulnValidation,
            AuthorizationLevel::ControlledExploit,
            AuthorizationLevel::PostExploitRedTeam,
        ];
        for w in levels.windows(2) {
            assert!(w[0].rank() < w[1].rank());
        }
    }

    #[test]
    fn authorization_level_serde_snake_case() {
        let s = serde_json::to_string(&AuthorizationLevel::ActiveRecon).unwrap();
        assert_eq!(s, "\"active_recon\"");
    }

    #[test]
    fn scoping_policy_defaults_when_absent() {
        // 旧 profile JSON 无 scoping_policy → 取安全默认 (require_human_scope_approval=true).
        let json = r#"{"id":"x","display_name":"X","max_authorization":"active_recon",
            "allowed_stage_kinds":["scoping"],"forbidden_stage_kinds":[],
            "approval_policy":{"before_active_scan":true,"before_scope_expansion":true},
            "cleanup_required":false,"evidence_required":true}"#;
        let p = load_profile_from_json(json).expect("parse");
        assert!(p.scoping_policy.require_human_scope_approval);
        assert_eq!(p.scoping_policy.subject_kind, SubjectKind::Freetext);
        assert_eq!(
            p.scoping_policy.asset_confirmation,
            AssetConfirmation::Interactive
        );
        assert!(!p.scoping_policy.require_subject);
        assert!(!p.scoping_policy.write_organizations);
    }

    #[test]
    fn scoping_policy_parses_explicit_block() {
        // red_team: 强制组织 + 单位候选 + 落组织.
        let json = r#"{"id":"red_team","display_name":"Red Team","max_authorization":"post_exploit_red_team",
            "allowed_stage_kinds":["scoping"],"forbidden_stage_kinds":[],
            "approval_policy":{"before_active_scan":true,"before_scope_expansion":true},
            "cleanup_required":true,"evidence_required":true,
            "scoping_policy":{"require_subject":true,"subject_kind":"organization",
                "require_unit_candidates":true,"asset_confirmation":"interactive",
                "require_human_scope_approval":true,"write_organizations":true}}"#;
        let p = load_profile_from_json(json).expect("parse");
        assert_eq!(p.scoping_policy.subject_kind, SubjectKind::Organization);
        assert!(p.scoping_policy.require_unit_candidates);
        assert!(p.scoping_policy.write_organizations);
        assert!(p.scoping_policy.require_human_scope_approval);
    }

    #[test]
    fn scoping_policy_rejects_unknown_field() {
        // deny_unknown_fields: 块内拼写错误的字段应当报错, 而非被静默忽略.
        let json = r#"{"id":"x","display_name":"X","max_authorization":"active_recon",
            "allowed_stage_kinds":["scoping"],"forbidden_stage_kinds":[],
            "cleanup_required":false,"evidence_required":true,
            "scoping_policy":{"require_subject":true,"subject_kind":"organization",
                "require_unit_candidates":false,"asset_confirmation":"interactive",
                "require_human_scope_approval":true,"write_organizations":true,
                "typo_field":1}}"#;
        assert!(load_profile_from_json(json).is_err());
    }

    #[test]
    fn scoping_policy_subject_kind_serde_snake_case() {
        let s = serde_json::to_string(&SubjectKind::OrganizationOrFreetext).unwrap();
        assert_eq!(s, "\"organization_or_freetext\"");
        let s2 = serde_json::to_string(&SubjectKind::CloudTenant).unwrap();
        assert_eq!(s2, "\"cloud_tenant\"");
    }

    #[test]
    fn intel_policy_defaults_when_absent() {
        // 旧 profile JSON 无 intel_policy → 取保守默认 (跑被动, 做富化, 不主动发现子公司).
        let json = r#"{"id":"x","display_name":"X","max_authorization":"active_recon",
            "allowed_stage_kinds":["target_intel"],"forbidden_stage_kinds":[],
            "cleanup_required":false,"evidence_required":true}"#;
        let p = load_profile_from_json(json).expect("parse");
        assert_eq!(p.intel_policy.passive_intel, PassiveIntelMode::Run);
        assert!(!p.intel_policy.discover_subsidiaries);
        assert!(p.intel_policy.enrich_assets);
    }

    #[test]
    fn intel_policy_parses_pentest_skip_and_red_team_full() {
        // 渗透: passive_intel=skip (资产明确直奔主动).
        let pentest = r#"{"id":"pentest","display_name":"P","max_authorization":"controlled_exploit",
            "allowed_stage_kinds":["target_intel"],"forbidden_stage_kinds":[],
            "cleanup_required":false,"evidence_required":true,
            "intel_policy":{"passive_intel":"skip","discover_subsidiaries":false,"enrich_assets":false}}"#;
        let p = load_profile_from_json(pentest).expect("parse");
        assert_eq!(p.intel_policy.passive_intel, PassiveIntelMode::Skip);
        assert!(!p.intel_policy.enrich_assets);

        // 红队: 先发现子公司 + 富化字段.
        let red = r#"{"id":"red_team","display_name":"R","max_authorization":"post_exploit_red_team",
            "allowed_stage_kinds":["target_intel"],"forbidden_stage_kinds":[],
            "cleanup_required":true,"evidence_required":true,
            "intel_policy":{"passive_intel":"run","discover_subsidiaries":true,"enrich_assets":true}}"#;
        let r = load_profile_from_json(red).expect("parse");
        assert_eq!(r.intel_policy.passive_intel, PassiveIntelMode::Run);
        assert!(r.intel_policy.discover_subsidiaries);
        assert!(r.intel_policy.enrich_assets);
    }

    #[test]
    fn intel_policy_rejects_unknown_field() {
        // deny_unknown_fields: 块内拼写错误的字段应当报错.
        let json = r#"{"id":"x","display_name":"X","max_authorization":"active_recon",
            "allowed_stage_kinds":["target_intel"],"forbidden_stage_kinds":[],
            "cleanup_required":false,"evidence_required":true,
            "intel_policy":{"passive_intel":"run","discover_subsidiaries":false,
                "enrich_assets":true,"typo_field":1}}"#;
        assert!(load_profile_from_json(json).is_err());
    }

    #[test]
    fn passive_intel_mode_serde_snake_case() {
        assert_eq!(
            serde_json::to_string(&PassiveIntelMode::Run).unwrap(),
            "\"run\""
        );
        assert_eq!(
            serde_json::to_string(&PassiveIntelMode::Skip).unwrap(),
            "\"skip\""
        );
    }
}
