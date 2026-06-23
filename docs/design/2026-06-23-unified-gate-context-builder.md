# 统一 GateContext 构造：GateContextBuilder

> 状态：进行中（2026-06-23 起）。Phase 1（本设计落地范围）= 行为保持的纯组装抽取；Phase 2（gate 收紧 / submit 预检补全口径）deferred。

## 1. 背景与问题

gate 的 `GateContext`（`golish-agent-kit/src/harness/gate/rule_engine.rs:216`）有 4 个字段：

```rust
pub struct GateContext {
    pub in_scope_assets: Option<Vec<String>>,
    pub asset_types: Option<HashMap<String, String>>,
    pub expected_techniques: Option<Vec<String>>,
    pub evidence_facts: Option<Vec<EvidenceFact>>,
}
```

它在 **3 个调用方**各自手搓构造，语义并不一致：

| 入口 | 位置 | in_scope_assets | asset_types | expected_techniques | evidence_facts |
|---|---|---|---|---|---|
| 主 agent stage-close | `execute.rs:2222` | fetch helper（Option） | fetch helper（Option） | `gate_expected_techniques` + subsidiary 注入 | ledger + db_truth + subsidiary 投影 |
| per-org fan-out | `org_gate.rs:198` | `repo.in_scope_assets` | `repo.in_scope_typed_assets` | **None**（回退 spec） | ledger + db_truth（无 subsidiary 投影） |
| submit 预检 | `harness_submit_tool.rs:351` | 仅 org 绑定时 | **None（Default）** | **None（Default）** | command-path facts + db_truth |

问题（与用户/另一会话评审一致）：

1. **submit 预检不是完整的 authoritative 口径**——它丢了 `asset_types` 和 `expected_techniques`，所以 submit 预览 PASS / stage-close 权威 BLOCK 可能不一致 → 子 agent resubmit 死循环。
2. **`(!x.is_empty()).then_some(x)` 归一逻辑** + **typed→HashMap 转换** + **facts 合并**散落 3 处，各写各的，易漂移。
3. 未来给 `GateContext` 加字段（如 claim #1/#2/#4 的 note/reason_kind/source/outcome）要改 3 处。

## 2. 目标 / 非目标

**目标（Phase 1）**：把 `GateContext` 的**纯组装**（归一 + 转换 + 合并）收成单一 `GateContextBuilder`，3 个入口全部走它。**行为逐字节保持**（每个入口喂入与现状完全相同的输入 → 产出完全相同的 `GateContext`）。

**非目标（Phase 2，deferred）**：
- 不在本 PR 收紧任何 gate（不让 submit 预检开始喂 asset_types/expected_techniques——那是**收紧**，会把现网 PASS 翻 BLOCK，须按既有 gray-switch 灰度 + 补测，单独 PR）。
- 不统一 repo **查询**编排（3 入口跨 `golish-agent-kit::DbRepoProvider` 与 `golish-agent-app::EvidenceLedgerQuery` 两套 trait、且 freshness/subsidiary 语义有意不同；查询层统一是后续工作）。

## 3. 设计

新增 `golish-agent-kit/src/harness/gate/context_builder.rs`，纯组装、零 IO、可单测：

```rust
pub struct GateContextBuilder { /* Vec/HashMap 累加器 + Option<expected_techniques> */ }

impl GateContextBuilder {
    pub fn new() -> Self;
    pub fn in_scope_assets(self, assets: Vec<String>) -> Self;          // 空 → None
    pub fn typed_assets(self, typed: Vec<(String, String)>) -> Self;    // 空 → None
    pub fn asset_types_map(self, map: HashMap<String, String>) -> Self; // 已有 map 时
    pub fn extend_evidence_facts(self, facts: impl IntoIterator<Item = EvidenceFact>) -> Self; // 可多次合并
    pub fn expected_techniques(self, t: Option<Vec<String>>) -> Self;   // None = 回退 spec
    pub fn build(self) -> GateContext;                                  // 唯一 empty→None 归一点
}
```

**归一只在 `build()` 发生**（单一真相源）。查询仍在各入口（不同 crate / 不同 freshness 语义），但组装收口。

## 4. 接线（行为保持）

- **execute.rs:2222**：4 个值现已是 Option → `.in_scope_assets(opt.unwrap_or_default()).asset_types_map(opt.unwrap_or_default()).extend_evidence_facts(opt.unwrap_or_default()).expected_techniques(opt)`。Option→Vec→（build 再归一）Option = 同构。
- **org_gate.rs:198**：删手搓 `then_some` + map 收集，改 builder。db_truth_facts 需在 move 前用 `&in_scope_assets`，顺序保留。
- **harness_submit_tool.rs:351**：改 builder；`asset_types`/`expected_techniques` **显式保持不设**（= None），加 `// Phase 2` 注释把「有意省略」从隐式 Default 变成**可见的决策**。

## 5. 验证

- 新增 `context_builder.rs` 单测：空→None、非空→Some、typed→map、facts 多源合并、expected_techniques None/Some 透传。
- `cargo nextest -p golish-agent-kit -p golish-agent-app` 零回归（既有 gate / org_gate / submit_tool 测全绿 = 行为保持铁证）。
- `cargo clippy … -D warnings` 零告警。

## 6. 风险

- 本 PR **不改 gate 判定**，风险面 = 组装等价性，由「既有测试零回归 + 新等价性单测」兜底。
- ⚠️ `org_gate.rs` / `execute.rs` / `coverage_truth.rs` 工作树有另一会话 perdim-freshness 未提交改动；本改动叠加其上、与之正交（只动 `GateContext{}` 构造，不动 freshness 取数）。

## 7. Phase 2（deferred，单独 PR）

1. submit 预检补 `asset_types` + `expected_techniques`（gray-switch，灰度翻新现网 PASS）。
2. 评估把 Site A/B 的 repo 查询编排也收进一个 `fetch + build` 门面（带 freshness/subsidiary flag）。
3. 给 `GateContext`/`CoverageCell` 加 note 强校验 / reason_kind / db-truth source（评审 claim #1/#2/#4）时，只在 builder 一处加字段。
