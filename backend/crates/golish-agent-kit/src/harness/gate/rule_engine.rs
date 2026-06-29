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
    CoverageCell, CoverageGapAction, CoverageStatus, FindingSeverity, HarnessFinding,
    HarnessRecoveryActions, StageClaim, StageDeliverable,
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
        /// T1（设计 2026-06-23-coverage-note-required）：true 时 `blocked` /
        /// `not_applicable` 终态格要求 `note` 非空，否则不算终态 → 缺口 BLOCK
        /// （堵「空 note 蒙混 blocked」）。缺省 false = 逐字节不变（旧「自报即终态」）。
        /// 按 spec 灰度逐阶段翻开。
        #[serde(default)]
        require_note_for_other: bool,
        on_fail: OnFail,
    },
    /// P5（2026-06-11）交叉校验：每个 status == found 的 coverage cell 必须有 ≥1 个
    /// technique 匹配的 claim/finding 佐证（item.technique == cell.technique 且
    /// item.subject == cell.asset，精确相等，D5）。found 之外的终态豁免（D3）：
    /// absence 无结构化观察可佐证，由 cell 自身 evidence/note 规则把关。
    CoverageCorroborated {
        /// Phase C（设计 2026-06-22 瘦身交付物）：true ⇒ 显式 no-op（立即 Pass）。当
        /// 本阶段从 DB 真值裁决覆盖（`facts_from_db_truth` + coverage_complete
        /// `authoritative_found`）时，agent 交「瘦」交付物——只跑采集工具、让 DB-truth
        /// 投影补格，故**不应**再被要求为每个 found cell 手打 technique-tagged
        /// claim/finding 佐证。缺省 false = P5 佐证校验（逐字节不变）。
        #[serde(default)]
        authoritative: bool,
        on_fail: OnFail,
    },
    /// 分母覆盖（设计 2026-06-05-vuln-triage-technique-matrix §5.3）。对 status ∈
    /// {found, checked_empty} 的每个 coverage cell 核「面覆盖」：默认全覆盖（D6）要求
    /// `tested_units == total_units`；抽样例外要求 `sampling_rationale` 非空且
    /// `tested_units*100 ≥ min_sample_ratio_pct*total_units`。blocked / not_applicable
    /// 免分母；`total_units==0` 的 found/checked_empty 记缺口（应改用 not_applicable）。
    CoverageDenominator {
        /// Slim DB-truth stages should not require the model to hand-copy
        /// denominator fields for cells the database already adjudicates.
        /// Completeness remains enforced by `coverage_complete`.
        #[serde(default)]
        authoritative: bool,
        #[serde(default = "default_sample_ratio_pct")]
        min_sample_ratio_pct: u8,
        on_fail: OnFail,
    },
    /// Source coverage (2026-06-23 provider-source closure): for techniques that
    /// claim `found` / `checked_empty`, require a terminal `source_query_log`
    /// row proving a provider/source was actually queried. This is deliberately
    /// weaker than `coverage_complete`: source rows prove "attempted source",
    /// never "found data" (DB/ledger truth still owns found).
    SourceCoverage {
        #[serde(default)]
        authoritative_techniques: Option<Vec<String>>,
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
            | GateRule::CoverageCorroborated { on_fail, .. }
            | GateRule::CoverageDenominator { on_fail, .. }
            | GateRule::SourceCoverage { on_fail, .. } => on_fail.reason.clone(),
            GateRule::NamedCheck { check, on_fail } => on_fail
                .as_ref()
                .map(|o| o.reason.clone())
                .unwrap_or_else(|| format!("{} check", check.as_str())),
        }
    }

    /// 稳定的 op 标识，用于可观测性日志（block 归因）。`named_check` 取其 check 名
    /// （scope / surface_coverage / min_invocations），其余取顶层 op tag。
    pub fn op_name(&self) -> &'static str {
        match self {
            GateRule::CountAtLeast { .. } => "count_at_least",
            GateRule::ForAll { .. } => "for_all",
            GateRule::NamedCheck { check, .. } => check.as_str(),
            GateRule::CoverageComplete { .. } => "coverage_complete",
            GateRule::CoverageCorroborated { .. } => "coverage_corroborated",
            GateRule::CoverageDenominator { .. } => "coverage_denominator",
            GateRule::SourceCoverage { .. } => "source_coverage",
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
    /// Host-aware coverage 2c（设计 2026-06-15-host-aware-coverage-2c §4.1）：value →
    /// `targets.type`，让 `coverage_complete` 按**权威**类型给资产分类（回退
    /// `from_value` → `Other`）。`None` = 回退按值推断（2a/2b 行为，逐字节一致）。
    pub asset_types: Option<std::collections::HashMap<String, String>>,
    pub expected_techniques: Option<Vec<String>>,
    /// PR3 (设计 2026-06-11-coverage-auto-derive §5.2) · 证据账本投影事实：
    /// 从 `audit_log` 三列 (`evidence_asset/technique/outcome`) 注入的只读三元组。
    /// `None` = 不启用投影（与旧行为逐字节一致）；规则侧还需
    /// `coverage_complete.derive_from_evidence=true` 才消费（双开关，灰度安全）。
    pub evidence_facts: Option<Vec<EvidenceFact>>,
    /// Source-query terminal facts from `source_query_log`.
    ///
    /// These facts answer "which provider/source did this run query and how did
    /// that source terminate?" They never project into `found`; exact technique
    /// rows with empty/error/blocked terminal status may close `coverage_complete`
    /// gaps as non-found terminal states, and `source_coverage` consumes them to
    /// prove the source/provider was actually attempted.
    pub source_queries: Option<Vec<SourceQueryFact>>,
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
/// 投影成 CheckedEmpty 终态，**绝不**当 Found）/ `Error`（跑了但失败：非零退出 /
/// 超时 / 502 等——T2，设计 2026-06-23-failure-outcome-not-checked-empty。投影成
/// 「失败阻断」终态，**绝不**当 Found / CheckedEmpty）。
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum EvidenceOutcome {
    Found,
    Empty,
    Error,
}

/// A terminal source/provider query row projected from `source_query_log`.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct SourceQueryFact {
    pub source: String,
    pub query: String,
    pub target: String,
    pub technique: Option<String>,
    pub status: String,
    pub evidence_ids: Vec<i64>,
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
    let outcome = match rule {
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
                NamedCheckKind::SurfaceCoverage if db_truth_backed(spec, ctx) => {
                    // DB-truth stages use the precise coverage matrix populated
                    // from ledger/DB facts. The legacy Surface Workbench
                    // category check is redundant there and would push the
                    // model back toward hand-written claims.
                    GateCheckOutcome::Pass
                }
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
            require_note_for_other,
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
            *require_note_for_other,
            on_fail,
        ),
        GateRule::CoverageCorroborated {
            on_fail,
            authoritative,
        } => coverage_corroborated(d, on_fail, *authoritative),
        GateRule::CoverageDenominator {
            authoritative,
            min_sample_ratio_pct,
            on_fail,
        } => coverage_denominator(d, *authoritative, *min_sample_ratio_pct, on_fail),
        GateRule::SourceCoverage {
            authoritative_techniques,
            on_fail,
        } => source_coverage(d, spec, ctx, authoritative_techniques.as_deref(), on_fail),
    };

    // Observability (2026-06-16): the semantic gate rules were the ONLY gate layer
    // with no tracing — a `coverage_complete` block (e.g. target_intel "never
    // attempted" cells) surfaced solely in the submit tool result, never in
    // backend.log, so a stuck stage looked like "4 structural checks pass" with no
    // visible cause. Emit the block here (mirrors `harness::gate::schema_check`) so
    // the actual blocking rule + first reason are greppable per submit.
    if let GateCheckOutcome::Block { reasons, .. } = &outcome {
        tracing::info!(
            target: "harness::gate::rule_engine",
            stage_id = %d.stage_id,
            stage_run_id = %d.stage_run_id,
            op = rule.op_name(),
            outcome = "block",
            reasons_count = reasons.len(),
            first_reason = reasons.first().map(String::as_str).unwrap_or(""),
            "gate_rule block"
        );
    }
    outcome
}

fn db_truth_backed(spec: &StageSpec, ctx: &GateContext) -> bool {
    spec.facts_from_db_truth && ctx.evidence_facts.as_deref().is_some_and(|f| !f.is_empty())
}

/// 所有 coverage 终态（`terminal_status` 缺省时用）。
const ALL_TERMINAL: [CoverageStatus; 4] = [
    CoverageStatus::Found,
    CoverageStatus::CheckedEmpty,
    CoverageStatus::Blocked,
    CoverageStatus::NotApplicable,
];

