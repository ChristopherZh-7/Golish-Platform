//! 数据驱动 gate 规则引擎（设计 `docs/design/2026-06-05-gate-rule-engine.md`）。
//!
//! stage JSON 用固定积木 op 声明过关标准；本模块是纯函数、DB-free、确定性解释器，
//! 输出复用 `GateCheckOutcome` / `HarnessRecoveryActions`，由 `gate/mod.rs` 并进聚合。
//! fail-closed：op / pred / over / field 全是 serde enum，写错的名字在 `StageSpec`
//! 反序列化期即报错（被 `resources` 的 `all_twelve_stage_specs_load` 单测抓住）；
//! 字段与集合不匹配（如对 claims 取 severity）在求值期返回 config-error Block。

use serde::{Deserialize, Serialize};

use super::super::stage_spec::StageSpec;
use super::super::types::{
    CoverageCell, CoverageStatus, FindingSeverity, HarnessFinding, HarnessRecoveryActions,
    StageClaim, StageDeliverable,
};
use super::{min_invocations_check, scope_check, surface_coverage_check, GateCheckOutcome};

/// 一条规则 = 一个顶层积木 op；求值产出一个 `GateCheckOutcome`。
#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(tag = "op", rename_all = "snake_case")]
pub enum GateRule {
    /// 满足 `where` 的元素至少 `min` 个。
    CountAtLeast {
        over: Collection,
        #[serde(default, rename = "where")]
        filter: Option<Pred>,
        min: u32,
        on_fail: OnFail,
    },
    /// 满足 `where` 的每个元素都必须满足 `require`（全称；空集合为真）。
    ForAll {
        over: Collection,
        #[serde(default, rename = "where")]
        filter: Option<Pred>,
        require: Pred,
        on_fail: OnFail,
    },
    /// 逃生舱（设计 2026-06-05-gate-rules-migration §5.1）：按名调用保留下来的
    /// Rust 领域 check。仅 3 个固定值（非通用扩展点）；某 check 被数据化后从枚举删除。
    /// `on_fail=Some` 时，仅当被调 check 返回 Block 才用它覆盖 reason/recovery。
    NamedCheck {
        check: NamedCheckKind,
        #[serde(default)]
        on_fail: Option<OnFail>,
    },
    /// Coverage 完整性（设计 2026-06-05-coverage-matrix §4.2）：对每个（自报）资产 ×
    /// `spec.expected_techniques` 的每类技术，矩阵里必须有一个 status ∈ `terminal_status`
    /// 的 cell；缺口（不在矩阵 / 非终态）= `not_attempted` = Block。
    /// `terminal_status=None` → 默认四种终态全算；`spec.expected_techniques` 空 → no-op Pass。
    /// 资产维度 MVP 取 deliverable.coverage 自报集合（§6.1 的 DB 注入待资产库，见 §8）。
    CoverageComplete {
        #[serde(default)]
        terminal_status: Option<Vec<CoverageStatus>>,
        /// P5（2026-06-11）：true 时，technique 标注且 subject == asset 的
        /// claim/finding 视作该 (asset × technique) 的 found 终态（自动派生）。
        /// 只补 covered 判定，不扩资产全集（D1）；absence 仍须显式 cell（D2/I8）。
        /// 缺省 false = 行为与旧版逐字节一致。
        #[serde(default)]
        derive_from_items: bool,
        /// PR3（设计 2026-06-11-coverage-auto-derive）：true 时，`ctx.evidence_facts`
        /// 中 asset+technique 精确匹配的账本事实视作该格终态——`Found` 事实 →
        /// Found 终态、`Empty` 事实 → CheckedEmpty 终态（各自受 `terminal_status`
        /// 约束；Empty **绝不**当 Found）。只补格、不造完整性（缺事实的格照旧
        /// 缺口 BLOCK）。缺省 false = 行为与旧版逐字节一致。
        #[serde(default)]
        derive_from_evidence: bool,
        /// Phase 0（设计 2026-06-12-redteam-phase0）：true 时 `found` 终态只认
        /// `ctx.evidence_facts` 的 Found 事实——自报 found cell / technique 标注的
        /// claim/finding 不再单独构成 found；`checked_empty` 收紧为「自报 + 真 Empty
        /// 事实」（I8）。`blocked`/`not_applicable` 仍自报。缺省 false = 逐字节不变。
        #[serde(default)]
        authoritative_found: bool,
        /// 仅这些 technique 收紧（None = 全部期望技术）。灰度用：落点未到位的技术
        /// （如 WHOIS/OSINT）暂不收紧，仍走旧自报。
        #[serde(default)]
        authoritative_techniques: Option<Vec<String>>,
        on_fail: OnFail,
    },
    /// P5（2026-06-11）交叉校验：每个 status == found 的 coverage cell 必须有 ≥1 个
    /// technique 匹配的 claim/finding 佐证（item.technique == cell.technique 且
    /// item.subject == cell.asset，精确相等，D5）。found 之外的终态豁免（D3）：
    /// absence 无结构化观察可佐证，由 cell 自身 evidence/note 规则把关。
    CoverageCorroborated { on_fail: OnFail },
    /// 分母覆盖（设计 2026-06-05-vuln-triage-technique-matrix §5.3）。对 status ∈
    /// {found, checked_empty} 的每个 coverage cell 核「面覆盖」：默认全覆盖（D6）要求
    /// `tested_units == total_units`；抽样例外要求 `sampling_rationale` 非空且
    /// `tested_units*100 ≥ min_sample_ratio_pct*total_units`。blocked / not_applicable
    /// 免分母；`total_units==0` 的 found/checked_empty 记缺口（应改用 not_applicable）。
    CoverageDenominator {
        #[serde(default = "default_sample_ratio_pct")]
        min_sample_ratio_pct: u8,
        on_fail: OnFail,
    },
}

/// `named_check` 逃生舱可调的内建 check（闭合枚举 → 写错名 serde 报错 fail-closed）。
#[derive(Debug, Clone, Copy, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum NamedCheckKind {
    Scope,
    SurfaceCoverage,
    MinInvocations,
}

impl NamedCheckKind {
    pub fn as_str(self) -> &'static str {
        match self {
            Self::Scope => "scope",
            Self::SurfaceCoverage => "surface_coverage",
            Self::MinInvocations => "min_invocations",
        }
    }
}

impl GateRule {
    /// agent 面向的 stage charter 用的简短描述（替代旧 `required_checks` 名字列表）。
    pub fn summary(&self) -> String {
        match self {
            GateRule::CountAtLeast { on_fail, .. }
            | GateRule::ForAll { on_fail, .. }
            | GateRule::CoverageComplete { on_fail, .. }
            | GateRule::CoverageCorroborated { on_fail }
            | GateRule::CoverageDenominator { on_fail, .. } => on_fail.reason.clone(),
            GateRule::NamedCheck { check, on_fail } => on_fail
                .as_ref()
                .map(|o| o.reason.clone())
                .unwrap_or_else(|| format!("{} check", check.as_str())),
        }
    }
}

/// MVP 只含有可寻址字段的两个集合（evidence_refs / skipped_checks 的计数已被
/// `vacuous_check` 覆盖，故不纳入 MVP）。
#[derive(Debug, Clone, Copy, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum Collection {
    Claims,
    Findings,
    /// Coverage matrix 单元格（设计 2026-06-05-coverage-matrix）。
    Coverage,
}

/// 叶子谓词，对单个元素求值。
#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(tag = "pred", rename_all = "snake_case")]
pub enum Pred {
    /// 数组 / 字符串字段非空。
    NonEmpty { field: ItemField },
    /// 字符串字段等于 `value`（severity 按 snake_case 文本比较）。
    Eq { field: ItemField, value: String },
    /// finding.severity rank ≥ min。
    SeverityAtLeast { min: FindingSeverity },
}

/// 可寻址的元素字段（按集合解析；不适用的组合在求值期 fail-closed）。
#[derive(Debug, Clone, Copy, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum ItemField {
    Kind,
    Subject,
    Summary,
    EvidenceRefs,
    EvidenceIds,
    Severity,
    /// coverage cell 字段（设计 2026-06-05-coverage-matrix）。
    Asset,
    Technique,
    Status,
}

/// 规则不满足时映射到 `GateCheckOutcome::Block` 的内容。
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct OnFail {
    pub reason: String,
    #[serde(default)]
    pub hints: Vec<String>,
    #[serde(default)]
    pub repair_tool_calls: Vec<String>,
    #[serde(default)]
    pub missing_evidence_kinds: Vec<String>,
}

/// Gate 求值上下文（Phase 2 ①③ seam · 设计 2026-06-05-vuln-triage-technique-matrix §5.5 +
/// coverage-matrix §6.5）。阶段收尾的**外层**把权威数据注入进来，gate 仍是纯函数 / DB-free：
///   - `in_scope_assets`：① in-scope 资产全集（外层从 DB / 资产库查得）。`Some` 时
///     `coverage_complete` 的资产维度用它（堵 agent 少报资产蒙混）；`None` 回退
///     `deliverable.coverage` 自报集合（现行为）。
///   - `expected_techniques`：③ 动态生成的期望技术（skeleton 按目标 / 资产产出）。`Some`
///     时覆盖 `spec.expected_techniques`；`None` 回退 spec（现行为）。
///
/// 二者缺省 `None` = 与旧 [`eval`] 逐字节一致。活体注入（DB 资产 / 动态 generator）待资产库
/// 合入 + DB §2.7（见设计 §6.5 Phase 2）；本结构是**已预埋的 seam**。
#[derive(Debug, Clone, Default)]
pub struct GateContext {
    pub in_scope_assets: Option<Vec<String>>,
    pub expected_techniques: Option<Vec<String>>,
    /// PR3 (设计 2026-06-11-coverage-auto-derive §5.2) · 证据账本投影事实：
    /// 从 `audit_log` 三列 (`evidence_asset/technique/outcome`) 注入的只读三元组。
    /// `None` = 不启用投影（与旧行为逐字节一致）；规则侧还需
    /// `coverage_complete.derive_from_evidence=true` 才消费（双开关，灰度安全）。
    pub evidence_facts: Option<Vec<EvidenceFact>>,
}

