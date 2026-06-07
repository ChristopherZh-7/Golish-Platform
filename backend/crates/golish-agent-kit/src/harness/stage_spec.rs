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

    // gate-rules-migration (2026-06-05): 旧 `required_checks: Vec<String>` 固定菜单
    // 已删除；过关标准统一由下方 `gate_rules` 声明（数据积木 + named_check 逃生舱）。
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

    /// P2 · 数据驱动 gate 规则（设计 2026-06-05）。每条规则用固定积木 op 声明一条
    /// 过关标准，由 `super::gate::rule_engine::eval` 执行。缺省空 = 行为与旧版逐字节一致。
    #[serde(default)]
    pub gate_rules: Vec<super::gate::rule_engine::GateRule>,

    /// Coverage matrix（设计 2026-06-05）：本 stage 期望覆盖的技术类清单，由
    /// `gate_rules` 的 `coverage_complete` op 读取，对每个（自报）资产核对是否每类
    /// 技术都有终态。缺省空 = `coverage_complete` 视为 no-op（向后兼容）。值约定为
    /// **OWASP WSTG / MITRE ATT&CK id**（"挂标准"）；MVP 暂不做词典校验，先用字符串
    /// （taxonomy 词典化 + 动态 skeleton 生成见设计 §6.5，待资产库合入后接）。
    #[serde(default)]
    pub expected_techniques: Vec<String>,
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

    const TARGET_INTEL_JSON: &str =
        include_str!("../../../../../resources/harness/stages/target_intel.json");

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
        assert!(s.allowed_tool_types.contains(&"recon/visual".to_string()));
        assert!(!s.allowed_tool_types.contains(&"web/injection".to_string()));
        // 边界重构（按是否接触目标）：被动子域名 / url-history 下沉 target_intel，
        // EAS 不再允许它们（只做接触目标的主动测绘）。
        assert!(!s
            .allowed_tool_types
            .contains(&"recon/subdomain".to_string()));
        assert!(!s
            .allowed_tool_types
            .contains(&"recon/url-history".to_string()));
    }

    #[test]
    fn external_attack_surface_min_invocations() {
        let s = load_stage_spec_from_json(EXTERNAL_ATTACK_SURFACE_JSON).expect("parse");
        assert_eq!(s.min_invocations.get("dns_resolve"), Some(&1));
        assert_eq!(s.min_invocations.get("http_probe"), Some(&1));
        // 边界重构：被动子域名枚举不再钉为 EAS 硬地板（移交 target_intel）。
        assert_eq!(s.min_invocations.get("subdomain_enum_passive"), None);
    }

    #[test]
    fn external_attack_surface_gate_rules_count() {
        // gate-rules-migration: eas 过关标准 = scope×2 + named_check:surface_coverage
        // + named_check:min_invocations = 4 条 gate_rules（取代旧 6 个 required_checks）。
        let s = load_stage_spec_from_json(EXTERNAL_ATTACK_SURFACE_JSON).expect("parse");
        assert_eq!(s.gate_rules.len(), 4);
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
        // 边界重构：EAS 从 target_intel 继承子域名（host 来源），不再自枚举。
        assert!(inh.evidence_kinds.contains(&"subdomain".to_string()));
    }

    #[test]
    fn target_intel_owns_passive_subdomain_and_url_history() {
        let s = load_stage_spec_from_json(TARGET_INTEL_JSON).expect("parse");
        // 零接触被动技术全部归 target_intel。
        assert!(s
            .allowed_tool_types
            .contains(&"recon/subdomain".to_string()));
        assert!(s
            .allowed_tool_types
            .contains(&"recon/url-history".to_string()));
        // 被动子域名枚举设为本阶段硬地板（与从 EAS 删除对称）。
        assert_eq!(s.min_invocations.get("subdomain_enum_passive"), Some(&1));
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

    #[test]
    fn gate_rules_default_empty_and_parses() {
        // 缺省：未写 gate_rules 的 spec 解出空数组（向后兼容）。用最小内联 spec
        // （eas.json 现已迁移到 gate_rules，不再是“无 gate_rules”的样例）。
        let minimal = r#"{"id":"scoping","kind":"scoping","risk_level":"low",
            "deliverable_schema":"StageDeliverable","gate_validator":"validate_stage_gate"}"#;
        let s = load_stage_spec_from_json(minimal).expect("parse");
        assert!(s.gate_rules.is_empty());

        // 能解析内联 gate_rules。
        let with_rules = r#"{
            "id":"verification","kind":"verification","risk_level":"critical",
            "deliverable_schema":"StageDeliverable","gate_validator":"validate_stage_gate",
            "gate_rules":[
              { "op":"for_all","over":"findings",
                "where":{"pred":"severity_at_least","min":"high"},
                "require":{"pred":"non_empty","field":"evidence_refs"},
                "on_fail":{"reason":"high+ finding needs evidence"} }
            ]
        }"#;
        let s2 = load_stage_spec_from_json(with_rules).expect("parse with rules");
        assert_eq!(s2.gate_rules.len(), 1);
    }

    #[test]
    fn expected_techniques_default_empty_and_parses() {
        // 缺省：未写 expected_techniques 的 spec 解出空数组（coverage_complete no-op）。
        let minimal = r#"{"id":"vuln_triage","kind":"vuln_triage","risk_level":"high",
            "deliverable_schema":"StageDeliverable","gate_validator":"validate_stage_gate"}"#;
        let s = load_stage_spec_from_json(minimal).expect("parse");
        assert!(s.expected_techniques.is_empty());

        // 能解析 WSTG / ATT&CK id 字符串数组。
        let with = r#"{"id":"vuln_triage","kind":"vuln_triage","risk_level":"high",
            "deliverable_schema":"StageDeliverable","gate_validator":"validate_stage_gate",
            "expected_techniques":["WSTG-INPV-05","WSTG-ATHZ-04","T1190"]}"#;
        let s2 = load_stage_spec_from_json(with).expect("parse expected_techniques");
        assert_eq!(s2.expected_techniques.len(), 3);
        assert_eq!(s2.expected_techniques[0], "WSTG-INPV-05");
    }
}
