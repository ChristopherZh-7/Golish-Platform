# 架构体检 backlog（拆 / 合并 / 优化）· 分块执行计划

> 日期：2026-05-30 ｜ 状态：In progress ｜ 触发：用户「还有什么需要拆/合并/优化」→「全部搞完」
> 配套：`AGENTS.md`（§1.3 复杂/跨 crate/IPC 改动先写计划；§2.7 高风险先确认；I2/I4/I5 不变量）。
> **重要**：本 backlog 跨多个工作流、含行为变更与跨 crate 重构，**不能**一次性压成一个大 diff。按本计划**分块执行 + 每块独立验证 + 独立 commit**。

---

## 分块总览（按优先级）

| 块 | 主题 | 风险 | 是否需用户确认 | 状态 |
|---|---|---|---|---|
| P0-a | `apply_verdicts` ai_verdict UPDATE 加作用域守卫（行为保持） | 低 | 否（纯加固） | ✅ 本轮完成 |
| P0-b | `custom_rules_delete` / `sensitive_scan_confirm` 加 project_path 作用域 | 中（IPC 加参 + 前端） | 否（合法流行为保持） | ⏳ 待执行 |
| P0-c | `conv_delete` 加 project_path 作用域 | 中（caller 是 store 回调，需把 project_path 透传进去） | ⚠️ 需确认透传方式 | ⏳ 待执行 |
| P0-d | `recording_delete` 作用域 | 高（`recording_list` 当前**未**作用域，全项目可见；只改 delete 会造成「看得到删不掉」不一致） | ⚠️ **行为变更，需确认产品意图** | ⏳ 待确认 |
| P1-a | `ToolConfig` 4 处孪生收敛到 ts-rs 单源（I5） | 高（跨 crate，可能引入依赖环；shape 或已分叉） | ⚠️ **需先写设计文档 + 确认** | ⏳ 待设计 |
| P1-b | 前端手写镜像类型（如 `lib/pentest/types.ts`）→ ts-rs（I5） | 中 | 否 | ⏳ 待执行 |
| P2 | 超 500 行文件拆分（后端 `js_collect`/`capture/engine`/`pentest-domain/models`…；前端 `mocks.ts` 4135 等） | 低-中（纯重构，逐文件验证） | 否 | ⏳ 待执行 |

---

## 详细说明

### P0 · IDOR/I2 残余（by-id 写操作缺作用域守卫）
P0-3b 收口了项目作用域的 SELECT/DELETE；本块补「按 id 的 DELETE/UPDATE」。仅对**确认带 `project_path` 的项目作用域表**加守卫：
- `custom_passive_rules`（`custom_rules.rs:120`）、`conversations`（`conversation_store/mod.rs:129`）、`sensitive_scan_results`（`sensitive_scan.rs:269/316`）、`recordings`（`recordings.rs:163`）。
- 全局/KB 表（`vuln_kb_pocs` / `vuln_kb_links` / `vuln_feeds`，无 `project_path`）→ **不动**，by-id 正确。
- 范式：repo 加 `*_by_id_scoped` fn（`WHERE id=$1 AND project_path IS NOT DISTINCT FROM $2`）+ 单测；命令层加 `project_path` 参 + 前端 wrapper/caller 透传。
- **fork（需用户拍板）**：`recordings` 的 list 当前未作用域——是「整体接入项目作用域（list+delete 一起）」还是「保持全局」？

### P1 · 合并/去重（I5 单一真相源）
- `ToolConfig`：`golish-pentest`(本体) / `golish-pentest-domain` / `golish-pentest-mcp` / `golish-agent-kit::tool_definitions` 至少 4 份。**先写设计文档**确认：① 谁是 owner crate ② 依赖图是否会成环 ③ 各 shape 是否已分叉（如 `pentest_phase` 只在一份）。属 AGENTS.md §1.3 必须先设计的改动。
- 前端手写镜像类型 → 由 ts-rs 生成。

### P2 · 拆分（项目自带 500 行模块预算）
后端（非测试）：`tools/integrations/capture/engine.rs` 1483 · `tools/pentest_bridge/js_collect.rs` 1357 · `golish-pentest-domain/src/models.rs` 1310 · `golish-integrations/{storage/external_file.rs 998, schema.rs 888, resolver.rs 739}` · `golish-pipeline/.../steps/single.rs` 960 · `tools/organizations.rs` 766 · `ai/db_bridge.rs` 747。
前端：`mocks.ts` 4135 · `TargetSurfaceWorkbench.tsx` 818 · `Settings/ProviderSettings/index.tsx` 794 · `VulnIntelPanel/PocTab.tsx` 757 · `PipelinePanel.tsx` 746 · `TargetGroupedView.tsx` 743 · `DagComponents.tsx` 739 · `lib/conversation-db-sync.ts` 707。
范式：按职责抽子模块/子组件，行为零变更，逐文件 `cargo check`/`tsc`+`vitest` 验证。

---

## 验证 & 收尾
- 每块改完：相关 crate `cargo check` + `nextest`；前端 `tsc` + `vitest`。
- 全部块完成后跑一次 `just precommit`。
- 每块独立 commit；commit 前不混入跨块改动。