/// 一条证据投影事实：账本里「在 `asset` 上跑了 `technique`」的确定性记录。
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct EvidenceFact {
    pub asset: String,
    pub technique: String,
    pub outcome: EvidenceOutcome,
    pub evidence_id: i64,
}

/// 事实的结局：`Found`（有产出）/ `Empty`（跑了→空 — I8 的被记录事实，
/// 投影成 CheckedEmpty 终态，**绝不**当 Found）。
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum EvidenceOutcome {
    Found,
    Empty,
}

/// 逐条规则求值，每条产出一个 outcome。数据 op（count_at_least/for_all）是纯函数、
/// DB-free；`named_check` op 转发到保留的 Rust 领域 check（同样只读 deliverable(+spec
/// 配置)，无 IO）。`spec` 供 `named_check:min_invocations` 读 `spec.min_invocations`。
pub fn eval(
    deliverable: &StageDeliverable,
    spec: &StageSpec,
    rules: &[GateRule],
) -> Vec<GateCheckOutcome> {
    eval_with_context(deliverable, spec, rules, &GateContext::default())
}

/// 同 [`eval`]，但接受 [`GateContext`] 注入权威 in-scope 资产集（①）/ 动态期望技术（③）。
/// Phase 2 seam：外层（阶段收尾）查库后注入，gate 仍纯函数。
pub fn eval_with_context(
    deliverable: &StageDeliverable,
    spec: &StageSpec,
    rules: &[GateRule],
    ctx: &GateContext,
) -> Vec<GateCheckOutcome> {
    rules
        .iter()
        .map(|r| eval_one(deliverable, spec, ctx, r))
        .collect()
}

fn eval_one(
    d: &StageDeliverable,
    spec: &StageSpec,
    ctx: &GateContext,
    rule: &GateRule,
) -> GateCheckOutcome {
    match rule {
        GateRule::CountAtLeast {
            over,
            filter,
            min,
            on_fail,
        } => match count_matching(d, *over, filter.as_ref()) {
            Ok(n) if n >= *min as usize => GateCheckOutcome::Pass,
            Ok(_) => block_from(on_fail),
            Err(e) => block_config_err(e),
        },
        GateRule::ForAll {
            over,
            filter,
            require,
            on_fail,
        } => match for_all_matching(d, *over, filter.as_ref(), require) {
            Ok(true) => GateCheckOutcome::Pass,
            Ok(false) => block_from(on_fail),
            Err(e) => block_config_err(e),
        },
        GateRule::NamedCheck { check, on_fail } => {
            let base = match check {
                NamedCheckKind::Scope => scope_check::run(d),
                NamedCheckKind::SurfaceCoverage => surface_coverage_check::run(d),
                NamedCheckKind::MinInvocations => min_invocations_check::run(d, spec),
            };
            match (&base, on_fail) {
                // 仅在该 check 真的 Block 且作者提供了 on_fail 时覆盖其 reason/recovery；
                // 否则原样沿用 check 自身的结论与 recovery。
                (GateCheckOutcome::Block { .. }, Some(of)) => block_from(of),
                _ => base,
            }
        }
        GateRule::CoverageComplete {
            terminal_status,
            derive_from_items,
            derive_from_evidence,
            authoritative_found,
            authoritative_techniques,
            on_fail,
        } => coverage_complete(
            d,
            spec,
            ctx,
            terminal_status.as_deref(),
            *derive_from_items,
            *derive_from_evidence,
            *authoritative_found,
            authoritative_techniques.as_deref(),
            on_fail,
        ),
        GateRule::CoverageCorroborated { on_fail } => coverage_corroborated(d, on_fail),
        GateRule::CoverageDenominator {
            min_sample_ratio_pct,
            on_fail,
        } => coverage_denominator(d, *min_sample_ratio_pct, on_fail),
    }
}

/// 所有 coverage 终态（`terminal_status` 缺省时用）。
const ALL_TERMINAL: [CoverageStatus; 4] = [
    CoverageStatus::Found,
    CoverageStatus::CheckedEmpty,
    CoverageStatus::Blocked,
    CoverageStatus::NotApplicable,
];

/// `coverage_complete` 求值（纯函数，设计 §4.2 + Phase 2 ①③ seam）。期望技术取
/// `ctx.expected_techniques`（③ 动态注入）否则 `spec.expected_techniques`（静态），空 →
/// no-op Pass。资产维度取 `ctx.in_scope_assets`（① 权威注入）否则 deliverable.coverage
/// 自报 distinct asset。对资产 × 期望技术逐格核终态，缺口聚合进 Block reason（前 N 个）。
#[allow(clippy::too_many_arguments)]
fn coverage_complete(
    d: &StageDeliverable,
    spec: &StageSpec,
    ctx: &GateContext,
    terminal_status: Option<&[CoverageStatus]>,
    derive_from_items: bool,
    derive_from_evidence: bool,
    authoritative_found: bool,
    authoritative_techniques: Option<&[String]>,
    on_fail: &OnFail,
) -> GateCheckOutcome {
    // ③ seam：动态注入的期望技术覆盖 spec 静态值。
    let techniques: &[String] = match ctx.expected_techniques.as_deref() {
        Some(t) => t,
        None => &spec.expected_techniques,
    };
    if techniques.is_empty() {
        return GateCheckOutcome::Pass; // no-op：未声明期望技术
    }
    let terminal = terminal_status.unwrap_or(&ALL_TERMINAL);

    // ① seam：资产维度取 ctx 注入的 in-scope 资产全集（权威，堵少报蒙混）；None 回退
    // deliverable.coverage 自报 distinct asset（first-seen 顺序保证确定性输出 = 现行为）。
    let assets: Vec<&str> = match ctx.in_scope_assets.as_deref() {
        Some(list) => list.iter().map(String::as_str).collect(),
        None => {
            let mut self_reported: Vec<&str> = Vec::new();
            for cell in &d.coverage {
                if !self_reported.contains(&cell.asset.as_str()) {
                    self_reported.push(cell.asset.as_str());
                }
            }
            self_reported
        }
    };

    // P0 (2026-06-11 coverage-empty-bypass): we already returned Pass above when
    // `techniques` is empty, so reaching here means the stage DECLARES expected
    // techniques. An empty asset set at this point means no authoritative
    // in-scope assets were injected AND the deliverable self-reported no coverage
    // at all — i.e. the agent omitted the matrix entirely. Treating that as a
    // vacuous Pass conflates "omitted" with "checked-empty" (AGENTS.md I8) and
    // lets the whole coverage gate be skipped. Block instead: a coverage-bearing
    // stage must submit at least one (asset × technique) terminal cell.
    if assets.is_empty() {
        return GateCheckOutcome::Block {
            reasons: vec![format!(
                "{}: coverage matrix is empty but the stage declares {} expected technique(s) — submit a per-asset terminal cell (found/checked_empty/blocked/not_applicable) for each, omission is not checked-empty (I8)",
                on_fail.reason,
                techniques.len()
            )],
            recovery: HarnessRecoveryActions {
                hints: on_fail.hints.clone(),
                repair_tool_calls: on_fail.repair_tool_calls.clone(),
                missing_evidence_kinds: on_fail.missing_evidence_kinds.clone(),
            },
        };
    }

    let mut gaps: Vec<String> = Vec::new();
    for asset in &assets {
        for tech in techniques {
            // Phase 0（设计 2026-06-12-redteam-phase0）：该 technique 是否进入「权威」
            // 模式（found 只认真值）。authoritative_techniques=None → 全部期望技术；
            // Some → 仅清单内技术收紧，其余仍走旧自报（灰度）。
            let authoritative = authoritative_found
                && authoritative_techniques.is_none_or(|list| list.iter().any(|t| t == tech));

            let cell_status = |want: CoverageStatus| {
                d.coverage
                    .iter()
                    .any(|c| c.asset == *asset && c.technique == *tech && c.status == want)
            };
            // 账本/DB 真值通道：asset+technique 精确匹配且 outcome 命中的事实存在？
            let has_fact = |want: EvidenceOutcome| {
                ctx.evidence_facts.as_deref().is_some_and(|facts| {
                    facts
                        .iter()
                        .any(|f| f.asset == *asset && f.technique == *tech && f.outcome == want)
                })
            };
            // P5 派生（D1/D2）：technique 标注且 subject == asset 的 claim/finding 视作
            // found（仅旧自报路径用；authoritative 模式下不再算 found）。
            let tagged_found = d
                .claims
                .iter()
                .any(|c| c.subject == *asset && c.technique.as_deref() == Some(tech.as_str()))
                || d.findings
                    .iter()
                    .any(|f| f.subject == *asset && f.technique.as_deref() == Some(tech.as_str()));

            // found 终态：
            // - authoritative：只认真值（账本/DB 的 Found 事实）。自报 cell / tagged
            //   claim 不再单独构成 found（堵 live run 实证的「dig 输出冒充 whois」）。
            // - 旧路径：自报 found cell || derive_from_items 的 tagged || derive_from_evidence 真值。
            let found_ok = terminal.contains(&CoverageStatus::Found)
                && if authoritative {
                    has_fact(EvidenceOutcome::Found)
                } else {
                    cell_status(CoverageStatus::Found)
                        || (derive_from_items && tagged_found)
                        || (derive_from_evidence && has_fact(EvidenceOutcome::Found))
                };
            // checked_empty 终态（I8）：
            // - authoritative：自报 + 真 Empty 账本事实双要（「跑了→空」必须被记录）。
            // - 旧路径：自报 cell || derive_from_evidence 的 Empty 事实。
            let empty_ok = terminal.contains(&CoverageStatus::CheckedEmpty)
                && if authoritative {
                    cell_status(CoverageStatus::CheckedEmpty) && has_fact(EvidenceOutcome::Empty)
                } else {
                    cell_status(CoverageStatus::CheckedEmpty)
                        || (derive_from_evidence && has_fact(EvidenceOutcome::Empty))
                };
            // blocked / not_applicable：自报 + note 的判断态，两路径一致（不收紧）。
            let other_ok = (terminal.contains(&CoverageStatus::Blocked)
                && cell_status(CoverageStatus::Blocked))
                || (terminal.contains(&CoverageStatus::NotApplicable)
                    && cell_status(CoverageStatus::NotApplicable));

            if !found_ok && !empty_ok && !other_ok {
                gaps.push(format!("({asset} × {tech})"));
            }
        }
    }

    if gaps.is_empty() {
        return GateCheckOutcome::Pass;
    }
    const MAX_SHOWN: usize = 8;
    let shown = gaps
        .iter()
        .take(MAX_SHOWN)
        .cloned()
        .collect::<Vec<_>>()
        .join(", ");
    let suffix = if gaps.len() > MAX_SHOWN {
        format!(" (+{} more)", gaps.len() - MAX_SHOWN)
    } else {
        String::new()
    };
    GateCheckOutcome::Block {
        reasons: vec![format!(
            "{}: never attempted {}{}",
            on_fail.reason, shown, suffix
        )],
        recovery: HarnessRecoveryActions {
            hints: on_fail.hints.clone(),
            repair_tool_calls: on_fail.repair_tool_calls.clone(),
            missing_evidence_kinds: on_fail.missing_evidence_kinds.clone(),
        },
    }
}

