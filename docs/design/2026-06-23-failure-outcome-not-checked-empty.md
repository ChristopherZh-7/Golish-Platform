# T2 · failure ≠ checked_empty：失败检查记 error（非「已查为空」）

> 评审 claim #2。状态：进行中（2026-06-23）。

## 1. 问题

`passive_intel_outcome_for_run(technique, raw, succeeded)`（`evidence_facts.rs:171`）在 `!succeeded`（非零退出 / 超时 / crt.sh 502 等外部抽风）时返回 `"empty"`，落库为 `evidence_outcome="empty"` → gate 投影成 `CheckedEmpty` 终态。这把两件本质不同的事混为一谈：

- **查过、确实为空**（真 checked_empty）
- **跑了但失败 / 拿不到**（数据源超时 / 工具崩 / 502）

后果：交付/审计里「失败阻断」显示成「已查为空」，与 I8（「已检查为空 ≠ 未检查」）的精神冲突——再加一层「失败 ≠ 已查为空」。

**既有设计张力**（必须保住）：`evidence_facts.rs:165-181` + `direct/mod.rs:375-380` 注释明确「失败也要落终态，否则 gate 对永远填不上的格（如 crt.sh）无限重试」。所以**不能**简单地让失败不落账——必须落一个**仍是终态、但语义为 error/blocked** 的事实。

## 2. 目标 / 非目标

**目标**：新增第三种 outcome `error`：失败检查 → `error`（≠ `empty`），gate 把它当**终态**（保住「不无限重试」），但记为「失败阻断」、**绝不**当 checked_empty。**gray-switch**（默认 off，逐字节不变）。

**非目标**：不改 schema（`audit_log.evidence_outcome` 是**自由文本 TEXT、无 CHECK 约束**，写 `"error"` 直接可用——无需 migration、无需 §2.7 schema 确认）；不强制 reason_kind（T1 已加字段）。

## 3. 设计

### 3.1 数据层（gray-switch）
`passive_intel_outcome_for_run` 加 `distinguish_failure: bool` 参数（保持纯函数、可双向单测）：

```rust
pub fn passive_intel_outcome_for_run(technique, raw_output, succeeded, distinguish_failure) -> &'static str {
    if succeeded { passive_intel_outcome(technique, raw_output) }
    else if distinguish_failure { "error" }  // T2：跑了但失败 → error
    else { "empty" }                          // 旧行为（flag off）
}
```

调用方 `direct/mod.rs:408` 传 `feature_flags::failure_outcome_error_enabled()`（env `GOLISH_FAILURE_OUTCOME_ERROR`，**默认 off**）。失败时 ledger 行的 body 已记真实原因（`direct/mod.rs:419-425`），审计可追。

### 3.2 outcome 枚举 + 投影
`EvidenceOutcome` 加 `Error`（rule_engine.rs）。三处 row→fact 映射认 `"error"`：
- `execute.rs:1514` / `evidence.rs:138` 的 `match`：加 `"error" => Error`。
- `org_gate.rs:47` `facts_from_rows`：`found→Found / error→Error / 其余→Empty`。

`EvidenceOutcome` 全部以 `==` 比较（无穷尽 match）→ 加变体零破坏。

### 3.3 gate（加性、inert 直到有 error 事实）
`coverage_complete` 加 `error_ok`：

```rust
// error 事实 = 终态（保住旧 failure→empty 的「落终态、不无限重试」性质），但按
// 「失败阻断」计，绝不当 found / checked_empty。终态条件取 CheckedEmpty∪Blocked
// （覆盖旧 empty 路径 + Blocked 语义）。
let error_ok = derive_from_evidence
    && has_fact(EvidenceOutcome::Error)
    && (terminal.contains(&CheckedEmpty) || terminal.contains(&Blocked));
if !found_ok && !empty_ok && !other_ok && !error_ok { gap }
```

gate 侧**无条件**上（additive）：无 `error` 事实时 `error_ok` 永假 = 逐字节不变；故安全先于数据层 flag 落地（避免「先发 error、gate 不认 → 掉行 → 无限重试」的次序坑）。

### 3.4 诊断/交付显示
`audit_log.evidence_outcome="error"` 落库即可被 `scripts/run_tree.py:507`（`GROUP BY evidence_outcome`）**自动**显示为独立一行（`<technique> | error | n`），与 `empty` 区分——「交付里显示失败阻断」在审计层兑现，无需额外改动。

## 4. 验证

- `passive_intel_outcome_for_run(.., succeeded=false, distinguish_failure=false)` → `"empty"`（旧）；`=true` → `"error"`。
- feature flag 纯函数（默认 off / `1`·`true` 开）。
- row→fact：`"error"` → `EvidenceOutcome::Error`（三处映射）。
- gate：仅 Error 事实 + derive_from_evidence + CheckedEmpty/Blocked 终态 → cell 终态（Pass，不 gap）；且 Error **不**满足 found_ok / empty_ok（不冒充 found/empty）。
- `cargo nextest -p golish-agent-kit -p golish-agent-app -p golish-agent-runtime` 零回归 + `clippy -D warnings`。

## 5. 风险 / 回滚 / 激活

- 默认 off ⇒ 落地零行为变化。
- **激活** = `GOLISH_FAILURE_OUTCOME_ERROR=1`（数据层开始把失败记 error；gate 已就绪）。回滚 = unset / `=0`。
- 无 schema 变更（自由文本列）。column comment 仍写「found|empty」（轻微滞后，可后续补 comment-only migration，非本 PR）。
- no-infinite-retry 性质保住：error 在 empty 原终态处仍终态。
