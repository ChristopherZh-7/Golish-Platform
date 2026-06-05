//! Harness shared DTO 类型 (Doc 3 §2 / §4 / §6 / §11).
//!
//! 本文件**只**放 cross-module 的 newtype / enum / struct, 不做 IO / 不带 trait
//! impl 块; 实施在各 module 里.

use serde::{Deserialize, Serialize};
use uuid::Uuid;

use golish_pentest::evidence_ledger::EvidenceAuditId;

/// Stage 种类 · 与 resources/harness/graph/operation_graph.json 的 nodes 一致.
///
/// Phase 1c MVP 仅实现 `ExternalAttackSurface`; 其它 stage 占位, 推 Phase 2-4.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum StageKind {
    Scoping,
    TargetIntel,
    ExternalAttackSurface,
    Enumeration,
    VulnTriage,
    Verification,
    AccessValidation,
    InternalDiscovery,
    ObjectivePathing,
    ObjectiveSimulation,
    Reporting,
    Cleanup,
}

impl StageKind {
    /// JSON / config 字符串映射. 仅 lossless 双向.
    pub fn as_str(self) -> &'static str {
        match self {
            Self::Scoping => "scoping",
            Self::TargetIntel => "target_intel",
            Self::ExternalAttackSurface => "external_attack_surface",
            Self::Enumeration => "enumeration",
            Self::VulnTriage => "vuln_triage",
            Self::Verification => "verification",
            Self::AccessValidation => "access_validation",
            Self::InternalDiscovery => "internal_discovery",
            Self::ObjectivePathing => "objective_pathing",
            Self::ObjectiveSimulation => "objective_simulation",
            Self::Reporting => "reporting",
            Self::Cleanup => "cleanup",
        }
    }

    /// 名为 `try_parse` 而非 `from_str` 以避免与 `std::str::FromStr::from_str` 重名;
    /// 调用方写 `StageKind::try_parse(...)` 即可.
    pub fn try_parse(s: &str) -> Option<Self> {
        Some(match s {
            "scoping" => Self::Scoping,
            "target_intel" => Self::TargetIntel,
            "external_attack_surface" => Self::ExternalAttackSurface,
            "enumeration" => Self::Enumeration,
            "vuln_triage" => Self::VulnTriage,
            "verification" => Self::Verification,
            "access_validation" => Self::AccessValidation,
            "internal_discovery" => Self::InternalDiscovery,
            "objective_pathing" => Self::ObjectivePathing,
            "objective_simulation" => Self::ObjectiveSimulation,
            "reporting" => Self::Reporting,
            "cleanup" => Self::Cleanup,
            _ => return None,
        })
    }
}

/// Stage risk level · stage_spec.json 字段映射.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum RiskLevel {
    Low,
    Medium,
    High,
    Critical,
}

/// IntentAxis · 用户意图四档 (Doc 3 §6.1).
///
/// gate 验证时读 (Doc 3 §2.3 effective_tool_allow_set 投影), agent 看不到.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum IntentAxis {
    PassiveObserve,
    ActiveProbe,
    VulnValidation,
    ExploitValidation,
}

/// Agent continuity 二值 (Doc 3 §10).
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum AgentContinuity {
    SingleSession,
    MultiSessionRelay,
}

/// Doc 3 §4.3 Finding severity (与 golish-pentest::models::Severity 同语义但独立类型,
/// 不创依赖循环).
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum FindingSeverity {
    Info,
    Low,
    Medium,
    High,
    Critical,
}

impl FindingSeverity {
    /// Ordering rank for threshold comparisons (Info=0 .. Critical=4). Used by
    /// the P2 verification gate to compare a finding's severity against a
    /// stage's `finding_verification.min_severity`.
    pub const fn rank(self) -> u8 {
        match self {
            Self::Info => 0,
            Self::Low => 1,
            Self::Medium => 2,
            Self::High => 3,
            Self::Critical => 4,
        }
    }
}