/// `coverage_denominator` 的 `min_sample_ratio_pct` 缺省 = 100（D6 默认全覆盖）。
fn default_sample_ratio_pct() -> u8 {
    100
}

/// `coverage_denominator` 求值（纯函数，设计 §5.3）。对每个 status ∈
/// {found, checked_empty} 的 cell 核「面覆盖」：全覆盖（tested==total）或合法抽样
/// （有 `sampling_rationale` 且 `tested*100 ≥ min_ratio_pct*total`）才算测完；否则记缺口。
/// blocked / not_applicable 免分母；`total_units==0` 的 found/checked_empty 记缺口
/// （应改用 not_applicable）。无缺口 → Pass；否则 Block，缺口聚合进 reason（前 N 个）。
fn coverage_denominator(
    d: &StageDeliverable,
    min_ratio_pct: u8,
    on_fail: &OnFail,
) -> GateCheckOutcome {
    let mut gaps: Vec<String> = Vec::new();
    for c in &d.coverage {
        if !matches!(
            c.status,
            CoverageStatus::Found | CoverageStatus::CheckedEmpty
        ) {
            continue; // blocked / not_applicable 免分母
        }
        if c.total_units == 0 {
            gaps.push(format!(
                "({} × {}): total_units=0 with status {}; use not_applicable if no testable units",
                c.asset,
                c.technique,
                status_to_str(c.status)
            ));
            continue;
        }
        let full = c.tested_units >= c.total_units;
        let sampled_ok = c
            .sampling_rationale
            .as_ref()
            .map(|r| !r.trim().is_empty())
            .unwrap_or(false)
            && (c.tested_units as u64) * 100 >= (min_ratio_pct as u64) * (c.total_units as u64);
        if !full && !sampled_ok {
            gaps.push(format!(
                "({} × {}): tested {}/{} without sampling_rationale (partial — default is full coverage)",
                c.asset, c.technique, c.tested_units, c.total_units
            ));
        }
    }

    if gaps.is_empty() {
        return GateCheckOutcome::Pass;
    }
    // 缺口聚合进 reason（与 coverage_complete 同款：前 N 个 + "(+k more)"）。
    const MAX_SHOWN: usize = 8;
    let shown = gaps
        .iter()
        .take(MAX_SHOWN)
        .cloned()
        .collect::<Vec<_>>()
        .join(", ");
    let suffix = if gaps.len() > MAX_SHOWN {
        format!(" (+{} more)", gaps.len() - MAX_SHOWN)
    } else {
        String::new()
    };
    GateCheckOutcome::Block {
        reasons: vec![format!("{}: {}{}", on_fail.reason, shown, suffix)],
        recovery: HarnessRecoveryActions {
            hints: on_fail.hints.clone(),
            repair_tool_calls: on_fail.repair_tool_calls.clone(),
            missing_evidence_kinds: on_fail.missing_evidence_kinds.clone(),
        },
    }
}

/// `coverage_corroborated` 求值（纯函数，P5 设计 D3/D5）。每个 status == found 的
/// coverage cell 必须有 ≥1 个 technique 匹配的 claim/finding 佐证（item.technique ==
/// cell.technique 且 item.subject == cell.asset，精确相等）。其余终态豁免（absence
/// 无结构化观察可佐证，由 cell 自身 evidence/note 规则把关）。
fn coverage_corroborated(d: &StageDeliverable, on_fail: &OnFail) -> GateCheckOutcome {
    let mut gaps: Vec<String> = Vec::new();
    for cell in &d.coverage {
        if cell.status != CoverageStatus::Found {
            continue;
        }
        let corroborated = d.claims.iter().any(|c| {
            c.subject == cell.asset && c.technique.as_deref() == Some(cell.technique.as_str())
        }) || d.findings.iter().any(|f| {
            f.subject == cell.asset && f.technique.as_deref() == Some(cell.technique.as_str())
        });
        if !corroborated {
            gaps.push(format!("({} × {})", cell.asset, cell.technique));
        }
    }
    if gaps.is_empty() {
        return GateCheckOutcome::Pass;
    }
    const MAX_SHOWN: usize = 8;
    let shown = gaps
        .iter()
        .take(MAX_SHOWN)
        .cloned()
        .collect::<Vec<_>>()
        .join(", ");
    let suffix = if gaps.len() > MAX_SHOWN {
        format!(" (+{} more)", gaps.len() - MAX_SHOWN)
    } else {
        String::new()
    };
    GateCheckOutcome::Block {
        reasons: vec![format!(
            "{}: no technique-tagged claim/finding backs {}{}",
            on_fail.reason, shown, suffix
        )],
        recovery: HarnessRecoveryActions {
            hints: on_fail.hints.clone(),
            repair_tool_calls: on_fail.repair_tool_calls.clone(),
            missing_evidence_kinds: on_fail.missing_evidence_kinds.clone(),
        },
    }
}

enum ItemRef<'a> {
    Claim(&'a StageClaim),
    Finding(&'a HarnessFinding),
    Coverage(&'a CoverageCell),
}

impl ItemRef<'_> {
    fn kind_name(&self) -> &'static str {
        match self {
            ItemRef::Claim(_) => "claim",
            ItemRef::Finding(_) => "finding",
            ItemRef::Coverage(_) => "coverage cell",
        }
    }
}

enum FieldVal<'a> {
    Text(&'a str),
    List(usize),
    Sev(FindingSeverity),
    Status(CoverageStatus),
}

fn items<'a>(d: &'a StageDeliverable, c: Collection) -> Vec<ItemRef<'a>> {
    match c {
        Collection::Claims => d.claims.iter().map(ItemRef::Claim).collect(),
        Collection::Findings => d.findings.iter().map(ItemRef::Finding).collect(),
        Collection::Coverage => d.coverage.iter().map(ItemRef::Coverage).collect(),
    }
}

fn count_matching(
    d: &StageDeliverable,
    c: Collection,
    filter: Option<&Pred>,
) -> Result<usize, String> {
    let mut n = 0;
    for it in &items(d, c) {
        let keep = match filter {
            Some(p) => pred_holds(it, p)?,
            None => true,
        };
        if keep {
            n += 1;
        }
    }
    Ok(n)
}

fn for_all_matching(
    d: &StageDeliverable,
    c: Collection,
    filter: Option<&Pred>,
    require: &Pred,
) -> Result<bool, String> {
    for it in &items(d, c) {
        let in_scope = match filter {
            Some(p) => pred_holds(it, p)?,
            None => true,
        };
        if in_scope && !pred_holds(it, require)? {
            return Ok(false);
        }
    }
    Ok(true)
}

fn pred_holds(item: &ItemRef, p: &Pred) -> Result<bool, String> {
    match p {
        Pred::NonEmpty { field } => match resolve(item, *field)? {
            FieldVal::Text(s) => Ok(!s.trim().is_empty()),
            FieldVal::List(len) => Ok(len > 0),
            FieldVal::Sev(_) => Err("non_empty not applicable to severity field".to_string()),
            FieldVal::Status(_) => Err("non_empty not applicable to status field".to_string()),
        },
        Pred::Eq { field, value } => match resolve(item, *field)? {
            FieldVal::Text(s) => Ok(s == value),
            FieldVal::Sev(sev) => Ok(sev_to_str(sev) == value),
            FieldVal::Status(st) => Ok(status_to_str(st) == value),
            FieldVal::List(_) => Err(format!("eq not applicable to list field {field:?}")),
        },
        Pred::SeverityAtLeast { min } => match resolve(item, ItemField::Severity)? {
            FieldVal::Sev(sev) => Ok(sev.rank() >= min.rank()),
            _ => Err("severity_at_least requires a severity field".to_string()),
        },
    }
}

fn resolve<'a>(item: &ItemRef<'a>, f: ItemField) -> Result<FieldVal<'a>, String> {
    match (item, f) {
        (ItemRef::Claim(c), ItemField::Kind) => Ok(FieldVal::Text(&c.kind)),
        (ItemRef::Claim(c), ItemField::Subject) => Ok(FieldVal::Text(&c.subject)),
        (ItemRef::Claim(c), ItemField::Summary) => Ok(FieldVal::Text(&c.summary)),
        (ItemRef::Claim(c), ItemField::EvidenceIds) => Ok(FieldVal::List(c.evidence_ids.len())),
        (ItemRef::Finding(f), ItemField::Kind) => Ok(FieldVal::Text(&f.kind)),
        (ItemRef::Finding(f), ItemField::Subject) => Ok(FieldVal::Text(&f.subject)),
        (ItemRef::Finding(f), ItemField::EvidenceRefs) => Ok(FieldVal::List(f.evidence_refs.len())),
        (ItemRef::Finding(f), ItemField::Severity) => Ok(FieldVal::Sev(f.severity)),
        // P5：technique 可寻址到 claims/findings（None → "" 使 non_empty = "已标注"）。
        (ItemRef::Claim(c), ItemField::Technique) => {
            Ok(FieldVal::Text(c.technique.as_deref().unwrap_or("")))
        }
        (ItemRef::Finding(f), ItemField::Technique) => {
            Ok(FieldVal::Text(f.technique.as_deref().unwrap_or("")))
        }
        (ItemRef::Coverage(c), ItemField::Asset) => Ok(FieldVal::Text(&c.asset)),
        (ItemRef::Coverage(c), ItemField::Technique) => Ok(FieldVal::Text(&c.technique)),
        (ItemRef::Coverage(c), ItemField::Status) => Ok(FieldVal::Status(c.status)),
        (ItemRef::Coverage(c), ItemField::EvidenceRefs) => {
            Ok(FieldVal::List(c.evidence_refs.len()))
        }
        (item, field) => Err(format!(
            "field {:?} not valid for {} item",
            field,
            item.kind_name()
        )),
    }
}

