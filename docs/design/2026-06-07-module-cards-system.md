# 模块卡体系（Agent-Readable Workspace）

> 2026-06-07 · 来源：用户在 BaJie MCP-agent-2 会话提出「给每个细小模块写 AI 可读的 MD 介绍」，参考 [Learn Harness Engineering](https://walkinglabs.github.io/learn-harness-engineering/en/) 的 **Project 2: Agent-Readable Workspace**。

## 目标

给这个大仓库（50 个 Rust crate + 前端多子系统）补上 harness 缺失的一层：**模块级 context 卡片**。让 AI agent 动某个模块前能一读就懂它的职责、接口、依赖、坑，而不必每次重读源码。

## 为什么（对齐文章逻辑）

harness engineering 的 Project 2 就是「把仓库结构改造得对 AI 友好 + 建立交接机制」。本仓库**已有**最小包（AGENTS.md / feature_list.json / agent-progress.md / init.sh），模块卡是补「进阶包：system-of-record 文档」那一层。文章最看重**闭环**：文档必须被规则引用，agent 才会真读、改完真回写，否则就是孤儿文档——所以本设计的核心不是「写一堆 md」，而是「把 md 接进 AGENTS.md 的工作流」。

## 决策

| 维度 | 决定 | 理由 |
|---|---|---|
| 粒度 | C 档：每个 crate 一张 + 每个**目录子模块**一张；单文件模块在所属卡的「关键文件」表带过 | 既详尽又不爆到 500+ 噪音卡 |
| 位置 | 集中式 `docs/modules/`，镜像 `backend/` `frontend/` 结构 | 可整树索引、对 AI 友好、与现有 `docs/` 一致、单文件模块不尴尬 |
| 索引 | 主索引 `docs/modules/INDEX.md`，按层分组的表，状态列 ✅/🚧/⬜ **兼当进度跟踪** | 文章强调的「导航入口」；省去额外跟踪文件 |
| 接线 | 已写进 AGENTS.md §1（开工读卡）/ §2.2 §2.3（改前读、改后更新）/ §2.4（卡住在 docs/modules、改模块同步更新卡）/ §4（收尾更新卡+索引） | 闭环：规则强制 agent 读卡 + 回写卡 |

## 卡片模板（固定字段）

每张卡固定包含：

1. **一句话职责**（blockquote）
2. **类型 / 路径 / 状态**
3. **何时该读这张卡**（给 AI 的触发提示）
4. **职责**（一段，说清楚边界）
5. **公开接口 / 关键类型**（表）
6. **依赖**（内部 crate + 关键外部）
7. **被谁依赖 / 改动影响面**
8. **子模块**（crate 卡才有，链到子卡）
9. **关键文件**（单文件模块一行带过）
10. **注意事项 / 坑**（含相关不变量 I1–I10、相关 design 链接）
11. **测试入口**（可跑的命令）

铁律：**所有内容必须源于实读源码（lib.rs / mod.rs / Cargo.toml + grep 依赖），禁止编造**。

## 子模块边界规则

- crate `src/` 下的**目录**（含 `mod.rs` + 多文件）= 真子系统 → 独立子卡
- 单文件 `xxx.rs` 模块 → 不单独成卡，进 crate 卡「关键文件」表
- `lib.rs` = crate 入口，不单独成卡

## 铺开计划（rollout）

总量约 **150–180 张**。分波次，每波更新 INDEX 状态列：

1. **Wave 0（已完成）**：模板打样 = `golish-tools` 全套（crate + file_ops/directory_ops/ast_grep/definitions）+ INDEX 起步 + AGENTS.md 接线 + 本设计文档。
2. **Wave 1**：50 个 crate 的 **crate 卡**（含各自子模块表）。优先序：基础层（platform/core/events/settings/db/models）→ 领域层（pentest/vuln/recon/scan）→ agent 层（agent-kit/runtime/bridge/sub-agents）→ app 层（*-app）→ 组合根（golish）→ rig forks。
3. **Wave 2**：各 crate 的**目录子模块卡**（同优先序）。
4. **Wave 3**：前端 `frontend/` 各子系统卡（components/hooks/lib/pages/services/store/styles）。

## 验证

- 每张卡的「测试入口」命令真实存在（`cargo nextest run -p <crate>` / `just test-fe`）。
- INDEX 状态列与实际文件一致。
- 抽查若干卡的「公开接口/依赖」与源码一致。
- 不跑 `just precommit` 改动代码部分（本任务只动 `docs/` + `AGENTS.md`，无代码/schema/IPC 变更）；提交由用户授权。

## 影响面 / 非目标

- 只新增 `docs/modules/**` 和改 `AGENTS.md`，**不动任何 Rust/TS 代码、schema、IPC**。
- 不替代 `docs/design/` `docs/superpowers/`（那是决策/计划记录）；模块卡是「模块是什么」的事实源。