/// Doc 3 §4.3 StageClaim · 每个 claim 必有 evidence_refs.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct StageClaim {
    pub kind: String,
    pub subject: String,
    pub summary: String,
    pub evidence_ids: Vec<EvidenceAuditId>,
}

/// Doc 3 §4.3 SkippedCheckRecord · reason 引用 Doc 1 §4.6 强制枚举 SkipReason.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct SkippedCheckRecord {
    pub check: String,
    pub reason: golish_pentest::evidence_ledger::SkipReason,
}

/// Doc 3 §4.3 Finding · 结构化交付.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct HarnessFinding {
    pub finding_id: Uuid,
    pub kind: String,
    pub subject: String,
    pub severity: FindingSeverity,
    pub evidence_refs: Vec<EvidenceAuditId>,
}

/// Coverage matrix 单元格状态（设计 `docs/design/2026-06-05-coverage-matrix.md`）。
///
/// 缺失（不在矩阵里）≡ `not_attempted` ≡ 不过关——这是 AGENTS.md I8
/// 「已检查为空 ≠ 未检查」的落地：`CheckedEmpty` 是**显式终态**且须有证据/理由。
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum CoverageStatus {
    /// 测了且有发现 → 必须挂 evidence_refs。
    Found,
    /// 测了、无发现 → 必须挂 evidence_refs / note（≠ 未测）。
    CheckedEmpty,
    /// 被阻断（WAF / 权限 / 越界）→ note 说明。
    Blocked,
    /// 该技术对该资产不适用 → note 说明。
    NotApplicable,
}

/// Coverage matrix 一个 (资产 × 技术) 单元格。
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct CoverageCell {
    /// 资产标识（如 host / URL）。in-scope 资产全集由 caller 从 DB 注入做完整性核对
    /// （设计 §6.1，活体接线 deferred 于资产库）。
    pub asset: String,
    /// 技术类（MVP 自由字符串；目标 = OWASP WSTG / MITRE ATT&CK id）。
    pub technique: String,
    pub status: CoverageStatus,
    #[serde(default)]
    pub evidence_refs: Vec<EvidenceAuditId>,
    /// checked_empty / blocked / not_applicable 的理由。
    #[serde(default)]
    pub note: Option<String>,
}

/// Doc 3 §4.3 StageDeliverable · 所有 stage 通用的 gate 输入 contract.
///
/// 原 `ExternalAttackSurfaceDeliverable`; Phase B 泛化为全 stage 通用 (字段本就
/// 与 stage 语义无关). 保留 `ExternalAttackSurfaceDeliverable` 别名做向后兼容,
/// 旧 hook / gate / 单测零改动. 刻意不放 `stage_kind`: gate 以 `StageSpec.kind`
/// 为准, hook 以 `HarnessStageHint.stage_kind` 为准 (避免冗余真相源).
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct StageDeliverable {
    pub stage_id: String,
    pub stage_run_id: Uuid,
    pub claims: Vec<StageClaim>,
    pub evidence_refs: Vec<EvidenceAuditId>,
    #[serde(default)]
    pub skipped_checks: Vec<SkippedCheckRecord>,
    pub findings: Vec<HarnessFinding>,
    /// **app-level hint**; gate 以 spec 侧字段为准（min_invocations 等经 gate_rules
    /// 的 named_check 强制），不信此 agent 可清空的字段。min_invocations_check 读它做
    /// MVP 近似匹配（见该 check）。
    #[serde(default)]
    pub required_checks_done: Vec<String>,
    /// Coverage matrix（设计 2026-06-05）：(资产 × 技术) → 终态 + 证据/理由。
    /// 缺省空 = 不声明覆盖（向后兼容）。`coverage_complete` gate op 据此核完整性。
    #[serde(default)]
    pub coverage: Vec<CoverageCell>,
}

/// 向后兼容别名 (Phase B 泛化前的名字). 新代码用 `StageDeliverable`.
pub type ExternalAttackSurfaceDeliverable = StageDeliverable;