/// Anchor-only coverage axis (design 2026-06-16-coverage-anchor-axis): drop any
/// asset that is a strict subdomain of ANOTHER asset in the same in-scope set, so
/// subdomains passively discovered + registered as `scope='in'` during a "no
/// enumeration denominator" stage (target_intel) do not inflate the (asset ×
/// technique) matrix — the root's own SUBDOMAIN cell already represents the
/// enumeration. Pure (no IO). The leading dot in the `.{parent}` suffix check stops
/// `ba.com` matching parent `a.com`. Maximal roots have no in-set parent so they
/// always remain ⇒ a non-empty input never yields an empty output (so this filter
/// can never trigger the empty-matrix BLOCK).
fn anchor_only_axis<'a>(assets: &[&'a str]) -> Vec<&'a str> {
    assets
        .iter()
        .copied()
        .filter(|a| {
            !assets.iter().any(|parent| {
                *parent != *a && a.len() > parent.len() && a.ends_with(&format!(".{parent}"))
            })
        })
        .collect()
}

/// Persist the COMPLETE coverage gap list to the per-run log (`run.log` +
/// `backend.log`, target `harness::gate::coverage`) when a coverage rule blocks.
///
/// The model-facing `reasons` only carries the first 8 cells (so the prompt isn't
/// flooded), which means a stuck stage's *full* `(asset × technique)` gap matrix
/// was previously unrecoverable — only those 8 survived anywhere on disk. This
/// emits the exact, untruncated set: `gaps_total` is always exact; the joined
/// list is capped only against pathological sizes. Observability only — it never
/// changes the gate verdict (the returned `GateCheckOutcome` is untouched).
fn log_coverage_gap_matrix(stage_id: &str, op: &str, gaps: &[String]) {
    const LOG_CAP: usize = 500;
    let shown = if gaps.len() > LOG_CAP {
        format!(
            "{} … (+{} more — see gaps_total)",
            gaps[..LOG_CAP].join(", "),
            gaps.len() - LOG_CAP
        )
    } else {
        gaps.join(", ")
    };
    tracing::info!(
        target: "harness::gate::coverage",
        stage_id = %stage_id,
        op = %op,
        gaps_total = gaps.len(),
        gaps = %shown,
        "coverage gap matrix (full)"
    );
}

fn coverage_gap_action(asset: &str, technique: &str) -> CoverageGapAction {
    CoverageGapAction {
        asset: asset.to_string(),
        technique: technique.to_string(),
        reason: "missing_terminal_coverage".to_string(),
        suggested_tools: suggested_tools_for_gap(technique),
    }
}

fn suggested_tools_for_gap(technique: &str) -> Vec<String> {
    match technique {
        "GOLISH-EAS-LIVENESS" => vec!["httpx".to_string(), "nmap -sn".to_string()],
        "GOLISH-EAS-PORT" => vec!["naabu".to_string(), "nmap".to_string()],
        "GOLISH-EAS-SERVICE-FINGERPRINT" => {
            vec!["nmap -sV".to_string(), "whatweb".to_string()]
        }
        _ => Vec::new(),
    }
}

