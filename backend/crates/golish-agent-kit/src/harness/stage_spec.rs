//! StageSpec DTO + JSON loader (Doc 3 §4).
//!
//! Phase 1c MVP: 仅 external_attack_surface stage.

use std::collections::HashMap;

use serde::{Deserialize, Serialize};
use thiserror::Error;

use super::types::{AgentContinuity, FindingSeverity, RiskLevel, StageKind};

/// P2 · per-stage "trustworthy conclusion" rule (verification gate).
///
/// Findings at/above `min_severity` must carry evidence: non-empty
/// `evidence_refs` (deliverable structural layer) and — when
/// `require_evidence_kinds` is set — at least one of those evidence rows must be
/// of a listed kind (ledger layer, enforced caller-side). Declarative: you set
/// this per stage in the stage JSON to define what "verified" means there.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct FindingVerificationRule {
    pub min_severity: FindingSeverity,
    #[serde(default)]
    pub require_evidence_kinds: Vec<String>,
}

/// Doc 3 §4.1 human_approval policy 嵌入字段.
#[derive(Debug, Clone, Default, Serialize, Deserialize)]
pub struct HumanApprovalPolicy {
    #[serde(default)]
    pub required_before: Vec<String>,
}

/// Doc 3 §9.2 carry_over 白名单条目.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct InheritsEvidenceFrom {
    pub stage_kind: StageKind,
    pub evidence_kinds: Vec<String>,
}

/// Doc 3 §4.1 StageSpec.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct StageSpec {
    pub id: String,
    pub kind: StageKind,
    pub risk_level: RiskLevel,

    #[serde(default)]
    pub requires_stages: Vec<StageKind>,
    #[serde(default)]
    pub allowed_next_stages: Vec<StageKind>,

    /// Category-based stage tool whitelist (deny-by-default). Each entry is a
    /// **type selector**: a bare category (`"recon"`), a `category/subcategory`
    /// (`"recon/dns"`), or a specific tool name (`"nmap"`). The per-stage tool
    /// boundary is enforced from this list via [`super::tool_taxonomy::stage_allows`]
    /// (only for scan invocations; agent/meta tools are exempt). Empty = no scan
    /// tools permitted (e.g. scoping / reporting). See
    /// `docs/design/2026-06-02-stage-tool-whitelist-enforcement.md`.
    #[serde(default)]
    pub allowed_tool_types: Vec<String>,

    pub deliverable_schema: String,
    pub gate_validator: String,

    #[serde(default)]
    pub required_checks: Vec<String>,
    #[serde(default)]
    pub min_invocations: HashMap<String, u32>,

    /// Doc 3 §8.3 vacuous detector 上限 (Other-type skip 数).
    #[serde(default)]
    pub max_other_skips: Option<u32>,

    #[serde(default)]
    pub human_approval: Option<HumanApprovalPolicy>,

    #[serde(default = "default_continuity")]
    pub agent_continuity: AgentContinuity,

    #[serde(default)]
    pub inherits_evidence_from: Vec<InheritsEvidenceFrom>,

    // ── P2 · 配置驱动的「过关证据」声明（你填这里，gate 照执，零代码） ──────────
    /// P2 · 该 stage 交付物必须含的 evidence 种类（ledger 回查；空=不强制）。
    /// 例：信息收集阶段填 ["dns_a","http_probe","subdomain"] 表示要有这些证据才过。
    #[serde(default)]
    pub required_evidence_kinds: Vec<String>,

    /// P2 · finding 验证规则：达到阈值 severity 的 finding 必须有证据 / PoC。
    /// 例：verification 阶段填 {"min_severity":"high","require_evidence_kinds":["poc","exploit_verified"]}。
    #[serde(default)]
    pub finding_verification: Option<FindingVerificationRule>,

    /// P2 · 交付物最少 finding / claim 数（None=不强制）。
    #[serde(default)]
    pub min_findings: Option<u32>,
    #[serde(default)]
    pub min_claims: Option<u32>,
}

fn default_continuity() -> AgentContinuity {
    AgentContinuity::SingleSession
}

