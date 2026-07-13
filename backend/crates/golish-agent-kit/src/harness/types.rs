//! Harness shared DTO 类型 (Doc 3 §2 / §4 / §6 / §11).
//!
//! 本文件**只**放 cross-module 的 newtype / enum / struct, 不做 IO / 不带 trait
//! impl 块; 实施在各 module 里.

use serde::{Deserialize, Serialize};
use uuid::Uuid;

use golish_pentest::evidence_ledger::EvidenceAuditId;

use super::stage_capability::StageCapabilitySuggestion;

/// serde helper: accept an explicit JSON `null` as the type's `Default` value.
///
/// Weak models routinely emit `null` for an "N/A" optional field (e.g.
/// `tested_units: null`, `coverage: null`) instead of omitting it. A bare
/// `#[serde(default)]` only covers a *missing* key, so an explicit `null` still
/// fails with `invalid type: null, expected u32 / a sequence` — which sent the
/// model into a submit→reject retry loop. Pair this with `default` on optional
/// fields so both "missing" and "null" collapse to the default.
fn null_as_default<'de, D, T>(de: D) -> Result<T, D::Error>
where
    D: serde::Deserializer<'de>,
    T: Default + Deserialize<'de>,
{
    Ok(Option::<T>::deserialize(de)?.unwrap_or_default())
}

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
    /// 候选合成阶段（设计 2026-07-02）：基于信息收集上下文 + vuln_triage found +
    /// RAG 先验推理，产出结构化攻击假设 [`AttackCandidate`] 清单。真打验证前的桥。
    AttackCandidate,
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
            Self::AttackCandidate => "attack_candidate",
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
            "attack_candidate" => Self::AttackCandidate,
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
    #[serde(default, deserialize_with = "null_as_default")]
    pub evidence_ids: Vec<EvidenceAuditId>,
    /// P5（2026-06-11）：该 claim 佐证的技术类 id（technique_taxonomy.json 登记，
    /// 如 GOLISH-INTEL-DNS / WSTG-INPV-05）。None = 未标注（旧数据 / 与 coverage
    /// 无关的 claim）。Some 时 schema_check 按词典 fail-closed 校验；
    /// coverage_complete(derive_from_items) / coverage_corroborated 据此关联矩阵。
    #[serde(default)]
    pub technique: Option<String>,
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
    #[serde(default, deserialize_with = "null_as_default")]
    pub evidence_refs: Vec<EvidenceAuditId>,
    /// P5（2026-06-11）：同 [`StageClaim::technique`]。
    #[serde(default)]
    pub technique: Option<String>,
}

/// AttackCandidate 优先级（设计 2026-07-02 §3.7）。`Default` = `Medium`。
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize, Default)]
#[serde(rename_all = "snake_case")]
pub enum CandidatePriority {
    High,
    #[default]
    Medium,
    Low,
}

/// AttackCandidate 处置状态机（设计 2026-07-02 §3.7）。`Default` = `Proposed`。
///
/// B 阶段产出 `Proposed`；`exploit_validation` 人审后 → `Approved` / `Rejected`；
/// C 真打后达终态 `Verified`（升 finding）/ `Refuted`（假阳性，须带证据 I8）/
/// `Blocked`（WAF/权限/授权，须 note）。
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize, Default)]
#[serde(rename_all = "snake_case")]
pub enum CandidateDisposition {
    #[default]
    Proposed,
    Approved,
    Rejected,
    Verified,
    Refuted,
    Blocked,
}

/// 结构化攻击假设（设计 2026-07-02 §3.3 / §3.7）。attack_candidate 阶段产物，挂
/// [`StageDeliverable::candidates`]；`candidate_grounded` gate 校验每条有
/// `rationale` + 非空 `evidence_refs`（堵凭空假设）。`parent_finding_id` 建立
/// a→b→c 攻击链血缘；`wave` 标记第几波（chain-wave 循环去重/收敛用）。
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct AttackCandidate {
    pub candidate_id: Uuid,
    pub target: String,
    pub hypothesis: String,
    /// WSTG / ATT&CK id（可选）。
    #[serde(default)]
    pub technique: Option<String>,
    pub rationale: String,
    #[serde(default, deserialize_with = "null_as_default")]
    pub evidence_refs: Vec<EvidenceAuditId>,
    /// wiki writeup / CVE id 等先验引用。
    #[serde(default, deserialize_with = "null_as_default")]
    pub prior_refs: Vec<String>,
    #[serde(default)]
    pub suggested_approach: String,
    #[serde(default)]
    pub priority: CandidatePriority,
    #[serde(default, deserialize_with = "null_as_default")]
    pub wave: u32,
    #[serde(default)]
    pub parent_finding_id: Option<Uuid>,
    #[serde(default)]
    pub disposition: CandidateDisposition,
}

