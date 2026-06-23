# 实现计划：统一 GateContextBuilder（Phase 1）

> 设计：`docs/design/2026-06-23-unified-gate-context-builder.md`。范围 = 行为保持的纯组装抽取 + 3 入口接线 + 等价性单测。

## Task 1 · 新建 builder（TDD）

文件：`backend/crates/golish-agent-kit/src/harness/gate/context_builder.rs`

- `GateContextBuilder`：`in_scope_assets: Vec<String>` / `asset_types: HashMap<String,String>` / `evidence_facts: Vec<EvidenceFact>` / `expected_techniques: Option<Vec<String>>` 累加器。
- 链式 setter：`new / in_scope_assets / typed_assets / asset_types_map / extend_evidence_facts / expected_techniques`。
- `build()`：唯一 `(!x.is_empty()).then_some(x)` 归一点。
- 单测：空→None；非空→Some；`typed_assets` 折成 map；`extend_evidence_facts` 多次合并累加；`expected_techniques(None/Some)` 透传；空 builder == `GateContext::default()`。

## Task 2 · 导出

- `gate/mod.rs`：`pub mod context_builder;` + `pub use context_builder::GateContextBuilder;`
- `harness/mod.rs`：`pub use gate::context_builder::GateContextBuilder;`（让 `crate::harness::GateContextBuilder` 可用）。

## Task 3 · 接线 3 入口（行为保持）

1. `org_gate.rs:~198`：手搓组装 → builder。保留 `&in_scope_assets` 在 move 前喂 `db_truth_facts`。
2. `harness_submit_tool.rs:~351`：手搓组装 → builder；显式不设 asset_types/expected_techniques + `// Phase 2` 注释；import 加 `GateContextBuilder`。
3. `execute.rs:~2222`：`GateContext{}` → builder（Option `.unwrap_or_default()` 适配）。

## Task 4 · 验证（证据优先）

- `cargo nextest -p golish-agent-kit -p golish-agent-app`：既有测试零回归 + 新 builder 测全绿。
- `cargo clippy -p golish-agent-kit -p golish-agent-app --all-targets --no-deps -- -D warnings`：零告警。
- `ReadLints` 受影响文件无错。
- 视情况 `just precommit`（按用户对耗时的偏好）。

## 回滚点

- 全部改动可单 commit 回退；builder 是新文件，3 处接线是等价替换，删除接线即回到现状。
