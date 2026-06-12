# Phase 4：前端多资产 / org 树重设计

> 日期：2026-06-12
> 状态：设计（设计级 · 最粗 · 跟随数据模型，最后做）。总纲见 `2026-06-12-redteam-db-truth-master.md`。
> 不变量：AGENTS.md I5（跨 IPC 类型 ts-rs 同步，不手写两份）、M2（前端调 Tauri 走 `frontend/lib/api/<domain>.ts`，禁裸 invoke）。

---

## 1. 问题

Phase 2/3 把数据模型从「单 org / 单资产」升级为「engagement = 一棵 org 树（母 + 子）× 每 org 多资产 × 多技术 coverage」。现有前端是围绕单 org / 单 target 的视图设计，无法表达：org 树、逐 org 的 coverage 进度、engagement 级聚合状态。**UI 必须跟着数据模型走——所以放最后**。

## 2. 目标 / 非目标

**目标**：让用户能在前端：① 看/编辑 engagement 的 org 树（母 + 子，含投资比例、纳入/排除）；② 看每个 org 的 coverage 进度（哪格 found/checked_empty/缺）；③ 看 engagement 级聚合（整棵树的完整性）；④ 在 scoping 审批子公司纳入清单。

**非目标**：本文档不定具体组件树/像素稿（设计级）。等 Phase 2/3 的后端类型（ts-rs）定型后再出实现计划 + UI 细节。

## 3. 设计方向（轮廓）

### 3.1 数据契约先行（I5）

所有新结构（org 树节点、engagement 状态、per-org coverage 矩阵）用 `#[derive(ts_rs::TS)]` 在后端定义，导出到 `frontend/lib/generated/`。**禁止前端手写镜像类型**（历史债已多，见 agent-progress 多处 I5 提醒）。

### 3.2 视图层次

- **Engagement 视图**（新顶层）：一棵 org 树 + engagement 级 coverage 汇总（N 个 org，各自 PASS/BLOCK/进行中）。
- **Org 节点**：母/子标识 + 投资比例 + 该 org 的 6 类被动 + 主动 coverage 矩阵（复用现有 coverage 矩阵组件，按 org 实例化）。
- **Scoping 审批**：子公司发现结果列表（含投资比例），用户勾选纳入/排除（接 `human_approval: scope_expansion`）。

### 3.3 复用与改造

- 现有 organization recon UI / target-surface-workbench（feature_list `target-surface-workbench` 当前 blocked）是天然落点——把它从「单 org」扩成「org 树」。
- coverage 矩阵渲染组件按 org 复用（多实例）。
- 走 `frontend/lib/api/<domain>.ts` wrapper（M2），不裸 invoke。

## 4. 影响面（设计级，待 Phase 2/3 类型定型后细化）

| 区域 | 改动 |
|---|---|
| 后端 ts-rs 类型 | org 树节点 / engagement 状态 / per-org coverage（新 `#[derive(TS)]`） |
| `frontend/lib/generated/` | 由 ts-rs 生成（不手写） |
| `frontend/lib/api/<domain>.ts` | engagement / org 树 / coverage 查询 wrapper |
| 前端组件 | Engagement 视图 + org 树 + per-org coverage + scoping 审批 |
| i18n | 新文案 en/zh-CN |

## 5. 风险 / 依赖

- **硬依赖 Phase 3** 的后端数据模型定型——在那之前不要动前端（否则类型反复改）。
- 大 org 树（100 公司 × 子）的前端性能（虚拟化/分页）。
- 三态 UI（loading/error/empty）每条异步路径都要画（项目规约）。

## 6. 验证（DoD 雏形）

- ts-rs 绑定无漂移（`just check-types`）。
- `just check-fe` + `just test-fe` 绿；关键路径 Playwright E2E。
- 人工：建一个母+2 子的 engagement → 前端能看到 org 树 + 各 org coverage + engagement 聚合 + scoping 勾选纳入。

> 本 Phase 是整条线的收尾。后端（Phase 0-3）可信且数据模型稳定后，UI 才有可靠的事实来源去渲染——重申总纲铁律：先准后量，UI 跟数据走。
