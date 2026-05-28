//! Profile DTO + JSON loader (Doc 3 §2).
//!
//! Phase 1c MVP: 仅 assessment profile · 加载 resources/harness/profiles/assessment.json.

use std::collections::HashSet;

use serde::{Deserialize, Serialize};
use thiserror::Error;

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
}
