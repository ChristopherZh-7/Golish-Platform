# Gate 彻底数据驱动化（删 `required_checks` → 单一 `gate_rules`）实现计划

> **面向 AI 代理的工作者：** 必需子技能：用 `.cursor/skills/executing-plans` 逐任务实现；每个任务遵循 `.cursor/skills/test-driven-development`（先写失败测试 → 跑红 → 实现 → 跑绿 → commit）。设计依据：`docs/design/2026-06-05-gate-rules-migration.md`。
>
> **目标：** 把 stage 过关标准的旧入口 `required_checks`（固定菜单 `match` + `_ => continue` 静默忽略）整个删掉，让 `gate_rules` 成为唯一入口。**行为零变更**（安全闸重构硬约束，见设计 §7）。
>
> **关键安全点：** 先加新、再双跑、后删旧（设计 §8）；每步独立 commit、可单独 revert；全程 `all_twelve_stage_specs_load` + `just test-harness` 兜底。

---

## 前置约定

- 受影响 crate：`golish-agent-kit`（gate + stage_spec + 12 份 embedded spec）+ 回归连带 `golish-agent-app`。
- 验证命令（每任务末跑相关子集，收口跑全套）：
  - `cd backend && cargo nextest run -p golish-agent-kit -E 'test(harness)' --status-level fail`
  - `cd backend && cargo nextest run -p golish-agent-kit -p golish-agent-app --status-level fail`
  - `cd backend && cargo clippy -p golish-agent-kit -p golish-agent-app -- -D warnings`
  - `cd backend && cargo fmt --check`
- spec 路径：`resources/harness/stages/<stage>.json`（12 份）。
- **勿混淆**：`StageSpec.required_checks`（要删）≠ `StageDeliverable.required_checks_done`（保留，min_invocations 用）。

---

## Task 1 · `rule_engine.rs` 加 `named_check` 积木 + `eval` 入参带 spec（TDD）

**1.1（红）** 在 `rule_engine.rs` 测试模块加失败测试：
- `parses_named_check_rule`：`{ "op":"named_check","check":"surface_coverage" }` 解析为 `GateRule::NamedCheck{ check: NamedCheckKind::SurfaceCoverage, .. }`。
- `unknown_named_check_fails_closed`：`{ "op":"named_check","check":"bogus" }` → `serde_json::from_str::<GateRule>` Err。

**1.2（绿）** 实现：
```rust
// GateRule 加变体
NamedCheck {
    check: NamedCheckKind,
    #[serde(default)]
    on_fail: Option<OnFail>,
},

#[derive(Debug, Clone, Copy, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum NamedCheckKind { Scope, SurfaceCoverage, MinInvocations }
```

**1.3** 改 `eval` 签名 + dispatch（设计 §5.4）：
```rust
use super::super::stage_spec::StageSpec;
use super::{scope_check, surface_coverage_check, min_invocations_check};

pub fn eval(d: &StageDeliverable, spec: &StageSpec, rules: &[GateRule]) -> Vec<GateCheckOutcome> {
    rules.iter().map(|r| eval_one(d, spec, r)).collect()
}

fn eval_one(d: &StageDeliverable, spec: &StageSpec, rule: &GateRule) -> GateCheckOutcome {
    match rule {
        GateRule::CountAtLeast { .. } | GateRule::ForAll { .. } => /* 现有逻辑，忽略 spec */,
        GateRule::NamedCheck { check, on_fail } => {
            let base = match check {
                NamedCheckKind::Scope => scope_check::run(d),
                NamedCheckKind::SurfaceCoverage => surface_coverage_check::run(d),
                NamedCheckKind::MinInvocations => min_invocations_check::run(d, spec),
            };
            match (base, on_fail) {
                (GateCheckOutcome::Block { .. }, Some(of)) => block_from(of), // 覆盖 reason/recovery
                (other, _) => other,
            }
        }
    }
}
```

**1.4（绿）** named_check dispatch 测试：Scope/SurfaceCoverage/MinInvocations 各一条「会 Block 的 deliverable」+ 一条「Pass」，断言与直接调对应 `*_check::run` 结论一致。

**1.5** 修现有 `eval` 调用方编译：`gate/mod.rs:145` 的 `rule_engine::eval(deliverable, &spec.gate_rules)` → `rule_engine::eval(deliverable, spec, &spec.gate_rules)`；`finding_verification_check.rs` 等价性测试里的 `rule_engine::eval(&d, &[gr])` → 传一个最小 spec（或加 `eval` 的测试辅助）。

**验证：** `cargo nextest -p golish-agent-kit -E 'test(rule_engine)'` 绿 + clippy。**commit**：`feat(harness): add named_check gate-rule op + thread spec into eval`。

---

## Task 2 · `gate/mod.rs` 确认 `eval` 接入（仍与旧 match 并存 · 加性）

**2.1** 确认 `validate_stage_gate_with_skeleton` 内：旧 `required_checks` match 段**保持不动**，`rule_engine::eval(deliverable, spec, &spec.gate_rules)` 已在 aggregate 前（Task 1.5 已改签名）。此步**不删任何东西**，只保证「加新积木后旧路仍绿」。

**2.2** 加集成测试 `named_check_surface_coverage_blocks_via_gate_rules`：一个只含 `gate_rules:[{op:named_check,check:surface_coverage}]`、无 required_checks 的内联 spec + 只有 Surface 无 JsApi 的 deliverable → `validate_stage_gate` Block 且 reason 含 `JsApi`。

**验证：** `cargo nextest -p golish-agent-kit -E 'test(harness::gate)'` 绿。**commit**：`test(harness): cover named_check dispatch through the aggregate gate`。

---