/// Model-authored decision kind for one server-seeded Candidate work item.
/// Runtime ownership, target identity, wave identity and execution authority are
/// intentionally absent from the wire contract.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum CandidateDecisionKind {
    Candidate,
    NoCandidate,
}

pub const MAX_CANDIDATE_MANIFEST_ITEMS: usize = 100;
pub const MAX_CANDIDATE_WORK_ITEM_KEY_BYTES: usize = 256;
pub const MAX_CANDIDATE_HYPOTHESIS_BYTES: usize = 4 * 1024;
pub const MAX_CANDIDATE_RATIONALE_BYTES: usize = 8 * 1024;
pub const MAX_CANDIDATE_TECHNIQUE_BYTES: usize = 128;
pub const MAX_CANDIDATE_REASON_CODE_BYTES: usize = 64;
pub const MAX_CANDIDATE_DECISION_EVIDENCE_IDS: usize = 64;
pub const MAX_CANDIDATE_ACCEPTANCE_BYTES: usize = 256 * 1024;

/// Narrow Candidate V2 wire draft. The server joins `work_item_key` against the
/// immutable manifest, derives the Candidate id/plan/hash/risk, and binds the
/// trusted operation/scope/org/execution/submission identities at final PASS.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct CandidateDecisionDraft {
    pub work_item_key: String,
    pub decision: CandidateDecisionKind,
    #[serde(default)]
    pub hypothesis: Option<String>,
    pub rationale: String,
    #[serde(default)]
    pub technique: Option<String>,
    #[serde(default, deserialize_with = "null_as_default")]
    pub evidence_refs: Vec<i64>,
    #[serde(default)]
    pub no_candidate_reason_code: Option<String>,
}

/// Coverage matrix 单元格状态（设计 `docs/design/2026-06-05-coverage-matrix.md`）。
///
/// 缺失（不在矩阵里）≡ `not_attempted` ≡ 不过关——这是 AGENTS.md I8
/// 「已检查为空 ≠ 未检查」的落地：`CheckedEmpty` 是**显式终态**；运行事实由
/// backend ledger/DB truth 裁决，模型侧 evidence id 字段只是可选调试引用。
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum CoverageStatus {
    /// 测了且有发现。
    Found,
    /// 测了、无发现（≠ 未测）。
    CheckedEmpty,
    /// 被阻断（WAF / 权限 / 越界）→ note 说明。
    Blocked,
    /// 该技术对该资产不适用 → note 说明。
    NotApplicable,
}

