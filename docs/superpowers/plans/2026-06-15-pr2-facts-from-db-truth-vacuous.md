# PR2 · 事实层由 DB 真值驱动（放宽 vacuous，opt-in 默认关）实现计划

> **面向 AI 代理的工作者：** 必需子技能：`.cursor/skills/executing-plans`。
> 关联设计：`docs/design/2026-06-15-db-truth-single-source-deliverable.md` §5 PR2、§3.3、§4 红线。
> 决策：D4=先只 target_intel；D5=保留轻量 submit 检查点（本 PR 不动 missing-deliverable/fail-closed）。

**目标：** 让 facts-only 阶段（先 `target_intel`）在 DB 真值已含本 run 事实时，弱模型即使交一份「近乎空」的 deliverable 也能过 gate——facts 全由 DB 真值（`coverage_complete` authoritative + `coverage_truth`）裁决，不再逼 agent 手抄 claims/coverage/evidence_refs。
**架构：** `coverage_complete`（authoritative_found）早已按 DB 真值判 found（per in-scope 资产 × 期望技术）。唯一仍按 deliverable 拦「空交付」的结构 check 是 `vacuous_check`。给 `vacuous_check` 注入 DB 真值 `evidence_facts` + 一个 opt-in spec 开关 `facts_from_db_truth`：开关开且账本/DB 有真实事实时，vacuous 的 (a) NoToolInvocation + (b) FakePattern 由 DB 真值满足；完整性仍由 `coverage_complete` 把关。`for_all`/`coverage_corroborated` 对空 `coverage`/`claims` 天然 vacuous-pass。
**安全：** 不放宽 `coverage_complete`/`corroborated`/`denominator`/`finding_verification`/`fabricated`/`freshness`；DB 真值为空（没干活）时 vacuous 照旧拦截；漏洞类阶段开关默认关，行为零变化。

**技术栈：** Rust（`golish-agent-kit`）；`cargo nextest`。

---

## 文件结构

| 文件 | 改动 | 职责 |
|---|---|---|
| `harness/stage_spec.rs` | 改 | `StageSpec` 加 `facts_from_db_truth: bool`（`#[serde(default)]`） |
| `harness/gate/vacuous_check.rs` | 改 | `run` 增 `evidence_facts` 参数 + DB-truth 放宽逻辑 + 更新单测 |
| `harness/gate/mod.rs` | 改 | 第 138 行调用传 `ctx.evidence_facts.as_deref()` |
| `harness/gate/finding_verification_check.rs` | 改 | 测试里的 `StageSpec` 结构字面量补 `facts_from_db_truth: false` |
| `resources/harness/stages/target_intel.json` | 改 | 加 `"facts_from_db_truth": true` |

---

## Task 1 · `StageSpec` 加 opt-in 字段

**文件：** `harness/stage_spec.rs`（`StageSpec` 末尾，`coverage_axis` 之后）

```rust
    /// Facts-from-DB-truth opt-in (design 2026-06-15 §5 PR2). When true AND the
    /// gate is handed real DB/ledger evidence facts, `vacuous_check` treats the
    /// stage's facts as coming from DB truth (coverage_complete裁决), so an
    /// otherwise-empty deliverable is not "vacuous". Completeness is still enforced
    /// by coverage_complete (per in-scope asset × expected technique). Default false
    /// = byte-for-byte unchanged. Enable only for facts-only intel/recon stages.
    #[serde(default)]
    pub facts_from_db_truth: bool,
```

**验证：** 见 Task 5（统一 cargo check）。

## Task 2 · `vacuous_check::run` 注入 DB 真值 + 放宽

**文件：** `harness/gate/vacuous_check.rs`

加 import：`use super::rule_engine::EvidenceFact;`

签名改为：
```rust
pub fn run(
    deliverable: &ExternalAttackSurfaceDeliverable,
    spec: &StageSpec,
    evidence_facts: Option<&[EvidenceFact]>,
) -> GateCheckOutcome {
    // design 2026-06-15 §5 PR2: facts-only opt-in 阶段，账本/DB 有真实事实时
    // (a)/(b) 由 DB 真值满足；completeness 仍由 coverage_complete 把关。
    let db_truth_backed =
        spec.facts_from_db_truth && evidence_facts.is_some_and(|f| !f.is_empty());
    let mut reasons = Vec::new();
    let mut missing_kinds = Vec::new();
    // (a) Vacuous
    if !db_truth_backed
        && deliverable.claims.is_empty()
        && deliverable.findings.is_empty()
        && deliverable.skipped_checks.is_empty()
    { /* …existing push… */ }
    // (b) FakePattern
    if !db_truth_backed && !spec.min_invocations.is_empty() { /* …existing… */ }
    // (c) SkipPattern — unchanged
    …
}
```

更新本文件 5 个单测：`run(&d, &spec)` → `run(&d, &spec, None)`（DB-truth 默认不参与，保旧行为）。新增 1 个单测：`facts_from_db_truth=true` + 非空 evidence_facts + 空 deliverable → Pass；空 evidence_facts → 仍 Block。

## Task 3 · gate 入口传参

**文件：** `harness/gate/mod.rs:138`
```rust
        vacuous_check::run(deliverable, spec, ctx.evidence_facts.as_deref()),
```

## Task 4 · 测试结构字面量补字段

**文件：** `harness/gate/finding_verification_check.rs`（`spec_with` 的 `StageSpec { … }`）末尾补：
```rust
            facts_from_db_truth: false,
```
（`rule_engine.rs` 的 `test_spec`/`spec_with_expected` 走 `load_stage_spec_from_json`，serde default，无需改。）

## Task 5 · 开关 + 编译

**文件：** `resources/harness/stages/target_intel.json`（顶层加，建议在 `max_other_skips` 附近）：
```json
  "facts_from_db_truth": true,
```
**验证：** `cd backend && cargo check -p golish-agent-kit`（编译过）。统一最终 `just precommit`。
**提交：** `feat(harness): facts-only stages satisfy vacuous from DB truth (PR2)`

---

## 自检
- **规格覆盖**：设计 §5 PR2「facts 由 DB 投影 + 放宽残留手填」→ vacuous 放宽 = 放开「逼 agent 手填」的唯一结构关；coverage/claims 的 for_all 与 corroborated 对空集天然通过，无需改。✓
- **占位符**：无。**类型一致**：`facts_from_db_truth` 在 spec/json/测试字面量三处一致；`EvidenceFact` 来自 `super::rule_engine`。
- **安全**：completeness 仍由 coverage_complete（DB authoritative）把关；空 DB→vacuous 照旧拦；漏洞类默认关。