## Task 3 · 12 份 spec 写入等价 `gate_rules`（旧 required_checks 暂留 · 中间态双跑）

按设计 §6 表，给每份 spec **加** `gate_rules`（**先不删** `required_checks`）。scope×2 = 两条 for_all 非空规则（设计 §5.2 JSON）。

- scoping / reporting：`gate_rules` 留空（或不加 scope×2）。
- target_intel / vuln_triage / verification / access_validation / internal_discovery / objective_pathing / objective_simulation / cleanup：scope×2。
  - verification：scope×2 **追加**到其现有 high+ 证据规则后（不动现有）。
- enumeration：scope×2 + `{op:named_check,check:min_invocations}`。
- external_attack_surface：scope×2 + `{op:named_check,check:surface_coverage}` + `{op:named_check,check:min_invocations}`。

**验证：** `cargo nextest -p golish-agent-kit -E 'test(all_twelve_stage_specs_load)'` + `just test-harness` 全绿（中间态：scope 被 required_checks 与 gate_rules 双跑，结论一致，仅 reasons 可能重复——安全）。**commit**：`refactor(harness): declare gate_rules equivalents in all 12 stage specs`。

---

## Task 4 · 删旧路（match + 字段 + spec 数组）

**4.1** `gate/mod.rs`：删除 `let mut ran ...; for name in &spec.required_checks { match ... }` 整段（含 `_ => continue`、`scope/surface_coverage/min_invocations` 分支、`ran` 去重）。保留 5 个结构 check + `rule_engine::eval(...)`。

**4.2** `stage_spec.rs`：删 `#[serde(default)] pub required_checks: Vec<String>,`（:66）。删/改引用它的测试（`external_attack_surface_required_checks_count` :164-167 → 删除或改断言 `gate_rules` 数量）。

**4.3** 12 份 spec：删每份的 `"required_checks": [...]` 数组。

**4.4** 全仓 grep `required_checks`（注意排除 `required_checks_done`），清理任何残留读取点（预期仅测试 + 本批文件）。

**验证：** `cargo check -p golish-agent-kit`（无残留）+ `cargo nextest -p golish-agent-kit -E 'test(harness)'` + `just test-harness` 全绿。**commit**：`refactor(harness)!: remove required_checks fixed-menu; gate_rules is the sole entry`。

---

## Task 5 · 等价性 / 回归测试（锁死行为零变更）

**5.1** 在 `gate/mod.rs` 测试模块加 `migrated_specs_preserve_scope_evidence_gate`：对 external_attack_surface（迁移后 embedded spec），构造「有一个 claim 缺 evidence_ids」的 deliverable → 断言 `validate_stage_gate` Block 且 reason 含证据缺失（证明 scope×2 接住了旧 scope_check 的语义）。

**5.2** `min_invocations` 等价：enumeration spec + `required_checks_done` 不含 `http_probe` 的 deliverable → Block（经 named_check 走到 min_invocations_check，与旧路同结论）。

**5.3** 确认既有 e2e_tests / execute_harness_loop_tests 等依赖 gate 的测试全绿（无回归）。

**验证：** `cargo nextest -p golish-agent-kit -p golish-agent-app --status-level fail` 全绿。**commit**：`test(harness): equivalence + regression for gate_rules migration`。

---

## Task 6 · 文档

**6.1** `docs/design/2026-06-02-harness-stage-spec-reference.md` §8 DSL 速查：加 `named_check`（3 个 kind）说明 + 标注「`required_checks` 已废弃删除，过关标准统一走 `gate_rules`」。

**6.2** 给被取代的旧设计加指针：在 `2026-06-05-gate-rule-engine.md` 顶部加 `> 后续：required_checks 已由 2026-06-05-gate-rules-migration.md 彻底删除（B）。`（不删原文，留决策史，AGENTS.md I6）。

**验证：** 文档自检（无断链）。**commit**：`docs(harness): document named_check + required_checks removal`。

---

## Task 7 · 收口验证 + 登记

**7.1** 全套门禁（受影响面）：
```bash
cd backend && cargo nextest run -p golish-agent-kit -p golish-agent-app --status-level fail
cd backend && cargo clippy -p golish-agent-kit -p golish-agent-app -- -D warnings
cd backend && cargo fmt --check
```
（可选完整门禁：`just precommit`，约 20-30 min；纯 Rust、无前端面。）

**7.2** 证据 + 退出码复制进 `agent-progress.md`「已记录证据」；`feature_list.json` 加 `gate-rules-migration-2026-06-05` 条目（`passing` + evidence，或 `blocked` 若有遗留）。

**7.3** commit：`chore(harness): record gate_rules migration evidence + feature status`。

---

## 自检

**规格覆盖度**（对照设计 `2026-06-05-gate-rules-migration.md`）：
- §5.1 named_check 积木 → Task 1 ✓
- §5.2 scope 数据化 → Task 3 ✓
- §5.3 surface/min_invocations named_check → Task 1+3 ✓
- §5.4 eval 带 spec → Task 1 ✓
- §5.5 删旧路 → Task 4 ✓
- §6 12 spec 迁移表 → Task 3+4 ✓
- §7 行为零变更 → Task 5 等价性 ✓
- §10 验证 → Task 5/7 ✓

**行为零变更红线**：Task 3 先双跑（加性、可回退）、Task 4 才删；Task 5 等价性测试锁死；每步 `just test-harness` 兜底。

**fail-closed**：named_check 名是 typed enum → `all_twelve_stage_specs_load` 在 Task 3/4 当场抓写错。

**YAGNI**：不加 min_invocations 真实计数、不数据化 surface_coverage 关键词表、不引入 and/or——均留作后续独立积木。