/// `blocked` / `not_applicable` 的**结构化原因类别**（T1，设计
/// `2026-06-23-coverage-note-required.md`）。与自由文本 `note` 互补：`note` 给人读，
/// `reason_kind` 给机器分类（审计/诊断/recovery 提示）。可选元数据——本期 gate 只强制
/// `note` 非空，不强制 `reason_kind`。
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum ReasonKind {
    /// 缺数据源（无对应 provider 配置/不可用）。
    ProviderMissing,
    /// 缺凭证（provider 在但无 API key / 认证失败）。
    CredentialMissing,
    /// 被限流（外部服务 rate limit）。
    RateLimited,
    /// 缺工具（CLI 工具未安装 / 不在 PATH）。
    ToolMissing,
    /// 越界（该资产/技术不在授权 scope）。
    OutOfScope,
    /// 该技术对该资产不适用（与 `CoverageStatus::NotApplicable` 配套的原因细化）。
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
    #[serde(default, deserialize_with = "null_as_default")]
    pub evidence_refs: Vec<EvidenceAuditId>,
    /// checked_empty / blocked / not_applicable 的理由。
    #[serde(default)]
    pub note: Option<String>,
    /// blocked / not_applicable 的结构化原因类别（T1，设计
    /// `2026-06-23-coverage-note-required.md`）。与 `note` 互补，可选；缺省 None
    /// （加性、向后兼容；旧交付物/未填均 None）。
    #[serde(default)]
    pub reason_kind: Option<ReasonKind>,
    /// 分母覆盖（设计 2026-06-05-vuln-triage-technique-matrix §5）。
    /// N：该 (资产×技术) 实际测过的可测单元数。
    #[serde(default, deserialize_with = "null_as_default")]
    pub tested_units: u32,
    /// M：分母，来自 enumeration 的可测单元清单（接口/参数/路径/服务）。
    #[serde(default, deserialize_with = "null_as_default")]
    pub total_units: u32,
    /// 抽样时必填的理由；None 时按全覆盖（tested==total）要求（D6）。
    #[serde(default)]
    pub sampling_rationale: Option<String>,
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
    /// Server-assigned (2026-06-16): the `submit_stage_deliverable` tool overwrites
    /// this with a fresh random UUID and no longer asks the model for it. The field
    /// is only logged + non-nil-checked (`schema_check`), never used for evidence
    /// binding (that keys off `evidence_ids`), so generating it server-side removes a
    /// weak model's ability to emit fabricated/patterned ids. `#[serde(default)]`
    /// lets the model omit it; a non-tool parse that omits it stays nil and
    /// `schema_check` still blocks (the nil guard is preserved as a safety net).
    #[serde(default)]
    pub stage_run_id: Uuid,
    #[serde(default, deserialize_with = "null_as_default")]
    pub claims: Vec<StageClaim>,
    #[serde(default, deserialize_with = "null_as_default")]
    pub evidence_refs: Vec<EvidenceAuditId>,
    #[serde(default, deserialize_with = "null_as_default")]
    pub skipped_checks: Vec<SkippedCheckRecord>,
    #[serde(default, deserialize_with = "null_as_default")]
    pub findings: Vec<HarnessFinding>,
    /// **app-level hint**; gate 以 spec 侧字段为准（min_invocations 等经 gate_rules
    /// 的 named_check 强制），不信此 agent 可清空的字段。min_invocations_check 读它做
    /// MVP 近似匹配（见该 check）。
    #[serde(default, deserialize_with = "null_as_default")]
    pub required_checks_done: Vec<String>,
    /// Coverage matrix（设计 2026-06-05）：(资产 × 技术) → 终态 + 证据/理由。
    /// 缺省空 = 不声明覆盖（向后兼容）。`coverage_complete` gate op 据此核完整性。
    #[serde(default, deserialize_with = "null_as_default")]
    pub coverage: Vec<CoverageCell>,
    /// 攻击假设清单（设计 2026-07-02）：`attack_candidate` 阶段产物；`verification`
    /// 阶段回填每条 candidate 的终态处置。缺省空 = 向后兼容（旧交付物 / 非
    /// candidate 相关阶段），保证旧 JSON 与旧 `StageDeliverable {..}` 字面量以外的
    /// 解析路径不破（I10）。
    #[serde(default, deserialize_with = "null_as_default")]
    pub candidates: Vec<AttackCandidate>,
    /// Candidate V2 decisions over the exact server-seeded manifest. Unlike the
    /// legacy `candidates` field, these drafts carry no trusted identity or
    /// executable plan authority.
    #[serde(default, deserialize_with = "null_as_default")]
    pub candidate_decisions: Vec<CandidateDecisionDraft>,
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
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct CoverageGapAction {
    pub asset: String,
    pub technique: String,
    pub reason: String,
    #[serde(default, skip_serializing_if = "Vec::is_empty")]
    pub suggested_capabilities: Vec<StageCapabilitySuggestion>,
    #[serde(default, skip_serializing_if = "Vec::is_empty")]
    pub suggested_tools: Vec<String>,
}

/// Doc 3 §8 GateResult 配套 · recovery_actions 喂回 refiner 用.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize, Default)]
pub struct HarnessRecoveryActions {
    /// 自然语言 hint, refiner 拼到 system prompt.
    pub hints: Vec<String>,
    /// 期望补跑的 tool calls.
    pub repair_tool_calls: Vec<String>,
    /// 期望补 evidence 的 kind (与 evidence_kinds.json 一致).
    pub missing_evidence_kinds: Vec<String>,
    /// Machine-readable coverage gaps for targeted repair. Kept in recovery so
    /// existing gates that only return textual reasons remain unchanged.
    #[serde(default, skip_serializing_if = "Vec::is_empty")]
    pub coverage_gap_actions: Vec<CoverageGapAction>,
}

impl HarnessRecoveryActions {
    pub fn is_empty(&self) -> bool {
        self.hints.is_empty()
            && self.repair_tool_calls.is_empty()
            && self.missing_evidence_kinds.is_empty()
            && self.coverage_gap_actions.is_empty()
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
    fn attack_candidate_stage_kind_roundtrip() {
        let k = StageKind::AttackCandidate;
        assert_eq!(k.as_str(), "attack_candidate");
        assert_eq!(StageKind::try_parse("attack_candidate"), Some(k));
        let s = serde_json::to_string(&k).unwrap();
        assert_eq!(s, "\"attack_candidate\"");
    }

    #[test]
    fn attack_candidate_serde_defaults_and_roundtrip() {
        // 最小 JSON（只给必填字段）应解析，可选字段走 default。
        let c: AttackCandidate = serde_json::from_str(
            r#"{"candidate_id":"3f8a1c2e-1d4b-4e6a-9b2c-7a1e5f0c9d33",
                "target":"api.example.com","hypothesis":"IDOR on /orders/{id}",
                "rationale":"sequential ids observed"}"#,
        )
        .expect("minimal candidate parses");
        assert_eq!(c.priority, CandidatePriority::Medium);
        assert_eq!(c.disposition, CandidateDisposition::Proposed);
        assert_eq!(c.wave, 0);
        assert!(c.evidence_refs.is_empty());
        assert!(c.parent_finding_id.is_none());

        // 显式 null 的可选集合字段不报错（弱模型习惯发 null）。
        let c2: AttackCandidate = serde_json::from_str(
            r#"{"candidate_id":"3f8a1c2e-1d4b-4e6a-9b2c-7a1e5f0c9d33",
                "target":"a","hypothesis":"h","rationale":"r",
                "evidence_refs":null,"prior_refs":null}"#,
        )
        .expect("null optional collections collapse to default");
        assert!(c2.evidence_refs.is_empty());
        assert!(c2.prior_refs.is_empty());
    }

    #[test]
    fn candidate_enums_serde_snake_case() {
        assert_eq!(
            serde_json::to_string(&CandidatePriority::High).unwrap(),
            "\"high\""
        );
        assert_eq!(
            serde_json::to_string(&CandidateDisposition::Verified).unwrap(),
            "\"verified\""
        );
    }

    #[test]
    fn stage_deliverable_candidates_default_and_roundtrip() {
        // 旧交付物（无 candidates 键）解析为空（向后兼容 I10）。
        let d: StageDeliverable = serde_json::from_str(
            r#"{"stage_id":"attack_candidate",
                "stage_run_id":"3f8a1c2e-1d4b-4e6a-9b2c-7a1e5f0c9d33",
                "claims":[],"evidence_refs":[],"findings":[]}"#,
        )
        .expect("old deliverable without candidates parses");
        assert!(d.candidates.is_empty());
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
            candidates: vec![],
            candidate_decisions: vec![],
        };
        let s = serde_json::to_string(&d).unwrap();
        let back: ExternalAttackSurfaceDeliverable = serde_json::from_str(&s).unwrap();
        assert_eq!(d.evidence_refs.len(), back.evidence_refs.len());
        assert_eq!(d.stage_id, back.stage_id);
    }

    #[test]
    fn coverage_cell_denominator_serde_roundtrip_and_default() {
        let c = CoverageCell {
            asset: "api.example.com".to_string(),
            technique: "WSTG-INPV-05".to_string(),
            status: CoverageStatus::CheckedEmpty,
            evidence_refs: vec![EvidenceAuditId::new(1)],
            note: Some("scanned".to_string()),
            reason_kind: None,
            tested_units: 12,
            total_units: 12,
            sampling_rationale: None,
        };
        let j = serde_json::to_string(&c).unwrap();
        let back: CoverageCell = serde_json::from_str(&j).unwrap();
        assert_eq!(back.tested_units, 12);
        assert_eq!(back.total_units, 12);
        assert!(back.sampling_rationale.is_none());

        // 旧 JSON（无新字段）→ serde default（0 / 0 / None），向后兼容
        let old: CoverageCell = serde_json::from_str(
            r#"{"asset":"a","technique":"t","status":"found","evidence_refs":[1]}"#,
        )
        .unwrap();
        assert_eq!(old.tested_units, 0);
        assert_eq!(old.total_units, 0);
        assert!(old.sampling_rationale.is_none());
        // T1：reason_kind 缺省 None（旧交付物向后兼容）。
        assert!(old.reason_kind.is_none());
    }

    #[test]
    fn reason_kind_serde_snake_case_and_default() {
        // snake_case round-trip for each variant + 缺省 None。
        let c = CoverageCell {
            asset: "a".to_string(),
            technique: "GOLISH-INTEL-OSINT".to_string(),
            status: CoverageStatus::Blocked,
            evidence_refs: vec![],
            note: Some("no github token configured".to_string()),
            reason_kind: Some(ReasonKind::CredentialMissing),
            tested_units: 0,
            total_units: 0,
            sampling_rationale: None,
        };
        let j = serde_json::to_string(&c).unwrap();
        assert!(
            j.contains("\"reason_kind\":\"credential_missing\""),
            "reason_kind must serialize snake_case: {j}"
        );
        let back: CoverageCell = serde_json::from_str(&j).unwrap();
        assert_eq!(back.reason_kind, Some(ReasonKind::CredentialMissing));

        // 未填 reason_kind 的 JSON → None。
        let none: CoverageCell =
            serde_json::from_str(r#"{"asset":"a","technique":"t","status":"blocked","note":"x"}"#)
                .unwrap();
        assert!(none.reason_kind.is_none());
    }

    // Fix6 (2026-06-14): weak models emit explicit `null` for "N/A" optional
    // fields instead of omitting them. `null_as_default` must collapse those to
    // the default instead of failing `invalid type: null, expected u32/sequence`
    // (the observed scoping submit→reject loop: tested_units:null, coverage:null).
    #[test]
    fn explicit_null_on_optional_fields_collapses_to_default() {
        // CoverageCell with null numeric + null evidence_refs parses (not an error).
        let cell: CoverageCell = serde_json::from_str(
            r#"{"asset":"a","technique":"t","status":"found",
                "evidence_refs":null,"tested_units":null,"total_units":null}"#,
        )
        .expect("null optional fields must parse, not error");
        assert_eq!(cell.tested_units, 0);
        assert_eq!(cell.total_units, 0);
        assert!(cell.evidence_refs.is_empty());

        // StageDeliverable with null coverage / skipped_checks / required_checks_done.
        let d: StageDeliverable = serde_json::from_str(
            r#"{"stage_id":"scoping","stage_run_id":"3f8a1c2e-1d4b-4e6a-9b2c-7a1e5f0c9d33",
                "claims":[],"evidence_refs":[],"findings":[],
                "coverage":null,"skipped_checks":null,"required_checks_done":null}"#,
        )
        .expect("null coverage/skipped_checks/required_checks_done must parse");
        assert!(d.coverage.is_empty());
        assert!(d.skipped_checks.is_empty());
        assert!(d.required_checks_done.is_empty());
    }

    #[test]
    fn stage_claim_and_finding_old_json_without_technique_parses() {
        // P5 向后兼容：旧 JSON（无 technique 字段）必须照常解析为 None。
        let c: StageClaim = serde_json::from_str(
            r#"{"kind":"dns_a_record","subject":"example.com","summary":"A 1.2.3.4","evidence_ids":[1]}"#,
        )
        .unwrap();
        assert!(c.technique.is_none());

        let f: HarnessFinding = serde_json::from_str(
            r#"{"finding_id":"3f8a1c2e-1d4b-4e6a-9b2c-7a1e5f0c9d33","kind":"subdomain","subject":"a.example.com","severity":"info","evidence_refs":[1]}"#,
        )
        .unwrap();
        assert!(f.technique.is_none());
    }

    #[test]
    fn stage_claim_and_finding_technique_roundtrip() {
        let c = StageClaim {
            kind: "dns_a_record".to_string(),
            subject: "example.com".to_string(),
            summary: "A 1.2.3.4".to_string(),
            evidence_ids: vec![EvidenceAuditId::new(1)],
            technique: Some("GOLISH-INTEL-DNS".to_string()),
        };
        let j = serde_json::to_string(&c).unwrap();
        let back: StageClaim = serde_json::from_str(&j).unwrap();
        assert_eq!(back.technique.as_deref(), Some("GOLISH-INTEL-DNS"));

        let f = HarnessFinding {
            finding_id: Uuid::new_v4(),
            kind: "subdomain".to_string(),
            subject: "a.example.com".to_string(),
            severity: FindingSeverity::Info,
            evidence_refs: vec![EvidenceAuditId::new(1)],
            technique: Some("GOLISH-INTEL-SUBDOMAIN".to_string()),
        };
        let j = serde_json::to_string(&f).unwrap();
        let back: HarnessFinding = serde_json::from_str(&j).unwrap();
        assert_eq!(back.technique.as_deref(), Some("GOLISH-INTEL-SUBDOMAIN"));
    }

    #[test]
    fn harness_stage_hint_constructor() {
        let h = HarnessStageHint::new(StageKind::ExternalAttackSurface);
        assert_eq!(h.stage_kind, StageKind::ExternalAttackSurface);
    }
}
