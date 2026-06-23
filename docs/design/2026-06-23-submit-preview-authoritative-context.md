# T3 · submit 预检补全 authoritative 口径（收尾 GateContextBuilder）

> 父设计：`docs/design/2026-06-23-unified-gate-context-builder.md` §7.1（Phase 2 第一项）。状态：进行中（2026-06-23）。

## 1. 问题

`harness_submit_tool.rs` 的 submit 预检 `gate_context` 走 `GateContextBuilder` 后仍**有意只喂** `in_scope_assets` + `evidence_facts`，`asset_types` / `expected_techniques` 保持 `None`。而主 agent stage-close（`execute.rs`）喂**全四字段**。后果：

- **asset_types 缺**：host-aware 阶段（EAS/enumeration）预检用值推断分类，close 用权威 `targets.type` 分类 → 同一格分类不同。
- **expected_techniques 缺**：预检回退 `spec.expected_techniques`（静态全集），close 用按类型派生的动态集 → 预检期望的技术集与 close 不一致。

两者都会造成「预检 PASS/BLOCK ≠ stage-close PASS/BLOCK」分歧（用户最初指出的 resubmit 死循环诱因之一）。

## 2. 目标

让 submit 预检喂入与 stage-close **同口径**的 `asset_types` + `expected_techniques`，消除分歧。**gray-switch** 包裹，可一键回退。

## 3. 设计

### 3.1 共享期望技术派生（消重）
`execute.rs::gate_expected_techniques`（私有）的逻辑 = `AssetClass::from_target_type` + `DefaultSprintContractGenerator::expected_techniques_for`。抽成 golish-agent-kit **pub** 函数 `sprint_contract::expected_techniques_for_target_types(stage, &[String]) -> Option<Vec<String>>`，stage-close 与 submit 预检**共用同一函数**（口径一致的根本保证）。`execute.rs::gate_expected_techniques` 改为薄委托（行为不变，既有测试零回归）。

### 3.2 扩 submit 预检 repo seam
`EvidenceLedgerQuery` 加两个默认空方法：
- `in_scope_typed_assets(org_id) -> Vec<(String,String)>`（喂 asset_types）
- `in_scope_target_types(org_id) -> Vec<String>`（喂 expected_techniques）

真实 impl（`db_bridge/evidence.rs::GolishDbRepoProvider`）委托既有 `in_scope_typed_assets_impl` / `in_scope_target_types_impl`。默认空 ⇒ 测试桩/无 DB 路径退回旧行为。

### 3.3 gate_context 接线（可测）
`gate_context(stage: StageKind, authoritative: bool)`：`authoritative` 由调用方从灰度开关求值后传入（**参数化便于确定性单测**，不在函数内读 env）。`authoritative=true` 且 org 绑定时：`typed_assets = in_scope_typed_assets`、`expected_techniques = expected_techniques_for_target_types(stage, target_types)`；否则两者保持空/None（= 旧行为）。统一走 `GateContextBuilder`。

**预检不做 subsidiary-inject**（scoping `--include-subsidiaries` 的细分维度需 engagement threshold，预检 seam 不持有；authoritative stage-close 仍强制，预检漏报该维只是少一个早期提示，close 兜底）。

### 3.4 灰度开关
`harness::feature_flags::submit_preview_authoritative_context_enabled()`（env `GOLISH_SUBMIT_PREVIEW_AUTHORITATIVE_CONTEXT`，**默认开**，`=0`/`false` 关）。默认开理由：本改动让预检**对齐已是权威的 stage-close**（非新增/收紧独立 gate），且每字段查询失败 fail-safe 退空 → 安全；保留 env kill-switch 以便线上回退。

## 4. 验证

- 新增单测：`expected_techniques_for_target_types`（空→None、域名类型→Some 含 intel 技术）；feature flag 纯函数（默认开 / `0`·`false` 关 / 其余开）；`gate_context(stage,false)` vs `(stage,true)` 在同一返回 typed/target 的 mock 下 → 前者 asset_types/expected_techniques 均 None、后者均 Some（两分支确定性）。
- `cargo nextest -p golish-agent-kit -p golish-agent-app` 零回归（含既有 `target_intel_*` 预检测）。
- `cargo clippy … -D warnings` 零告警。

## 5. 风险 / 回滚

- 默认开 = 下次构建即生效；env `GOLISH_SUBMIT_PREVIEW_AUTHORITATIVE_CONTEXT=0` 一键回退。
- 预检多 2 条 org 隔离查询（typed assets / target types），均与 stage-close 同款、有索引；fail-safe 退空。
- 预检 verdict 现对齐 close：原先「预检假 PASS」→ 现按 close 口径，给 agent 更准的早期 needs_fix；不影响 close 的权威裁决。
