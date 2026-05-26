//! StageSpec DTO + JSON loader (Doc 3 §4).
//!
//! Phase 1c MVP: 仅 external_attack_surface stage.

use std::collections::HashMap;

use serde::{Deserialize, Serialize};
use thiserror::Error;

use super::types::{AgentContinuity, RiskLevel, StageKind};

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

    pub allowed_tools: Vec<String>,
    #[serde(default)]
    pub forbidden_tools: Vec<String>,

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

    const EXTERNAL_ATTACK_SURFACE_JSON: &str = include_str!(
        "../../../../../resources/harness/stages/external_attack_surface.json"
    );

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
    fn external_attack_surface_tool_allow_and_deny() {
        let s = load_stage_spec_from_json(EXTERNAL_ATTACK_SURFACE_JSON).expect("parse");
        assert!(s.allowed_tools.contains(&"dns_resolve".to_string()));
        assert!(s.allowed_tools.contains(&"http_probe".to_string()));
        assert!(s.forbidden_tools.contains(&"metasploit".to_string()));
        assert!(s.forbidden_tools.contains(&"sqlmap".to_string()));
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
        assert_eq!(s.required_checks.len(), 5);
    }

    #[test]
    fn external_attack_surface_human_approval_required_before() {
        let s = load_stage_spec_from_json(EXTERNAL_ATTACK_SURFACE_JSON).expect("parse");
        let ha = s.human_approval.expect("human_approval");
        assert!(ha.required_before.contains(&"active_scan".to_string()));
        assert!(ha.required_before.contains(&"exploit_validation".to_string()));
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