#[derive(Debug, Error)]
pub enum StageSpecLoadError {
    #[error("invalid JSON: {0}")]
    Json(#[from] serde_json::Error),
}

pub fn load_stage_spec_from_json(raw: &str) -> Result<StageSpec, StageSpecLoadError> {
    let spec: StageSpec = serde_json::from_str(raw)?;
    Ok(spec)
}

#[cfg(test)]
mod tests {
    use super::*;

    const EXTERNAL_ATTACK_SURFACE_JSON: &str =
        include_str!("../../../../../resources/harness/stages/external_attack_surface.json");

    #[test]
    fn load_external_attack_surface_basic_shape() {
        let s = load_stage_spec_from_json(EXTERNAL_ATTACK_SURFACE_JSON).expect("parse");
        assert_eq!(s.id, "external_attack_surface");
        assert_eq!(s.kind, StageKind::ExternalAttackSurface);
        assert_eq!(s.risk_level, RiskLevel::Medium);
        assert_eq!(s.deliverable_schema, "ExternalAttackSurfaceDeliverable");
        assert_eq!(s.gate_validator, "validate_external_attack_surface_gate");
    }

    #[test]
    fn external_attack_surface_requires_and_next_stages() {
        let s = load_stage_spec_from_json(EXTERNAL_ATTACK_SURFACE_JSON).expect("parse");
        assert!(s.requires_stages.contains(&StageKind::Scoping));
        assert!(s.requires_stages.contains(&StageKind::TargetIntel));
        assert!(s.allowed_next_stages.contains(&StageKind::Enumeration));
        assert!(s.allowed_next_stages.contains(&StageKind::Reporting));
    }

    #[test]
    fn external_attack_surface_allowed_tool_types() {
        let s = load_stage_spec_from_json(EXTERNAL_ATTACK_SURFACE_JSON).expect("parse");
        assert!(s.allowed_tool_types.contains(&"recon/dns".to_string()));
        assert!(s.allowed_tool_types.contains(&"recon/http".to_string()));
        assert!(!s.allowed_tool_types.contains(&"web/injection".to_string()));
    }

    #[test]
    fn external_attack_surface_min_invocations() {
        let s = load_stage_spec_from_json(EXTERNAL_ATTACK_SURFACE_JSON).expect("parse");
        assert_eq!(s.min_invocations.get("dns_resolve"), Some(&1));
        assert_eq!(s.min_invocations.get("http_probe"), Some(&1));
        assert_eq!(s.min_invocations.get("subdomain_enum_passive"), Some(&1));
    }

    #[test]
    fn external_attack_surface_required_checks_count() {
        let s = load_stage_spec_from_json(EXTERNAL_ATTACK_SURFACE_JSON).expect("parse");
        assert_eq!(s.required_checks.len(), 6);
    }

    #[test]
    fn external_attack_surface_human_approval_required_before() {
        let s = load_stage_spec_from_json(EXTERNAL_ATTACK_SURFACE_JSON).expect("parse");
        let ha = s.human_approval.expect("human_approval");
        assert!(ha.required_before.contains(&"active_scan".to_string()));
        assert!(ha
            .required_before
            .contains(&"exploit_validation".to_string()));
    }

    #[test]
    fn external_attack_surface_inherits_evidence_from_target_intel() {
        let s = load_stage_spec_from_json(EXTERNAL_ATTACK_SURFACE_JSON).expect("parse");
        assert_eq!(s.inherits_evidence_from.len(), 1);
        let inh = &s.inherits_evidence_from[0];
        assert_eq!(inh.stage_kind, StageKind::TargetIntel);
        assert!(inh.evidence_kinds.contains(&"dns_a".to_string()));
        assert!(inh.evidence_kinds.contains(&"asn".to_string()));
        assert!(inh.evidence_kinds.contains(&"whois".to_string()));
    }

    #[test]
    fn external_attack_surface_agent_continuity_single_session() {
        let s = load_stage_spec_from_json(EXTERNAL_ATTACK_SURFACE_JSON).expect("parse");
        assert_eq!(s.agent_continuity, AgentContinuity::SingleSession);
    }

    #[test]
    fn external_attack_surface_max_other_skips() {
        let s = load_stage_spec_from_json(EXTERNAL_ATTACK_SURFACE_JSON).expect("parse");
        assert_eq!(s.max_other_skips, Some(2));
    }
}