fn sev_to_str(s: FindingSeverity) -> &'static str {
    match s {
        FindingSeverity::Info => "info",
        FindingSeverity::Low => "low",
        FindingSeverity::Medium => "medium",
        FindingSeverity::High => "high",
        FindingSeverity::Critical => "critical",
    }
}

fn status_to_str(s: CoverageStatus) -> &'static str {
    match s {
        CoverageStatus::Found => "found",
        CoverageStatus::CheckedEmpty => "checked_empty",
        CoverageStatus::Blocked => "blocked",
        CoverageStatus::NotApplicable => "not_applicable",
    }
}

fn block_from(on_fail: &OnFail) -> GateCheckOutcome {
    GateCheckOutcome::Block {
        reasons: vec![on_fail.reason.clone()],
        recovery: HarnessRecoveryActions {
            hints: on_fail.hints.clone(),
            repair_tool_calls: on_fail.repair_tool_calls.clone(),
            missing_evidence_kinds: on_fail.missing_evidence_kinds.clone(),
        },
    }
}

fn block_config_err(msg: String) -> GateCheckOutcome {
    GateCheckOutcome::Block {
        reasons: vec![format!("gate_rule config error: {msg}")],
        recovery: HarnessRecoveryActions::default(),
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use golish_pentest::evidence_ledger::EvidenceAuditId;
    use uuid::Uuid;

    #[test]
    fn parses_count_at_least_rule() {
        let json = r#"{
            "op": "count_at_least",
            "over": "findings",
            "where": { "pred": "eq", "field": "kind", "value": "subdomain" },
            "min": 1,
            "on_fail": { "reason": "need a subdomain" }
        }"#;
        let rule: GateRule = serde_json::from_str(json).expect("parse");
        assert!(matches!(rule, GateRule::CountAtLeast { min: 1, .. }));
    }

    #[test]
    fn parses_for_all_rule() {
        let json = r#"{
            "op": "for_all",
            "over": "findings",
            "where": { "pred": "severity_at_least", "min": "high" },
            "require": { "pred": "non_empty", "field": "evidence_refs" },
            "on_fail": { "reason": "high+ finding needs evidence" }
        }"#;
        let rule: GateRule = serde_json::from_str(json).expect("parse");
        assert!(matches!(rule, GateRule::ForAll { .. }));
    }

    #[test]
    fn unknown_op_fails_closed() {
        let json = r#"{ "op": "coverage_matrix", "over": "findings", "min": 1,
                        "on_fail": { "reason": "x" } }"#;
        assert!(serde_json::from_str::<GateRule>(json).is_err());
    }

    #[test]
    fn unknown_pred_fails_closed() {
        let json = r#"{ "op": "for_all", "over": "findings",
            "require": { "pred": "magic", "field": "kind" },
            "on_fail": { "reason": "x" } }"#;
        assert!(serde_json::from_str::<GateRule>(json).is_err());
    }

    fn finding(kind: &str, sev: FindingSeverity, refs: Vec<i64>) -> HarnessFinding {
        HarnessFinding {
            finding_id: Uuid::new_v4(),
            kind: kind.to_string(),
            subject: "x.example.com".to_string(),
            severity: sev,
            evidence_refs: refs.into_iter().map(EvidenceAuditId::new).collect(),
            technique: None,
        }
    }

    fn deliverable(findings: Vec<HarnessFinding>, claims: Vec<StageClaim>) -> StageDeliverable {
        StageDeliverable {
            stage_id: "verification".to_string(),
            stage_run_id: Uuid::new_v4(),
            claims,
            evidence_refs: vec![],
            skipped_checks: vec![],
            findings,
            required_checks_done: vec![],
            coverage: vec![],
        }
    }

    fn parse(json: &str) -> GateRule {
        serde_json::from_str(json).expect("parse")
    }

    /// Minimal spec for data-op tests (they ignore `spec`); named_check tests
    /// that need `spec.min_invocations` build their own.
    fn test_spec() -> StageSpec {
        crate::harness::stage_spec::load_stage_spec_from_json(
            r#"{"id":"verification","kind":"verification","risk_level":"critical",
                "deliverable_schema":"StageDeliverable","gate_validator":"validate_stage_gate"}"#,
        )
        .expect("minimal spec parses")
    }

    #[test]
    fn count_at_least_passes_when_enough() {
        let rule = parse(
            r#"{ "op":"count_at_least","over":"findings",
                 "where":{"pred":"eq","field":"kind","value":"subdomain"},
                 "min":1,"on_fail":{"reason":"need subdomain"} }"#,
        );
        let d = deliverable(
            vec![finding("subdomain", FindingSeverity::Info, vec![1])],
            vec![],
        );
        assert!(eval(&d, &test_spec(), &[rule])[0].is_pass());
    }

    #[test]
    fn count_at_least_blocks_when_short() {
        let rule = parse(
            r#"{ "op":"count_at_least","over":"findings",
                 "where":{"pred":"eq","field":"kind","value":"subdomain"},
                 "min":1,"on_fail":{"reason":"need subdomain"} }"#,
        );
        let d = deliverable(
            vec![finding("http_service", FindingSeverity::Info, vec![1])],
            vec![],
        );
        match &eval(&d, &test_spec(), &[rule])[0] {
            GateCheckOutcome::Block { reasons, .. } => assert_eq!(reasons[0], "need subdomain"),
            GateCheckOutcome::Pass => panic!("expected Block"),
        }
    }

    #[test]
    fn for_all_high_findings_need_evidence() {
        let rule = parse(
            r#"{ "op":"for_all","over":"findings",
                 "where":{"pred":"severity_at_least","min":"high"},
                 "require":{"pred":"non_empty","field":"evidence_refs"},
                 "on_fail":{"reason":"high+ needs evidence","missing_evidence_kinds":["poc"]} }"#,
        );
        // critical without evidence -> Block; low without evidence -> ignored.
        let blocked = deliverable(
            vec![finding("rce", FindingSeverity::Critical, vec![])],
            vec![],
        );
        match &eval(&blocked, &test_spec(), std::slice::from_ref(&rule))[0] {
            GateCheckOutcome::Block { recovery, .. } => {
                assert!(recovery.missing_evidence_kinds.contains(&"poc".to_string()))
            }
            GateCheckOutcome::Pass => panic!("expected Block"),
        }
        let ok = deliverable(vec![finding("info", FindingSeverity::Low, vec![])], vec![]);
        assert!(eval(&ok, &test_spec(), &[rule])[0].is_pass());
    }

    #[test]
    fn for_all_empty_collection_is_vacuously_true() {
        let rule = parse(
            r#"{ "op":"for_all","over":"findings",
                 "require":{"pred":"non_empty","field":"evidence_refs"},
                 "on_fail":{"reason":"x"} }"#,
        );
        assert!(eval(&deliverable(vec![], vec![]), &test_spec(), &[rule])[0].is_pass());
    }

    #[test]
    fn field_not_valid_for_collection_blocks_as_config_error() {
        // severity_at_least over claims -> claims have no severity -> config error Block.
        let rule = parse(
            r#"{ "op":"count_at_least","over":"claims",
                 "where":{"pred":"severity_at_least","min":"high"},
                 "min":1,"on_fail":{"reason":"x"} }"#,
        );
        let d = deliverable(
            vec![],
            vec![StageClaim {
                kind: "k".into(),
                subject: "s".into(),
                summary: "sm".into(),
                evidence_ids: vec![EvidenceAuditId::new(1)],
                technique: None,
            }],
        );
        match &eval(&d, &test_spec(), &[rule])[0] {
            GateCheckOutcome::Block { reasons, .. } => assert!(reasons[0].contains("config error")),
            GateCheckOutcome::Pass => panic!("expected config-error Block"),
        }
    }

    // ── named_check 逃生舱（设计 2026-06-05-gate-rules-migration） ─────────────

    #[test]
    fn parses_named_check_rule() {
        let rule: GateRule =
            serde_json::from_str(r#"{ "op":"named_check","check":"surface_coverage" }"#)
                .expect("parse");
        assert!(matches!(
            rule,
            GateRule::NamedCheck {
                check: NamedCheckKind::SurfaceCoverage,
                ..
            }
        ));
    }

    #[test]
    fn unknown_named_check_fails_closed() {
        assert!(
            serde_json::from_str::<GateRule>(r#"{ "op":"named_check","check":"bogus" }"#).is_err()
        );
    }

    #[test]
    fn named_check_scope_blocks_on_claim_without_evidence() {
        // 经 named_check 转发到 scope_check：claim 缺 evidence_ids → Block。
        let rule = parse(r#"{ "op":"named_check","check":"scope" }"#);
        let d = deliverable(
            vec![],
            vec![StageClaim {
                kind: "http_service".into(),
                subject: "api.example.com".into(),
                summary: "200".into(),
                evidence_ids: vec![],
                technique: None,
            }],
        );
        assert!(!eval(&d, &test_spec(), &[rule])[0].is_pass());
    }

    #[test]
    fn named_check_surface_coverage_passes_on_surface_only() {
        // 2026-06-09 阶段重排：EAS surface_coverage 只硬要求 Surface（JsApi 移交
        // enumeration 的 coverage_complete(GOLISH-ENUM-JSAPI)），故只有 Surface 也通过。
        let rule = parse(r#"{ "op":"named_check","check":"surface_coverage" }"#);
        let d = deliverable(
            vec![finding("http_service", FindingSeverity::Info, vec![1])],
            vec![],
        );
        assert!(eval(&d, &test_spec(), &[rule])[0].is_pass());
    }

    #[test]
    fn named_check_min_invocations_uses_spec_and_blocks_when_tool_absent() {
        // 经 named_check 转发到 min_invocations_check，读 spec.min_invocations；
        // required_checks_done 为空 → Block。
        let spec = crate::harness::stage_spec::load_stage_spec_from_json(
            r#"{"id":"enumeration","kind":"enumeration","risk_level":"medium",
                "deliverable_schema":"StageDeliverable","gate_validator":"validate_stage_gate",
                "min_invocations":{"dns_resolve":1}}"#,
        )
        .unwrap();
        let rule = parse(r#"{ "op":"named_check","check":"min_invocations" }"#);
        let d = deliverable(vec![], vec![]);
        assert!(!eval(&d, &spec, &[rule])[0].is_pass());
    }

    // ── coverage matrix 数据积木（设计 2026-06-05-coverage-matrix） ────────────

    fn cov_cell(
        asset: &str,
        technique: &str,
        status: CoverageStatus,
        refs: Vec<i64>,
    ) -> CoverageCell {
        CoverageCell {
            asset: asset.to_string(),
            technique: technique.to_string(),
            status,
            evidence_refs: refs.into_iter().map(EvidenceAuditId::new).collect(),
            note: None,
            tested_units: 0,
            total_units: 0,
            sampling_rationale: None,
        }
    }

    fn deliverable_with_coverage(cells: Vec<CoverageCell>) -> StageDeliverable {
        let mut d = deliverable(vec![], vec![]);
        d.coverage = cells;
        d
    }

    #[test]
    fn coverage_found_cell_must_cite_evidence() {
        // #4：for_all over coverage where status==found require non_empty evidence_refs。
        let rule = parse(
            r#"{ "op":"for_all","over":"coverage",
                 "where":{"pred":"eq","field":"status","value":"found"},
                 "require":{"pred":"non_empty","field":"evidence_refs"},
                 "on_fail":{"reason":"found cell needs evidence"} }"#,
        );
        // found 缺证据 → Block。
        let bad = deliverable_with_coverage(vec![cov_cell(
            "api.ex.com",
            "idor",
            CoverageStatus::Found,
            vec![],
        )]);
        assert!(!eval(&bad, &test_spec(), std::slice::from_ref(&rule))[0].is_pass());
        // found 有证据 + checked_empty 缺证据（被 where 过滤掉）→ Pass。
        let ok = deliverable_with_coverage(vec![
            cov_cell("api.ex.com", "idor", CoverageStatus::Found, vec![1]),
            cov_cell("api.ex.com", "xss", CoverageStatus::CheckedEmpty, vec![]),
        ]);
        assert!(eval(&ok, &test_spec(), &[rule])[0].is_pass());
    }

    #[test]
    fn coverage_count_found_status() {
        // count_at_least over coverage where status==found min 2。
        let rule = parse(
            r#"{ "op":"count_at_least","over":"coverage",
                 "where":{"pred":"eq","field":"status","value":"found"},
                 "min":2,"on_fail":{"reason":"need >=2 found"} }"#,
        );
        let d = deliverable_with_coverage(vec![
            cov_cell("a", "idor", CoverageStatus::Found, vec![1]),
            cov_cell("a", "xss", CoverageStatus::CheckedEmpty, vec![1]),
        ]);
        // 只有 1 个 found → Block。
        assert!(!eval(&d, &test_spec(), &[rule])[0].is_pass());
    }

    // ── coverage_complete op（设计 2026-06-05-coverage-matrix §4.2） ───────────

    /// spec with the given `expected_techniques`（其余走 serde default）。
    fn spec_with_expected(techs: &[&str]) -> StageSpec {
        let arr = techs
            .iter()
            .map(|t| format!("\"{t}\""))
            .collect::<Vec<_>>()
            .join(",");
        crate::harness::stage_spec::load_stage_spec_from_json(&format!(
            r#"{{"id":"vuln_triage","kind":"vuln_triage","risk_level":"high",
                "deliverable_schema":"StageDeliverable","gate_validator":"validate_stage_gate",
                "expected_techniques":[{arr}]}}"#
        ))
        .expect("spec parses")
    }

    fn coverage_complete_rule() -> GateRule {
        parse(
            r#"{ "op":"coverage_complete",
                 "on_fail":{"reason":"coverage incomplete","hints":["fill every expected technique"]} }"#,
        )
    }

    #[test]
    fn parses_coverage_complete_rule() {
        let rule = coverage_complete_rule();
        assert!(matches!(
            rule,
            GateRule::CoverageComplete {
                terminal_status: None,
                ..
            }
        ));
    }

    #[test]
    fn coverage_complete_bad_terminal_status_fails_closed() {
        // status 写错值（不在闭合枚举）→ serde 解析报错。
        assert!(serde_json::from_str::<GateRule>(
            r#"{ "op":"coverage_complete","terminal_status":["bogus"],
                 "on_fail":{"reason":"x"} }"#
        )
        .is_err());
    }

    // ── PR3 (设计 2026-06-11-coverage-auto-derive §4): derive_from_evidence 投影 ──

    fn evidence_derive_rule(terminal: Option<&str>) -> GateRule {
        let t = terminal
            .map(|s| format!(r#""terminal_status":[{s}],"#))
            .unwrap_or_default();
        parse(&format!(
            r#"{{ "op":"coverage_complete",{t}"derive_from_evidence":true,
                 "on_fail":{{"reason":"coverage incomplete"}} }}"#
        ))
    }

    fn fact(asset: &str, technique: &str, outcome: EvidenceOutcome, id: i64) -> EvidenceFact {
        EvidenceFact {
            asset: asset.to_string(),
            technique: technique.to_string(),
            outcome,
            evidence_id: id,
        }
    }

    /// ctx: 资产轴 {a} + 期望技术 {techs} + 注入的事实。
    fn projection_ctx(techs: &[&str], facts: Option<Vec<EvidenceFact>>) -> GateContext {
        GateContext {
            in_scope_assets: Some(vec!["a".to_string()]),
            expected_techniques: Some(techs.iter().map(|s| s.to_string()).collect()),
            evidence_facts: facts,
        }
    }

    // ── Phase 0 (设计 2026-06-12-redteam-phase0): authoritative found ──────────
    // found 终态只认真值（账本/DB 的 Found 事实）；自报 cell / tagged claim 不再
    // 单独构成 found。derive_from_items + derive_from_evidence 都开，以证明
    // authoritative 覆盖（override）它们。

    fn authoritative_rule(techniques: Option<&[&str]>) -> GateRule {
        let techs = match techniques {
            Some(t) => {
                let arr = t
                    .iter()
                    .map(|s| format!("\"{s}\""))
                    .collect::<Vec<_>>()
                    .join(",");
                format!(r#","authoritative_techniques":[{arr}]"#)
            }
            None => String::new(),
        };
        parse(&format!(
            r#"{{ "op":"coverage_complete","authoritative_found":true,"derive_from_items":true,"derive_from_evidence":true{techs},
                 "on_fail":{{"reason":"coverage incomplete"}} }}"#
        ))
    }

    #[test]
    fn parses_authoritative_found_fields() {
        let rule = parse(
            r#"{ "op":"coverage_complete","authoritative_found":true,
                 "authoritative_techniques":["GOLISH-INTEL-DNS"],
                 "on_fail":{"reason":"x"} }"#,
        );
        assert!(matches!(
            rule,
            GateRule::CoverageComplete {
                authoritative_found: true,
                authoritative_techniques: Some(ref t),
                ..
            } if t == &vec!["GOLISH-INTEL-DNS".to_string()]
        ));
        // 缺省：两字段不写 → false / None（零回归）。
        assert!(matches!(
            coverage_complete_rule(),
            GateRule::CoverageComplete {
                authoritative_found: false,
                authoritative_techniques: None,
                ..
            }
        ));
    }

    #[test]
    fn authoritative_found_self_report_without_fact_blocks() {
        // 回归基线：deepseek live run 把 dig 输出自报成 WHOIS found。
        let rule = authoritative_rule(None);
        let d = deliverable_with_coverage(vec![cov_cell(
            "a",
            "GOLISH-INTEL-WHOIS",
            CoverageStatus::Found,
            vec![1],
        )]);
        let ctx = projection_ctx(&["GOLISH-INTEL-WHOIS"], None);
        assert!(
            !eval_with_context(&d, &test_spec(), &[rule], &ctx)[0].is_pass(),
            "authoritative: self-reported found without a Found fact must BLOCK"
        );
    }

    #[test]
    fn authoritative_found_with_fact_passes() {
        let rule = authoritative_rule(None);
        let d = deliverable_with_coverage(vec![cov_cell(
            "a",
            "GOLISH-INTEL-DNS",
            CoverageStatus::Found,
            vec![1],
        )]);
        let ctx = projection_ctx(
            &["GOLISH-INTEL-DNS"],
            Some(vec![fact(
                "a",
                "GOLISH-INTEL-DNS",
                EvidenceOutcome::Found,
                7,
            )]),
        );
        assert!(eval_with_context(&d, &test_spec(), &[rule], &ctx)[0].is_pass());
    }

    #[test]
    fn authoritative_found_tagged_claim_without_fact_blocks() {
        // technique 标注的 claim 在旧 derive_from_items 下会放行；authoritative 下不行。
        let rule = authoritative_rule(None);
        let mut d = deliverable_with_coverage(vec![]);
        d.claims.push(StageClaim {
            kind: "whois_data_observed".to_string(),
            subject: "a".to_string(),
            summary: "fabricated whois".to_string(),
            evidence_ids: vec![],
            technique: Some("GOLISH-INTEL-WHOIS".to_string()),
        });
        let ctx = projection_ctx(&["GOLISH-INTEL-WHOIS"], None);
        assert!(
            !eval_with_context(&d, &test_spec(), &[rule], &ctx)[0].is_pass(),
            "authoritative: a technique-tagged claim is no longer a found source"
        );
    }

    #[test]
    fn authoritative_techniques_scopes_tightening() {
        // 灰度：只 DNS 收紧；WHOIS 不在清单 → 仍走旧自报，自报 found 算终态。
        let rule = authoritative_rule(Some(&["GOLISH-INTEL-DNS"]));
        let d = deliverable_with_coverage(vec![cov_cell(
            "a",
            "GOLISH-INTEL-WHOIS",
            CoverageStatus::Found,
            vec![1],
        )]);
        let ctx = projection_ctx(&["GOLISH-INTEL-WHOIS"], None);
        assert!(
            eval_with_context(&d, &test_spec(), &[rule], &ctx)[0].is_pass(),
            "technique outside authoritative_techniques keeps legacy self-report"
        );
    }

    #[test]
    fn authoritative_checked_empty_requires_empty_fact() {
        let d = deliverable_with_coverage(vec![cov_cell(
            "a",
            "GOLISH-INTEL-WHOIS",
            CoverageStatus::CheckedEmpty,
            vec![1],
        )]);
        // 无 Empty 事实 → BLOCK（I8：checked_empty 必须有「跑了→空」记录）。
        let ctx_none = projection_ctx(&["GOLISH-INTEL-WHOIS"], None);
        assert!(
            !eval_with_context(&d, &test_spec(), &[authoritative_rule(None)], &ctx_none)[0]
                .is_pass(),
            "authoritative checked_empty without an Empty fact must BLOCK (I8)"
        );
        // 有 Empty 事实 → PASS。
        let ctx_empty = projection_ctx(
            &["GOLISH-INTEL-WHOIS"],
            Some(vec![fact(
                "a",
                "GOLISH-INTEL-WHOIS",
                EvidenceOutcome::Empty,
                7,
            )]),
        );
        assert!(
            eval_with_context(&d, &test_spec(), &[authoritative_rule(None)], &ctx_empty)[0]
                .is_pass(),
            "authoritative checked_empty with a real Empty fact passes"
        );
    }

    #[test]
    fn authoritative_off_keeps_legacy_self_report() {
        // 不开 authoritative_found（缺省）→ 自报 found 仍算终态（零回归）。
        let rule = parse(
            r#"{ "op":"coverage_complete","derive_from_items":true,
                 "on_fail":{"reason":"x"} }"#,
        );
        let d = deliverable_with_coverage(vec![cov_cell(
            "a",
            "GOLISH-INTEL-WHOIS",
            CoverageStatus::Found,
            vec![1],
        )]);
        let ctx = projection_ctx(&["GOLISH-INTEL-WHOIS"], None);
        assert!(eval_with_context(&d, &test_spec(), &[rule], &ctx)[0].is_pass());
    }

    // 约束1+4：账本有 (a×DNS) 的 Found 事实 → 该格视为已覆盖，空矩阵也 Pass。
    #[test]
    fn derive_from_evidence_found_fills_cell() {
        let rule = evidence_derive_rule(None);
        let d = deliverable_with_coverage(vec![]); // 模型一格未写
        let ctx = projection_ctx(
            &["GOLISH-INTEL-DNS"],
            Some(vec![fact(
                "a",
                "GOLISH-INTEL-DNS",
                EvidenceOutcome::Found,
                7,
            )]),
        );
        assert!(
            eval_with_context(&d, &test_spec(), &[rule], &ctx)[0].is_pass(),
            "ledger-proven cell must satisfy completeness without a hand-written matrix"
        );
    }

    // 约束2 (I8 红线)：Empty 事实 = CheckedEmpty 终态，绝不被当 Found——
    // terminal 收窄到 ["found"] 时 Empty 事实不算覆盖，缺口仍 BLOCK；
    // terminal 缺省（含 checked_empty）时才满足。
    #[test]
    fn empty_fact_is_checked_empty_not_found() {
        let d = deliverable_with_coverage(vec![]);
        let facts = Some(vec![fact(
            "a",
            "GOLISH-INTEL-DNS",
            EvidenceOutcome::Empty,
            7,
        )]);

        let narrowed = evidence_derive_rule(Some(r#""found""#));
        let ctx = projection_ctx(&["GOLISH-INTEL-DNS"], facts.clone());
        match &eval_with_context(&d, &test_spec(), &[narrowed], &ctx)[0] {
            GateCheckOutcome::Block { reasons, .. } => {
                assert!(reasons[0].contains("(a × GOLISH-INTEL-DNS)"), "{reasons:?}");
            }
            GateCheckOutcome::Pass => {
                panic!("an Empty fact must NEVER satisfy a found-only terminal")
            }
        }

        let default_terminal = evidence_derive_rule(None);
        assert!(
            eval_with_context(&d, &test_spec(), &[default_terminal], &ctx)[0].is_pass(),
            "Empty fact projects to CheckedEmpty, which the default terminal set accepts"
        );
    }

    // 约束2：无任何事实 → not_attempted 缺口照旧 BLOCK（缺证据 ≠ checked_empty）。
    #[test]
    fn no_fact_still_blocks() {
        let rule = evidence_derive_rule(None);
        let d = deliverable_with_coverage(vec![]);
        // 事实属于别的格 (b×DNS)，(a×DNS) 仍无事实。
        let ctx = projection_ctx(
            &["GOLISH-INTEL-DNS"],
            Some(vec![fact(
                "b",
                "GOLISH-INTEL-DNS",
                EvidenceOutcome::Found,
                7,
            )]),
        );
        match &eval_with_context(&d, &test_spec(), &[rule], &ctx)[0] {
            GateCheckOutcome::Block { reasons, .. } => {
                assert!(reasons[0].contains("(a × GOLISH-INTEL-DNS)"), "{reasons:?}");
            }
            GateCheckOutcome::Pass => {
                panic!("a cell without any fact must stay a gap (fail-closed)")
            }
        }
    }

    // 约束4：completeness 永不被投影放宽——期望两列、账本只证了一列 → 另一列 BLOCK。
    #[test]
    fn evidence_derive_does_not_fabricate_completeness() {
        let rule = evidence_derive_rule(None);
        let d = deliverable_with_coverage(vec![]);
        let ctx = projection_ctx(
            &["GOLISH-INTEL-DNS", "GOLISH-INTEL-WHOIS"],
            Some(vec![fact(
                "a",
                "GOLISH-INTEL-DNS",
                EvidenceOutcome::Found,
                7,
            )]),
        );
        match &eval_with_context(&d, &test_spec(), &[rule], &ctx)[0] {
            GateCheckOutcome::Block { reasons, .. } => {
                assert!(
                    reasons[0].contains("(a × GOLISH-INTEL-WHOIS)")
                        && !reasons[0].contains("(a × GOLISH-INTEL-DNS)"),
                    "{reasons:?}"
                );
            }
            GateCheckOutcome::Pass => {
                panic!("projection must only fill evidenced cells, never completeness")
            }
        }
    }

    // 兼容：derive_from_evidence=false（facts 注入也不消费）/ evidence_facts=None
    // （开关开了也无可投影）→ 与旧行为一致：缺口 BLOCK。
    #[test]
    fn disabled_is_byte_identical() {
        let d = deliverable_with_coverage(vec![]);
        let facts = Some(vec![fact(
            "a",
            "GOLISH-INTEL-DNS",
            EvidenceOutcome::Found,
            7,
        )]);

        // 规则未开 derive_from_evidence → 注入的 facts 被忽略。
        let off = coverage_complete_rule();
        let ctx_with_facts = projection_ctx(&["GOLISH-INTEL-DNS"], facts);
        assert!(
            !eval_with_context(&d, &test_spec(), &[off], &ctx_with_facts)[0].is_pass(),
            "rule with derive_from_evidence=false must ignore injected facts"
        );

        // 规则开了但 ctx 无 facts → 同样旧行为。
        let on = evidence_derive_rule(None);
        let ctx_no_facts = projection_ctx(&["GOLISH-INTEL-DNS"], None);
        assert!(
            !eval_with_context(&d, &test_spec(), &[on], &ctx_no_facts)[0].is_pass(),
            "derive_from_evidence with no facts must keep the legacy gap BLOCK"
        );
    }

    #[test]
    fn coverage_complete_noop_when_no_expected_techniques() {
        // expected_techniques 空（test_spec 无该字段）→ 即使有缺口也 Pass（no-op）。
        let rule = coverage_complete_rule();
        let d =
            deliverable_with_coverage(vec![cov_cell("a", "idor", CoverageStatus::Found, vec![1])]);
        assert!(eval(&d, &test_spec(), &[rule])[0].is_pass());
    }

    #[test]
    fn coverage_complete_blocks_on_missing_technique() {
        let rule = coverage_complete_rule();
        let spec = spec_with_expected(&["idor", "xss"]);
        // 资产 a 只覆盖了 idor，缺 a×xss → Block，reason 含缺失对。
        let d =
            deliverable_with_coverage(vec![cov_cell("a", "idor", CoverageStatus::Found, vec![1])]);
        match &eval(&d, &spec, &[rule])[0] {
            GateCheckOutcome::Block { reasons, .. } => {
                assert!(reasons[0].contains("(a × xss)"), "{:?}", reasons);
            }
            GateCheckOutcome::Pass => panic!("expected Block on missing technique"),
        }
    }

    #[test]
    fn coverage_complete_passes_when_all_techniques_terminal() {
        let rule = coverage_complete_rule();
        let spec = spec_with_expected(&["idor", "xss"]);
        // 两类技术都有终态（found / checked_empty）→ Pass。
        let d = deliverable_with_coverage(vec![
            cov_cell("a", "idor", CoverageStatus::Found, vec![1]),
            cov_cell("a", "xss", CoverageStatus::CheckedEmpty, vec![1]),
        ]);
        assert!(eval(&d, &spec, &[rule])[0].is_pass());
    }

    #[test]
    fn coverage_complete_terminal_status_found_only_blocks_checked_empty() {
        // terminal_status 限定 ["found"] 时，checked_empty 的格不算终态 → 缺口 Block。
        let rule = parse(
            r#"{ "op":"coverage_complete","terminal_status":["found"],
                 "on_fail":{"reason":"coverage incomplete"} }"#,
        );
        let spec = spec_with_expected(&["idor", "xss"]);
        let d = deliverable_with_coverage(vec![
            cov_cell("a", "idor", CoverageStatus::Found, vec![1]),
            cov_cell("a", "xss", CoverageStatus::CheckedEmpty, vec![1]),
        ]);
        match &eval(&d, &spec, &[rule])[0] {
            GateCheckOutcome::Block { reasons, .. } => {
                assert!(reasons[0].contains("(a × xss)"), "{:?}", reasons);
            }
            GateCheckOutcome::Pass => panic!("found-only terminal must block checked_empty cell"),
        }
    }

    #[test]
    fn coverage_complete_multi_asset_reports_per_asset_gaps() {
        let rule = coverage_complete_rule();
        let spec = spec_with_expected(&["idor"]);
        // 两个资产，b 缺 idor → 仅 b×idor 缺口。
        let d = deliverable_with_coverage(vec![
            cov_cell("a", "idor", CoverageStatus::Found, vec![1]),
            cov_cell("b", "xss", CoverageStatus::CheckedEmpty, vec![1]),
        ]);
        match &eval(&d, &spec, &[rule])[0] {
            GateCheckOutcome::Block { reasons, .. } => {
                assert!(reasons[0].contains("(b × idor)"), "{:?}", reasons);
                assert!(!reasons[0].contains("(a × idor)"), "{:?}", reasons);
            }
            GateCheckOutcome::Pass => panic!("expected Block for b×idor gap"),
        }
    }

    #[test]
    fn coverage_complete_blocks_empty_coverage_when_techniques_expected() {
        // P0 (2026-06-11 coverage-empty-bypass): a coverage-bearing stage
        // (expected_techniques non-empty) that submits an EMPTY coverage matrix
        // must BLOCK. Previously, with no injected in_scope_assets, the
        // self-reported asset set was empty → the asset×technique loop ran zero
        // times → vacuous Pass, letting an agent skip the matrix entirely. That
        // conflates "omitted" with "checked-empty" (AGENTS.md I8). Real runs
        // showed 8/9 target_intel deliverables submitting empty coverage and
        // passing — this test pins the corrected behavior.
        let rule = coverage_complete_rule();
        let spec = spec_with_expected(&["idor", "xss"]);
        let d = deliverable_with_coverage(vec![]);
        match &eval(&d, &spec, &[rule])[0] {
            GateCheckOutcome::Block { reasons, .. } => {
                assert!(
                    reasons[0].contains("coverage matrix is empty"),
                    "{:?}",
                    reasons
                );
            }
            GateCheckOutcome::Pass => {
                panic!("empty coverage on a stage that declares expected_techniques must BLOCK")
            }
        }
    }

    #[test]
    fn coverage_complete_empty_coverage_still_noop_without_expected_techniques() {
        // Guard the fix's blast radius: a stage with NO expected_techniques (e.g.
        // scoping) stays a no-op Pass even with empty coverage (backward compat).
        let rule = coverage_complete_rule();
        let d = deliverable_with_coverage(vec![]);
        assert!(
            eval(&d, &test_spec(), &[rule])[0].is_pass(),
            "no expected_techniques → empty coverage must remain a no-op Pass"
        );
    }

    // ── coverage_denominator op（设计 2026-06-05-vuln-triage-technique-matrix §5.3） ──

    /// coverage cell with denominator fields（分母测试用）。
    fn cov_cell_dn(
        asset: &str,
        technique: &str,
        status: CoverageStatus,
        refs: Vec<i64>,
        tested: u32,
        total: u32,
        rationale: Option<&str>,
    ) -> CoverageCell {
        CoverageCell {
            asset: asset.to_string(),
            technique: technique.to_string(),
            status,
            evidence_refs: refs.into_iter().map(EvidenceAuditId::new).collect(),
            note: None,
            tested_units: tested,
            total_units: total,
            sampling_rationale: rationale.map(str::to_string),
        }
    }

    fn denominator_rule(min_ratio: Option<u8>) -> GateRule {
        let body = match min_ratio {
            Some(r) => format!(
                r#"{{ "op":"coverage_denominator","min_sample_ratio_pct":{r},
                     "on_fail":{{"reason":"coverage below denominator"}} }}"#
            ),
            None => r#"{ "op":"coverage_denominator",
                     "on_fail":{"reason":"coverage below denominator"} }"#
                .to_string(),
        };
        parse(&body)
    }

    #[test]
    fn coverage_denominator_default_ratio_is_100() {
        match denominator_rule(None) {
            GateRule::CoverageDenominator {
                min_sample_ratio_pct,
                ..
            } => assert_eq!(min_sample_ratio_pct, 100),
            _ => panic!("expected CoverageDenominator"),
        }
    }

    #[test]
    fn denominator_blocks_partial_without_rationale() {
        // found cell tested<total 且无 rationale → Block，reason 含 tested N/M。
        let rule = denominator_rule(None);
        let d = deliverable_with_coverage(vec![cov_cell_dn(
            "a",
            "WSTG-INPV-05",
            CoverageStatus::Found,
            vec![1],
            3,
            5000,
            None,
        )]);
        match eval_one(&d, &test_spec(), &GateContext::default(), &rule) {
            GateCheckOutcome::Block { reasons, .. } => {
                assert!(reasons.iter().any(|r| r.contains("3/5000")), "{reasons:?}");
            }
            GateCheckOutcome::Pass => panic!("expected Block on partial coverage"),
        }
    }

    #[test]
    fn denominator_passes_full_and_sampled() {
        // 全覆盖：tested==total，无需 rationale。
        let full = deliverable_with_coverage(vec![cov_cell_dn(
            "a",
            "WSTG-INPV-05",
            CoverageStatus::CheckedEmpty,
            vec![1],
            5000,
            5000,
            None,
        )]);
        assert!(eval_one(
            &full,
            &test_spec(),
            &GateContext::default(),
            &denominator_rule(None)
        )
        .is_pass());

        // 抽样：80% ≥ 阈值 80 且声明了 rationale → Pass。
        let sampled = deliverable_with_coverage(vec![cov_cell_dn(
            "a",
            "WSTG-INPV-05",
            CoverageStatus::CheckedEmpty,
            vec![1],
            4000,
            5000,
            Some("long-tail low-risk endpoints sampled"),
        )]);
        assert!(eval_one(
            &sampled,
            &test_spec(),
            &GateContext::default(),
            &denominator_rule(Some(80))
        )
        .is_pass());
    }

    #[test]
    fn denominator_sampled_below_ratio_blocks() {
        // 有 rationale 但覆盖率不足阈值（2% < 80%）→ 仍 Block。
        let rule = denominator_rule(Some(80));
        let d = deliverable_with_coverage(vec![cov_cell_dn(
            "a",
            "WSTG-INPV-05",
            CoverageStatus::CheckedEmpty,
            vec![1],
            100,
            5000,
            Some("only sampled a handful"),
        )]);
        assert!(!eval_one(&d, &test_spec(), &GateContext::default(), &rule).is_pass());
    }

    #[test]
    fn denominator_ignores_blocked_and_not_applicable() {
        // blocked / not_applicable 免分母（即使 tested=0/total=0）→ Pass。
        let rule = denominator_rule(None);
        let d = deliverable_with_coverage(vec![
            cov_cell_dn(
                "a",
                "WSTG-INPV-19",
                CoverageStatus::Blocked,
                vec![],
                0,
                0,
                None,
            ),
            cov_cell_dn(
                "a",
                "WSTG-INPV-18",
                CoverageStatus::NotApplicable,
                vec![],
                0,
                0,
                None,
            ),
        ]);
        assert!(eval_one(&d, &test_spec(), &GateContext::default(), &rule).is_pass());
    }

    #[test]
    fn denominator_blocks_zero_total_for_checked_empty() {
        // total_units==0 且 status=checked_empty → Block（应改用 not_applicable）。
        let rule = denominator_rule(None);
        let d = deliverable_with_coverage(vec![cov_cell_dn(
            "a",
            "WSTG-INPV-05",
            CoverageStatus::CheckedEmpty,
            vec![1],
            0,
            0,
            None,
        )]);
        match eval_one(&d, &test_spec(), &GateContext::default(), &rule) {
            GateCheckOutcome::Block { reasons, .. } => {
                assert!(
                    reasons.iter().any(|r| r.contains("total_units=0")),
                    "{reasons:?}"
                );
            }
            GateCheckOutcome::Pass => panic!("expected Block for total_units=0 checked_empty"),
        }
    }

    // ── Phase 2 ①③ seam：GateContext 注入资产 / 期望技术（设计 §5.5 + coverage-matrix §6.5） ──

    #[test]
    fn coverage_complete_injected_in_scope_assets_govern_asset_dimension() {
        // ① seam：注入 in-scope 资产 {a, b}；deliverable 只自报覆盖 a → b×idor 缺口 → Block。
        // 默认 ctx 时只核自报的 a → Pass。证明资产维度可从外层权威注入（堵少报蒙混）。
        let rule = coverage_complete_rule();
        let spec = spec_with_expected(&["idor"]);
        let d =
            deliverable_with_coverage(vec![cov_cell("a", "idor", CoverageStatus::Found, vec![1])]);

        // 默认 ctx：自报资产集 = {a}，a 覆盖了 idor → Pass。
        assert!(eval_with_context(
            &d,
            &spec,
            std::slice::from_ref(&rule),
            &GateContext::default()
        )[0]
        .is_pass());

        // 注入 {a, b}：b×idor 不在矩阵 → Block 含缺口。
        let ctx = GateContext {
            in_scope_assets: Some(vec!["a".to_string(), "b".to_string()]),
            ..Default::default()
        };
        match &eval_with_context(&d, &spec, &[rule], &ctx)[0] {
            GateCheckOutcome::Block { reasons, .. } => {
                assert!(reasons[0].contains("(b × idor)"), "{reasons:?}");
            }
            GateCheckOutcome::Pass => panic!("injected in-scope asset b must be required"),
        }
    }

    #[test]
    fn coverage_complete_ctx_expected_techniques_override_spec() {
        // ③ seam：spec 无 expected_techniques（no-op），但 ctx 动态注入 ["idor"] →
        // 仍核完整性 → 资产 a 缺 idor → Block。证明期望技术可由 skeleton / 外层动态注入。
        let rule = coverage_complete_rule();
        let spec = test_spec(); // expected_techniques 空
        let d =
            deliverable_with_coverage(vec![cov_cell("a", "xss", CoverageStatus::Found, vec![1])]);

        // 默认 ctx：spec 空 → no-op Pass。
        assert!(eval(&d, &spec, std::slice::from_ref(&rule))[0].is_pass());

        // ctx 注入期望技术 ["idor"]：a 缺 idor → Block。
        let ctx = GateContext {
            expected_techniques: Some(vec!["idor".to_string()]),
            ..Default::default()
        };
        match &eval_with_context(&d, &spec, &[rule], &ctx)[0] {
            GateCheckOutcome::Block { reasons, .. } => {
                assert!(reasons[0].contains("(a × idor)"), "{reasons:?}");
            }
            GateCheckOutcome::Pass => panic!("ctx-injected expected technique must be enforced"),
        }
    }

    // ── P5 Task 3: ItemField::Technique resolves on claims/findings ───────────

    #[test]
    fn technique_field_resolves_on_claims_and_findings() {
        let rule = parse(
            r#"{ "op":"for_all","over":"claims",
                 "require":{"pred":"non_empty","field":"technique"},
                 "on_fail":{"reason":"claims must be technique-tagged"} }"#,
        );
        let mut d = StageDeliverable {
            stage_id: "vuln_triage".to_string(),
            stage_run_id: uuid::Uuid::new_v4(),
            claims: vec![StageClaim {
                kind: "dns_a_record".to_string(),
                subject: "example.com".to_string(),
                summary: "A 1.2.3.4".to_string(),
                evidence_ids: vec![],
                technique: Some("GOLISH-INTEL-DNS".to_string()),
            }],
            evidence_refs: vec![],
            skipped_checks: vec![],
            findings: vec![],
            required_checks_done: vec![],
            coverage: vec![],
        };
        assert!(eval(&d, &test_spec(), std::slice::from_ref(&rule))[0].is_pass());

        d.claims.push(StageClaim {
            kind: "untagged".to_string(),
            subject: "example.com".to_string(),
            summary: "no technique".to_string(),
            evidence_ids: vec![],
            technique: None,
        });
        assert!(!eval(&d, &test_spec(), &[rule])[0].is_pass());
    }

    #[test]
    fn technique_eq_pred_matches_on_findings() {
        let rule = parse(
            r#"{ "op":"count_at_least","over":"findings",
                 "where":{"pred":"eq","field":"technique","value":"GOLISH-INTEL-SUBDOMAIN"},
                 "min":1,"on_fail":{"reason":"need a subdomain-technique finding"} }"#,
        );
        let d = StageDeliverable {
            stage_id: "vuln_triage".to_string(),
            stage_run_id: uuid::Uuid::new_v4(),
            claims: vec![],
            evidence_refs: vec![],
            skipped_checks: vec![],
            findings: vec![HarnessFinding {
                finding_id: uuid::Uuid::new_v4(),
                kind: "subdomain".to_string(),
                subject: "a.example.com".to_string(),
                severity: FindingSeverity::Info,
                evidence_refs: vec![],
                technique: Some("GOLISH-INTEL-SUBDOMAIN".to_string()),
            }],
            required_checks_done: vec![],
            coverage: vec![],
        };
        assert!(eval(&d, &test_spec(), &[rule])[0].is_pass());
    }

    // ── P5 Task 4: coverage_complete derive_from_items ────────────────────────

    #[test]
    fn coverage_complete_derive_from_items_fills_gap_from_tagged_claim() {
        let rule = parse(
            r#"{ "op":"coverage_complete","derive_from_items":true,
                 "on_fail":{"reason":"intel coverage incomplete"} }"#,
        );
        let spec = spec_with_expected(&["GOLISH-INTEL-DNS", "GOLISH-INTEL-WHOIS"]);
        let mut d = deliverable_with_coverage(vec![cov_cell(
            "example.com",
            "GOLISH-INTEL-WHOIS",
            CoverageStatus::CheckedEmpty,
            vec![1],
        )]);
        d.claims.push(StageClaim {
            kind: "dns_a_record".to_string(),
            subject: "example.com".to_string(),
            summary: "A 1.2.3.4".to_string(),
            evidence_ids: vec![],
            technique: Some("GOLISH-INTEL-DNS".to_string()),
        });
        assert!(eval(&d, &spec, &[rule])[0].is_pass());
    }

    #[test]
    fn coverage_complete_derive_off_keeps_blocking() {
        let rule = coverage_complete_rule(); // no derive_from_items
        let spec = spec_with_expected(&["GOLISH-INTEL-DNS", "GOLISH-INTEL-WHOIS"]);
        let mut d = deliverable_with_coverage(vec![cov_cell(
            "example.com",
            "GOLISH-INTEL-WHOIS",
            CoverageStatus::CheckedEmpty,
            vec![1],
        )]);
        d.claims.push(StageClaim {
            kind: "dns_a_record".to_string(),
            subject: "example.com".to_string(),
            summary: "A 1.2.3.4".to_string(),
            evidence_ids: vec![],
            technique: Some("GOLISH-INTEL-DNS".to_string()),
        });
        match &eval(&d, &spec, &[rule])[0] {
            GateCheckOutcome::Block { reasons, .. } => {
                assert!(
                    reasons[0].contains("(example.com × GOLISH-INTEL-DNS)"),
                    "{reasons:?}"
                );
            }
            GateCheckOutcome::Pass => panic!("derive off must keep current blocking behavior"),
        }
    }

    #[test]
    fn coverage_complete_derive_requires_matching_subject() {
        let rule = parse(
            r#"{ "op":"coverage_complete","derive_from_items":true,
                 "on_fail":{"reason":"intel coverage incomplete"} }"#,
        );
        let spec = spec_with_expected(&["GOLISH-INTEL-DNS", "GOLISH-INTEL-WHOIS"]);
        let mut d = deliverable_with_coverage(vec![cov_cell(
            "example.com",
            "GOLISH-INTEL-WHOIS",
            CoverageStatus::CheckedEmpty,
            vec![1],
        )]);
        d.claims.push(StageClaim {
            kind: "dns_a_record".to_string(),
            subject: "other.com".to_string(),
            summary: "A 5.6.7.8".to_string(),
            evidence_ids: vec![],
            technique: Some("GOLISH-INTEL-DNS".to_string()),
        });
        match &eval(&d, &spec, &[rule])[0] {
            GateCheckOutcome::Block { reasons, .. } => {
                assert!(
                    reasons[0].contains("(example.com × GOLISH-INTEL-DNS)"),
                    "{reasons:?}"
                );
            }
            GateCheckOutcome::Pass => panic!("subject mismatch must not derive coverage"),
        }
    }

    // ── P5 Task 5: coverage_corroborated ──────────────────────────────────────

    #[test]
    fn coverage_corroborated_blocks_unbacked_found_cell() {
        let rule = parse(
            r#"{ "op":"coverage_corroborated",
                 "on_fail":{"reason":"found cells must be corroborated by technique-tagged items"} }"#,
        );
        let d = deliverable_with_coverage(vec![cov_cell(
            "example.com",
            "GOLISH-INTEL-DNS",
            CoverageStatus::Found,
            vec![1],
        )]);
        match &eval(&d, &test_spec(), &[rule])[0] {
            GateCheckOutcome::Block { reasons, .. } => {
                assert!(
                    reasons[0].contains("(example.com × GOLISH-INTEL-DNS)"),
                    "{reasons:?}"
                );
            }
            GateCheckOutcome::Pass => panic!("uncorroborated found cell must Block"),
        }
    }

    #[test]
    fn coverage_corroborated_passes_with_matching_tagged_item() {
        let rule = parse(
            r#"{ "op":"coverage_corroborated",
                 "on_fail":{"reason":"found cells must be corroborated by technique-tagged items"} }"#,
        );
        let mut d = deliverable_with_coverage(vec![cov_cell(
            "example.com",
            "GOLISH-INTEL-DNS",
            CoverageStatus::Found,
            vec![1],
        )]);
        d.claims.push(StageClaim {
            kind: "dns_a_record".to_string(),
            subject: "example.com".to_string(),
            summary: "A 1.2.3.4".to_string(),
            evidence_ids: vec![],
            technique: Some("GOLISH-INTEL-DNS".to_string()),
        });
        assert!(eval(&d, &test_spec(), &[rule])[0].is_pass());
    }

    #[test]
    fn coverage_corroborated_exempts_non_found_cells() {
        let rule = parse(
            r#"{ "op":"coverage_corroborated",
                 "on_fail":{"reason":"found cells must be corroborated by technique-tagged items"} }"#,
        );
        let d = deliverable_with_coverage(vec![
            cov_cell(
                "a",
                "GOLISH-INTEL-DNS",
                CoverageStatus::CheckedEmpty,
                vec![1],
            ),
            cov_cell("a", "GOLISH-INTEL-WHOIS", CoverageStatus::Blocked, vec![]),
            cov_cell(
                "a",
                "GOLISH-INTEL-ASN",
                CoverageStatus::NotApplicable,
                vec![],
            ),
        ]);
        assert!(eval(&d, &test_spec(), &[rule])[0].is_pass());
    }

    // ── 设计 2026-06-12 §5.3: DB 业务表投影 fact（哨兵 id=0）端到端安全保证 ──
    // ① coverage_complete derive_from_evidence 用哨兵 fact 补格（投影只看
    //    asset/technique/outcome，与 evidence_id 无关）；
    // ② coverage_corroborated 只查自报 coverage cell，投影格不在 d.coverage 里，
    //    故天然绕过它，不被误 BLOCK（这是 DB 真值投影方案安全的核心保证）。
    #[test]
    fn db_truth_sentinel_fact_fills_coverage_without_corroboration_block() {
        let complete = evidence_derive_rule(None);
        let corroborated = parse(
            r#"{ "op":"coverage_corroborated",
                 "on_fail":{"reason":"found cells must be corroborated by technique-tagged items"} }"#,
        );
        // agent 自报 coverage 全空（没写 ASN 格、没 claim）。
        let d = deliverable_with_coverage(vec![]);
        // 业务表投影 fact：asset=a × ASN，Found，哨兵 evidence_id=0。
        let ctx = projection_ctx(
            &["GOLISH-INTEL-ASN"],
            Some(vec![fact(
                "a",
                "GOLISH-INTEL-ASN",
                EvidenceOutcome::Found,
                0,
            )]),
        );
        let outcomes = eval_with_context(&d, &test_spec(), &[complete, corroborated], &ctx);
        assert!(
            outcomes[0].is_pass(),
            "DB-truth sentinel fact (id=0) fills (a × ASN) via derive_from_evidence (projection is id-agnostic)"
        );
        assert!(
            outcomes[1].is_pass(),
            "coverage_corroborated only inspects self-reported cells; the projected cell is not in d.coverage, so it isn't (and needn't be) corroborated"
        );
    }
}
