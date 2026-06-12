# Phase 3：多 org coverage 轴（母 + 子逐个为单元）

> 日期：2026-06-12
> 状态：设计（设计级，待 Phase 2 落地后细化）。总纲见 `2026-06-12-redteam-db-truth-master.md`。
> 关联：`2026-06-10-coverage-asset-scope-isolation.md`（coverage 按 organization_id 隔离——本 Phase 的地基）。

---

## 1. 问题

Phase 2 把 org 树（母 + 合格子）建出来后，target_intel / EAS / enumeration 需要**对每个 org 都完整收集**，而不是只收母公司。当前 coverage 轴是「资产 × 技术」，且单次 run 绑一个 `harness_org_id`（06-10）。要支持「母先收 → 分发给子公司逐个收」，coverage 轴要升到「**org × 资产 × 技术**」，调度要能逐 org 推进。

## 2. 目标 / 非目标

**目标**：让一次 engagement 覆盖整棵 org 树——每个 org（母 + 每个子）作为一个独立的信息收集单元，各自走完整的 6 类被动 + 主动技术 coverage；gate 的完整性判定覆盖所有 org，漏掉任何一个 org 的任何一格 = BLOCK。

**非目标**：
- 不改 Phase 0 的 found 判定逻辑（DB 真值权威对每个 org 自动生效——coverage_truth 已按 org_id 查）。
- 不做前端（Phase 4）。
- 不重做调度框架——尽量复用现有 sub-agent 分发 + org 隔离。

## 3. 设计（两种粒度，择一/渐进）

### 3.1 方案 A（推荐起步）：逐 org 串行/并行跑同一 stage

把「一次 stage run 绑一个 org」升级为「engagement 含 N 个 org，对每个 org 跑一遍 stage 链」：
- orchestrator 从 `organizations`（Phase 2 落的 org 树）取 org 列表；对每个 org 设 `harness_org_id` 跑 target_intel→EAS→enumeration。
- coverage gate 天然按 org 隔离（06-10 已实现：`in_scope_assets` 按 `organization_id` 过滤）——每个 org 的 gate 只看自己的资产 × 技术。
- 「母先 → 子」= org 列表排序（母在前），可串行（稳）或受控并行（快，但注意 provider 限流）。
- engagement 级 PASS = 所有 org 各自 PASS。

优点：复用 06-10 的 org 隔离，改动小（主要在调度层 + engagement 级聚合）。缺点：N 个 org = N 趟 stage，编排/进度需要 engagement 级视图。

### 3.2 方案 B（更彻底）：coverage 轴显式升 org×资产×技术

把 coverage 矩阵的资产轴显式扩成 `(org_id, asset)` 二元组，一趟 run 内 gate 遍历所有 org 的所有资产：
- `GateContext.in_scope_assets` 从 `Vec<String>` 升为 `Vec<(org_id, asset)>`（或注入分组）。
- `coverage_complete` 遍历 `(org, asset, technique)`，缺任一 = gap。
- coverage_truth 一次查所有 org。

优点：一趟 run 给 engagement 级完整性判定。缺点：动 coverage 引擎核心数据结构（`in_scope_assets` 签名、注入链路、所有调用点），影响面大、回归风险高。

**推荐**：先方案 A（调度层迭代 org，gate 不动），验稳后若需要 engagement 级单趟判定再评估方案 B。

## 4. 「母先收 → 分发子公司」的实现取向

- 母公司 target_intel 先跑（org 列表母在前）。
- 子公司收集可由**子 agent 分发**承担：每个子公司一个子任务（sub_agent_enricher/pentester），主控聚合。注意 `sub_agent_models` 覆盖在 stage-run 下当前失效（agent-progress 2026-06-12「发现 1」记录的 registry wiring bug）——多 org 重 agent 跑前需修，否则全压主模型 + provider 限流。
- 每个子公司的资产（根域名）来自 Phase 2 落的 `targets(organization_id=子org, scope='in')`。

## 5. 影响面（设计级）

| 区域 | 改动 | 风险 |
|---|---|---|
| orchestrator / stage_run | engagement 含多 org；逐 org 设 `harness_org_id` 跑 stage 链 + engagement 级聚合 | 中 |
| coverage gate（方案 B 才动） | `in_scope_assets` 升 `(org,asset)`（方案 A 不动） | 高（方案 B），低（方案 A） |
| `coverage_truth.rs` | 已按 org_id 查，方案 A 无需改；方案 B 需批量多 org | 低/中 |
| sub_agent_models registry | 修 stage-run 下 override 失效（前置 bug，agent-progress 已记） | 中 |
| 进度/报告 | engagement 级（多 org）进度视图 | 中（连 Phase 4） |

## 6. 红线 / 验证 / 风险

- 红线同总纲 §8（org 隔离不串、findings 永空、Phase 0 DB 真值对每 org 生效）。
- 风险：① org 数 × 资产数爆炸 → provider 限流（kimi 那次 NVIDIA 429 的教训）+ 跑时长；建议串行或低并发 + 强模型/付费档。② sub_agent_models 失效必须先修。③ 历史 organization_id=NULL 的脏数据按 org 过滤后不进分母（06-10 已知，可接受）。
- 验证：活体 `engagement = 母+2 子` → 每个 org 各自 target_intel 完整性 gate 生效；漏一个子 org 的一格 → engagement BLOCK；org 隔离不互相投影。

## 7. 依赖

Phase 2（org 树落库）是硬前置；Phase 0（每 org 的 found 都靠 DB 真值）保证扩 org 后可信性自动继承。Phase 4（前端）跟在本 Phase 之后。
