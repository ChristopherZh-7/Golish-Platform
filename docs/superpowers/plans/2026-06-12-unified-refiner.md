# 统一 Refiner（纠错通道收敛）实现计划

> **面向 AI 代理的工作者：** 必需子技能：使用 superpowers:executing-plans 逐任务实现此计划。

**目标：** gate BLOCK 后的全部纠错收敛到唯一模块 `refiner.rs`（确定性分类 + 每类独立 prompt 模板 + submit-only 锁决策），并砍掉两个「后端代为合成 StageDeliverable」的兜底。
**架构：** 见 `docs/design/2026-06-12-unified-refiner.md`。gate / enforce_* 只产「事实」（reasons、fabricated ids、missing kinds、expired ids…），渲染权全部上收 Refiner 纯函数；execute.rs 重试循环只消费 `RefineDecision { class, correction, submit_only_lock }`。deliverable 永远出自主 agent（红线）。
**技术栈：** Rust 2021，`golish-agent-kit` crate，cargo nextest，TDD。

**前置确认（执行者必读）**：
- 行号基于 2026-06-12 工作树（`execute.rs` 3567 行）；执行时以符号搜索为准，行号仅作导航。
- 每个 PR 结束跑 `just precommit`；中途任务跑 `cargo nextest run -p golish-agent-kit --status-level fail`。
- `backend/crates/golish-pentest/src/handlers/env.rs` 存在一个未提交的 `let mut` 修复（E0384，编译必需），勿回退它。
- 高风险注意：本计划**删除** `synthesize_from_evidence` / `synthesize_confirm_only_deliverable` 及 `stage_spec` 字段——已获用户 2026-06-12 批准（设计文档 §7 D1-D3），无需再次确认。

---

## 文件结构

| 文件 | 职责 | 动作 |
|---|---|---|
| `backend/crates/golish-agent-kit/src/task_orchestrator/refiner.rs` | 新模块：RefineClass/RefineInput/RefineDecision + `classify` + 6 个 `render_*` 模板 + 单测 | 新建 |
| `backend/crates/golish-agent-kit/src/task_orchestrator/mod.rs` | 挂 `mod refiner;` | 修改 |
| `.../task_orchestrator/subtask_phases/execute.rs` | `HarnessGateOutcome` 增事实字段；enforce_* 改置标记；两个 gate 调用点接 Refiner；删两个 synthesize_* 与投影/confirm-only 分支；删 reflect() 调用路径 | 修改 |
| `.../task_orchestrator/subtask_phases/execute_harness_loop_tests.rs` | `HarnessGateOutcome` 构造处补新字段 | 修改 |
| `backend/crates/golish-agent-kit/src/harness/stage_spec.rs` | 删 `synthesize_from_evidence_when_missing` 字段 + 2 个单测 | 修改 |
| `backend/crates/golish-agent-kit/resources/harness/stages/target_intel.json`（以 Glob 实际路径为准） | 删 `"synthesize_from_evidence_when_missing": true` | 修改 |
| `.../task_orchestrator/types.rs` + `prompts/pipeline.rs` | `reflect()` / reflector prompts 标 `#[deprecated]` | 修改 |
| `docs/design/2026-06-11-substantive-stage-evidence-projection-fallback.md` | 头部加 Superseded 注记 | 修改 |

---

## PR-R1 · Refiner 骨架 + 接线（行为≈现状，修复两处互打）

### Task 1：refiner.rs 类型 + 分类器（TDD）

**文件**：新建 `backend/crates/golish-agent-kit/src/task_orchestrator/refiner.rs`；`mod.rs` 加 `mod refiner;`（紧邻 `mod subtask_phases;`）。

**步骤 1.1** 写类型与分类器骨架（先空实现 `todo!()`）：