/// HarnessStageHint · 嵌入到 PlannedSubtask, Task 1c.6 在 task_orchestrator 用.
///
/// 用 newtype 包 StageKind 是为了在 PlannedSubtask 末尾 ts-rs 友好扩字段 +
/// serde(default) 后向兼容; 不直接放裸 StageKind.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct HarnessStageHint {
    pub stage_kind: StageKind,
}

impl HarnessStageHint {
    pub fn new(stage_kind: StageKind) -> Self {
        Self { stage_kind }
    }
}

/// Doc 3 §8 GateResult 配套 · recovery_actions 喂回 refiner 用.
#[derive(Debug, Clone, Serialize, Deserialize, Default)]
pub struct HarnessRecoveryActions {
    /// 自然语言 hint, refiner 拼到 system prompt.
    pub hints: Vec<String>,
    /// 期望补跑的 tool calls.
    pub repair_tool_calls: Vec<String>,
    /// 期望补 evidence 的 kind (与 evidence_kinds.json 一致).
    pub missing_evidence_kinds: Vec<String>,
}

impl HarnessRecoveryActions {
    pub fn is_empty(&self) -> bool {
        self.hints.is_empty()
            && self.repair_tool_calls.is_empty()
            && self.missing_evidence_kinds.is_empty()
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn stage_kind_serde_snake_case_roundtrip() {
        for kind in [
            StageKind::Scoping,
            StageKind::TargetIntel,
            StageKind::ExternalAttackSurface,
            StageKind::Enumeration,
            StageKind::VulnTriage,
            StageKind::Verification,
            StageKind::Cleanup,
        ] {
            let s = serde_json::to_string(&kind).unwrap();
            let back: StageKind = serde_json::from_str(&s).unwrap();
            assert_eq!(kind, back);
            // as_str / try_parse 双向也要一致
            assert_eq!(StageKind::try_parse(kind.as_str()), Some(kind));
        }
    }

    #[test]
    fn stage_kind_try_parse_unknown_returns_none() {
        assert_eq!(StageKind::try_parse("garbage"), None);
        assert_eq!(StageKind::try_parse(""), None);
    }

    #[test]
    fn risk_level_serde() {
        let r = serde_json::to_string(&RiskLevel::Medium).unwrap();
        assert_eq!(r, "\"medium\"");
    }

    #[test]
    fn intent_axis_four_variants_serde() {
        for v in [
            IntentAxis::PassiveObserve,
            IntentAxis::ActiveProbe,
            IntentAxis::VulnValidation,
            IntentAxis::ExploitValidation,
        ] {
            let s = serde_json::to_string(&v).unwrap();
            let back: IntentAxis = serde_json::from_str(&s).unwrap();
            assert_eq!(v, back);
        }
    }

    #[test]
    fn harness_recovery_actions_is_empty_when_default() {
        let r = HarnessRecoveryActions::default();
        assert!(r.is_empty());
    }

    #[test]
    fn harness_recovery_actions_not_empty_with_hint() {
        let mut r = HarnessRecoveryActions::default();
        r.hints.push("补一个 DNS 解析".to_string());
        assert!(!r.is_empty());
    }

    #[test]
    fn deliverable_serde_roundtrip() {
        let d = ExternalAttackSurfaceDeliverable {
            stage_id: "external_attack_surface".to_string(),
            stage_run_id: Uuid::new_v4(),
            claims: vec![],
            evidence_refs: vec![EvidenceAuditId::new(1), EvidenceAuditId::new(2)],
            skipped_checks: vec![],
            findings: vec![],
            required_checks_done: vec!["scope_status_present".to_string()],
            coverage: vec![],
        };
        let s = serde_json::to_string(&d).unwrap();
        let back: ExternalAttackSurfaceDeliverable = serde_json::from_str(&s).unwrap();
        assert_eq!(d.evidence_refs.len(), back.evidence_refs.len());
        assert_eq!(d.stage_id, back.stage_id);
    }

    #[test]
    fn harness_stage_hint_constructor() {
        let h = HarnessStageHint::new(StageKind::ExternalAttackSurface);
        assert_eq!(h.stage_kind, StageKind::ExternalAttackSurface);
    }
}