/// E1 PR-B（设计 2026-06-18-canonical-asset-identity-and-coverage-join-key）：把资产串
/// 归一成「join 身份键」，让 coverage join 两侧（in-scope 轴 / 自报 cell / 账本 fact /
/// claim subject）对同一资产的不同书写（`http://x` vs `https://x` vs `x`、大小写、FQDN
/// 尾点）能匹配上，根治「身份漂移 → fact 静默不命中 → 永判 not_attempted → 无限
/// needs_fix」死循环。
///
/// 关键：**保留 URL 路径**——刻意不走 `canonical_asset_key` 的「URL 折叠到 host」，否则
/// 会把 EAS / enumeration 的「URL 端点」(`https://a.com/login`) 与其「主机」(`a.com`)
/// 错误合并成同一格（端点的 DIR/PARAM/JSAPI 与主机的 PORT/SERVICE 是不同覆盖行）。这里
/// 只抹平 scheme / 大小写 / 尾点这些纯书写差异，资产粒度不变。纯函数、确定性。红线：
/// 绝不截断 / 合并到 apex，绝不把不同资产并成一格。
fn canon_asset(s: &str) -> String {
    let lowered = s.trim().to_ascii_lowercase();
    // 去掉单个前导 scheme（`http://` / `https://` / …），保留 host[+path]。
    let no_scheme = match lowered.split_once("://") {
        Some((scheme, rest))
            if !scheme.is_empty() && scheme.chars().all(|c| c.is_ascii_alphabetic()) =>
        {
            rest
        }
        _ => lowered.as_str(),
    };
    // 抹平 FQDN 尾点（`pingan.com.` → `pingan.com`）。
    no_scheme.trim_end_matches('.').to_string()
}

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
    require_note_for_other: bool,
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

    // Anchor-only denominator (design 2026-06-16-coverage-anchor-axis): on a stage
    // that declares "no enumeration denominator" (target_intel), drop assets that
    // are subdomains of another in-scope asset so this stage's own passively-
    // discovered subdomains don't inflate the matrix (the "treadmill" that drove
    // every org to its iteration cap). Maximal roots always remain, so the empty
    // check below is never reached via this filter (a non-empty axis stays
    // non-empty). Default off ⇒ byte-for-byte unchanged for every other stage.
    let assets: Vec<&str> = if spec.coverage_anchor_only {
        anchor_only_axis(&assets)
    } else {
        assets
    };

    // E1 PR-B（B1 · 设计 2026-06-18-canonical-asset-identity）：按规范覆盖身份去重资产
    // 轴——把同一资产的漂移写法（`http://x` / `x` / `X.` / 大小写）折叠成一行（保留首个
    // 原串供 asset_types 查表 + gap 消息），避免同一资产被当成多行、重复要求技术覆盖。
    // canon 保留 URL 路径，故 EAS 的「URL 端点」与其「主机」仍区分、不会被错误折叠。
    let assets: Vec<&str> = {
        let mut seen = std::collections::HashSet::new();
        assets
            .into_iter()
            .filter(|a| seen.insert(canon_asset(a)))
            .collect()
    };

    // P0 (2026-06-11 coverage-empty-bypass): we already returned Pass above when
    // `techniques` is empty, so reaching here means the stage DECLARES expected
    // techniques. An empty SELF-REPORTED asset set means the agent omitted the
    // matrix entirely. That must still BLOCK because "omitted" is not
    // "checked-empty" (AGENTS.md I8).
    //
    // If the caller injected `Some([])` as the authoritative in-scope axis,
    // however, DB truth says there are no assets to cover. That is a real
    // vacuous pass, and must match the read-only `check_stage_asset_coverage`
    // precheck; otherwise zero-asset orgs enter repair mode and workers start
    // inventing probes.
    if assets.is_empty() {
        if ctx
            .in_scope_assets
            .as_ref()
            .is_some_and(|list| list.is_empty())
        {
            return GateCheckOutcome::Pass;
        }
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
                ..Default::default()
            },
        };
    }

    let mut gaps: Vec<CoverageGapAction> = Vec::new();
    for asset in &assets {
        // Host-aware coverage (design 2026-06-15 §4.0): when enabled, hold each
        // asset only to the techniques that apply to its class (a bare IP is not
        // asked for SUBDOMAIN/DNS/CT). Flag off ⇒ the full `techniques` list for
        // every asset (byte-identical to before). `Other`/unknown keeps the full
        // list (fail-safe: an unclassified asset is never under-checked).
        let asset_techniques: Vec<&String> = if spec.host_aware_coverage {
            // 2c-1 (设计 host-aware-coverage-2c §4.1): classify from the injected
            // authoritative `targets.type` when present; else fall back to value
            // inference (2a/2b), then `Other` (full set). `asset` is `&&str`, so
            // `*asset` is the `&str` map key.
            let class = crate::harness::technique_resolver::AssetClass::classify(
                ctx.asset_types
                    .as_ref()
                    .and_then(|m| m.get(*asset))
                    .map(String::as_str),
                asset,
            );
            techniques
                .iter()
                .filter(|t| {
                    crate::harness::technique_resolver::technique_applies_to_value(
                        spec.kind,
                        class,
                        asset,
                        t.as_str(),
                    )
                })
                .collect()
        } else {
            techniques.iter().collect()
        };
        // E1 PR-B：把当前资产轴值归一成身份键，下面 join 两侧都用它比较，
        // 容忍同一资产的不同写法（http://x / x / X. / 大小写）。
        let asset_key = canon_asset(asset);
        for tech in asset_techniques {
            // Phase 0（设计 2026-06-12-redteam-phase0）：该 technique 是否进入「权威」
            // 模式（found 只认真值）。authoritative_techniques=None → 全部期望技术；
            // Some → 仅清单内技术收紧，其余仍走旧自报（灰度）。
            let authoritative = authoritative_found
                && authoritative_techniques.is_none_or(|list| list.iter().any(|t| t == tech));

            let cell_status = |want: CoverageStatus| {
                d.coverage.iter().any(|c| {
                    canon_asset(&c.asset) == asset_key && c.technique == *tech && c.status == want
                })
            };
            // 账本/DB 真值通道：asset+technique 精确匹配且 outcome 命中的事实存在？
            let has_fact = |want: EvidenceOutcome| {
                ctx.evidence_facts.as_deref().is_some_and(|facts| {
                    facts.iter().any(|f| {
                        canon_asset(&f.asset) == asset_key
                            && f.technique == *tech
                            && f.outcome == want
                    })
                })
            };
            // P5 派生（D1/D2）：technique 标注且 subject == asset 的 claim/finding 视作
            // found（仅旧自报路径用；authoritative 模式下不再算 found）。
            let tagged_found = d.claims.iter().any(|c| {
                canon_asset(&c.subject) == asset_key
                    && c.technique.as_deref() == Some(tech.as_str())
            }) || d.findings.iter().any(|f| {
                canon_asset(&f.subject) == asset_key
                    && f.technique.as_deref() == Some(tech.as_str())
            });

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
            // - authoritative：自报 checked_empty 必须有真 Empty 账本事实；如果工具落账点
            //   已经直接派生 Empty fact（如 EAS active probe / source terminal），该事实本身
            //   也可关闭缺口，避免模型为了镜像账本再手抄 coverage matrix。
            // - 旧路径：自报 cell || derive_from_evidence 的 Empty 事实。
            let empty_ok = terminal.contains(&CoverageStatus::CheckedEmpty)
                && if authoritative {
                    has_fact(EvidenceOutcome::Empty)
                        && (cell_status(CoverageStatus::CheckedEmpty)
                            || (derive_from_evidence && !cell_status(CoverageStatus::Found)))
                } else {
                    cell_status(CoverageStatus::CheckedEmpty)
                        || (derive_from_evidence && has_fact(EvidenceOutcome::Empty))
                };
            // blocked / not_applicable 终态：自报 cell 命中即算。T1（设计
            // 2026-06-23-coverage-note-required）：`require_note_for_other` 开时额外
            // 要求 `note` 非空（堵「空 note 蒙混 blocked」）；缺省 false ⇒ note 子句
            // 恒真 = 与旧 `cell_status` 逐字节一致。
            let cell_other_ok = |want: CoverageStatus| {
                d.coverage.iter().any(|c| {
                    canon_asset(&c.asset) == asset_key
                        && c.technique == *tech
                        && c.status == want
                        && (!require_note_for_other
                            || c.note.as_deref().is_some_and(|n| !n.trim().is_empty()))
                })
            };
            let other_ok = (terminal.contains(&CoverageStatus::Blocked)
                && cell_other_ok(CoverageStatus::Blocked))
                || (terminal.contains(&CoverageStatus::NotApplicable)
                    && cell_other_ok(CoverageStatus::NotApplicable));

            // T2（设计 2026-06-23-failure-outcome-not-checked-empty）：error 事实
            // （跑了但失败：超时 / 非零退出 / 502）= 终态——保住旧 failure→empty 的
            // 「落终态、不无限重试」性质，但按「失败阻断」计，绝不当 found / checked_empty。
            // 终态条件取 CheckedEmpty∪Blocked（覆盖旧 empty 路径 + Blocked 语义）。无
            // error 事实时恒假 = 逐字节不变（additive，gate 侧无需灰度）。
            let error_ok = derive_from_evidence
                && has_fact(EvidenceOutcome::Error)
                && (terminal.contains(&CoverageStatus::CheckedEmpty)
                    || terminal.contains(&CoverageStatus::Blocked));

            // Source-query terminal rows are source-attempt facts, not discovery
            // facts. They can close a missing cell only as a non-found terminal
            // state (checked_empty / blocked), never rescue a self-reported
            // `found` cell that lacks DB truth. This is what lets a slim
            // target_intel deliverable pass after `recon_lookup_whois` records
            // "RDAP ran and returned empty" once per org, without making the
            // model hand-write hundreds of WHOIS checked_empty cells.
            let source_terminal_ok = !cell_status(CoverageStatus::Found)
                && derive_from_evidence
                && ctx.source_queries.as_deref().is_some_and(|rows| {
                    rows.iter().any(|row| {
                        source_row_terminal_for_coverage(row, tech, &asset_key, terminal)
                    })
                });

            if !found_ok && !empty_ok && !other_ok && !error_ok && !source_terminal_ok {
                gaps.push(coverage_gap_action(asset, tech));
            }
        }
    }

    if gaps.is_empty() {
        return GateCheckOutcome::Pass;
    }
    let gap_labels = gaps
        .iter()
        .map(|gap| format!("({} × {})", gap.asset, gap.technique))
        .collect::<Vec<_>>();
    const MAX_SHOWN: usize = 8;
    let shown = gap_labels
        .iter()
        .take(MAX_SHOWN)
        .cloned()
        .collect::<Vec<_>>()
        .join(", ");
    let suffix = if gap_labels.len() > MAX_SHOWN {
        format!(" (+{} more)", gap_labels.len() - MAX_SHOWN)
    } else {
        String::new()
    };
    log_coverage_gap_matrix(&d.stage_id, "coverage_complete", &gap_labels);
    GateCheckOutcome::Block {
        reasons: vec![format!(
            "{}: never attempted {}{}",
            on_fail.reason, shown, suffix
        )],
        recovery: HarnessRecoveryActions {
            hints: on_fail.hints.clone(),
            repair_tool_calls: on_fail.repair_tool_calls.clone(),
            missing_evidence_kinds: on_fail.missing_evidence_kinds.clone(),
            coverage_gap_actions: gaps,
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
    authoritative: bool,
    min_ratio_pct: u8,
    on_fail: &OnFail,
) -> GateCheckOutcome {
    if authoritative {
        return GateCheckOutcome::Pass;
    }
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
    log_coverage_gap_matrix(&d.stage_id, "coverage_denominator", &gaps);
    GateCheckOutcome::Block {
        reasons: vec![format!("{}: {}{}", on_fail.reason, shown, suffix)],
        recovery: HarnessRecoveryActions {
            hints: on_fail.hints.clone(),
            repair_tool_calls: on_fail.repair_tool_calls.clone(),
            missing_evidence_kinds: on_fail.missing_evidence_kinds.clone(),
            ..Default::default()
        },
    }
}

/// `coverage_corroborated` 求值（纯函数，P5 设计 D3/D5）。每个 status == found 的
/// coverage cell 必须有 ≥1 个 technique 匹配的 claim/finding 佐证（item.technique ==
/// cell.technique 且 item.subject == cell.asset，精确相等）。其余终态豁免（absence
/// 无结构化观察可佐证，由 cell 自身 evidence/note 规则把关）。
fn coverage_corroborated(
    d: &StageDeliverable,
    on_fail: &OnFail,
    authoritative: bool,
) -> GateCheckOutcome {
    // Phase C (设计 2026-06-22): authoritative stages adjudicate coverage from DB
    // truth (the projection fills the matrix), so a slim deliverable must not be
    // blocked for lacking a technique-tagged claim/finding per found cell —
    // corroboration is an explicit no-op here.
    if authoritative {
        return GateCheckOutcome::Pass;
    }
    let mut gaps: Vec<String> = Vec::new();
    for cell in &d.coverage {
        if cell.status != CoverageStatus::Found {
            continue;
        }
        // E1 PR-B：佐证 join 同样按规范身份比较，容忍 cell.asset 与 claim/finding
        // subject 的同一资产不同写法。
        let cell_asset_key = canon_asset(&cell.asset);
        let corroborated = d.claims.iter().any(|c| {
            canon_asset(&c.subject) == cell_asset_key
                && c.technique.as_deref() == Some(cell.technique.as_str())
        }) || d.findings.iter().any(|f| {
            canon_asset(&f.subject) == cell_asset_key
                && f.technique.as_deref() == Some(cell.technique.as_str())
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
    log_coverage_gap_matrix(&d.stage_id, "coverage_corroborated", &gaps);
    GateCheckOutcome::Block {
        reasons: vec![format!(
            "{}: no technique-tagged claim/finding backs {}{}",
            on_fail.reason, shown, suffix
        )],
        recovery: HarnessRecoveryActions {
            hints: on_fail.hints.clone(),
            repair_tool_calls: on_fail.repair_tool_calls.clone(),
            missing_evidence_kinds: on_fail.missing_evidence_kinds.clone(),
            ..Default::default()
        },
    }
}

fn source_coverage(
    d: &StageDeliverable,
    spec: &StageSpec,
    ctx: &GateContext,
    authoritative_techniques: Option<&[String]>,
    on_fail: &OnFail,
) -> GateCheckOutcome {
    let techniques: Vec<&str> = match authoritative_techniques {
        Some(list) => list.iter().map(String::as_str).collect(),
        None => match ctx.expected_techniques.as_deref() {
            Some(t) => t.iter().map(String::as_str).collect(),
            None => spec
                .expected_techniques
                .iter()
                .map(String::as_str)
                .collect(),
        },
    };
    if techniques.is_empty() {
        return GateCheckOutcome::Pass;
    }

    let rows = ctx.source_queries.as_deref().unwrap_or(&[]);
    let assets = source_coverage_assets(d, spec, ctx);
    if rows.is_empty() && assets.is_empty() {
        return GateCheckOutcome::Pass;
    }

    let mut gaps = Vec::new();
    for tech in techniques {
        if rows.iter().any(|row| source_row_covers_tech(row, tech)) {
            continue;
        }
        // If every applicable cell is explicitly blocked / not_applicable with
        // a note, the model has declared "there is no usable source/provider" and
        // `coverage_complete` already treats that as terminal. Do not force a
        // source row for a source that cannot be invoked.
        if !assets.is_empty() && all_assets_have_other_terminal(d, &assets, tech) {
            continue;
        }
        gaps.push(tech.to_string());
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
    log_coverage_gap_matrix(&d.stage_id, "source_coverage", &gaps);
    GateCheckOutcome::Block {
        reasons: vec![format!(
            "{}: missing terminal source/provider query for {}{}",
            on_fail.reason, shown, suffix
        )],
        recovery: HarnessRecoveryActions {
            hints: on_fail.hints.clone(),
            repair_tool_calls: on_fail.repair_tool_calls.clone(),
            missing_evidence_kinds: on_fail.missing_evidence_kinds.clone(),
            ..Default::default()
        },
    }
}

fn source_coverage_assets(
    d: &StageDeliverable,
    spec: &StageSpec,
    ctx: &GateContext,
) -> Vec<String> {
    let mut assets: Vec<String> = match ctx.in_scope_assets.as_deref() {
        Some(list) => list.to_vec(),
        None => {
            let mut self_reported = Vec::new();
            for cell in &d.coverage {
                if !self_reported.contains(&cell.asset) {
                    self_reported.push(cell.asset.clone());
                }
            }
            self_reported
        }
    };
    if spec.coverage_anchor_only {
        let refs: Vec<&str> = assets.iter().map(String::as_str).collect();
        assets = anchor_only_axis(&refs)
            .into_iter()
            .map(str::to_string)
            .collect();
    }
    let mut seen = std::collections::HashSet::new();
    assets
        .into_iter()
        .filter(|a| seen.insert(canon_asset(a)))
        .collect()
}

fn all_assets_have_other_terminal(d: &StageDeliverable, assets: &[String], tech: &str) -> bool {
    assets.iter().all(|asset| {
        let asset_key = canon_asset(asset);
        d.coverage.iter().any(|cell| {
            canon_asset(&cell.asset) == asset_key
                && cell.technique == tech
                && matches!(
                    cell.status,
                    CoverageStatus::Blocked | CoverageStatus::NotApplicable
                )
                && cell.note.as_deref().is_some_and(|n| !n.trim().is_empty())
        })
    })
}

fn source_row_covers_tech(row: &SourceQueryFact, tech: &str) -> bool {
    if !is_terminal_source_status(&row.status) {
        return false;
    }
    if row.technique.as_deref() == Some(tech) {
        return true;
    }
    provider_survey_covers_tech(row, tech)
}

fn is_terminal_source_status(status: &str) -> bool {
    matches!(
        status,
        "found" | "empty" | "checked_empty" | "error" | "blocked"
    )
}

fn provider_survey_covers_tech(row: &SourceQueryFact, tech: &str) -> bool {
    matches!(
        tech,
        "GOLISH-INTEL-DNS"
            | "GOLISH-INTEL-SUBDOMAIN"
            | "GOLISH-INTEL-ASN"
            | "GOLISH-INTEL-CT"
            | "GOLISH-INTEL-OSINT"
    ) && matches!(row.query.as_str(), "map_assets" | "recon_map_assets")
        && row.source != "rdap"
}

fn source_row_terminal_for_coverage(
    row: &SourceQueryFact,
    tech: &str,
    asset_key: &str,
    terminal: &[CoverageStatus],
) -> bool {
    if !is_terminal_source_status(&row.status) {
        return false;
    }

    let exact_technique = row.technique.as_deref() == Some(tech);
    let provider_survey = row.technique.is_none() && provider_survey_covers_tech(row, tech);
    if !exact_technique && !provider_survey {
        return false;
    }

    // Exact technique rows may be either org-wide (`target` empty) or asset-specific.
    // Provider survey rows are org-wide by construction, so do not match their
    // `target` field against each asset.
    if exact_technique && !row.target.trim().is_empty() && canon_asset(&row.target) != asset_key {
        return false;
    }

    match row.status.as_str() {
        "empty" | "checked_empty" => terminal.contains(&CoverageStatus::CheckedEmpty),
        "blocked" => terminal.contains(&CoverageStatus::Blocked),
        "found" if provider_survey => terminal.contains(&CoverageStatus::CheckedEmpty),
        // A source error is terminal for loop-breaking purposes, but it remains
        // non-found. Accept either blocked or checked_empty terminal sets because
        // older gate rules used checked_empty as the only non-found terminal.
        "error" => {
            terminal.contains(&CoverageStatus::Blocked)
                || terminal.contains(&CoverageStatus::CheckedEmpty)
        }
        "found" => false,
        _ => false,
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
            ..Default::default()
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
    fn db_truth_backed_surface_coverage_named_check_is_noop() {
        let spec = crate::harness::stage_spec::load_stage_spec_from_json(
            r#"{"id":"external_attack_surface","kind":"external_attack_surface","risk_level":"medium",
                "deliverable_schema":"ExternalAttackSurfaceDeliverable",
                "gate_validator":"validate_external_attack_surface_gate",
                "facts_from_db_truth":true}"#,
        )
        .expect("spec parses");
        let rule = parse(r#"{ "op":"named_check","check":"surface_coverage" }"#);
        let d = deliverable(
            vec![],
            vec![StageClaim {
                kind: "generic_note".into(),
                subject: "a.com".into(),
                summary: "DB-backed EAS result".into(),
                evidence_ids: vec![EvidenceAuditId::new(1)],
                technique: None,
            }],
        );
        let ctx = crate::harness::gate::GateContextBuilder::new()
            .extend_evidence_facts(vec![fact(
                "a.com",
                "GOLISH-EAS-LIVENESS",
                EvidenceOutcome::Found,
                1,
            )])
            .build();
        assert!(
            eval_with_context(&d, &spec, &[rule], &ctx)[0].is_pass(),
            "DB-truth EAS should not require a hand-written Surface category claim"
        );
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
            reason_kind: None,
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
            asset_types: None,
            expected_techniques: Some(techs.iter().map(|s| s.to_string()).collect()),
            evidence_facts: facts,
            source_queries: None,
        }
    }

    // ── Host-aware coverage (design 2026-06-15-host-aware-coverage, Phase 2a) ──

    /// target_intel spec with the host-aware flag set (kind drives the matrix).
    fn target_intel_spec(host_aware: bool) -> StageSpec {
        crate::harness::stage_spec::load_stage_spec_from_json(&format!(
            r#"{{"id":"target_intel","kind":"target_intel","risk_level":"low",
                "deliverable_schema":"StageDeliverable","gate_validator":"validate_stage_gate",
                "host_aware_coverage":{host_aware}}}"#
        ))
        .expect("target_intel spec parses")
    }

    /// target_intel spec with the anchor-only coverage flag set.
    fn target_intel_anchor_spec(anchor_only: bool) -> StageSpec {
        crate::harness::stage_spec::load_stage_spec_from_json(&format!(
            r#"{{"id":"target_intel","kind":"target_intel","risk_level":"low",
                "deliverable_schema":"StageDeliverable","gate_validator":"validate_stage_gate",
                "coverage_anchor_only":{anchor_only}}}"#
        ))
        .expect("target_intel anchor spec parses")
    }

    #[test]
    fn coverage_anchor_only_drops_subdomains_of_in_scope_roots() {
        // 设计 2026-06-16-coverage-anchor-axis: target_intel 的覆盖分母只数「锚点/根域」。
        // 被动枚举登记进 scope=in 的子域（pingan.com 的下级）不该撑大分母——根域的
        // SUBDOMAIN 技术格已代表「枚举过子域」。锚点过滤后只有 pingan.com 要核 6 类，
        // 4scloud-web.pingan.com（其子域）被剔出分母。
        let techs = [
            "GOLISH-INTEL-DNS",
            "GOLISH-INTEL-SUBDOMAIN",
            "GOLISH-INTEL-CT",
            "GOLISH-INTEL-WHOIS",
            "GOLISH-INTEL-ASN",
            "GOLISH-INTEL-OSINT",
        ];
        // 交付物只为根域 pingan.com 给齐 6 类终态（blocked，免证据，专测分母轴）。
        let cells: Vec<CoverageCell> = techs
            .iter()
            .map(|t| cov_cell("pingan.com", t, CoverageStatus::Blocked, vec![]))
            .collect();
        let d = deliverable_with_coverage(cells);
        let ctx = GateContext {
            in_scope_assets: Some(vec![
                "pingan.com".to_string(),
                "4scloud-web.pingan.com".to_string(),
            ]),
            asset_types: None,
            expected_techniques: Some(techs.iter().map(|s| s.to_string()).collect()),
            evidence_facts: None,
            source_queries: None,
        };
        let rule = coverage_complete_rule();

        // anchor_only OFF：子域也要核 6 类，但交付物没覆盖它 → BLOCK。
        let spec_off = target_intel_anchor_spec(false);
        assert!(
            !eval_with_context(&d, &spec_off, std::slice::from_ref(&rule), &ctx)[0].is_pass(),
            "anchor_only off: discovered subdomain inflates the denominator → BLOCK"
        );

        // anchor_only ON：子域被剔出分母，只核 pingan.com（已给齐 6 类）→ PASS。
        let spec_on = target_intel_anchor_spec(true);
        assert!(
            eval_with_context(&d, &spec_on, &[rule], &ctx)[0].is_pass(),
            "anchor_only on: subdomain dropped from denominator, only root required → PASS"
        );
    }

    #[test]
    fn coverage_anchor_only_keeps_all_when_no_parent_in_axis() {
        // 锚点过滤只剔「在轴内有父根域」的子域；若轴里没有父（如错配场景：39 个子域、
        // 没有根域），它们彼此是兄弟、都保留（不凭空变空轴）。证明过滤不误伤、且
        // 非空轴永不变空。
        let techs = ["GOLISH-INTEL-DNS"];
        let ctx = GateContext {
            in_scope_assets: Some(vec!["a.pa18.com".to_string(), "b.pa18.com".to_string()]),
            asset_types: None,
            expected_techniques: Some(techs.iter().map(|s| s.to_string()).collect()),
            evidence_facts: None,
            source_queries: None,
        };
        // 只覆盖了 a，没覆盖 b → 两个都在轴里时必 BLOCK（证明 b 没被剔除）。
        let d = deliverable_with_coverage(vec![cov_cell(
            "a.pa18.com",
            "GOLISH-INTEL-DNS",
            CoverageStatus::Blocked,
            vec![],
        )]);
        let rule = coverage_complete_rule();
        let spec_on = target_intel_anchor_spec(true);
        assert!(
            !eval_with_context(&d, &spec_on, &[rule], &ctx)[0].is_pass(),
            "siblings with no in-axis parent are all kept → b.pa18.com still required → BLOCK"
        );
    }

    #[test]
    fn host_aware_coverage_relaxes_ip_not_domain() {
        // 设计 §6 parity：开关开后唯一变化 = 裸 IP 的域名专属格（SUBDOMAIN/DNS/CT）
        // BLOCK→PASS；域名仍核全 6 项。
        let techs = [
            "GOLISH-INTEL-DNS",
            "GOLISH-INTEL-SUBDOMAIN",
            "GOLISH-INTEL-CT",
            "GOLISH-INTEL-WHOIS",
            "GOLISH-INTEL-ASN",
            "GOLISH-INTEL-OSINT",
        ];
        // domain 有全部 6 个 Found 事实；IP 只有 WHOIS/ASN/OSINT（org 级）。
        let mut facts: Vec<EvidenceFact> = techs
            .iter()
            .map(|t| fact("a.com", t, EvidenceOutcome::Found, 1))
            .collect();
        for t in [
            "GOLISH-INTEL-WHOIS",
            "GOLISH-INTEL-ASN",
            "GOLISH-INTEL-OSINT",
        ] {
            facts.push(fact("1.2.3.4", t, EvidenceOutcome::Found, 1));
        }
        let ctx = GateContext {
            in_scope_assets: Some(vec!["a.com".to_string(), "1.2.3.4".to_string()]),
            asset_types: None,
            expected_techniques: Some(techs.iter().map(|s| s.to_string()).collect()),
            evidence_facts: Some(facts),
            source_queries: None,
        };
        let d = deliverable_with_coverage(vec![]);

        // Flag OFF：IP 缺 SUBDOMAIN/DNS/CT → BLOCK。
        let spec_off = target_intel_spec(false);
        assert!(
            !eval_with_context(&d, &spec_off, &[evidence_derive_rule(None)], &ctx)[0].is_pass(),
            "host-aware off: bare IP held to domain-only techniques → BLOCK"
        );

        // Flag ON：IP 只核 WHOIS/ASN/OSINT（都 Found）；域名仍全 6（都 Found）→ PASS。
        let spec_on = target_intel_spec(true);
        assert!(
            eval_with_context(&d, &spec_on, &[evidence_derive_rule(None)], &ctx)[0].is_pass(),
            "host-aware on: IP no longer asked for SUBDOMAIN/DNS/CT → PASS"
        );
    }

    // ── Host-aware coverage 2b parity: EAS + enumeration (design §3.2/§3.3) ──
    // The per-asset matrix landed inert (commit e12a7638); these two prove the
    // stage-flag flip (external_attack_surface/enumeration spec.json
    // host_aware_coverage:true) actually relaxes the gate as designed and nothing
    // else (domain/host still held to the full set).

    /// Minimal stage spec for an arbitrary kind with the host-aware flag set
    /// (kind drives the technique matrix; mirrors [`target_intel_spec`]).
    fn host_aware_spec(id: &str, kind: &str, host_aware: bool) -> StageSpec {
        crate::harness::stage_spec::load_stage_spec_from_json(&format!(
            r#"{{"id":"{id}","kind":"{kind}","risk_level":"medium",
                "deliverable_schema":"StageDeliverable","gate_validator":"validate_stage_gate",
                "host_aware_coverage":{host_aware}}}"#
        ))
        .expect("host-aware spec parses")
    }

    #[test]
    fn host_aware_coverage_relaxes_url_not_host_in_eas() {
        // 设计 §3.2 parity（2b）：EAS 开关开后唯一变化 = 裸 URL 端点的 PORT/SERVICE
        // BLOCK→PASS（其主机已由 host/IP 资产覆盖），仍核 LIVENESS；域名仍核全 3。
        let techs = [
            "GOLISH-EAS-LIVENESS",
            "GOLISH-EAS-PORT",
            "GOLISH-EAS-SERVICE-FINGERPRINT",
        ];
        // domain 有全部 3 个 Found；URL 端点只有 LIVENESS。
        let mut facts: Vec<EvidenceFact> = techs
            .iter()
            .map(|t| fact("a.com", t, EvidenceOutcome::Found, 1))
            .collect();
        facts.push(fact(
            "https://a.com/login",
            "GOLISH-EAS-LIVENESS",
            EvidenceOutcome::Found,
            1,
        ));
        let ctx = GateContext {
            in_scope_assets: Some(vec!["a.com".to_string(), "https://a.com/login".to_string()]),
            asset_types: None,
            expected_techniques: Some(techs.iter().map(|s| s.to_string()).collect()),
            evidence_facts: Some(facts),
            source_queries: None,
        };
        let d = deliverable_with_coverage(vec![]);

        // Flag OFF：URL 端点被要求 PORT/SERVICE，缺 → BLOCK。
        let spec_off = host_aware_spec("external_attack_surface", "external_attack_surface", false);
        assert!(
            !eval_with_context(&d, &spec_off, &[evidence_derive_rule(None)], &ctx)[0].is_pass(),
            "host-aware off: bare URL endpoint held to PORT/SERVICE-FINGERPRINT → BLOCK"
        );

        // Flag ON：URL 只核 LIVENESS（Found）；域名仍全 3（都 Found）→ PASS。
        let spec_on = host_aware_spec("external_attack_surface", "external_attack_surface", true);
        assert!(
            eval_with_context(&d, &spec_on, &[evidence_derive_rule(None)], &ctx)[0].is_pass(),
            "host-aware on: URL endpoint no longer asked for PORT/SERVICE-FINGERPRINT → PASS"
        );
    }

    #[test]
    fn host_aware_coverage_drops_content_enum_for_ip_in_enumeration() {
        // 设计 §3.3 parity（2b）：enumeration 开关开后唯一变化 = 裸 IP（非 web）的
        // DIR/PARAM/JSAPI BLOCK→PASS（内容枚举仅对 web 资产有意义）；域名仍核全 3。
        let techs = ["GOLISH-ENUM-DIR", "GOLISH-ENUM-PARAM", "GOLISH-ENUM-JSAPI"];
        // domain 有全部 3 个 Found；裸 IP 无内容枚举事实。
        let facts: Vec<EvidenceFact> = techs
            .iter()
            .map(|t| fact("a.com", t, EvidenceOutcome::Found, 1))
            .collect();
        let ctx = GateContext {
            in_scope_assets: Some(vec!["a.com".to_string(), "1.2.3.4".to_string()]),
            asset_types: None,
            expected_techniques: Some(techs.iter().map(|s| s.to_string()).collect()),
            evidence_facts: Some(facts),
            source_queries: None,
        };
        let d = deliverable_with_coverage(vec![]);

        // Flag OFF：裸 IP 被要求 DIR/PARAM/JSAPI，缺 → BLOCK。
        let spec_off = host_aware_spec("enumeration", "enumeration", false);
        assert!(
            !eval_with_context(&d, &spec_off, &[evidence_derive_rule(None)], &ctx)[0].is_pass(),
            "host-aware off: bare IP held to content-enumeration techniques → BLOCK"
        );

        // Flag ON：裸 IP 非内容枚举目标（web-only）→ 只剩域名（全 Found）→ PASS。
        let spec_on = host_aware_spec("enumeration", "enumeration", true);
        assert!(
            eval_with_context(&d, &spec_on, &[evidence_derive_rule(None)], &ctx)[0].is_pass(),
            "host-aware on: bare IP dropped from content enumeration → PASS"
        );
    }

    #[test]
    fn host_aware_uses_authoritative_type_over_value() {
        // 2c-1（设计 host-aware-coverage-2c §4.1）：值像 IP、但权威类型是 `domain` 的
        // 资产，必须仍核全 6 项——权威类型压过 from_value（后者会按值误判成 IP 而放松）。
        let techs = [
            "GOLISH-INTEL-DNS",
            "GOLISH-INTEL-SUBDOMAIN",
            "GOLISH-INTEL-CT",
            "GOLISH-INTEL-WHOIS",
            "GOLISH-INTEL-ASN",
            "GOLISH-INTEL-OSINT",
        ];
        let asset = "1.2.3.4"; // 值看着像 IP …
                               // 只有 IP 适用子集（WHOIS/ASN/OSINT）有 Found；SUBDOMAIN/DNS/CT 缺。
        let facts: Vec<EvidenceFact> = [
            "GOLISH-INTEL-WHOIS",
            "GOLISH-INTEL-ASN",
            "GOLISH-INTEL-OSINT",
        ]
        .iter()
        .map(|t| fact(asset, t, EvidenceOutcome::Found, 1))
        .collect();
        let mut asset_types = std::collections::HashMap::new();
        asset_types.insert(asset.to_string(), "domain".to_string()); // … 但权威类型是域名
        let ctx = GateContext {
            in_scope_assets: Some(vec![asset.to_string()]),
            asset_types: Some(asset_types),
            expected_techniques: Some(techs.iter().map(|s| s.to_string()).collect()),
            evidence_facts: Some(facts),
            source_queries: None,
        };
        let d = deliverable_with_coverage(vec![]);
        // host-aware ON：权威类型=域名 ⇒ 核全 6，缺 3 → BLOCK（若只按 from_value 会判 IP 而 PASS）。
        let spec_on = target_intel_spec(true);
        assert!(
            !eval_with_context(&d, &spec_on, &[evidence_derive_rule(None)], &ctx)[0].is_pass(),
            "authoritative type=domain holds the IP-looking asset to all 6 → BLOCK"
        );
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
    fn authoritative_empty_fact_closes_cell_without_handwritten_coverage() {
        let d = deliverable_with_coverage(vec![]);
        let ctx_empty = projection_ctx(
            &["GOLISH-EAS-LIVENESS"],
            Some(vec![fact(
                "a",
                "GOLISH-EAS-LIVENESS",
                EvidenceOutcome::Empty,
                7,
            )]),
        );

        assert!(
            eval_with_context(&d, &test_spec(), &[authoritative_rule(None)], &ctx_empty)[0]
                .is_pass(),
            "authoritative Empty evidence fact should be terminal without forcing the model to mirror it into coverage"
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
            GateCheckOutcome::Block { reasons, recovery } => {
                assert!(reasons[0].contains("(a × xss)"), "{:?}", reasons);
                assert_eq!(recovery.coverage_gap_actions.len(), 1);
                let action = &recovery.coverage_gap_actions[0];
                assert_eq!(action.asset, "a");
                assert_eq!(action.technique, "xss");
                assert_eq!(action.reason, "missing_terminal_coverage");
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
    fn coverage_complete_passes_empty_coverage_when_injected_scope_is_empty() {
        // Explicit DB truth: there are zero in-scope assets for this org/stage.
        // This is different from an agent omitting coverage with no authoritative
        // axis, and it must stay consistent with check_stage_asset_coverage.
        let rule = coverage_complete_rule();
        let spec = spec_with_expected(&["GOLISH-EAS-LIVENESS", "GOLISH-EAS-PORT"]);
        let d = deliverable_with_coverage(vec![]);
        let ctx = GateContext {
            in_scope_assets: Some(vec![]),
            ..GateContext::default()
        };
        assert!(
            eval_with_context(&d, &spec, &[rule], &ctx)[0].is_pass(),
            "explicit zero in-scope assets should be a vacuous coverage pass"
        );
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

    // ── T1 (设计 2026-06-23-coverage-note-required) · require_note_for_other ──

    fn cov_cell_noted(
        asset: &str,
        technique: &str,
        status: CoverageStatus,
        note: &str,
    ) -> CoverageCell {
        CoverageCell {
            asset: asset.to_string(),
            technique: technique.to_string(),
            status,
            evidence_refs: vec![],
            note: Some(note.to_string()),
            reason_kind: None,
            tested_units: 0,
            total_units: 0,
            sampling_rationale: None,
        }
    }

    fn note_required_rule() -> GateRule {
        parse(
            r#"{ "op":"coverage_complete","require_note_for_other":true,
                 "on_fail":{"reason":"coverage incomplete"} }"#,
        )
    }

    #[test]
    fn require_note_for_other_defaults_false_blocked_without_note_passes() {
        // 缺省（不写该字段）→ blocked 空 note 仍算终态 → Pass（逐字节不变）。
        let rule = coverage_complete_rule();
        let spec = spec_with_expected(&["idor"]);
        let d =
            deliverable_with_coverage(vec![cov_cell("a", "idor", CoverageStatus::Blocked, vec![])]);
        assert!(eval(&d, &spec, &[rule])[0].is_pass());
    }

    #[test]
    fn require_note_for_other_blocks_blocked_cell_without_note() {
        // 开关开 + blocked 空 note → 不算终态 → a×idor 缺口 → Block。
        let spec = spec_with_expected(&["idor"]);
        let d =
            deliverable_with_coverage(vec![cov_cell("a", "idor", CoverageStatus::Blocked, vec![])]);
        match &eval(&d, &spec, &[note_required_rule()])[0] {
            GateCheckOutcome::Block { reasons, .. } => {
                assert!(reasons[0].contains("(a × idor)"), "{reasons:?}");
            }
            GateCheckOutcome::Pass => {
                panic!("blocked cell without note must BLOCK when require_note_for_other")
            }
        }
    }

    #[test]
    fn require_note_for_other_passes_blocked_cell_with_note() {
        // 开关开 + blocked 带非空 note → 终态 → Pass。
        let spec = spec_with_expected(&["idor"]);
        let d = deliverable_with_coverage(vec![cov_cell_noted(
            "a",
            "idor",
            CoverageStatus::Blocked,
            "WAF 403 on every payload",
        )]);
        assert!(eval(&d, &spec, &[note_required_rule()])[0].is_pass());
    }

    #[test]
    fn require_note_for_other_whitespace_note_does_not_count() {
        // 仅空白 note 视同空（trim 后为空）→ Block。
        let spec = spec_with_expected(&["idor"]);
        let d = deliverable_with_coverage(vec![cov_cell_noted(
            "a",
            "idor",
            CoverageStatus::Blocked,
            "   ",
        )]);
        assert!(matches!(
            eval(&d, &spec, &[note_required_rule()])[0],
            GateCheckOutcome::Block { .. }
        ));
    }

    #[test]
    fn require_note_for_other_applies_to_not_applicable_too() {
        let spec = spec_with_expected(&["idor"]);
        // not_applicable 空 note → Block。
        let d_empty = deliverable_with_coverage(vec![cov_cell(
            "a",
            "idor",
            CoverageStatus::NotApplicable,
            vec![],
        )]);
        assert!(matches!(
            eval(&d_empty, &spec, &[note_required_rule()])[0],
            GateCheckOutcome::Block { .. }
        ));
        // not_applicable 带 note → Pass。
        let d_noted = deliverable_with_coverage(vec![cov_cell_noted(
            "a",
            "idor",
            CoverageStatus::NotApplicable,
            "no auth surface on a static asset",
        )]);
        assert!(eval(&d_noted, &spec, &[note_required_rule()])[0].is_pass());
    }

    // ── T2 (设计 2026-06-23-failure-outcome-not-checked-empty) · Error 事实 ──

    #[test]
    fn error_fact_is_terminal_under_derive_from_evidence() {
        // derive_from_evidence + Error 事实 + 默认终态集（含 CheckedEmpty/Blocked）→
        // cell 落终态（不 gap）→ Pass。保住「失败也落终态、不无限重试」。
        let rule = evidence_derive_rule(None);
        let ctx = projection_ctx(
            &["t"],
            Some(vec![fact("a", "t", EvidenceOutcome::Error, 9)]),
        );
        assert!(
            eval_with_context(
                &deliverable_with_coverage(vec![]),
                &test_spec(),
                &[rule],
                &ctx
            )[0]
            .is_pass(),
            "an Error fact must make the cell terminal (no infinite retry)"
        );
    }

    #[test]
    fn error_fact_does_not_satisfy_found_only_terminal() {
        // terminal=["found"]：Error 事实既不算 found，也不在 CheckedEmpty/Blocked 终态
        // 集 → 缺口 Block。证明 error 绝不冒充 found。
        let rule = evidence_derive_rule(Some(r#""found""#));
        let ctx = projection_ctx(
            &["t"],
            Some(vec![fact("a", "t", EvidenceOutcome::Error, 9)]),
        );
        match &eval_with_context(
            &deliverable_with_coverage(vec![]),
            &test_spec(),
            &[rule],
            &ctx,
        )[0]
        {
            GateCheckOutcome::Block { reasons, .. } => {
                assert!(reasons[0].contains("(a × t)"), "{reasons:?}");
            }
            GateCheckOutcome::Pass => panic!("error fact must not satisfy a found-only terminal"),
        }
    }

    #[test]
    fn error_fact_inert_without_derive_from_evidence() {
        // 不开 derive_from_evidence：Error 事实不投影 → a×t 无终态 → Block（gate 侧
        // 对 error 的处理 additive，需 derive_from_evidence 才消费）。
        let rule = coverage_complete_rule();
        let ctx = projection_ctx(
            &["t"],
            Some(vec![fact("a", "t", EvidenceOutcome::Error, 9)]),
        );
        assert!(matches!(
            eval_with_context(
                &deliverable_with_coverage(vec![]),
                &test_spec(),
                &[rule],
                &ctx
            )[0],
            GateCheckOutcome::Block { .. }
        ));
    }

    // ── Source coverage (2026-06-23 provider-source closure) ────────────────

    fn source_query(
        source: &str,
        query: &str,
        technique: Option<&str>,
        status: &str,
    ) -> SourceQueryFact {
        SourceQueryFact {
            source: source.to_string(),
            query: query.to_string(),
            target: String::new(),
            technique: technique.map(str::to_string),
            status: status.to_string(),
            evidence_ids: vec![9],
        }
    }

    fn source_coverage_rule(techniques: Option<&[&str]>) -> GateRule {
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
            r#"{{ "op":"source_coverage"{techs},
                 "on_fail":{{"reason":"source coverage incomplete"}} }}"#
        ))
    }

    #[test]
    fn source_coverage_blocks_found_without_terminal_source_row() {
        let d = deliverable_with_coverage(vec![cov_cell(
            "a.com",
            "GOLISH-INTEL-WHOIS",
            CoverageStatus::Found,
            vec![1],
        )]);
        let ctx = GateContext {
            in_scope_assets: Some(vec!["a.com".to_string()]),
            expected_techniques: Some(vec!["GOLISH-INTEL-WHOIS".to_string()]),
            ..Default::default()
        };
        assert!(matches!(
            eval_with_context(
                &d,
                &target_intel_spec(false),
                &[source_coverage_rule(None)],
                &ctx
            )[0],
            GateCheckOutcome::Block { .. }
        ));
    }

    #[test]
    fn source_coverage_accepts_provider_survey_for_provider_backed_intel() {
        let techs = [
            "GOLISH-INTEL-DNS",
            "GOLISH-INTEL-SUBDOMAIN",
            "GOLISH-INTEL-ASN",
            "GOLISH-INTEL-CT",
            "GOLISH-INTEL-OSINT",
        ];
        let ctx = GateContext {
            in_scope_assets: Some(vec!["a.com".to_string()]),
            expected_techniques: Some(techs.iter().map(|s| s.to_string()).collect()),
            source_queries: Some(vec![source_query(
                "builtin_provider",
                "map_assets",
                None,
                "found",
            )]),
            ..Default::default()
        };
        assert!(
            eval_with_context(
                &deliverable_with_coverage(vec![]),
                &target_intel_spec(false),
                &[source_coverage_rule(None)],
                &ctx
            )[0]
            .is_pass(),
            "a terminal recon_map_assets provider row proves provider-backed sources were attempted"
        );
    }

    #[test]
    fn source_coverage_accepts_rdap_for_whois() {
        let ctx = GateContext {
            in_scope_assets: Some(vec!["a.com".to_string()]),
            expected_techniques: Some(vec!["GOLISH-INTEL-WHOIS".to_string()]),
            source_queries: Some(vec![source_query(
                "rdap",
                "lookup_whois",
                Some("GOLISH-INTEL-WHOIS"),
                "empty",
            )]),
            ..Default::default()
        };
        assert!(
            eval_with_context(
                &deliverable_with_coverage(vec![]),
                &target_intel_spec(false),
                &[source_coverage_rule(None)],
                &ctx
            )[0]
            .is_pass(),
            "terminal RDAP source row proves WHOIS was attempted even when empty"
        );
    }

    #[test]
    fn source_coverage_exempts_not_applicable_with_note() {
        let d = deliverable_with_coverage(vec![cov_cell_noted(
            "a.com",
            "GOLISH-INTEL-OSINT",
            CoverageStatus::NotApplicable,
            "no OSINT provider configured for this engagement",
        )]);
        let ctx = GateContext {
            in_scope_assets: Some(vec!["a.com".to_string()]),
            expected_techniques: Some(vec!["GOLISH-INTEL-OSINT".to_string()]),
            ..Default::default()
        };
        assert!(
            eval_with_context(
                &d,
                &target_intel_spec(false),
                &[source_coverage_rule(None)],
                &ctx
            )[0]
            .is_pass(),
            "explicit not_applicable+note remains terminal without forcing a source call"
        );
    }

    #[test]
    fn source_query_fact_does_not_make_authoritative_found_pass() {
        let d = deliverable_with_coverage(vec![cov_cell(
            "a.com",
            "GOLISH-INTEL-CT",
            CoverageStatus::Found,
            vec![1],
        )]);
        let ctx = GateContext {
            in_scope_assets: Some(vec!["a.com".to_string()]),
            expected_techniques: Some(vec!["GOLISH-INTEL-CT".to_string()]),
            source_queries: Some(vec![source_query(
                "builtin_provider",
                "map_assets",
                None,
                "found",
            )]),
            ..Default::default()
        };
        assert!(matches!(
            eval_with_context(
                &d,
                &target_intel_spec(false),
                &[authoritative_rule(None)],
                &ctx
            )[0],
            GateCheckOutcome::Block { .. }
        ));
    }

    #[test]
    fn source_query_empty_closes_authoritative_coverage_as_checked_empty() {
        let ctx = GateContext {
            in_scope_assets: Some(vec!["a.com".to_string(), "b.com".to_string()]),
            expected_techniques: Some(vec!["GOLISH-INTEL-WHOIS".to_string()]),
            source_queries: Some(vec![source_query(
                "rdap",
                "lookup_whois",
                Some("GOLISH-INTEL-WHOIS"),
                "empty",
            )]),
            ..Default::default()
        };
        assert!(
            eval_with_context(
                &deliverable_with_coverage(vec![]),
                &target_intel_spec(false),
                &[authoritative_rule(None)],
                &ctx
            )[0]
            .is_pass(),
            "an exact terminal RDAP empty row should close WHOIS cells as checked_empty, not require hand-written per-asset cells"
        );
    }

    #[test]
    fn source_query_found_does_not_close_authoritative_coverage_without_db_truth() {
        let ctx = GateContext {
            in_scope_assets: Some(vec!["a.com".to_string()]),
            expected_techniques: Some(vec!["GOLISH-INTEL-WHOIS".to_string()]),
            source_queries: Some(vec![source_query(
                "rdap",
                "lookup_whois",
                Some("GOLISH-INTEL-WHOIS"),
                "found",
            )]),
            ..Default::default()
        };
        assert!(matches!(
            eval_with_context(
                &deliverable_with_coverage(vec![]),
                &target_intel_spec(false),
                &[authoritative_rule(None)],
                &ctx
            )[0],
            GateCheckOutcome::Block { .. }
        ));
    }

    #[test]
    fn provider_survey_found_closes_provider_backed_coverage_as_non_found_terminal() {
        let ctx = GateContext {
            in_scope_assets: Some(vec!["www.a.com".to_string()]),
            expected_techniques: Some(vec!["GOLISH-INTEL-SUBDOMAIN".to_string()]),
            source_queries: Some(vec![source_query("quake", "map_assets", None, "found")]),
            ..Default::default()
        };
        assert!(
            eval_with_context(
                &deliverable_with_coverage(vec![]),
                &target_intel_spec(false),
                &[authoritative_rule(None)],
                &ctx
            )[0]
            .is_pass(),
            "an org-level provider survey row should close provider-backed cells the DB did not mark found"
        );
    }

    #[test]
    fn provider_survey_does_not_close_whois_coverage() {
        let ctx = GateContext {
            in_scope_assets: Some(vec!["a.com".to_string()]),
            expected_techniques: Some(vec!["GOLISH-INTEL-WHOIS".to_string()]),
            source_queries: Some(vec![source_query("quake", "map_assets", None, "found")]),
            ..Default::default()
        };
        assert!(matches!(
            eval_with_context(
                &deliverable_with_coverage(vec![]),
                &target_intel_spec(false),
                &[authoritative_rule(None)],
                &ctx
            )[0],
            GateCheckOutcome::Block { .. }
        ));
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
            reason_kind: None,
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
                authoritative,
                min_sample_ratio_pct,
                ..
            } => {
                assert!(!authoritative);
                assert_eq!(min_sample_ratio_pct, 100);
            }
            _ => panic!("expected CoverageDenominator"),
        }
    }

    #[test]
    fn coverage_denominator_authoritative_is_noop() {
        let rule = parse(
            r#"{ "op":"coverage_denominator","authoritative":true,
                 "on_fail":{"reason":"coverage below denominator"} }"#,
        );
        let d = deliverable_with_coverage(vec![cov_cell_dn(
            "a",
            "GOLISH-EAS-SERVICE-FINGERPRINT",
            CoverageStatus::Found,
            vec![1],
            0,
            0,
            None,
        )]);
        assert!(
            eval_one(&d, &test_spec(), &GateContext::default(), &rule).is_pass(),
            "authoritative DB-truth stages should not require hand-copied denominator fields"
        );
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

    // ── E1 PR-B: 资产身份漂移归一（canonical_asset_key join 修复） ──────────────

    #[test]
    fn coverage_complete_canonicalizes_asset_identity_across_drift() {
        // 设计 2026-06-18-canonical-asset-identity：同一资产在轴里是 `pingan.com`、
        // 在账本 fact 里是带 scheme+大写+尾点的漂移写法 `HTTPS://Pingan.com.`。归一前
        // 字面不等 → has_fact 不命中 → never_attempted BLOCK（死循环根）；归一后对上 → PASS。
        let techs = ["GOLISH-INTEL-DNS"];
        let facts = vec![fact(
            "HTTPS://Pingan.com.",
            "GOLISH-INTEL-DNS",
            EvidenceOutcome::Found,
            1,
        )];
        let ctx = GateContext {
            in_scope_assets: Some(vec!["pingan.com".to_string()]),
            asset_types: None,
            expected_techniques: Some(techs.iter().map(|s| s.to_string()).collect()),
            evidence_facts: Some(facts),
            source_queries: None,
        };
        let d = deliverable_with_coverage(vec![]);
        assert!(
            eval_with_context(
                &d,
                &target_intel_spec(false),
                &[evidence_derive_rule(None)],
                &ctx
            )[0]
            .is_pass(),
            "drifted-identity evidence fact must credit the in-scope asset after canonicalization"
        );
    }

    #[test]
    fn coverage_complete_drift_does_not_over_merge_distinct_assets() {
        // 反作弊 parity：归一只让「同一资产不同写法」对上，绝不把「不同资产」混为一格。
        // 轴是 pingan.com，账本只有 other.com 的 fact → 仍 BLOCK。
        let techs = ["GOLISH-INTEL-DNS"];
        let facts = vec![fact(
            "other.com",
            "GOLISH-INTEL-DNS",
            EvidenceOutcome::Found,
            1,
        )];
        let ctx = GateContext {
            in_scope_assets: Some(vec!["pingan.com".to_string()]),
            asset_types: None,
            expected_techniques: Some(techs.iter().map(|s| s.to_string()).collect()),
            evidence_facts: Some(facts),
            source_queries: None,
        };
        let d = deliverable_with_coverage(vec![]);
        assert!(
            !eval_with_context(
                &d,
                &target_intel_spec(false),
                &[evidence_derive_rule(None)],
                &ctx
            )[0]
            .is_pass(),
            "a fact for a different asset must NOT satisfy pingan.com (no over-merge)"
        );
    }

    #[test]
    fn coverage_complete_dedups_drifted_in_scope_axis() {
        // B1：in-scope 轴里同一资产的漂移写法折叠成一行。无事实 → BLOCK，但缺口只数到
        // 一行（不是每个漂移写法各报一个），证明轴已按规范身份去重。
        let techs = ["GOLISH-INTEL-DNS"];
        let ctx = GateContext {
            in_scope_assets: Some(vec![
                "pingan.com".to_string(),
                "http://pingan.com".to_string(),
                "PINGAN.COM.".to_string(),
            ]),
            asset_types: None,
            expected_techniques: Some(techs.iter().map(|s| s.to_string()).collect()),
            evidence_facts: None,
            source_queries: None,
        };
        let d = deliverable_with_coverage(vec![]);
        match &eval_with_context(
            &d,
            &target_intel_spec(false),
            &[evidence_derive_rule(None)],
            &ctx,
        )[0]
        {
            GateCheckOutcome::Block { reasons, .. } => {
                let gaps = reasons[0].matches("GOLISH-INTEL-DNS").count();
                assert_eq!(
                    gaps, 1,
                    "drift-duplicate in-scope rows must collapse to one gap: {reasons:?}"
                );
            }
            GateCheckOutcome::Pass => panic!("no evidence facts must BLOCK"),
        }
    }

    #[test]
    fn coverage_complete_dedup_preserves_distinct_eas_endpoint() {
        // B1 反作弊：去重只折叠漂移写法，绝不把 EAS 的 URL 端点折进其主机——
        // `a.com` 与 `https://a.com/login` 是两行，各自要核 → 缺一即 BLOCK。
        let techs = ["GOLISH-EAS-LIVENESS"];
        let facts = vec![fact(
            "a.com",
            "GOLISH-EAS-LIVENESS",
            EvidenceOutcome::Found,
            1,
        )];
        let ctx = GateContext {
            in_scope_assets: Some(vec!["a.com".to_string(), "https://a.com/login".to_string()]),
            asset_types: None,
            expected_techniques: Some(techs.iter().map(|s| s.to_string()).collect()),
            evidence_facts: Some(facts),
            source_queries: None,
        };
        let d = deliverable_with_coverage(vec![]);
        let spec = host_aware_spec("external_attack_surface", "external_attack_surface", false);
        assert!(
            !eval_with_context(&d, &spec, &[evidence_derive_rule(None)], &ctx)[0].is_pass(),
            "URL endpoint must stay a distinct row (only a.com has the fact) → BLOCK"
        );
    }

    #[test]
    fn coverage_complete_liveness_fact_must_preserve_url_port_endpoint() {
        let techs = ["GOLISH-EAS-LIVENESS"];
        let d = deliverable_with_coverage(vec![]);
        let spec = host_aware_spec("external_attack_surface", "external_attack_surface", false);

        let host_only_ctx = GateContext {
            in_scope_assets: Some(vec!["http://linquankuaipin.com:90".to_string()]),
            asset_types: None,
            expected_techniques: Some(techs.iter().map(|s| s.to_string()).collect()),
            evidence_facts: Some(vec![fact(
                "linquankuaipin.com",
                "GOLISH-EAS-LIVENESS",
                EvidenceOutcome::Found,
                1,
            )]),
            source_queries: None,
        };
        assert!(
            !eval_with_context(&d, &spec, &[evidence_derive_rule(None)], &host_only_ctx)[0]
                .is_pass(),
            "host-only liveness must not close a distinct URL:port endpoint"
        );

        let endpoint_ctx = GateContext {
            in_scope_assets: Some(vec!["http://linquankuaipin.com:90".to_string()]),
            asset_types: None,
            expected_techniques: Some(techs.iter().map(|s| s.to_string()).collect()),
            evidence_facts: Some(vec![fact(
                "linquankuaipin.com:90",
                "GOLISH-EAS-LIVENESS",
                EvidenceOutcome::Found,
                1,
            )]),
            source_queries: None,
        };
        assert!(
            eval_with_context(&d, &spec, &[evidence_derive_rule(None)], &endpoint_ctx)[0].is_pass(),
            "endpoint liveness fact must close the matching URL:port cell"
        );
    }

    #[test]
    fn coverage_corroborated_canonicalizes_asset_identity() {
        // found cell 的 asset(`http://pingan.com`) 与佐证 claim 的 subject(`PINGAN.COM.`)
        // 是同一资产的不同写法 → 归一后判已佐证 PASS。
        let rule = parse(
            r#"{ "op":"coverage_corroborated","on_fail":{"reason":"found needs corroboration"} }"#,
        );
        let mut d = deliverable_with_coverage(vec![cov_cell(
            "http://pingan.com",
            "GOLISH-INTEL-DNS",
            CoverageStatus::Found,
            vec![1],
        )]);
        d.claims.push(StageClaim {
            kind: "dns_a_record".to_string(),
            subject: "PINGAN.COM.".to_string(),
            summary: "A 1.2.3.4".to_string(),
            evidence_ids: vec![],
            technique: Some("GOLISH-INTEL-DNS".to_string()),
        });
        assert!(
            eval(&d, &test_spec(), &[rule])[0].is_pass(),
            "drifted cell.asset vs claim.subject must corroborate after canonicalization"
        );
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

    #[test]
    fn coverage_corroborated_authoritative_is_noop() {
        // Phase C (设计 2026-06-22 瘦身交付物): authoritative stages adjudicate
        // coverage from DB truth, so a slim deliverable's found cell with NO
        // technique-tagged claim/finding must NOT block (paired opposite of
        // `coverage_corroborated_blocks_unbacked_found_cell`, same input).
        let rule = parse(
            r#"{ "op":"coverage_corroborated", "authoritative":true,
                 "on_fail":{"reason":"found cells must be corroborated by technique-tagged items"} }"#,
        );
        let d = deliverable_with_coverage(vec![cov_cell(
            "example.com",
            "GOLISH-INTEL-DNS",
            CoverageStatus::Found,
            vec![1],
        )]);
        assert!(
            eval(&d, &test_spec(), &[rule])[0].is_pass(),
            "authoritative coverage_corroborated must no-op (slim deliverable)"
        );
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