```rust
//! 统一 Refiner（设计 2026-06-12-unified-refiner）：gate 判错后的唯一纠错通道。
//! 确定性分类（按危害优先级取主因）→ 每类独立 prompt 模板 → submit-only 锁决策。
//! 红线：只产纠正文本，绝不合成 StageDeliverable。

use std::collections::HashMap;

use crate::harness::gate::rule_engine::EvidenceFact;
use crate::harness::StageKind;

/// 主因分类（HarnessTrace / 日志可观测）。优先级 = 枚举声明序。
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub(crate) enum RefineClass {
    /// D · 引用了账本不存在的 evidence id（伪造，最高危）。
    Fabricated,
    /// A · missing deliverable，但活已干（账本有真 ids）或属 confirm-only 阶段
    /// （无扫描工具，唯一动作就是 submit）→ 锁 tool_choice。
    SubmitOnly,
    /// B · missing deliverable 且账本空（活没干）→ 重做。
    RedoStage,
    /// C · 交了但 vacuous / coverage 缺口 → 诊断式（DB 真值现状 + 命令）。
    CoverageOrVacuous,
    /// E · 缺 required evidence kinds / 引用硬过期证据。
    EvidenceQuality,
    /// G · red_team scoping 流程缺失（enforce_scoping_red_team_flow 已产文本，透传）。
    ScopingFlow,
    /// 兜底：其它 BLOCK 原因。
    Generic,
}

/// gate + enforce_* 产出的全部「事实」。分类与渲染的唯一输入，无 IO。
pub(crate) struct RefineInput<'a> {
    pub stage: StageKind,
    pub gate_reasons: &'a [String],
    pub gate_recovery: Option<&'a crate::harness::HarnessRecoveryActions>,
    pub missing_deliverable: bool,
    /// `StageSpec.allowed_tool_types.is_empty()`（scoping / reporting）。
    pub confirm_only_stage: bool,
    pub fabricated_ids: &'a [i64],
    /// 账本真实 ids（newest first，gather_missing_deliverable_ids 或
    /// enforce_evidence_existence 填）。
    pub available_real_ids: &'a [i64],
    /// id → evidence kind 标签（A 类模板 `#2247 (dns_a)` 用）。
    pub evidence_kind_labels: &'a HashMap<i64, String>,
    pub missing_kinds: &'a [String],
    pub expired_ids: &'a [i64],
    pub red_team_flow_correction: Option<&'a str>,
    /// C 类诊断用（含 DB 哨兵 facts），与注入 gate 的同一份。
    pub evidence_facts: Option<&'a [EvidenceFact]>,
}

pub(crate) struct RefineDecision {
    pub class: RefineClass,
    pub correction: String,
    pub submit_only_lock: bool,
}

/// 纯函数主入口：分类 → 渲染主因模板（+次因附录一行）→ 锁决策。
pub(crate) fn refine(input: &RefineInput<'_>) -> RefineDecision {
    let class = classify(input);
    let mut correction = match class {
        RefineClass::Fabricated => render_fabricated(input),
        RefineClass::SubmitOnly => render_submit_only(input),
        RefineClass::RedoStage => render_redo_stage(input),
        RefineClass::CoverageOrVacuous => render_coverage_or_vacuous(input),
        RefineClass::EvidenceQuality => render_evidence_quality(input),
        RefineClass::ScopingFlow => input
            .red_team_flow_correction
            .unwrap_or_default()
            .to_string(),
        RefineClass::Generic => render_generic(input),
    };
    if let Some(note) = secondary_note(input, class) {
        correction.push_str(&note);
    }
    RefineDecision {
        class,
        correction,
        submit_only_lock: matches!(class, RefineClass::SubmitOnly),
    }
}

fn classify(input: &RefineInput<'_>) -> RefineClass {
    if !input.fabricated_ids.is_empty() {
        return RefineClass::Fabricated;
    }
    if input.missing_deliverable {
        if input.confirm_only_stage || !input.available_real_ids.is_empty() {
            return RefineClass::SubmitOnly;
        }
        return RefineClass::RedoStage;
    }
    if reasons_hit_coverage_or_vacuous(input.gate_reasons) {
        return RefineClass::CoverageOrVacuous;
    }
    if !input.missing_kinds.is_empty() || !input.expired_ids.is_empty() {
        return RefineClass::EvidenceQuality;
    }
    if input.red_team_flow_correction.is_some() {
        return RefineClass::ScopingFlow;
    }
    RefineClass::Generic
}

/// vacuous + coverage(complete/corroborated/denominator) 全走 C 类（设计 §5.1：
/// PR-C 诊断从「仅 coverage」扩展到 vacuous——两连 BLOCK 缺的那块）。
fn reasons_hit_coverage_or_vacuous(reasons: &[String]) -> bool {
    reasons.iter().any(|r| {
        r.contains("vacuous")
            || r.contains("GOLISH-INTEL-")
            || r.contains("never attempted")
            || r.contains("corroborat")
    })
}

/// 主因之外的并存问题压成一行附录（设计 §5.2，防信号丢失又不回到大杂烩）。
fn secondary_note(input: &RefineInput<'_>, class: RefineClass) -> Option<String> {
    if class == RefineClass::EvidenceQuality {
        return None;
    }
    let mut parts: Vec<String> = Vec::new();
    if !input.missing_kinds.is_empty() {
        parts.push(format!("missing evidence kinds {:?}", input.missing_kinds));
    }
    if !input.expired_ids.is_empty() {
        parts.push(format!("hard-expired evidence ids {:?}", input.expired_ids));
    }
    if parts.is_empty() {
        None
    } else {
        Some(format!("\n\nAlso fix: {}.", parts.join("; ")))
    }
}
```

（`render_*` 本任务先 `fn render_x(_: &RefineInput<'_>) -> String { String::new() }` 占位，Task 2 填实。）

**步骤 1.2** 同文件写分类器单测（先红）：

```rust
#[cfg(test)]
mod tests {
    use super::*;
    use std::collections::HashMap;

    fn base_input<'a>(
        reasons: &'a [String],
        kinds: &'a HashMap<i64, String>,
    ) -> RefineInput<'a> {
        RefineInput {
            stage: StageKind::TargetIntel,
            gate_reasons: reasons,
            gate_recovery: None,
            missing_deliverable: false,
            confirm_only_stage: false,
            fabricated_ids: &[],
            available_real_ids: &[],
            evidence_kind_labels: kinds,
            missing_kinds: &[],
            expired_ids: &[],
            red_team_flow_correction: None,
            evidence_facts: None,
        }
    }

    #[test]
    fn fabricated_wins_over_everything() {
        let reasons = vec!["deliverable vacuous: no claims".to_string()];
        let kinds = HashMap::new();
        let mut i = base_input(&reasons, &kinds);
        i.fabricated_ids = &[1, 2];
        i.missing_deliverable = true;
        let d = refine(&i);
        assert_eq!(d.class, RefineClass::Fabricated);
        assert!(!d.submit_only_lock);
    }

    #[test]
    fn missing_with_real_ids_locks_submit_only() {
        let kinds = HashMap::new();
        let mut i = base_input(&[], &kinds);
        i.missing_deliverable = true;
        i.available_real_ids = &[2247, 2245];
        let d = refine(&i);
        assert_eq!(d.class, RefineClass::SubmitOnly);
        assert!(d.submit_only_lock, "submit-only 锁必须触发（修复截胡 bug 的回归锚点）");
    }

    #[test]
    fn confirm_only_missing_locks_submit_only_without_ids() {
        let kinds = HashMap::new();
        let mut i = base_input(&[], &kinds);
        i.missing_deliverable = true;
        i.confirm_only_stage = true;
        let d = refine(&i);
        assert_eq!(d.class, RefineClass::SubmitOnly);
        assert!(d.submit_only_lock);
    }

    #[test]
    fn missing_with_empty_ledger_redoes_stage() {
        let kinds = HashMap::new();
        let mut i = base_input(&[], &kinds);
        i.missing_deliverable = true;
        let d = refine(&i);
        assert_eq!(d.class, RefineClass::RedoStage);
        assert!(!d.submit_only_lock);
    }

    #[test]
    fn vacuous_routes_to_coverage_class() {
        let reasons =
            vec!["deliverable vacuous: no claims, no findings, no skipped_checks".to_string()];
        let kinds = HashMap::new();
        let d = refine(&base_input(&reasons, &kinds));
        assert_eq!(d.class, RefineClass::CoverageOrVacuous);
    }

    #[test]
    fn coverage_never_attempted_routes_to_coverage_class() {
        let reasons = vec!["GOLISH-INTEL-DNS on *.moresec.cn never attempted".to_string()];
        let kinds = HashMap::new();
        let d = refine(&base_input(&reasons, &kinds));
        assert_eq!(d.class, RefineClass::CoverageOrVacuous);
    }

    #[test]
    fn quality_marks_route_to_evidence_quality() {
        let kinds = HashMap::new();
        let missing = vec!["dns_a".to_string()];
        let mut i = base_input(&[], &kinds);
        i.missing_kinds = &missing;
        let d = refine(&i);
        assert_eq!(d.class, RefineClass::EvidenceQuality);
    }

    #[test]
    fn secondary_note_appends_when_quality_coexists() {
        let reasons = vec!["deliverable vacuous: no claims".to_string()];
        let kinds = HashMap::new();
        let expired = [99i64];
        let mut i = base_input(&reasons, &kinds);
        i.expired_ids = &expired;
        let d = refine(&i);
        assert_eq!(d.class, RefineClass::CoverageOrVacuous);
        assert!(d.correction.contains("Also fix:"));
    }
}
```

**验证**：

```bash
cd backend && cargo nextest run -p golish-agent-kit -E 'test(refiner)' --status-level fail
```

预期：步骤 1.1 后编译过、1.2 测试全绿（分类器无占位逻辑，模板空字符串不影响分类断言）。

**提交**：`feat(agent-kit): refiner skeleton — deterministic correction classifier (design 2026-06-12-unified-refiner)`

### Task 2：六个模板渲染函数（文本迁移，TDD）

**文件**：`refiner.rs`。文本素材**逐字迁移**自 `execute.rs` 现有函数（迁移源行号）：

| render fn | 迁移源 | 备注 |
|---|---|---|
| `render_fabricated` | `block_outcome_for_fabricated` 的 `real_ids_hint` + `correction`（execute.rs:1628-1644） | 入参换 `input.fabricated_ids` / `input.available_real_ids` |
| `render_submit_only` | `build_submit_only_correction`（execute.rs:1656-1679） | id 标签用 `input.evidence_kind_labels`；**新增** confirm-only 变体（下方全文） |
| `render_redo_stage` | `missing_deliverable_gate_outcome` 的 correction（execute.rs:2396-2403） | |
| `render_coverage_or_vacuous` | `build_gate_correction` 素体（2148-2181）+ `build_db_truth_diagnosis` 调用 + `PASSIVE_INTEL_TECHNIQUES` 命令段（2188-2205） | **无论 vacuous 还是 coverage 都附诊断段**（设计 §5.1 C 类扩展） |
| `render_evidence_quality` | `enforce_evidence_kinds`（1529-1533）+ `enforce_evidence_freshness`（1590-1594）的 correction 文本 | 两者都中时各一段 |
| `render_generic` | `build_gate_correction` 素体（2148-2181），不附诊断段 | |

confirm-only 变体（`render_submit_only` 内 `input.confirm_only_stage && input.available_real_ids.is_empty()` 分支）全文：

```rust
format!(
    "The '{stage}' stage is confirm-only: it runs NO scan tools, so there is no \
     evidence to collect and nothing to re-do. Your ONLY remaining action is to call \
     the `submit_stage_deliverable` tool ONCE with a StageDeliverable containing a \
     single confirmation claim for this stage (evidence_ids may be empty for a \
     confirm-only stage). Do NOT run tools, do NOT narrate — just submit.",
    stage = input.stage.as_str(),
)
```

**步骤 2.1** 先写模板单测（红）：每类断言关键要素——

```rust
#[test]
fn submit_only_template_lists_real_ids_with_kind_labels() {
    let mut kinds = HashMap::new();
    kinds.insert(2247i64, "dns_a".to_string());
    let mut i = base_input(&[], &kinds);
    i.missing_deliverable = true;
    i.available_real_ids = &[2247];
    let d = refine(&i);
    assert!(d.correction.contains("#2247 (dns_a)"));
    assert!(d.correction.contains("submit_stage_deliverable"));
    assert!(d.correction.contains("Do NOT re-run"));
}

#[test]
fn confirm_only_template_says_submit_with_empty_evidence() {
    let kinds = HashMap::new();
    let mut i = base_input(&[], &kinds);
    i.missing_deliverable = true;
    i.confirm_only_stage = true;
    let d = refine(&i);
    assert!(d.correction.contains("confirm-only"));
    assert!(d.correction.contains("evidence_ids may be empty"));
}

#[test]
fn coverage_template_includes_db_diagnosis_and_commands_for_vacuous() {
    use crate::harness::gate::rule_engine::{EvidenceFact, EvidenceOutcome};
    let reasons = vec!["deliverable vacuous: no claims".to_string()];
    let kinds = HashMap::new();
    let facts = vec![EvidenceFact {
        asset: "moresec.cn".into(),
        technique: "GOLISH-INTEL-SUBDOMAIN".into(),
        outcome: EvidenceOutcome::Found,
        evidence_id: 0,
    }];
    let mut i = base_input(&reasons, &kinds);
    i.evidence_facts = Some(&facts);
    let d = refine(&i);
    assert!(d.correction.contains("Suggested next commands"),
        "vacuous BLOCK 也必须附命令诊断（两连 BLOCK 修复锚点）");
    assert!(d.correction.contains("GOLISH-INTEL-DNS"));
}

#[test]
fn fabricated_template_names_fake_and_real_ids() {
    let kinds = HashMap::new();
    let mut i = base_input(&[], &kinds);
    i.fabricated_ids = &[1, 2, 3];
    i.available_real_ids = &[2247];
    let d = refine(&i);
    assert!(d.correction.contains("[1, 2, 3]"));
    assert!(d.correction.contains("2247"));
}
```

**步骤 2.2** 填实 6 个 `render_*`（迁移文本）。`render_coverage_or_vacuous` 需要 `build_db_truth_diagnosis` / `passive_intel_command_hint` / `PASSIVE_INTEL_TECHNIQUES`——把这三个从 `execute.rs:2086-2142` **搬进 refiner.rs**（pub(crate)），execute.rs 原处删除、调用点跟随（搬迁不是复制，全工作区唯一定义）。

**验证**：

```bash
cd backend && cargo nextest run -p golish-agent-kit -E 'test(refiner)' --status-level fail
cargo build -p golish-agent-kit
```

预期：refiner 测试全绿；`build_db_truth_diagnosis` 搬迁后 `execute.rs` 内 `rg "build_db_truth_diagnosis" execute.rs` 仅剩 `refiner::` 前缀引用或 0 处。

**提交**：`feat(agent-kit): refiner per-class templates; C-class diagnosis now covers vacuous blocks`

### Task 3：HarnessGateOutcome 事实字段 + enforce_* 改置标记

**文件**：`execute.rs`、`execute_harness_loop_tests.rs`。

**步骤 3.1** `HarnessGateOutcome`（execute.rs:1695）增字段（全部带注释说明是 Refiner 输入）：

```rust
    /// 设计 2026-06-12-unified-refiner · gate 原始拒绝理由（渲染权上收 Refiner）。
    gate_reasons: Vec<String>,
    gate_recovery: Option<crate::harness::HarnessRecoveryActions>,
    /// enforce_evidence_kinds 置：stage 要求但 deliverable 证据缺失的 kinds。
    missing_kinds: Vec<String>,
    /// enforce_evidence_freshness 置：硬过期的 evidence ids。
    expired_ids: Vec<i64>,
    /// enforce_scoping_red_team_flow 置：已渲染好的流程纠正（G 类透传）。
    red_team_flow_correction: Option<String>,
    /// StageSpec.allowed_tool_types.is_empty()（A 类 confirm-only 变体判定）。
    confirm_only_stage: bool,
    /// missing-deliverable 时账本真实 id → kind 标签（A 类模板）。
    evidence_kind_labels: std::collections::HashMap<i64, String>,
```

**步骤 3.2** `apply_harness_gate_hook`：
- BLOCK 分支（execute.rs:2004 附近）`repair_correction` 改填 `None`，新字段填 `decision.reasons.clone()` / `decision.recovery_actions.clone()`；
- `confirm_only_stage` 在函数内已有局部判定（1872-1874 `confirm_only`）——把该值写进返回的 outcome（PASS/BLOCK 两路都填）；
- `missing_deliverable_gate_outcome`（2393-2416）：correction 改 `None`（B 类渲染交 Refiner），其余字段补默认值。

**步骤 3.3** 四个 enforce_* / block_outcome_for_fabricated 删除 correction 拼接，改置标记：

```rust
// enforce_evidence_kinds（1528-1537 替换为）：
outcome.gate_allowed = false;
outcome.missing_kinds = missing;

// enforce_evidence_freshness（1589-1598 替换为）：
outcome.gate_allowed = false;
outcome.expired_ids = expired.iter().map(|e| e.value()).collect();

// enforce_scoping_red_team_flow（1486-1490 替换为）：
outcome.gate_allowed = false;
outcome.red_team_flow_correction = Some(correction);

// block_outcome_for_fabricated（1620-1648 收缩为）：
outcome.gate_allowed = false;
outcome.fabricated_evidence_refs = fabricated.to_vec();
outcome.available_real_ids = available_real_ids.to_vec();
```

（`EvidenceAuditId` 取裸值的方法名以该类型实际 API 为准：`rg "impl EvidenceAuditId" -A 10 backend/crates/golish-pentest/src/evidence_ledger`，无 `value()` 则用现有 getter / `.0`。）

**步骤 3.4** `refine_missing_deliverable_correction`（1401-1441）重构为只采集事实、不渲染不返回 bool：

```rust
/// missing-deliverable BLOCK 时查账本真实 ids + kind 标签，填进 outcome 供
/// Refiner 分类（A/B）与渲染。查询失败 / 账本空 = 不填（B 类自然兜住）。
async fn gather_missing_deliverable_ids(&self, outcome: &mut HarnessGateOutcome) {
    if outcome.gate_allowed || !outcome.missing_deliverable {
        return;
    }
    let Some(sid) = self.chat_session_id.as_deref() else {
        return;
    };
    let ids = match self.repo.recent_evidence_ids(sid, 25).await {
        Ok(ids) => ids,
        Err(e) => {
            tracing::warn!(
                target: "harness::hook",
                error = %e,
                "refiner fact-gathering: evidence-id lookup failed; redo-class will apply"
            );
            return;
        }
    };
    if ids.is_empty() {
        return;
    }
    outcome.evidence_kind_labels = self.repo.evidence_kinds_for(&ids).await.unwrap_or_default();
    outcome.available_real_ids = ids;
}
```

`build_submit_only_correction`（1656-1679）删除（文本已迁 refiner）。

**步骤 3.5** 修编译错：`execute_harness_loop_tests.rs:426/441` 与 execute.rs 内所有 `HarnessGateOutcome { ... }` 字面量补新字段默认值（`gate_reasons: vec![]` 等）。涉及既有单测 3236/3265/3301 一带：断言从「correction 文本包含 X」改为「标记字段被置 + `refiner::refine` 输出包含 X」（链式拼接断言删除，对应行为已废）。

**验证**：

```bash
cd backend && cargo nextest run -p golish-agent-kit --status-level fail
cargo clippy -p golish-agent-kit -- -D warnings
```

预期：全绿零 warning。`rg '\{correction\}\\n\\n\{prev\}' execute.rs` → 0 匹配（链式拼接已绝迹）。

**提交**：`refactor(agent-kit): enforce_* emit fact marks only; correction rendering moves to refiner`

### Task 4：两个 gate 调用点接 Refiner

**文件**：`execute.rs`。

**步骤 4.1** 循环内调用点（276-318）改为：

```rust
if let Some(mut outcome) = gate_outcome {
    self.enforce_evidence_existence(&mut outcome).await;
    self.enforce_evidence_kinds(&mut outcome).await;
    self.enforce_evidence_freshness(&mut outcome).await;
    self.enforce_scoping_red_team_flow(&mut outcome, exec_ctx).await;
    self.gather_missing_deliverable_ids(&mut outcome).await;
    if !outcome.gate_allowed {
        let decision = super::super::refiner::refine(&refine_input_from(&outcome, refine_facts.as_deref()));
        tracing::info!(
            target: "harness::hook",
            stage = %outcome.gated_stage.as_str(),
            class = ?decision.class,
            submit_only = decision.submit_only_lock,
            "refiner decision"
        );
        outcome.repair_correction = Some(decision.correction);
        if reflector_attempt < MAX_REFLECTOR_RETRIES {
            pending_gate_correction = outcome.repair_correction.clone();
            pending_submit_only = decision.submit_only_lock;
            last_result = Some(AgentResult { content: gated_content, ..agent_result });
            continue;
        }
    }
    self.consume_gate_outcome(task_id, outcome).await;
}
```

要点：
- `refine_facts`：`fetch_evidence_facts_for_gate` 的返回在传给 hook 前 `clone()` 一份留接线处（`let refine_facts = evidence_facts.clone();`，量小可接受），C 类诊断与 gate 注入用同一数据源；
- 新增小适配器（execute.rs 内，HarnessGateOutcome 旁）：

```rust
fn refine_input_from<'a>(
    outcome: &'a HarnessGateOutcome,
    facts: Option<&'a [crate::harness::gate::rule_engine::EvidenceFact]>,
) -> crate::task_orchestrator::refiner::RefineInput<'a> {
    crate::task_orchestrator::refiner::RefineInput {
        stage: outcome.gated_stage,
        gate_reasons: &outcome.gate_reasons,
        gate_recovery: outcome.gate_recovery.as_ref(),
        missing_deliverable: outcome.missing_deliverable,
        confirm_only_stage: outcome.confirm_only_stage,
        fabricated_ids: &outcome.fabricated_evidence_refs,
        available_real_ids: &outcome.available_real_ids,
        evidence_kind_labels: &outcome.evidence_kind_labels,
        missing_kinds: &outcome.missing_kinds,
        expired_ids: &outcome.expired_ids,
        red_team_flow_correction: outcome.red_team_flow_correction.as_deref(),
        evidence_facts: facts,
    }
}
```

- 原「BLOCK 且 `repair_correction.is_some()` 才重试」改为「BLOCK 即重试」（Refiner 恒产 correction，Generic 类兜底）。

**步骤 4.2** 耗尽后调用点（439-451）同样接 `gather_missing_deliverable_ids` + `refine`（只回填 `outcome.repair_correction` 供 HarnessTrace `first_blocking_reason`，不重试）。删除旧 `let _ = self.refine_missing_deliverable_correction(...)`。

**步骤 4.3** 全量回归 + 活体冒烟（可选，若本机有 key）：

```bash
cd backend && cargo nextest run -p golish-agent-kit --status-level fail && cargo clippy -p golish-agent-kit -- -D warnings
```

**验证**：测试全绿。关键回归锚点（应有专测，沿用/新增于 execute.rs tests mod）：missing + 账本有 ids → `pending_submit_only == true` 路径可达（写一个直接调 `refine_input_from` + `refine` 的单测断言锁触发）。

**提交**：`feat(agent-kit): wire both gate sites through the unified refiner (fixes submit-only preemption + vacuous diagnosis gap)`

---

## PR-R2 · 砍投影兜底

### Task 5：删 synthesize_from_evidence 全套

**文件**：`execute.rs`、`stage_spec.rs`、`target_intel.json`、06-11 设计文档。

**步骤 5.1** `execute.rs`：
- 删 `synthesize_from_evidence` 函数（2318-2360）及其 6 个单测（3351-3489 `synthesize_from_evidence` 系列）；
- `apply_harness_gate_hook` 的投影分支（1884-1933）收缩为：

```rust
            } else {
                // S4: fail-closed — BLOCK + Refiner 渲染纠正（A/B 类由账本事实决定）。
                tracing::warn!(
                    target: "harness::hook",
                    stage_kind = ?stage_hint.stage_kind,
                    subtask_title = %planned.title,
                    content_len = content.len(),
                    "harness gate: stage-tagged subtask produced no parseable StageDeliverable JSON block — BLOCK (fail-closed)"
                );
                return (content, missing_deliverable_gate_outcome(stage_hint.stage_kind, confirm_only));
            }
```

（`missing_deliverable_gate_outcome` 增 `confirm_only: bool` 入参填 `outcome.confirm_only_stage`——本 PR 先传 hook 内已算好的局部值。）

**步骤 5.2** `stage_spec.rs`：删 `synthesize_from_evidence_when_missing` 字段（117-120）+ 两个单测（381-411）；`finding_verification_check.rs:100` 等构造处同步删字段（`rg -l "synthesize_from_evidence_when_missing" backend/` 逐个清）。

**步骤 5.3** target_intel stage spec JSON（`rg -l "synthesize_from_evidence_when_missing" resources/ backend/`）删该键。

**步骤 5.4** `docs/design/2026-06-11-substantive-stage-evidence-projection-fallback.md` 头部加：

```markdown
> Superseded by `docs/design/2026-06-12-unified-refiner.md`（投影兜底被统一 Refiner 取代：missing-deliverable 保持 missing，由 A/B 类纠正驱动主 agent 自己提交）。
```

**验证**：

```bash
rg -n "synthesize_from_evidence" backend/ resources/   # 预期 0 匹配
cd backend && cargo nextest run -p golish-agent-kit --status-level fail
```

**提交**：`feat(agent-kit)!: remove evidence-projection fallback — missing deliverable stays missing, refiner drives the agent to submit`

---

## PR-R3 · 砍 confirm-only 合成

### Task 6：confirm-only missing 走 A 类锁

**文件**：`execute.rs`。

**步骤 6.1** 删 `synthesize_confirm_only_deliverable`（2262-2297）与 hook 的 confirm-only 分支（1865-1883），整段 `match parse_deliverable_from_content` 的 `None` 臂只剩：

```rust
        None => {
            let confirm_only = crate::harness::load_embedded_stage_spec(stage_hint.stage_kind)
                .map(|s| s.allowed_tool_types.is_empty())
                .unwrap_or(false);
            tracing::warn!(
                target: "harness::hook",
                stage_kind = ?stage_hint.stage_kind,
                subtask_title = %planned.title,
                content_len = content.len(),
                confirm_only,
                "harness gate: stage-tagged subtask produced no parseable StageDeliverable JSON block — BLOCK (fail-closed)"
            );
            return (content, missing_deliverable_gate_outcome(stage_hint.stage_kind, confirm_only));
        }
```

**步骤 6.2** 若存在引用 `synthesize_confirm_only_deliverable` 的单测（`rg -n "synthesize_confirm_only" backend/`）：改为断言「confirm-only stage missing → outcome.missing_deliverable && outcome.confirm_only_stage」+「`refine` 给 SubmitOnly + 锁」。

**验证**：

```bash
rg -n "synthesize_confirm_only" backend/   # 预期 0 匹配
cd backend && cargo nextest run -p golish-agent-kit --status-level fail
```

**提交**：`feat(agent-kit)!: drop confirm-only deliverable synthesis — submit-only lock drives the agent to submit its own confirmation`

---

## PR-R4 · F 类（text-only）模板化，删 reflect() LLM 调用

### Task 7：text-only 路径走 Refiner 模板

**文件**：`refiner.rs`、`execute.rs`、`types.rs`、`prompts/pipeline.rs`。

**步骤 7.1** `refiner.rs` 增 `RefineClass::TextOnly` 变体 + 模板 + 单测：

```rust
/// F · gate 之前的检测：响应是纯散文、无工具调用。
pub(crate) fn refine_text_only(stage_title: &str) -> RefineDecision {
    RefineDecision {
        class: RefineClass::TextOnly,
        correction: format!(
            "Your previous response for subtask '{stage_title}' was plain prose with no \
             tool calls — narration alone makes NO progress and cannot be verified. Take \
             concrete action now: run this stage's required tools to collect evidence, \
             then call `submit_stage_deliverable` citing the resulting evidence ids. Do \
             NOT restate plans or summaries; your next message must begin with a tool call."
        ),
        submit_only_lock: false,
    }
}

#[test]
fn text_only_template_demands_tool_call_not_redo() {
    let d = refine_text_only("Passive Target Intelligence");
    assert_eq!(d.class, RefineClass::TextOnly);
    assert!(d.correction.contains("must begin with a tool call"));
    assert!(!d.correction.contains("re-do the stage"));
}
```

**步骤 7.2** `execute.rs` text-only 检测处（248-259）改为直接灌纠正、不再依赖下轮 `reflect()`：

```rust
                    if reflector_attempt < MAX_REFLECTOR_RETRIES
                        && looks_like_text_only_response(&agent_result.content)
                    {
                        let decision =
                            super::super::refiner::refine_text_only(&planned.title);
                        tracing::info!(
                            target: "harness::hook",
                            class = ?decision.class,
                            "[TaskMode/Refiner] text-only response → deterministic correction (attempt {})",
                            reflector_attempt + 1,
                        );
                        pending_gate_correction = Some(decision.correction);
                        last_result = Some(agent_result);
                        continue;
                    }
```

**步骤 7.3** 删 `executor.reflect()` 分支（212-243 的 `else` 臂整段——`pending_gate_correction` 现在覆盖了它唯一的进入路径；保险检查 `rg -n "\.reflect\(" backend/crates/golish-agent-kit/`，kit 内 0 调用后），`types.rs:231` trait 方法与 `prompts/pipeline.rs` 的 `reflector_system_prompt`/`reflector_user_prompt` 加 `#[deprecated(note = "superseded by task_orchestrator::refiner (design 2026-06-12-unified-refiner)")]`——trait 实现方（bridge）删除另起 PR，本 PR 不动 bridge。

> 若 `#[deprecated]` 触发 workspace `-D warnings`（实现处 / 调用处仍存在），降级方案：只删 kit 内调用 + 在 trait 方法 doc 注释标 `Deprecated:`，属性留给 bridge 清理 PR。

**验证**：

```bash
cd backend && cargo nextest run -p golish-agent-kit --status-level fail
cargo clippy -p golish-agent-kit -- -D warnings
rg -n "\.reflect\(" backend/crates/golish-agent-kit/src/   # 预期 0 匹配
```

**提交**：`feat(agent-kit): text-only responses get a deterministic refiner correction; deprecate LLM reflect()`

---

## 收口 · 活体对照 + 全量门禁

### Task 8：活体验证 + 记录

**步骤 8.1** `just precommit` 全绿（fmt + check-fe + test-fe + lint-rust + test-rust-all）。

**步骤 8.2** 活体对照（需用户提供 LLM key，同上次命令）：

```bash
cd backend && cargo build -p golish
nohup ./target/debug/golish --stage-run -p xiaomi -m mimo-v2.5-pro --to target_intel \
  --org 默安科技 --target moresec.cn --auto-approve --verbose \
  > /tmp/golish-refiner-run.log 2>&1 &
```

断言（日志逐条核对）：
1. `rg "refiner decision" /tmp/golish-refiner-run.log` —— 每次 BLOCK 都有 class 记录；
2. missing-deliverable 场景出现 `class=SubmitOnly submit_only=true`（截胡 bug 修复实证）；
3. vacuous BLOCK 的纠正喂回文本含 `Suggested next commands`（C 类扩展实证，查 transcript 或 debug 日志）；
4. `rg "synthesizing" /tmp/golish-refiner-run.log` → 0 匹配（合成兜底绝迹）。

**步骤 8.3** 证据（命令 + 退出码 + 关键输出）写入 `agent-progress.md` 会话记录；`feature_list.json` 对应条目 `verification` 逐条核对、`evidence` 填实、状态按 AGENTS.md §3 判定（任一缺失停在 in_progress）。

---

## 自检记录（writing-plans §自检，已执行）

1. **规格覆盖度**：设计 §5.1 分类表 → Task 1；§5.2 模板 → Task 2；§5.4 接线 → Task 3-4；§5.3 砍除清单 7 行 → Task 5（投影 4 行）/ Task 6（confirm-only）/ Task 7（reflect()）/ Task 5.4（文档 Superseded）；§9 活体收口 → Task 8。无遗漏。
2. **占位符扫描**：无 TODO/待定；所有代码步骤带代码或逐字迁移源行号。
3. **类型一致性**：`RefineClass/RefineInput/RefineDecision/refine/refine_text_only/gather_missing_deliverable_ids/refine_input_from` 各任务间名称一致；`EvidenceAuditId` 取值方法在 Task 3.3 标注了核实步骤。
